use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use mantle::difficulty::BlockTime;
use mantle::network::Network;
use mantle_oracle::UtcDate;

use crate::price_series::PriceSeries;

const COINGECKO_RANGE_URL: &str = "https://api.coingecko.com/api/v3/coins/bitcoin/market_chart/range";
const MS_PER_DAY: u64 = 86_400_000;

#[derive(Debug)]
pub enum ReferenceError {
    UnsupportedNetwork,
    Http(String),
    Malformed(String),
    MissingSnapshot(UtcDate),
}

impl fmt::Display for ReferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedNetwork => formatter.write_str("reference prices only exist for mainnet"),
            Self::Http(message) => write!(formatter, "reference fetch failed: {message}"),
            Self::Malformed(message) => write!(formatter, "reference response malformed: {message}"),
            Self::MissingSnapshot(date) => write!(formatter, "no reference snapshot for {date}"),
        }
    }
}

impl std::error::Error for ReferenceError {}

pub struct ReferencePrices {
    agent: ureq::Agent,
    cache_path: PathBuf,
    snapshots: BTreeMap<String, f64>,
    cache_dirty: bool,
}

impl ReferencePrices {
    #[must_use]
    pub fn with_cache(path: impl Into<PathBuf>) -> Self {
        Self {
            agent: ureq::Agent::new_with_defaults(),
            cache_path: path.into(),
            snapshots: BTreeMap::new(),
            cache_dirty: false,
        }
    }

    pub fn load_cache(&mut self) {
        self.snapshots = ReferenceCache::read(&self.cache_path);
        self.cache_dirty = false;
    }

    pub fn store_cache(&mut self) {
        if !self.cache_dirty {
            return;
        }
        ReferenceCache::write(&self.cache_path, &self.snapshots);
        self.cache_dirty = false;
    }

    #[must_use]
    pub fn snapshot(&self, date: UtcDate) -> Option<f64> {
        self.snapshots.get(&date.to_string()).copied()
    }

    #[must_use]
    pub fn day_average(&self, date: UtcDate) -> Option<f64> {
        let open = self.snapshot(date)?;
        let close = self.snapshot(date.next())?;
        Some(f64::midpoint(open, close))
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn ensure_daily_snapshots(
        &mut self,
        dates: &[UtcDate],
        network: Network,
    ) -> Result<(), ReferenceError> {
        if network != Network::Mainnet {
            return Err(ReferenceError::UnsupportedNetwork);
        }
        let needed = Self::required_snapshots(dates);
        if needed.iter().all(|date| self.snapshot(*date).is_some()) {
            return Ok(());
        }
        let from = needed
            .first()
            .and_then(|date| date.midnight().ok())
            .map(|time| u64::from(time.to_u32()))
            .unwrap_or_default();
        let to = needed
            .last()
            .and_then(|date| date.midnight().ok())
            .map(|time| u64::from(time.to_u32()))
            .map_or(from, |last| last + 86_400)
            .min(Self::now_seconds().saturating_sub(1));
        let body = self.fetch_range(from, to)?;
        self.absorb_range_body(&body);
        self.cache_dirty = true;
        for date in needed {
            if self.snapshot(date).is_none() {
                return Err(ReferenceError::MissingSnapshot(date));
            }
        }
        Ok(())
    }

    fn required_snapshots(dates: &[UtcDate]) -> BTreeSet<UtcDate> {
        let mut needed = BTreeSet::new();
        for date in dates {
            needed.insert(*date);
            needed.insert(date.next());
        }
        needed
    }

    fn now_seconds() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default()
    }

    fn fetch_range(&self, from: u64, to: u64) -> Result<String, ReferenceError> {
        let url = format!("{COINGECKO_RANGE_URL}?vs_currency=usd&from={from}&to={to}");
        let mut response = self
            .agent
            .get(&url)
            .call()
            .map_err(|error| ReferenceError::Http(error.to_string()))?;
        response
            .body_mut()
            .read_to_string()
            .map_err(|error| ReferenceError::Http(error.to_string()))
    }

    fn absorb_range_body(&mut self, body: &str) {
        let Ok(pairs) = PriceSeries::parse(body) else {
            return;
        };

        for (milliseconds, price) in pairs {
            if milliseconds % MS_PER_DAY != 0 {
                continue;
            }
            let Ok(seconds) = u32::try_from(milliseconds / 1000) else {
                continue;
            };
            let date = UtcDate::containing(BlockTime::new(seconds));
            self.snapshots.insert(date.to_string(), price);
        }
    }
}

struct ReferenceCache;

impl ReferenceCache {
    fn read(path: &Path) -> BTreeMap<String, f64> {
        let Ok(body) = std::fs::read_to_string(path) else {
            return BTreeMap::new();
        };
        body.lines().filter_map(ReferenceCache::parse_line).collect()
    }

    fn parse_line(line: &str) -> Option<(String, f64)> {
        let (date, price) = line.split_once(' ')?;
        let price = price.trim().parse::<f64>().ok()?;
        Some((date.trim().to_string(), price))
    }

    fn write(path: &Path, snapshots: &BTreeMap<String, f64>) {
        use std::fmt::Write;
        let mut body = String::new();
        for (date, price) in snapshots {
            let _ = writeln!(body, "{date} {price}");
        }
        let temporary = path.with_extension("tmp");
        if std::fs::write(&temporary, body)
            .and_then(|()| std::fs::rename(&temporary, path))
            .is_err()
        {
            eprintln!("warning: could not write reference cache {}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::PathBuf;

    use mantle_oracle::UtcDate;

    use super::{ReferenceCache, ReferencePrices};

    struct TempCachePath;

    impl TempCachePath {
        fn unique(name: &str) -> PathBuf {
            std::env::temp_dir().join(format!(
                "mantle-reference-{}-{name}.txt",
                std::process::id()
            ))
        }
    }

    #[test]
    fn cache_round_trips_through_disk() {
        let path = TempCachePath::unique("roundtrip");
        let mut snapshots = std::collections::BTreeMap::new();
        snapshots.insert("2026-08-27".to_string(), 77_000.5);
        snapshots.insert("2026-08-28".to_string(), 77_370.25);
        ReferenceCache::write(&path, &snapshots);
        assert_eq!(ReferenceCache::read(&path), snapshots);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cache_read_skips_malformed_lines() {
        let path = TempCachePath::unique("malformed");
        let mut file = std::fs::File::create(&path).expect("create temp file");
        let _ = writeln!(file, "2026-08-27 77000.5");
        let _ = writeln!(file, "garbage");
        let _ = writeln!(file, "2026-08-28 not-a-number");
        let read = ReferenceCache::read(&path);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get("2026-08-27"), Some(&77_000.5));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn day_average_is_the_midpoint_of_surrounding_midnights() {
        let mut references = ReferencePrices::with_cache("/nonexistent/reference-cache");
        references.snapshots.insert("2026-08-27".to_string(), 76_000.0);
        references.snapshots.insert("2026-08-28".to_string(), 78_000.0);
        let date = UtcDate::parse("2026-08-27").expect("valid date");
        assert_eq!(references.day_average(date), Some(77_000.0));
        assert_eq!(references.day_average(date.next()), None);
    }
}
