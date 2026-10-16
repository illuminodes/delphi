use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::{ChainSnapshot, Event, NoteClass, NoteSink, Outcome, PriceHub};

/// Owns the price hub and drives it from the worker threads' events.
pub struct Daemon<S: NoteSink> {
    hub: PriceHub<S>,
}

impl<S: NoteSink> Daemon<S> {
    pub const WAKE: Duration = Duration::from_millis(500);

    #[must_use]
    pub const fn new(hub: PriceHub<S>) -> Self {
        Self { hub }
    }

    #[must_use]
    pub const fn hub(&self) -> &PriceHub<S> {
        &self.hub
    }

    /// Handle events until the deadline passes or every sender is gone.
    pub fn run(&mut self, events: &Receiver<Event>, deadline: Option<Instant>) {
        loop {
            if deadline.is_some_and(|limit| Instant::now() >= limit) {
                return;
            }
            match events.recv_timeout(Self::WAKE) {
                Ok(event) => self.handle(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            for (window, outcome) in self.hub.tick(Instant::now()) {
                Self::report(&window, &outcome);
            }
        }
    }

    /// Ask relays to drop what earlier versions published as addressable notes.
    pub fn retire_addressable(&mut self) {
        let (window, outcome) = self.hub.retire_addressable();
        Self::report(&window, &outcome);
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Chain(snapshot) => self.apply(&snapshot),
            Event::Transaction(transaction, at) => {
                if let Some((window, outcome)) = self.hub.absorb(&transaction, at, Instant::now()) {
                    Self::report(&window, &outcome);
                }
            }
            Event::Notice(message) => eprintln!("{message}"),
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
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle::hash::Hash256;
    use delphi_oracle::OracleReport;
    use nostro2::NostrNote;

    use super::Daemon;
    use crate::synthetic_market::SyntheticMarket;
    use crate::{
        AuditLedger, AuditRecord, ChainSnapshot, ChainWindow, Event, MempoolEstimator, MempoolWindow, NoteSink,
        PriceHub, PriceSigner, RecentBlocks, RelayPublisher, TransactionStream,
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

    struct DaemonFixture;

    impl DaemonFixture {
        fn daemon(interval: Duration) -> Daemon<RecordingSink> {
            let publisher = RelayPublisher::new(RecordingSink::default(), PriceSigner::generate());
            let window = MempoolWindow::new(
                "mempool",
                BlockHeight::new(0),
                MempoolEstimator::new(200, 10),
                3,
            );
            let ledger = Arc::new(Mutex::new(AuditLedger::volatile()));
            Daemon::new(
                PriceHub::new(publisher, ledger, window, 0.15, interval).with_transaction_stream(TransactionStream::new(0)),
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
                anchor: Some(price),
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
        let (sender, receiver) = mpsc::channel();
        sender.send(DaemonFixture::chain_event(81_000.0, 100)).expect("send");
        let mut market = SyntheticMarket::new(80_000.0, 5);
        for _ in 0..300 {
            sender
                .send(Event::Transaction(market.transaction(), BlockTime::new(1)))
                .expect("send");
        }
        sender.send(Event::Notice("hello".to_string())).expect("send");

        daemon.run(&receiver, Some(Instant::now() + Duration::from_millis(1_800)));
        let windows = DaemonFixture::windows(&daemon);

        assert!(windows.contains(&"chain-1h".to_string()), "{windows:?}");
        assert!(windows.contains(&"mempool".to_string()), "{windows:?}");
    }

    #[test]
    fn the_loop_ends_when_every_sender_is_gone() {
        let mut daemon = DaemonFixture::daemon(Duration::from_secs(10));
        let (sender, receiver) = mpsc::channel::<Event>();
        drop(sender);
        let started = Instant::now();

        daemon.run(&receiver, None);

        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn the_loop_ends_at_its_deadline_with_senders_alive() {
        let mut daemon = DaemonFixture::daemon(Duration::from_secs(10));
        let (_sender, receiver) = mpsc::channel::<Event>();
        let started = Instant::now();

        daemon.run(&receiver, Some(started + Duration::from_millis(700)));

        assert!(started.elapsed() >= Duration::from_millis(700));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn a_new_block_event_restarts_the_mempool_window() {
        let mut daemon = DaemonFixture::daemon(Duration::from_secs(10));
        let (sender, receiver) = mpsc::channel();
        sender.send(DaemonFixture::chain_event(80_000.0, 100)).expect("send");
        let mut market = SyntheticMarket::new(80_000.0, 5);
        for _ in 0..50 {
            sender
                .send(Event::Transaction(market.transaction(), BlockTime::new(1)))
                .expect("send");
        }
        sender.send(DaemonFixture::chain_event(80_000.0, 101)).expect("send");
        drop(sender);

        daemon.run(&receiver, None);

        assert_eq!(daemon.hub().mempool_outputs(), 0);
        assert_eq!(daemon.hub().tip(), Some(BlockHeight::new(101)));
    }
}
