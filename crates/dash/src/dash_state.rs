use nostr_minions::NostrNote;

use crate::{ChainBlocks, FitBoard, FitNote, HistoryRange, MempoolStream, NoteKind, OracleStatus, PriceBoard};

/// Everything one publisher has sent: prices and their history, the fits
/// behind them, and the daemon's last word about itself.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DashState {
    prices: PriceBoard,
    fits: FitBoard,
    status: Option<OracleStatus>,
    blocks: Option<ChainBlocks>,
    mempool: MempoolStream,
    last_live: Option<i64>,
    history_since: i64,
}

impl DashState {
    /// A live note older than this means the stream has gone quiet.
    pub const LIVE_WITHIN_SECONDS: i64 = 90;

    /// Drops the price history and makes room for a new range; the relay refills it. Audit notes
    /// from before the range, still arriving from the previous query, are ignored.
    pub fn restart_history(&mut self, range: HistoryRange, now: i64) {
        self.prices = PriceBoard::new(range.note_capacity() as usize);
        self.history_since = range.since(now);
    }

    pub fn accept(&mut self, note: &NostrNote) {
        if note.kind == NoteKind::TRANSACTION {
            self.mempool.accept(note);
            return;
        }
        if note.kind == NoteKind::AUDIT && note.created_at < self.history_since {
            return;
        }
        if note.kind == NoteKind::LIVE {
            self.last_live = Some(self.last_live.map_or(note.created_at, |last| last.max(note.created_at)));
        }
        let window = NoteKind::window(note);
        if window == Some(OracleStatus::WINDOW) {
            if let Some(status) = OracleStatus::from_note(note) {
                self.status = Some(status);
            }
        } else if window == Some(ChainBlocks::WINDOW) {
            if let Some(blocks) = ChainBlocks::from_note(note) {
                self.blocks = Some(blocks);
            }
        } else if window.is_some_and(FitNote::is_fit_window) {
            self.fits.accept(note);
        } else {
            self.prices.accept(note);
        }
    }

    #[must_use]
    pub const fn prices(&self) -> &PriceBoard {
        &self.prices
    }

    #[must_use]
    pub const fn fits(&self) -> &FitBoard {
        &self.fits
    }

    #[must_use]
    pub const fn status(&self) -> Option<&OracleStatus> {
        self.status.as_ref()
    }

    #[must_use]
    pub const fn blocks(&self) -> Option<&ChainBlocks> {
        self.blocks.as_ref()
    }

    #[must_use]
    pub const fn mempool(&self) -> &MempoolStream {
        &self.mempool
    }

    /// Whether a live note arrived recently enough to call the stream live.
    #[must_use]
    pub fn is_live(&self, now: i64) -> bool {
        self.last_live
            .is_some_and(|last| now - last <= Self::LIVE_WITHIN_SECONDS)
    }

    #[must_use]
    pub const fn rejected(&self) -> usize {
        self.prices.rejected() + self.fits.rejected()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::DashState;
    use crate::NoteKind;

    struct StateFixture;

    impl StateFixture {
        fn live(window: &str, content: &str, at: i64) -> NostrNote {
            let mut note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .content(content)
                .tag("w", window)
                .build();
            note.created_at = at;
            note
        }
    }

    #[test]
    fn a_fit_note_never_becomes_a_price_card() {
        let mut state = DashState::default();
        state.accept(&StateFixture::live(
            "fit-mempool",
            r#"{"source":"mempool","price":1.0,"curve":{"first_price":2.0,"step":0.5,"scores":[1.0,0.0]},"implied":{"low":0.8,"high":1.25,"counts":[1]}}"#,
            1,
        ));

        assert!(state.prices().is_empty());
        assert!(state.fits().latest("mempool").is_some());
        assert_eq!(state.rejected(), 0);
    }

    #[test]
    fn a_live_price_reaches_the_price_board() {
        let mut state = DashState::default();
        state.accept(&StateFixture::live(
            "mempool",
            r#"{"source":"mempool","price":1.0,"samples":1,"transactions":1}"#,
            1,
        ));

        assert!(state.prices().latest("mempool").is_some());
    }

    #[test]
    fn a_blocks_note_is_kept_apart_from_prices() {
        let mut state = DashState::default();
        state.accept(&StateFixture::live(
            "blocks",
            r#"{"blocks":[{"height":7,"hash":"ab","time":1,"transactions":1}]}"#,
            1,
        ));

        assert_eq!(state.blocks().and_then(|blocks| blocks.latest()).map(|block| block.height), Some(7));
        assert!(state.prices().is_empty());
        assert_eq!(state.rejected(), 0);
    }

    #[test]
    fn restarting_the_history_clears_prices_but_keeps_the_rest() {
        let mut state = DashState::default();
        let mut transaction = nostr_minions::NostrNoteBuilder::new()
            .kind(NoteKind::TRANSACTION)
            .content(r#"{"txid":"ab","verdict":"accepted","stage":"sampled","price":null}"#)
            .tag("w", "tx")
            .build();
        transaction.created_at = 1;
        state.accept(&transaction);
        state.accept(&StateFixture::live("mempool", r#"{"source":"mempool","price":100.0,"samples":300,"transactions":100}"#, 5));
        assert!(!state.prices().is_empty());

        state.restart_history(crate::HistoryRange::days(1), 200_000);

        assert!(state.prices().is_empty());
        assert_eq!(state.mempool().total(), 1);
    }

    #[test]
    fn audits_from_before_the_range_are_ignored_after_a_restart() {
        let mut state = DashState::default();
        state.restart_history(crate::HistoryRange::days(1), 200_000);
        let audit = |at: i64| {
            let mut note = NostrNoteBuilder::new()
                .kind(NoteKind::AUDIT)
                .content(format!(r#"{{"price":100.0,"samples":900,"deviation":0.1,"start_height":1,"stop_height":{at},"start_time":1,"stop_time":{at}}}"#))
                .tag("w", "chain-1d")
                .build();
            note.created_at = at;
            note
        };

        state.accept(&audit(100_000));
        assert!(state.prices().is_empty());
        state.accept(&audit(150_000));
        assert_eq!(state.prices().history("chain-1d").len(), 1);
    }

    #[test]
    fn a_transaction_note_goes_to_the_mempool_stream_only() {
        let mut state = DashState::default();
        let mut note = nostr_minions::NostrNoteBuilder::new()
            .kind(NoteKind::TRANSACTION)
            .content(r#"{"txid":"ab","verdict":"accepted","stage":"sampled","price":null}"#)
            .tag("w", "tx")
            .build();
        note.created_at = 1;
        state.accept(&note);

        assert_eq!(state.mempool().total(), 1);
        assert!(state.prices().is_empty());
        assert_eq!(state.rejected(), 0);
    }

    #[test]
    fn a_status_note_is_kept_apart_and_marks_the_stream_live() {
        let mut state = DashState::default();
        assert!(!state.is_live(100));

        state.accept(&StateFixture::live("status", r#"{"tip":5}"#, 100));

        assert_eq!(state.status().and_then(|status| status.tip), Some(5));
        assert!(state.prices().is_empty());
        assert!(state.is_live(100 + DashState::LIVE_WITHIN_SECONDS));
        assert!(!state.is_live(101 + DashState::LIVE_WITHIN_SECONDS));
    }
}
