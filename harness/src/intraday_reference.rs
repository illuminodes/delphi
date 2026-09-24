use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::price_series::PriceSeries;

const COINGECKO_RANGE_URL: &str =
    "https://api.coingecko.com/api/v3/coins/bitcoin/market_chart/range";

/// The free tier serves hourly data for ranges up to 90 days and rejects
/// anything wider, so long spans are fetched in chunks.
const MAX_RANGE_SECONDS: u32 = 80 * 86_400;

#[derive(Debug)]
pub enum IntradayError {
    Http(String),
    Malformed(String),
    Empty,
}

impl fmt::Display for IntradayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(message) => write!(formatter, "intraday fetch failed: {message}"),
            Self::Malformed(message) => write!(formatter, "intraday response malformed: {message}"),
            Self::Empty => formatter.write_str("intraday response held no price points"),
        }
    }
}

impl std::error::Error for IntradayError {}

/// Hourly (or finer) reference prices keyed by unix second, with linear
/// interpolation between the surrounding points.
pub struct IntradayReference {
    points: BTreeMap<u32, f64>,
    cache_path: PathBuf,
    dirty: bool,
}

impl IntradayReference {
    #[must_use]
    pub fn with_cache(path: impl Into<PathBuf>) -> Self {
        Self {
            points: BTreeMap::new(),
            cache_path: path.into(),
            dirty: false,
        }
    }

    pub fn load_cache(&mut self) {
        self.points = IntradayCache::read(&self.cache_path);
        self.dirty = false;
    }

    pub fn store_cache(&mut self) {
        if self.dirty {
            IntradayCache::write(&self.cache_path, &self.points);
            self.dirty = false;
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    #[must_use]
    pub fn span(&self) -> Option<(u32, u32)> {
        let first = *self.points.keys().next()?;
        let last = *self.points.keys().next_back()?;
        Some((first, last))
    }

    /// Whether the cache already covers `start..=stop` with a point no more
    /// than `tolerance` seconds outside each end.
    #[must_use]
    pub fn covers(&self, start: u32, stop: u32, tolerance: u32) -> bool {
        self.span().is_some_and(|(first, last)| {
            first <= start.saturating_add(tolerance) && last.saturating_add(tolerance) >= stop
        })
    }

    /// The reference price at `at`, interpolated between the two surrounding
    /// samples. Returns `None` outside the covered span.
    #[must_use]
    pub fn price_at(&self, at: u32) -> Option<f64> {
        if let Some(exact) = self.points.get(&at) {
            return Some(*exact);
        }
        let (&before_time, &before_price) = self.points.range(..=at).next_back()?;
        let (&after_time, &after_price) = self.points.range(at..).next()?;
        if after_time == before_time {
            return Some(before_price);
        }
        let span = f64::from(after_time - before_time);
        let offset = f64::from(at - before_time);
        Some(before_price + (after_price - before_price) * (offset / span))
    }

    /// The mean reference price across `start..=stop`, sampled at every cached
    /// point inside the window plus both interpolated edges.
    #[must_use]
    pub fn mean_over(&self, start: u32, stop: u32) -> Option<f64> {
        let mut total = self.price_at(start)? + self.price_at(stop)?;
        let mut count = 2.0_f64;
        for price in self.points.range(start..=stop).map(|(_, price)| *price) {
            total += price;
            count += 1.0;
        }
        Some(total / count)
    }

    /// [`Self::mean_over`] with the window clamped to the covered span, as
    /// long as neither end lies more than `tolerance` seconds outside it.
    #[must_use]
    pub fn mean_near(&self, start: u32, stop: u32, tolerance: u32) -> Option<f64> {
        if !self.covers(start, stop, tolerance) {
            return None;
        }
        let (first, last) = self.span()?;
        let clamped_start = start.clamp(first, last);
        let clamped_stop = stop.clamp(first, last);
        self.mean_over(clamped_start, clamped_stop.max(clamped_start))
    }

    /// Fetch `start..=stop` from `CoinGecko` unless the cache already covers it.
    ///
    /// # Errors
    /// Returns [`IntradayError`] if the request fails or the body is malformed.
    pub fn ensure_span(&mut self, start: u32, stop: u32) -> Result<(), IntradayError> {
        if self.covers(start, stop, 7_200) {
            return Ok(());
        }
        let mut absorbed = 0;
        let mut chunk_start = start;
        while chunk_start < stop {
            let chunk_stop = chunk_start.saturating_add(MAX_RANGE_SECONDS).min(stop);
            let body = Self::fetch(u64::from(chunk_start), u64::from(chunk_stop))?;
            let pairs = PriceSeries::parse(&body)
                .map_err(|error| IntradayError::Malformed(error.to_string()))?;
            for (milliseconds, price) in pairs {
                if let Ok(seconds) = u32::try_from(milliseconds / 1000) {
                    self.points.insert(seconds, price);
                    absorbed += 1;
                }
            }
            if chunk_stop == stop {
                break;
            }
            chunk_start = chunk_stop;
        }
        if absorbed == 0 {
            return Err(IntradayError::Empty);
        }
        self.dirty = true;
        Ok(())
    }

    fn fetch(from: u64, to: u64) -> Result<String, IntradayError> {
        let url = format!("{COINGECKO_RANGE_URL}?vs_currency=usd&from={from}&to={to}");
        let agent = ureq::Agent::new_with_defaults();
        let mut response = agent
            .get(&url)
            .call()
            .map_err(|error| IntradayError::Http(error.to_string()))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| IntradayError::Http(error.to_string()))
    }
}

struct IntradayCache;

impl IntradayCache {
    fn read(path: &Path) -> BTreeMap<u32, f64> {
        let Ok(body) = std::fs::read_to_string(path) else {
            return BTreeMap::new();
        };
        body.lines().filter_map(Self::parse_line).collect()
    }

    fn parse_line(line: &str) -> Option<(u32, f64)> {
        let (time, price) = line.split_once(' ')?;
        Some((time.trim().parse().ok()?, price.trim().parse().ok()?))
    }

    fn write(path: &Path, points: &BTreeMap<u32, f64>) {
        use std::fmt::Write;
        let mut body = String::new();
        for (time, price) in points {
            let _ = writeln!(body, "{time} {price}");
        }
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, body)
            .and_then(|()| std::fs::rename(&temporary, path))
            .is_err()
        {
            eprintln!("warning: could not write intraday cache {}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{IntradayCache, IntradayReference};

    struct Fixture;

    impl Fixture {
        fn hourly() -> IntradayReference {
            let mut reference = IntradayReference::with_cache("/nonexistent/intraday");
            reference.points.insert(1_000, 100.0);
            reference.points.insert(2_000, 200.0);
            reference.points.insert(3_000, 300.0);
            reference
        }

        fn temp_path(name: &str) -> PathBuf {
            std::env::temp_dir()
                .join(format!("mantle-intraday-{}-{name}.txt", std::process::id()))
        }
    }

    #[test]
    fn returns_exact_points_directly() {
        assert_eq!(Fixture::hourly().price_at(2_000), Some(200.0));
    }

    #[test]
    fn interpolates_between_surrounding_points() {
        assert_eq!(Fixture::hourly().price_at(1_500), Some(150.0));
        assert_eq!(Fixture::hourly().price_at(2_750), Some(275.0));
    }

    #[test]
    fn refuses_to_extrapolate_outside_the_span() {
        let reference = Fixture::hourly();
        assert_eq!(reference.price_at(500), None);
        assert_eq!(reference.price_at(4_000), None);
    }

    #[test]
    fn mean_averages_edges_and_interior_points() {
        assert_eq!(Fixture::hourly().mean_over(1_000, 3_000), Some(200.0));
    }

    #[test]
    fn mean_is_none_when_the_window_leaves_the_span() {
        assert_eq!(Fixture::hourly().mean_over(500, 3_000), None);
    }

    #[test]
    fn mean_near_clamps_a_window_just_past_the_span() {
        let reference = Fixture::hourly();
        let mean = reference.mean_near(2_900, 3_200, 300).expect("within tolerance");
        assert!((mean - 890.0 / 3.0).abs() < 1e-9, "{mean}");
        assert_eq!(reference.mean_near(3_100, 3_200, 300), Some(300.0));
        assert_eq!(reference.mean_near(3_100, 3_500, 300), None);
    }

    #[test]
    fn reports_coverage_within_tolerance() {
        let reference = Fixture::hourly();
        assert!(reference.covers(1_500, 2_500, 0));
        assert!(reference.covers(900, 3_050, 100));
        assert!(!reference.covers(500, 3_000, 100));
    }

    #[test]
    fn span_reports_first_and_last_points() {
        assert_eq!(Fixture::hourly().span(), Some((1_000, 3_000)));
        assert_eq!(IntradayReference::with_cache("/none").span(), None);
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let path = Fixture::temp_path("roundtrip");
        let mut points = std::collections::BTreeMap::new();
        points.insert(1_788_048_000_u32, 78_225.13);
        points.insert(1_788_051_600_u32, 78_184.08);
        IntradayCache::write(&path, &points);
        assert_eq!(IntradayCache::read(&path), points);
        let _ = std::fs::remove_file(&path);
    }
}
