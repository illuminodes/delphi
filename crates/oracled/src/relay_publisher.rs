use std::time::Instant;

use nostro2::NostrNote;
use nostro2_relay::NostrRelay;

use crate::{NotePayload, PriceSigner, PriceSignerError, PublishPolicy, Verdict};

/// Where signed notes go.
pub trait NoteSink {
    /// # Errors
    /// Returns a description of why the note could not be queued.
    fn deliver(&self, note: &NostrNote) -> Result<(), String>;
}

impl NoteSink for NostrRelay {
    fn deliver(&self, note: &NostrNote) -> Result<(), String> {
        self.send(note).map_err(|error| error.to_string())
    }
}

/// What happened to one offered update.
#[derive(Debug)]
pub enum Outcome {
    Published(Verdict),
    Held,
    Rejected(PriceSignerError),
    Undelivered(String),
}

/// Counts of outcomes since the publisher started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublishTally {
    pub published: u64,
    pub held: u64,
    pub rejected: u64,
    pub undelivered: u64,
}

/// Signs price updates and sends the ones the policy allows.
///
/// A window is recorded as published only after the sink accepted the note,
/// so a failed delivery is retried on the next update instead of waiting out
/// a heartbeat.
pub struct RelayPublisher<S: NoteSink> {
    sink: S,
    signer: PriceSigner,
    policy: PublishPolicy,
    tally: PublishTally,
}

impl<S: NoteSink> RelayPublisher<S> {
    #[must_use]
    pub fn new(sink: S, signer: PriceSigner, policy: PublishPolicy) -> Self {
        Self {
            sink,
            signer,
            policy,
            tally: PublishTally::default(),
        }
    }

    pub fn offer(&mut self, update: &impl NotePayload, now: Instant) -> Outcome {
        if !update.is_publishable() {
            self.tally.rejected += 1;
            return Outcome::Rejected(PriceSignerError::Unpublishable(update.window().to_string()));
        }
        let verdict = self.policy.judge(update.window(), update.price(), now);
        if !verdict.publishes() {
            self.tally.held += 1;
            return Outcome::Held;
        }
        let note = match self.signer.sign(update) {
            Ok(note) => note,
            Err(error) => {
                self.tally.rejected += 1;
                return Outcome::Rejected(error);
            }
        };
        if let Err(message) = self.sink.deliver(&note) {
            self.tally.undelivered += 1;
            return Outcome::Undelivered(message);
        }
        self.policy.record(update.window(), update.price(), now);
        self.tally.published += 1;
        Outcome::Published(verdict)
    }

    #[must_use]
    pub const fn tally(&self) -> PublishTally {
        self.tally
    }

    #[must_use]
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    #[must_use]
    pub const fn signer(&self) -> &PriceSigner {
        &self.signer
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::{Duration, Instant};

    use nostro2::NostrNote;
    use nostro2_signer::NostrKeypair;
    use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;

    use super::{NoteSink, Outcome, PublishTally, RelayPublisher};
    use crate::{PriceSigner, PriceSource, PriceUpdate, PublishPolicy, Verdict};

    #[derive(Default)]
    struct RecordingSink {
        notes: RefCell<Vec<NostrNote>>,
        failing: RefCell<bool>,
    }

    impl NoteSink for RecordingSink {
        fn deliver(&self, note: &NostrNote) -> Result<(), String> {
            if *self.failing.borrow() {
                return Err("ring full".to_string());
            }
            self.notes.borrow_mut().push(note.clone());
            Ok(())
        }
    }

    struct PublisherFixture;

    impl PublisherFixture {
        fn publisher() -> RelayPublisher<RecordingSink> {
            RelayPublisher::new(
                RecordingSink::default(),
                PriceSigner::new(<NostrKeypair as KeypairTrait>::generate()),
                PublishPolicy::new(0.001, Duration::from_mins(1)),
            )
        }

        fn update(window: &str, price: f64) -> PriceUpdate {
            PriceUpdate::new(
                window,
                price,
                300,
                PriceSource::Mempool {
                    transactions: 150,
                    band_low: price * 0.99,
                    band_high: price * 1.01,
                },
            )
        }
    }

    #[test]
    fn the_first_update_is_signed_and_delivered() {
        let mut publisher = PublisherFixture::publisher();
        let outcome = publisher.offer(&PublisherFixture::update("w", 77_000.0), Instant::now());

        assert!(matches!(outcome, Outcome::Published(Verdict::First)));
        assert_eq!(publisher.sink().notes.borrow().len(), 1);
    }

    #[test]
    fn an_unchanged_price_is_held_back() {
        let mut publisher = PublisherFixture::publisher();
        let start = Instant::now();
        publisher.offer(&PublisherFixture::update("w", 77_000.0), start);
        let outcome = publisher.offer(&PublisherFixture::update("w", 77_000.0), start);

        assert!(matches!(outcome, Outcome::Held));
        assert_eq!(publisher.sink().notes.borrow().len(), 1);
    }

    #[test]
    fn a_moved_price_is_published_again() {
        let mut publisher = PublisherFixture::publisher();
        let start = Instant::now();
        publisher.offer(&PublisherFixture::update("w", 77_000.0), start);
        let outcome = publisher.offer(&PublisherFixture::update("w", 77_500.0), start);

        assert!(matches!(outcome, Outcome::Published(Verdict::Moved)));
    }

    #[test]
    fn a_failed_delivery_is_retried_on_the_next_update() {
        let mut publisher = PublisherFixture::publisher();
        let start = Instant::now();
        *publisher.sink().failing.borrow_mut() = true;
        let failed = publisher.offer(&PublisherFixture::update("w", 77_000.0), start);
        *publisher.sink().failing.borrow_mut() = false;
        let retried = publisher.offer(&PublisherFixture::update("w", 77_000.0), start);

        assert!(matches!(failed, Outcome::Undelivered(_)));
        assert!(matches!(retried, Outcome::Published(Verdict::First)));
    }

    #[test]
    fn an_unpublishable_update_is_rejected_and_not_recorded() {
        let mut publisher = PublisherFixture::publisher();
        let start = Instant::now();
        let outcome = publisher.offer(&PublisherFixture::update("w", f64::NAN), start);
        let next = publisher.offer(&PublisherFixture::update("w", 77_000.0), start);

        assert!(matches!(outcome, Outcome::Rejected(_)));
        assert!(matches!(next, Outcome::Published(Verdict::First)));
        assert_eq!(publisher.sink().notes.borrow().len(), 1);
    }

    #[test]
    fn the_tally_counts_every_outcome() {
        let mut publisher = PublisherFixture::publisher();
        let start = Instant::now();
        publisher.offer(&PublisherFixture::update("w", 77_000.0), start);
        publisher.offer(&PublisherFixture::update("w", 77_000.0), start);
        publisher.offer(&PublisherFixture::update("w", f64::NAN), start);
        *publisher.sink().failing.borrow_mut() = true;
        publisher.offer(&PublisherFixture::update("w", 90_000.0), start);

        assert_eq!(
            publisher.tally(),
            PublishTally {
                published: 1,
                held: 1,
                rejected: 1,
                undelivered: 1,
            }
        );
    }
}
