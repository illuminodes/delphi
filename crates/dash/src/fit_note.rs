use std::fmt;

use nostr_minions::NostrNote;
use serde::Deserialize;

use crate::NoteKind;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ScoreCurveWire {
    pub first_price: Option<f64>,
    pub step: Option<f64>,
    #[serde(default)]
    pub scores: Vec<Option<f64>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ImpliedWire {
    pub low: Option<f64>,
    pub high: Option<f64>,
    #[serde(default)]
    pub counts: Vec<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Denomination {
    pub usd: u32,
    #[serde(default)]
    pub points: usize,
    #[serde(default)]
    pub inside: usize,
    pub median: Option<f64>,
}

impl Denomination {
    /// The median's distance from `price`, in percent.
    #[must_use]
    pub fn offset_percent(&self, price: f64) -> Option<f64> {
        let median = self.median?;
        (price > 0.0).then(|| (median - price) / price * 100.0)
    }
}

#[derive(Debug, Deserialize)]
struct FitWire {
    source: Option<String>,
    price: Option<f64>,
    rough_price: Option<f64>,
    deviation: Option<f64>,
    samples: Option<usize>,
    stop_height: Option<u32>,
    stop_time: Option<i64>,
    band_low: Option<f64>,
    band_high: Option<f64>,
    octave_ratio: Option<f64>,
    curve: Option<ScoreCurveWire>,
    implied: Option<ImpliedWire>,
    #[serde(default)]
    denominations: Vec<Denomination>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FitNoteError {
    WrongKind(u32),
    NotAFit,
    Content(String),
    MissingField(&'static str),
}

impl fmt::Display for FitNoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind(kind) => write!(formatter, "kind {kind} is not a fit note"),
            Self::NotAFit => formatter.write_str("w tag is not a fit window"),
            Self::Content(message) => write!(formatter, "content is not a fit: {message}"),
            Self::MissingField(name) => write!(formatter, "content lacks {name}"),
        }
    }
}

impl std::error::Error for FitNoteError {}

/// The evidence behind one window's price: the rough stage's score at each
/// candidate price, the round-USD implied prices near the result, and how
/// each round amount agrees with it.
#[derive(Debug, Clone, PartialEq)]
pub struct FitNote {
    window: String,
    source: String,
    created_at: i64,
    price: f64,
    rough_price: f64,
    deviation: Option<f64>,
    samples: usize,
    stop_height: Option<u32>,
    stop_time: Option<i64>,
    band: Option<(f64, f64)>,
    octave_ratio: Option<f64>,
    curve: Vec<(f64, f64)>,
    implied_low: f64,
    implied_high: f64,
    implied_counts: Vec<u32>,
    denominations: Vec<Denomination>,
}

impl FitNote {
    pub const PREFIX: &'static str = "fit-";
    /// The windows the oracle fits.
    pub const WINDOWS: [&'static str; 3] = ["mempool", "chain-1h", "chain-1d"];

    #[must_use]
    pub fn is_fit_window(window: &str) -> bool {
        window.starts_with(Self::PREFIX)
    }

    pub fn from_note(note: &NostrNote) -> Result<Self, FitNoteError> {
        if !NoteKind::carries_live(note.kind) {
            return Err(FitNoteError::WrongKind(note.kind));
        }
        let window = NoteKind::window(note)
            .and_then(|tag| tag.strip_prefix(Self::PREFIX))
            .filter(|window| !window.is_empty())
            .ok_or(FitNoteError::NotAFit)?
            .to_string();
        let wire: FitWire =
            serde_json::from_str(&note.content).map_err(|error| FitNoteError::Content(error.to_string()))?;
        let price = Self::positive(wire.price).ok_or(FitNoteError::MissingField("price"))?;
        let curve = Self::curve_points(wire.curve.as_ref()).ok_or(FitNoteError::MissingField("curve"))?;
        let implied = wire.implied.ok_or(FitNoteError::MissingField("implied"))?;
        let implied_low = Self::positive(implied.low).ok_or(FitNoteError::MissingField("implied.low"))?;
        let implied_high = Self::positive(implied.high)
            .filter(|high| *high > implied_low)
            .ok_or(FitNoteError::MissingField("implied.high"))?;
        Ok(Self {
            window,
            source: wire.source.unwrap_or_default(),
            created_at: note.created_at,
            price,
            rough_price: Self::positive(wire.rough_price).unwrap_or(price),
            deviation: wire.deviation.filter(|value| value.is_finite()),
            samples: wire.samples.unwrap_or(0),
            stop_height: wire.stop_height,
            stop_time: wire.stop_time,
            band: Self::positive(wire.band_low).zip(Self::positive(wire.band_high)),
            octave_ratio: wire.octave_ratio.filter(|value| value.is_finite()),
            curve,
            implied_low,
            implied_high,
            implied_counts: implied.counts,
            denominations: wire.denominations,
        })
    }

    fn positive(value: Option<f64>) -> Option<f64> {
        value.filter(|value| value.is_finite() && *value > 0.0)
    }

    /// Candidate prices rebuilt from the first price and the ratio between
    /// neighbours, paired with their scores and sorted by rising price.
    fn curve_points(wire: Option<&ScoreCurveWire>) -> Option<Vec<(f64, f64)>> {
        let wire = wire?;
        let first = Self::positive(wire.first_price)?;
        let step = Self::positive(wire.step)?;
        let mut price = first;
        let mut points = Vec::with_capacity(wire.scores.len());
        for score in &wire.scores {
            if let Some(score) = score.filter(|score| score.is_finite()) {
                points.push((price, score));
            }
            price *= step;
        }
        points.sort_by(|left, right| left.0.total_cmp(&right.0));
        (points.len() >= 2).then_some(points)
    }

    #[must_use]
    pub fn window(&self) -> &str {
        &self.window
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub const fn created_at(&self) -> i64 {
        self.created_at
    }

    #[must_use]
    pub const fn price(&self) -> f64 {
        self.price
    }

    #[must_use]
    pub const fn rough_price(&self) -> f64 {
        self.rough_price
    }

    #[must_use]
    pub const fn deviation(&self) -> Option<f64> {
        self.deviation
    }

    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }

    #[must_use]
    pub const fn stop_height(&self) -> Option<u32> {
        self.stop_height
    }

    #[must_use]
    pub const fn stop_time(&self) -> Option<i64> {
        self.stop_time
    }

    #[must_use]
    pub const fn band(&self) -> Option<(f64, f64)> {
        self.band
    }

    #[must_use]
    pub const fn octave_ratio(&self) -> Option<f64> {
        self.octave_ratio
    }

    #[must_use]
    pub fn curve(&self) -> &[(f64, f64)] {
        &self.curve
    }

    #[must_use]
    pub const fn implied_range(&self) -> (f64, f64) {
        (self.implied_low, self.implied_high)
    }

    #[must_use]
    pub fn implied_counts(&self) -> &[u32] {
        &self.implied_counts
    }

    #[must_use]
    pub fn implied_total(&self) -> u32 {
        self.implied_counts.iter().sum()
    }

    #[must_use]
    pub fn denominations(&self) -> &[Denomination] {
        &self.denominations
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::{FitNote, FitNoteError};
    use crate::NoteKind;

    struct FitFixture;

    impl FitFixture {
        const CONTENT: &'static str = r#"{"window":"fit-mempool","source":"mempool","price":80000.00,"rough_price":80400.00,"deviation":0.1500,"samples":420,"stop_height":900,"stop_time":1000,"band_low":70000.00,"band_high":92000.00,"octave_ratio":0.8800,"curve":{"first_price":160000.00,"step":0.98855309,"scores":[0.100,1.000,null,0.400]},"implied":{"low":64000.00,"high":100000.00,"counts":[0,3,9,2]},"denominations":[{"usd":100,"points":40,"inside":31,"median":80800.00},{"usd":5,"points":0,"inside":0,"median":null}]}"#;

        fn note(tag: &str, content: &str) -> NostrNote {
            let mut note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .content(content)
                .tag("w", tag)
                .build();
            note.created_at = 50;
            note
        }
    }

    #[test]
    fn a_fit_note_names_its_price_window() {
        let fit = FitNote::from_note(&FitFixture::note("fit-mempool", FitFixture::CONTENT)).expect("fit");

        assert_eq!(fit.window(), "mempool");
        assert_eq!(fit.source(), "mempool");
        assert_eq!(fit.band(), Some((70_000.0, 92_000.0)));
        assert_eq!(fit.octave_ratio(), Some(0.88));
        assert_eq!(fit.implied_total(), 14);
    }

    #[test]
    fn the_curve_is_rebuilt_in_rising_price_order_without_gaps() {
        let fit = FitNote::from_note(&FitFixture::note("fit-mempool", FitFixture::CONTENT)).expect("fit");
        let prices: Vec<f64> = fit.curve().iter().map(|point| point.0).collect();

        assert_eq!(prices.len(), 3);
        assert!(prices.windows(2).all(|pair| pair[0] < pair[1]));
        assert!((prices[2] - 160_000.0).abs() < 1e-6);
        assert!((fit.curve()[1].1 - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_denomination_reports_its_offset_from_the_price() {
        let fit = FitNote::from_note(&FitFixture::note("fit-mempool", FitFixture::CONTENT)).expect("fit");
        let hundred = fit.denominations()[0];

        assert!((hundred.offset_percent(fit.price()).expect("offset") - 1.0).abs() < 1e-9);
        assert_eq!(fit.denominations()[1].offset_percent(fit.price()), None);
    }

    #[test]
    fn a_price_window_is_not_a_fit() {
        assert_eq!(
            FitNote::from_note(&FitFixture::note("mempool", FitFixture::CONTENT)),
            Err(FitNoteError::NotAFit)
        );
        assert!(FitNote::is_fit_window("fit-chain-1h"));
        assert!(!FitNote::is_fit_window("chain-1h"));
    }

    #[test]
    fn a_fit_without_a_curve_is_refused() {
        let note = FitFixture::note("fit-chain-1h", r#"{"price":1.0,"implied":{"low":1,"high":2,"counts":[]}}"#);

        assert_eq!(FitNote::from_note(&note), Err(FitNoteError::MissingField("curve")));
    }

    #[test]
    fn a_null_band_is_absent() {
        let content = FitFixture::CONTENT.replace(r#""band_low":70000.00"#, r#""band_low":null"#);
        let fit = FitNote::from_note(&FitFixture::note("fit-chain-1d", &content)).expect("fit");

        assert_eq!(fit.band(), None);
    }
}
