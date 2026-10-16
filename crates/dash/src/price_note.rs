use std::fmt;

use nostr_minions::NostrNote;
use serde::Deserialize;

use crate::{NoteKind, SeriesPoint};

#[derive(Debug, Clone, PartialEq)]
pub struct ChainDetail {
    pub deviation: Option<f64>,
    pub stop_hash: Option<String>,
    pub start_height: u32,
    pub stop_height: u32,
    pub start_time: u32,
    pub stop_time: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MempoolDetail {
    pub transactions: usize,
    pub band_low: Option<f64>,
    pub band_high: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PriceOrigin {
    Chain(ChainDetail),
    Mempool(MempoolDetail),
}

#[derive(Debug, Clone, PartialEq)]
pub enum PriceNoteError {
    WrongKind(u32),
    MissingWindow,
    Content(String),
    MissingField(&'static str),
    UnknownSource(String),
}

impl fmt::Display for PriceNoteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind(kind) => write!(formatter, "kind {kind} is not a price note"),
            Self::MissingWindow => formatter.write_str("note has no w tag"),
            Self::Content(message) => write!(formatter, "content is not a price: {message}"),
            Self::MissingField(name) => write!(formatter, "content lacks {name}"),
            Self::UnknownSource(source) => write!(formatter, "unknown source {source}"),
        }
    }
}

impl std::error::Error for PriceNoteError {}

#[derive(Debug, Deserialize)]
struct PriceWire {
    source: Option<String>,
    price: Option<f64>,
    samples: Option<usize>,
    deviation: Option<f64>,
    start_height: Option<u32>,
    stop_height: Option<u32>,
    start_time: Option<u32>,
    stop_time: Option<u32>,
    transactions: Option<usize>,
    band_low: Option<f64>,
    band_high: Option<f64>,
    stop_hash: Option<String>,
}

/// One price: a stored audit note for a chain window, or a live mempool note.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceNote {
    window: String,
    price: f64,
    samples: usize,
    created_at: i64,
    origin: PriceOrigin,
    id: Option<String>,
    replaces: Option<String>,
}

impl PriceNote {
    pub fn from_note(note: &NostrNote) -> Result<Self, PriceNoteError> {
        if note.kind != NoteKind::AUDIT && note.kind != NoteKind::LIVE {
            return Err(PriceNoteError::WrongKind(note.kind));
        }
        let window = NoteKind::window(note)
            .ok_or(PriceNoteError::MissingWindow)?
            .to_string();
        let wire: PriceWire = serde_json::from_str(&note.content)
            .map_err(|error| PriceNoteError::Content(error.to_string()))?;
        let price = wire
            .price
            .filter(|price| price.is_finite() && *price > 0.0)
            .ok_or(PriceNoteError::MissingField("price"))?;
        let source = if note.kind == NoteKind::AUDIT {
            Some("chain")
        } else {
            wire.source.as_deref()
        };
        let origin = Self::origin_of(source, &wire)?;
        Ok(Self {
            window,
            price,
            samples: wire.samples.unwrap_or(0),
            created_at: note.created_at,
            origin,
            id: note.id.clone(),
            replaces: note.tags.find_tags_ref("e").first().map(|id| (*id).to_string()),
        })
    }

    fn origin_of(source: Option<&str>, wire: &PriceWire) -> Result<PriceOrigin, PriceNoteError> {
        match source {
            Some("chain") => Ok(PriceOrigin::Chain(ChainDetail {
                deviation: wire.deviation,
                stop_hash: wire.stop_hash.clone(),
                start_height: wire.start_height.ok_or(PriceNoteError::MissingField("start_height"))?,
                stop_height: wire.stop_height.ok_or(PriceNoteError::MissingField("stop_height"))?,
                start_time: wire.start_time.ok_or(PriceNoteError::MissingField("start_time"))?,
                stop_time: wire.stop_time.ok_or(PriceNoteError::MissingField("stop_time"))?,
            })),
            Some("mempool") => Ok(PriceOrigin::Mempool(MempoolDetail {
                transactions: wire.transactions.unwrap_or(0),
                band_low: wire.band_low,
                band_high: wire.band_high,
            })),
            Some(other) => Err(PriceNoteError::UnknownSource(other.to_string())),
            None => Err(PriceNoteError::MissingField("source")),
        }
    }

    #[must_use]
    pub fn window(&self) -> &str {
        &self.window
    }

    #[must_use]
    pub const fn price(&self) -> f64 {
        self.price
    }

    #[must_use]
    pub const fn samples(&self) -> usize {
        self.samples
    }

    #[must_use]
    pub const fn created_at(&self) -> i64 {
        self.created_at
    }

    #[must_use]
    pub const fn origin(&self) -> &PriceOrigin {
        &self.origin
    }

    #[must_use]
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// The audit note this one replaces after a reorg.
    #[must_use]
    pub fn replaces(&self) -> Option<&str> {
        self.replaces.as_deref()
    }

    /// Where this price sits in its window's history: the stop height for a
    /// chain window, the arrival time for the mempool.
    #[must_use]
    pub fn history_key(&self) -> i64 {
        match &self.origin {
            PriceOrigin::Chain(detail) => i64::from(detail.stop_height),
            PriceOrigin::Mempool(_) => self.created_at,
        }
    }

    /// This price as one point of its window's history.
    #[must_use]
    pub fn point(&self) -> SeriesPoint {
        match &self.origin {
            PriceOrigin::Chain(detail) => SeriesPoint::new(
                self.price,
                detail.deviation,
                self.samples,
                Some(detail.stop_height),
                i64::from(detail.stop_time),
            ),
            PriceOrigin::Mempool(_) => SeriesPoint::new(self.price, None, self.samples, None, self.created_at),
        }
    }

    #[must_use]
    pub fn band(&self) -> Option<(f64, f64)> {
        match &self.origin {
            PriceOrigin::Mempool(detail) => detail.band_low.zip(detail.band_high),
            PriceOrigin::Chain(_) => None,
        }
    }

    #[must_use]
    pub fn age_seconds(&self, now: i64) -> i64 {
        (now - self.created_at).max(0)
    }

    #[must_use]
    pub const fn stale_after_seconds(&self) -> i64 {
        match self.origin {
            PriceOrigin::Mempool(_) => 5 * 60,
            PriceOrigin::Chain(_) => 45 * 60,
        }
    }

    #[must_use]
    pub fn is_stale(&self, now: i64) -> bool {
        self.age_seconds(now) > self.stale_after_seconds()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::{PriceNote, PriceNoteError, PriceOrigin};
    use crate::NoteKind;

    struct NoteFixture;

    impl NoteFixture {
        fn note(kind: u32, window: Option<&str>, content: &str, created_at: i64) -> NostrNote {
            let mut builder = NostrNoteBuilder::new().kind(kind).content(content);
            if let Some(window) = window {
                builder = builder.tag("w", window);
            }
            let mut note = builder.build();
            note.created_at = created_at;
            note
        }

        fn chain(created_at: i64) -> NostrNote {
            Self::note(
                NoteKind::AUDIT,
                Some("chain-1h"),
                r#"{"window":"chain-1h","stop_hash":"00ab","price":83551.5,"samples":7336,"deviation":0.158,"start_height":900000,"stop_height":900005,"start_time":1000,"stop_time":4600}"#,
                created_at,
            )
        }

        fn mempool(created_at: i64) -> NostrNote {
            Self::note(
                NoteKind::LIVE,
                Some("mempool"),
                r#"{"window":"mempool","source":"mempool","price":83520.25,"samples":783,"transactions":401,"band_low":83400.0,"band_high":83640.5}"#,
                created_at,
            )
        }
    }

    #[test]
    fn a_chain_note_carries_its_heights_and_deviation() {
        let price = PriceNote::from_note(&NoteFixture::chain(100)).expect("chain note");

        assert_eq!(price.window(), "chain-1h");
        assert!((price.price() - 83_551.5).abs() < 1e-9);
        assert_eq!(price.samples(), 7_336);
        let PriceOrigin::Chain(detail) = price.origin() else {
            panic!("chain origin expected");
        };
        assert_eq!(detail.stop_height, 900_005);
        assert_eq!(detail.deviation, Some(0.158));
        assert_eq!(detail.stop_hash.as_deref(), Some("00ab"));
        assert_eq!(price.band(), None);
    }

    #[test]
    fn an_audit_note_is_a_point_at_its_stop_block() {
        let point = PriceNote::from_note(&NoteFixture::chain(100)).expect("chain note").point();

        assert_eq!(point.stop_time(), 4_600);
        assert_eq!(point.stop_height(), Some(900_005));
        assert!(point.is_plottable());
    }

    #[test]
    fn a_mempool_note_is_a_point_at_its_arrival() {
        let point = PriceNote::from_note(&NoteFixture::mempool(777)).expect("mempool").point();

        assert_eq!(point.stop_time(), 777);
        assert_eq!(point.stop_height(), None);
    }

    #[test]
    fn a_mempool_note_carries_its_band_and_transaction_count() {
        let price = PriceNote::from_note(&NoteFixture::mempool(100)).expect("mempool note");

        assert_eq!(price.band(), Some((83_400.0, 83_640.5)));
        let PriceOrigin::Mempool(detail) = price.origin() else {
            panic!("mempool origin expected");
        };
        assert_eq!(detail.transactions, 401);
    }

    #[test]
    fn a_null_band_is_read_as_absent() {
        let note = NoteFixture::note(
            NoteKind::LIVE,
            Some("mempool"),
            r#"{"source":"mempool","price":1.0,"samples":1,"transactions":1,"band_low":null,"band_high":null}"#,
            1,
        );

        assert_eq!(PriceNote::from_note(&note).expect("note").band(), None);
    }

    #[test]
    fn another_kind_is_refused() {
        let note = NoteFixture::note(1, Some("x"), "{}", 1);

        assert_eq!(PriceNote::from_note(&note), Err(PriceNoteError::WrongKind(1)));
    }

    #[test]
    fn a_note_without_a_window_tag_is_refused() {
        let note = NoteFixture::note(NoteKind::LIVE, None, "{}", 1);

        assert_eq!(PriceNote::from_note(&note), Err(PriceNoteError::MissingWindow));
    }

    #[test]
    fn content_that_is_not_json_is_refused() {
        let note = NoteFixture::note(NoteKind::LIVE, Some("w"), "not json", 1);

        assert!(matches!(PriceNote::from_note(&note), Err(PriceNoteError::Content(_))));
    }

    #[test]
    fn a_missing_or_non_positive_price_is_refused() {
        for content in [
            r#"{"source":"chain"}"#,
            r#"{"source":"chain","price":null}"#,
            r#"{"source":"chain","price":0}"#,
            r#"{"source":"chain","price":-5}"#,
        ] {
            let note = NoteFixture::note(NoteKind::LIVE, Some("w"), content, 1);

            assert_eq!(
                PriceNote::from_note(&note),
                Err(PriceNoteError::MissingField("price")),
                "{content}"
            );
        }
    }

    #[test]
    fn an_unknown_source_is_refused() {
        let note = NoteFixture::note(NoteKind::LIVE, Some("w"), r#"{"source":"cex","price":1}"#, 1);

        assert_eq!(
            PriceNote::from_note(&note),
            Err(PriceNoteError::UnknownSource("cex".to_string()))
        );
    }

    #[test]
    fn a_chain_note_missing_a_height_is_refused() {
        let note = NoteFixture::note(
            NoteKind::AUDIT,
            Some("chain-1h"),
            r#"{"price":1.0,"start_height":1,"start_time":1,"stop_time":2}"#,
            1,
        );

        assert_eq!(
            PriceNote::from_note(&note),
            Err(PriceNoteError::MissingField("stop_height"))
        );
    }

    #[test]
    fn staleness_uses_the_source_specific_limit() {
        let mempool = PriceNote::from_note(&NoteFixture::mempool(1_000)).expect("mempool");
        let chain = PriceNote::from_note(&NoteFixture::chain(1_000)).expect("chain");

        assert!(!mempool.is_stale(1_000 + 300));
        assert!(mempool.is_stale(1_000 + 301));
        assert!(!chain.is_stale(1_000 + 301));
        assert!(chain.is_stale(1_000 + 2_701));
    }

    #[test]
    fn a_note_from_the_future_has_zero_age() {
        let price = PriceNote::from_note(&NoteFixture::mempool(2_000)).expect("mempool");

        assert_eq!(price.age_seconds(1_000), 0);
    }
}
