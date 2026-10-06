use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::tx::Transaction;
use mantle_oracle::PriceBand;

use crate::{
    AuditLedger, AuditRecord, ChainSnapshot, DaemonStatus, FitUpdate, MempoolWindow, NoteClass, NotePayload,
    NoteSink, Outcome, RecentBlocks, RelayPublisher, TransactionNote, TransactionStream,
};

/// Joins the chain prices, the live mempool window and the publisher.
///
/// Every audit record goes out once and is entered in the ledger. Everything
/// else is live: the mempool price and its fit at each check, the chain fits
/// and the newest blocks at each block and again every `repeat`, and the
/// daemon status every `repeat`. Nothing is priced from the mempool before
/// the chain anchors it. Each mempool transaction goes out as its own note, tagged with the
/// filter's verdict, as far as the [`TransactionStream`] allows.
pub struct PriceHub<S: NoteSink> {
    publisher: RelayPublisher<S>,
    ledger: Arc<Mutex<AuditLedger>>,
    window: MempoolWindow,
    tolerance: f64,
    interval: Duration,
    repeat: Duration,
    started: Instant,
    anchor: Option<f64>,
    tip: Option<BlockHeight>,
    chain_fits: Vec<FitUpdate>,
    blocks: RecentBlocks,
    stream: TransactionStream,
    last_tick: Option<Instant>,
    last_repeat: Option<Instant>,
}

impl<S: NoteSink> PriceHub<S> {
    pub const REPEAT: Duration = Duration::from_mins(1);

    /// Addressable windows earlier versions published, retired by
    /// [`Self::retire_addressable`].
    pub const RETIRED_WINDOWS: [&'static str; 8] = [
        "chain-1h",
        "chain-1d",
        "chain-1w",
        "mempool",
        "fit-chain-1h",
        "fit-chain-1d",
        "fit-chain-1w",
        "fit-mempool",
    ];

    #[must_use]
    pub fn new(
        publisher: RelayPublisher<S>,
        ledger: Arc<Mutex<AuditLedger>>,
        window: MempoolWindow,
        tolerance: f64,
        interval: Duration,
    ) -> Self {
        Self {
            publisher,
            ledger,
            window,
            tolerance,
            interval,
            repeat: Self::REPEAT,
            started: Instant::now(),
            anchor: None,
            tip: None,
            chain_fits: Vec::new(),
            blocks: RecentBlocks::default(),
            stream: TransactionStream::default(),
            last_tick: None,
            last_repeat: None,
        }
    }

    #[must_use]
    pub const fn with_repeat(mut self, repeat: Duration) -> Self {
        self.repeat = repeat;
        self
    }

    #[must_use]
    pub const fn with_transaction_stream(mut self, stream: TransactionStream) -> Self {
        self.stream = stream;
        self
    }

    /// Ask relays to drop the addressable notes earlier versions published.
    pub fn retire_addressable(&mut self) -> (String, Outcome) {
        let outcome = match self
            .publisher
            .signer()
            .sign_deletion(NoteClass::Addressable.kind(), &Self::RETIRED_WINDOWS)
        {
            Ok(note) => self.publisher.send(&note),
            Err(error) => Outcome::Rejected(error),
        };
        ("retire".to_string(), outcome)
    }

    /// Publish the snapshot's audit records and fits, and restart the mempool
    /// window if the tip is not the one it was counting from.
    pub fn apply_chain(&mut self, snapshot: &ChainSnapshot) -> Vec<(String, Outcome)> {
        self.anchor = snapshot.anchor.or(self.anchor);
        let mut outcomes: Vec<(String, Outcome)> =
            snapshot.audits.iter().map(|record| self.audit(record)).collect();
        if self.tip != Some(snapshot.tip) {
            self.window.reset(snapshot.tip);
            self.tip = Some(snapshot.tip);
        }
        if !snapshot.fits.is_empty() {
            self.chain_fits.clone_from(&snapshot.fits);
            outcomes.extend(self.send_chain_fits());
        }
        if !snapshot.blocks.is_empty() && snapshot.blocks != self.blocks {
            self.blocks.clone_from(&snapshot.blocks);
            outcomes.push(self.offer(&self.blocks.clone()));
        }
        outcomes
    }

    fn audit(&mut self, record: &AuditRecord) -> (String, Outcome) {
        let (window, outcome) = self.offer(record);
        if let Outcome::Published(_, id) = &outcome {
            let report = record.report();
            let stored = self
                .ledger
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .record(record.chain_window(), report.stop_height, record.stop_hash(), id);
            if let Err(error) = stored {
                eprintln!("audit ledger: {error}");
            }
        }
        (format!("{window}@{}", record.report().stop_height), outcome)
    }

    fn send_chain_fits(&mut self) -> Vec<(String, Outcome)> {
        let fits = std::mem::take(&mut self.chain_fits);
        let outcomes = fits.iter().map(|fit| self.offer(fit)).collect();
        self.chain_fits = fits;
        outcomes
    }

    fn offer(&mut self, payload: &impl NotePayload) -> (String, Outcome) {
        (payload.window().to_string(), self.publisher.offer(payload))
    }

    /// Add a mempool transaction to the window and, when the stream lets it through, publish
    /// its verdict.
    pub fn absorb(&mut self, transaction: &Transaction, at: BlockTime, now: Instant) -> Option<(String, Outcome)> {
        let (verdict, admitted) = self.window.absorb(transaction, at);
        if !self.stream.admit(now) {
            return None;
        }
        let note = TransactionNote::new(transaction, at, verdict, admitted, self.anchor);
        Some(self.offer(&note))
    }

    /// Stream the mempool price and fit once the interval has passed and an
    /// anchor exists, and the chain fits and status once `repeat` has passed.
    pub fn tick(&mut self, now: Instant) -> Vec<(String, Outcome)> {
        let mut outcomes = Vec::new();
        if self
            .last_repeat
            .is_none_or(|last| now.saturating_duration_since(last) >= self.repeat)
        {
            self.last_repeat = Some(now);
            outcomes.push(self.offer(&self.status(now)));
            outcomes.extend(self.send_chain_fits());
            if !self.blocks.is_empty() {
                outcomes.push(self.offer(&self.blocks.clone()));
            }
        }
        if self
            .last_tick
            .is_some_and(|last| now.saturating_duration_since(last) < self.interval)
        {
            return outcomes;
        }
        self.last_tick = Some(now);
        let Some(band) = self.band() else {
            return outcomes;
        };
        if let Some(update) = self.window.update(band) {
            outcomes.push(self.offer(&update));
            if let Some(fit) = self.window.fit(band) {
                outcomes.push(self.offer(&fit));
            }
        }
        outcomes
    }

    fn band(&self) -> Option<PriceBand> {
        self.anchor
            .and_then(|anchor| PriceBand::around(anchor, self.tolerance))
    }

    #[must_use]
    pub fn status(&self, now: Instant) -> DaemonStatus {
        DaemonStatus {
            tip: self.tip,
            anchor: self.anchor,
            band: self.band(),
            mempool_outputs: self.window.outputs(),
            mempool_transactions: self.window.admitted_transactions(),
            audits_published: self.publisher.tally().audit,
            transaction_notes: self.publisher.tally().transactions,
            transaction_notes_dropped: self.stream.dropped(),
            uptime: now.saturating_duration_since(self.started),
        }
    }

    #[must_use]
    pub const fn anchor(&self) -> Option<f64> {
        self.anchor
    }

    #[must_use]
    pub const fn tip(&self) -> Option<BlockHeight> {
        self.tip
    }

    #[must_use]
    pub fn mempool_outputs(&self) -> usize {
        self.window.outputs()
    }

    #[must_use]
    pub const fn publisher(&self) -> &RelayPublisher<S> {
        &self.publisher
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle::hash::Hash256;
    use mantle_oracle::OracleReport;
    use nostro2::NostrNote;

    use super::PriceHub;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{
        AuditLedger, AuditRecord, AuditStatus, BlockDigest, ChainSnapshot, ChainWindow, MempoolEstimator,
        MempoolWindow, NoteClass, NoteSink, PriceSigner, RecentBlocks, RelayPublisher, TransactionNote,
        TransactionStream,
    };

    #[derive(Default)]
    struct RecordingSink {
        notes: RefCell<Vec<NostrNote>>,
    }

    impl NoteSink for RecordingSink {
        fn deliver(&self, note: &NostrNote) -> Result<(), String> {
            self.notes.borrow_mut().push(note.clone());
            Ok(())
        }
    }

    struct HubFixture;

    impl HubFixture {
        const INTERVAL: Duration = Duration::from_secs(10);

        fn hub() -> PriceHub<RecordingSink> {
            Self::hub_with(Arc::new(Mutex::new(AuditLedger::volatile())))
        }

        fn hub_with(ledger: Arc<Mutex<AuditLedger>>) -> PriceHub<RecordingSink> {
            let publisher = RelayPublisher::new(RecordingSink::default(), PriceSigner::generate());
            let window = MempoolWindow::new(
                "mempool",
                BlockHeight::new(0),
                MempoolEstimator::new(200, 10),
                3,
            );
            PriceHub::new(publisher, ledger, window, 0.15, Self::INTERVAL)
                .with_transaction_stream(TransactionStream::new(0))
        }

        fn audit(window: ChainWindow, price: f64, stop: u32) -> AuditRecord {
            AuditRecord::new(
                window,
                OracleReport {
                    central_price: price,
                    deviation: 0.01,
                    sample_count: 5_000,
                    start_height: BlockHeight::new(stop - 5),
                    stop_height: BlockHeight::new(stop),
                    start_time: BlockTime::new(1),
                    stop_time: BlockTime::new(1_780_000_000 + stop),
                },
                Hash256::hash(&stop.to_le_bytes()),
            )
        }

        fn snapshot(price: f64, tip: u32, windows: &[ChainWindow]) -> ChainSnapshot {
            ChainSnapshot {
                tip: BlockHeight::new(tip),
                anchor: Some(price),
                audits: windows.iter().map(|window| Self::audit(*window, price, tip)).collect(),
                fits: Vec::new(),
                blocks: RecentBlocks::default(),
            }
        }

        fn fill(hub: &mut PriceHub<RecordingSink>, transactions: usize, price: f64) {
            let mut market = SyntheticMarket::new(price, 17);
            let now = Instant::now();
            for _ in 0..transactions {
                hub.absorb(&market.transaction(), BlockTime::new(1), now);
            }
        }

        fn windows(hub: &PriceHub<RecordingSink>, class: NoteClass) -> Vec<String> {
            hub.publisher()
                .sink()
                .notes
                .borrow()
                .iter()
                .filter(|note| note.kind == class.kind())
                .map(|note| note.tags.find_tags_ref("w")[0].to_string())
                .collect()
        }
    }

    #[test]
    fn each_audit_record_is_published_and_entered_in_the_ledger() {
        let ledger = Arc::new(Mutex::new(AuditLedger::volatile()));
        let mut hub = HubFixture::hub_with(Arc::clone(&ledger));
        let outcomes = hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &ChainWindow::ALL));

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(_, outcome)| outcome.is_published()));
        assert_eq!(HubFixture::windows(&hub, NoteClass::Audit), vec!["chain-1h", "chain-1d"]);
        let status = ledger.lock().expect("lock").status(
            ChainWindow::Daily,
            BlockHeight::new(100),
            Hash256::hash(&100_u32.to_le_bytes()),
        );
        assert_eq!(status, AuditStatus::Done);
    }

    #[test]
    fn a_new_tip_resets_the_mempool_window_and_the_same_tip_keeps_it() {
        let mut hub = HubFixture::hub();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[ChainWindow::Hourly]));
        HubFixture::fill(&mut hub, 50, 80_000.0);
        let filled = hub.mempool_outputs();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[]));

        assert!(filled > 0);
        assert_eq!(hub.mempool_outputs(), filled);

        hub.apply_chain(&HubFixture::snapshot(80_000.0, 101, &[ChainWindow::Hourly]));
        assert_eq!(hub.mempool_outputs(), 0);
        assert_eq!(hub.tip(), Some(BlockHeight::new(101)));
    }

    #[test]
    fn nothing_is_priced_before_the_chain_provides_an_anchor() {
        let mut hub = HubFixture::hub();
        HubFixture::fill(&mut hub, 300, 80_000.0);
        let windows: Vec<String> = hub.tick(Instant::now()).into_iter().map(|(window, _)| window).collect();

        assert_eq!(windows, vec!["status"]);
    }

    #[test]
    fn every_check_streams_the_mempool_price_and_its_fit() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(81_000.0, 100, &[]));
        HubFixture::fill(&mut hub, 300, 80_000.0);
        hub.tick(start);
        let second: Vec<String> = hub
            .tick(start + HubFixture::INTERVAL)
            .into_iter()
            .map(|(window, _)| window)
            .collect();

        assert_eq!(second, vec!["mempool", "fit-mempool"]);
        let live = HubFixture::windows(&hub, NoteClass::Live);
        assert_eq!(live.iter().filter(|window| *window == "mempool").count(), 2);
        let notes = hub.publisher().sink().notes.borrow();
        let note = notes
            .iter()
            .find(|note| note.tags.find_tags_ref("w") == ["mempool"])
            .expect("note");
        assert!(note.content.contains("\"band_low\""));
    }

    #[test]
    fn ticks_inside_the_interval_do_nothing() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        hub.apply_chain(&HubFixture::snapshot(81_000.0, 100, &[]));
        HubFixture::fill(&mut hub, 300, 80_000.0);
        hub.tick(start);

        assert!(hub.tick(start + Duration::from_secs(5)).is_empty());
        assert!(!hub.tick(start + HubFixture::INTERVAL).is_empty());
    }

    #[test]
    fn chain_fits_go_out_with_the_block_and_again_each_repeat() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        let mut snapshot = HubFixture::snapshot(80_000.0, 100, &[]);
        let sample = SyntheticMarket::new(80_000.0, 3).sample(600);
        let fit = sample
            .fit(&mantle_oracle::RoughPriceEstimator::new(), None)
            .expect("fit");
        snapshot.fits = vec![crate::FitUpdate::new("chain-1h", "chain", fit)];
        hub.apply_chain(&snapshot);
        hub.tick(start);
        hub.tick(start + Duration::from_secs(30));
        hub.tick(start + PriceHub::<RecordingSink>::REPEAT);

        let live = HubFixture::windows(&hub, NoteClass::Live);
        assert_eq!(live.iter().filter(|window| *window == "fit-chain-1h").count(), 3);
        assert_eq!(live.iter().filter(|window| *window == "status").count(), 2);
    }

    #[test]
    fn new_blocks_go_out_once_and_again_each_repeat() {
        let mut hub = HubFixture::hub();
        let start = Instant::now();
        hub.tick(start);
        let mut snapshot = HubFixture::snapshot(80_000.0, 100, &[]);
        snapshot.blocks.record(BlockDigest {
            height: BlockHeight::new(100),
            hash: "h".to_string(),
            previous: String::new(),
            time: 1,
            version: 1,
            difficulty: 1.0,
            transactions: 1,
            outputs: 1,
            size: 100,
            weight: 400,
            output_value: mantle::amount::Satoshi::new(0),
            coinbase_value: mantle::amount::Satoshi::new(0),
            subsidy: mantle::amount::Satoshi::new(0),
            tally: mantle_oracle::TransactionTally::default(),
            admitted: Vec::new(),
            funnel: None,
        });
        hub.apply_chain(&snapshot);
        hub.apply_chain(&snapshot);
        hub.tick(start + PriceHub::<RecordingSink>::REPEAT);

        let live = HubFixture::windows(&hub, NoteClass::Live);
        assert_eq!(live.iter().filter(|window| *window == RecentBlocks::WINDOW).count(), 2);
    }

    #[test]
    fn each_streamed_transaction_carries_its_verdict_up_to_the_limit() {
        let mut hub = HubFixture::hub().with_transaction_stream(TransactionStream::new(5));
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[]));
        HubFixture::fill(&mut hub, 8, 80_000.0);

        let notes = hub.publisher().sink().notes.borrow();
        let streamed: Vec<_> = notes
            .iter()
            .filter(|note| note.kind == NoteClass::Transaction.kind())
            .collect();
        assert_eq!(streamed.len(), 5);
        assert!(streamed.iter().all(|note| note.tags.find_tags_ref("w") == [TransactionNote::WINDOW]));
        assert!(streamed.iter().all(|note| note.tags.find_tags_ref(TransactionNote::VERDICT_TAG).len() == 1));
        assert!(streamed.iter().all(|note| note.tags.find_tags_ref(TransactionNote::STAGE_TAG).len() == 1));
        drop(notes);
        let status = hub.status(Instant::now());
        assert_eq!(status.transaction_notes, 5);
        assert_eq!(status.transaction_notes_dropped, 3);
    }

    #[test]
    fn the_anchor_survives_a_snapshot_without_one() {
        let mut hub = HubFixture::hub();
        hub.apply_chain(&HubFixture::snapshot(80_000.0, 100, &[]));
        let mut later = HubFixture::snapshot(90_000.0, 101, &[]);
        later.anchor = None;
        hub.apply_chain(&later);

        assert_eq!(hub.anchor(), Some(80_000.0));
    }

    #[test]
    fn retiring_sends_one_deletion_for_every_old_window() {
        let mut hub = HubFixture::hub();
        let (_, outcome) = hub.retire_addressable();

        assert!(outcome.is_published());
        let notes = hub.publisher().sink().notes.borrow();
        assert_eq!(notes[0].kind, PriceSigner::DELETION_KIND);
        assert_eq!(
            notes[0].tags.find_tags_ref("a").len(),
            PriceHub::<RecordingSink>::RETIRED_WINDOWS.len()
        );
    }

    #[test]
    fn a_price_outside_the_anchor_band_is_not_published() {
        let mut hub = HubFixture::hub();
        hub.apply_chain(&HubFixture::snapshot(120_000.0, 100, &[]));
        HubFixture::fill(&mut hub, 300, 80_000.0);
        hub.tick(Instant::now());

        let notes = hub.publisher().sink().notes.borrow();
        let mempool_prices: Vec<f64> = notes
            .iter()
            .filter(|note| note.tags.find_tags_ref("w") == ["mempool"])
            .filter_map(|note| {
                serde_json::from_str::<serde_json::Value>(&note.content).ok()?["price"].as_f64()
            })
            .collect();
        let band = (120_000.0 / 1.15)..=(120_000.0 * 1.15);
        assert!(mempool_prices.iter().all(|price| band.contains(price)), "{mempool_prices:?}");
    }
}
