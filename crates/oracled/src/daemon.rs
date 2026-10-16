use std::time::{Duration, Instant};

use crate::{ChainSnapshot, EstimatorHandle, Event, EventReceiver, NoteClass, NoteSink, Outcome, PriceHub};

/// Owns the price hub and drives it from the worker threads' events.
///
/// With an estimator attached, each mempool bootstrap runs on its thread and
/// its result arrives as an event; without one it runs on the hub thread.
pub struct Daemon<S: NoteSink> {
    hub: PriceHub<S>,
    estimator: Option<EstimatorHandle>,
    estimates_skipped: u64,
}

impl<S: NoteSink> Daemon<S> {
    /// How often the [`crate::Ticker`] should wake the hub.
    pub const WAKE: Duration = Duration::from_millis(500);

    #[must_use]
    pub const fn new(hub: PriceHub<S>) -> Self {
        Self {
            hub,
            estimator: None,
            estimates_skipped: 0,
        }
    }

    #[must_use]
    pub fn with_estimator(mut self, estimator: EstimatorHandle) -> Self {
        self.estimator = Some(estimator);
        self
    }

    /// Mempool checks skipped because the estimator was still busy.
    #[must_use]
    pub const fn estimates_skipped(&self) -> u64 {
        self.estimates_skipped
    }

    #[must_use]
    pub const fn hub(&self) -> &PriceHub<S> {
        &self.hub
    }

    /// Handle events until a [`Event::Stop`] arrives or every sender is gone,
    /// then stop the estimator. An attached estimator holds a sender of its
    /// own, so with one attached only [`Event::Stop`] ends the loop.
    pub fn run(&mut self, events: &mut EventReceiver) {
        self.drive(events);
        if let Some(estimator) = &mut self.estimator {
            estimator.stop();
        }
    }

    fn drive(&mut self, events: &mut EventReceiver) {
        let mut reported_drops = 0;
        while let Some(event) = events.recv() {
            match event {
                Event::Stop => return,
                Event::Tick(now) => {
                    self.tick(now);
                    let dropped = events.dropped();
                    if dropped >= reported_drops + 1_000 {
                        eprintln!("{dropped} mempool transactions dropped while the hub was busy");
                        reported_drops = dropped;
                    }
                }
                other => self.handle(other),
            }
        }
    }

    /// Ask relays to drop what earlier versions published as addressable notes.
    pub fn retire_addressable(&mut self) {
        let (window, outcome) = self.hub.retire_addressable();
        Self::report(&window, &outcome);
    }

    fn tick(&mut self, now: Instant) {
        let Some(estimator) = &self.estimator else {
            for (window, outcome) in self.hub.tick(now) {
                Self::report(&window, &outcome);
            }
            return;
        };
        let (outcomes, job) = self.hub.tick_deferred(now);
        if let Some(job) = job
            && !estimator.submit(job)
        {
            self.estimates_skipped += 1;
        }
        for (window, outcome) in outcomes {
            Self::report(&window, &outcome);
        }
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Estimate(result) => {
                for (window, outcome) in self.hub.publish_estimate(*result) {
                    Self::report(&window, &outcome);
                }
            }
            Event::Chain(snapshot) => self.apply(&snapshot),
            Event::Transaction(transaction, at) => {
                if let Some((window, outcome)) = self.hub.absorb(&transaction, at, Instant::now()) {
                    Self::report(&window, &outcome);
                }
            }
            Event::Notice(message) => eprintln!("{message}"),
            Event::Tick(_) | Event::Stop => {}
            Event::Ack { id, accepted, message } => {
                if let Some(audit) = self.hub.acknowledge(&id, accepted) {
                    if !accepted {
                        eprintln!(
                            "relay refused audit {}@{} ({id}): {message}",
                            audit.window.tag(),
                            audit.height
                        );
                    }
                } else if !accepted {
                    eprintln!("relay refused {id}: {message}");
                }
            }
        }
    }

    fn apply(&mut self, snapshot: &ChainSnapshot) {
        for (window, outcome) in self.hub.apply_chain(snapshot) {
            Self::report(&window, &outcome);
        }
    }

    fn report(window: &str, outcome: &Outcome) {
        match outcome {
            Outcome::Published(NoteClass::Live | NoteClass::Transaction, _) => {}
            Outcome::Published(class, id) => eprintln!("published {window} ({class:?} {id})"),
            Outcome::Rejected(error) => eprintln!("rejected {window}: {error}"),
            Outcome::Undelivered(message) => eprintln!("undelivered {window}: {message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle::hash::Hash256;
    use delphi_oracle::OracleReport;
    use nostro2::NostrNote;

    use super::Daemon;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{
        AuditLedger, AuditRecord, ChainSnapshot, ChainWindow, Event, EventBus, MempoolEstimator, MempoolWindow, NoteSink,
        PriceHub, PriceSigner, RecentBlocks, RelayPublisher, TransactionStream,
    };

    #[derive(Default)]
    struct RecordingSink {
        notes: RefCell<Vec<NostrNote>>,
        stop_after_fit: Option<crate::EventSender>,
    }

    impl NoteSink for RecordingSink {
        fn deliver(&self, note: &NostrNote) -> Result<(), String> {
            self.notes.borrow_mut().push(note.clone());
            if let Some(stop) = &self.stop_after_fit
                && note.tags.find_tags_ref("w") == ["fit-mempool"]
            {
                stop.send(Event::Stop);
            }
            Ok(())
        }
    }

    struct DaemonFixture;

    impl DaemonFixture {
        fn daemon(interval: Duration) -> Daemon<RecordingSink> {
            Self::daemon_with(interval, RecordingSink::default())
        }

        fn daemon_with(interval: Duration, sink: RecordingSink) -> Daemon<RecordingSink> {
            let publisher = RelayPublisher::new(sink, PriceSigner::generate());
            let window = MempoolWindow::new(
                "mempool",
                BlockHeight::new(0),
                MempoolEstimator::new(200, 10),
                3,
            );
            Daemon::new(
                PriceHub::new(publisher, AuditLedger::volatile(), window, 0.15, interval).with_transaction_stream(TransactionStream::new(0)),
            )
        }

        fn chain_event(price: f64, tip: u32) -> Event {
            let report = OracleReport {
                central_price: price,
                deviation: 0.01,
                sample_count: 5_000,
                start_height: BlockHeight::new(tip - 5),
                stop_height: BlockHeight::new(tip),
                start_time: BlockTime::new(1),
                stop_time: BlockTime::new(2),
            };
            Event::Chain(ChainSnapshot {
                tip: BlockHeight::new(tip),
                tip_time: BlockTime::new(2),
                anchor: crate::Anchor::checked(price, None, BlockHeight::new(tip)),
                audits: vec![AuditRecord::new(
                    ChainWindow::Hourly,
                    report,
                    Hash256::hash(&tip.to_le_bytes()),
                )],
                fits: Vec::new(),
                blocks: RecentBlocks::default(),
            })
        }

        fn windows(daemon: &Daemon<RecordingSink>) -> Vec<String> {
            daemon
                .hub()
                .publisher()
                .sink()
                .notes
                .borrow()
                .iter()
                .map(|note| note.tags.find_tags_ref("w")[0].to_string())
                .collect()
        }
    }

    #[test]
    fn events_flow_through_to_published_notes() {
        let mut daemon = DaemonFixture::daemon(Duration::from_millis(600));
        let (sender, mut receiver) = EventBus::open(1_024);
        assert!(sender.send(DaemonFixture::chain_event(81_000.0, 100)));
        let mut market = SyntheticMarket::new(80_000.0, 5);
        for _ in 0..300 {
            assert!(sender.send(Event::Transaction(market.transaction(), BlockTime::new(1))));
        }
        assert!(sender.send(Event::Notice("hello".to_string())));
        let start = Instant::now();
        assert!(sender.send(Event::Tick(start)));
        assert!(sender.send(Event::Tick(start + Duration::from_millis(600))));
        assert!(sender.send(Event::Stop));

        daemon.run(&mut receiver);
        let windows = DaemonFixture::windows(&daemon);

        assert!(windows.contains(&"chain-1h".to_string()), "{windows:?}");
        assert!(windows.contains(&"mempool".to_string()), "{windows:?}");
    }

    #[test]
    fn with_an_estimator_the_mempool_price_comes_back_as_an_event() {
        let (sender, mut receiver) = EventBus::open(1_024);
        let estimator =
            crate::EstimatorWorker::spawn(MempoolEstimator::new(200, 10), 3, sender.clone()).expect("spawn");
        let sink = RecordingSink {
            stop_after_fit: Some(sender.clone()),
            ..RecordingSink::default()
        };
        let mut daemon = DaemonFixture::daemon_with(Duration::from_millis(600), sink).with_estimator(estimator);
        assert!(sender.send(DaemonFixture::chain_event(81_000.0, 100)));
        let mut market = SyntheticMarket::new(80_000.0, 5);
        for _ in 0..300 {
            assert!(sender.send(Event::Transaction(market.transaction(), BlockTime::new(1))));
        }
        assert!(sender.send(Event::Tick(Instant::now())));
        drop(sender);

        daemon.run(&mut receiver);
        let windows = DaemonFixture::windows(&daemon);

        assert!(windows.contains(&"mempool".to_string()), "{windows:?}");
        assert!(windows.contains(&"fit-mempool".to_string()), "{windows:?}");
    }

    #[test]
    fn the_loop_ends_when_every_sender_is_gone() {
        let mut daemon = DaemonFixture::daemon(Duration::from_secs(10));
        let (sender, mut receiver) = EventBus::open(4);
        drop(sender);

        daemon.run(&mut receiver);
    }

    #[test]
    fn the_loop_ends_at_a_stop_with_senders_alive_and_leaves_the_rest() {
        let mut daemon = DaemonFixture::daemon(Duration::from_secs(10));
        let (sender, mut receiver) = EventBus::open(8);
        assert!(sender.send(Event::Stop));
        assert!(sender.send(DaemonFixture::chain_event(80_000.0, 100)));

        daemon.run(&mut receiver);

        assert_eq!(daemon.hub().tip(), None);
    }

    #[test]
    fn a_new_block_event_restarts_the_mempool_window() {
        let mut daemon = DaemonFixture::daemon(Duration::from_secs(10));
        let (sender, mut receiver) = EventBus::open(1_024);
        assert!(sender.send(DaemonFixture::chain_event(80_000.0, 100)));
        let mut market = SyntheticMarket::new(80_000.0, 5);
        for _ in 0..50 {
            assert!(sender.send(Event::Transaction(market.transaction(), BlockTime::new(1))));
        }
        assert!(sender.send(DaemonFixture::chain_event(80_000.0, 101)));
        drop(sender);

        daemon.run(&mut receiver);

        assert_eq!(daemon.hub().mempool_outputs(), 0);
        assert_eq!(daemon.hub().tip(), Some(BlockHeight::new(101)));
    }
}
