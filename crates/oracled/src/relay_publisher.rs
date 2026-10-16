use nostro2::NostrNote;
use nostro2_relay::NostrRelay;

use crate::{NoteClass, NotePayload, PriceSigner, PriceSignerError};

/// Where signed notes go.
pub trait NoteSink {
    /// # Errors
    /// Returns a description of why the note could not be queued.
    fn deliver(&self, note: &NostrNote) -> Result<(), String>;

    /// Whether each delivered note is later confirmed by an ack naming its
    /// id. A sink that does not ack counts a note as stored once delivered.
    fn acknowledges(&self) -> bool {
        false
    }
}

impl NoteSink for NostrRelay {
    fn deliver(&self, note: &NostrNote) -> Result<(), String> {
        self.send(note).map_err(|error| error.to_string())
    }

    fn acknowledges(&self) -> bool {
        true
    }
}

/// What happened to one offered payload.
#[derive(Debug)]
pub enum Outcome {
    /// Handed to the sink, with the note id.
    Published(NoteClass, String),
    Rejected(PriceSignerError),
    Undelivered(String),
}

impl Outcome {
    #[must_use]
    pub const fn is_published(&self) -> bool {
        matches!(self, Self::Published(..))
    }
}

/// Counts of outcomes since the publisher started.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublishTally {
    pub audit: u64,
    pub live: u64,
    pub transactions: u64,
    pub other: u64,
    pub rejected: u64,
    pub undelivered: u64,
}

impl PublishTally {
    #[must_use]
    pub const fn published(&self) -> u64 {
        self.audit + self.live + self.transactions + self.other
    }

    const fn count(&mut self, class: NoteClass) {
        match class {
            NoteClass::Audit => self.audit += 1,
            NoteClass::Live => self.live += 1,
            NoteClass::Transaction => self.transactions += 1,
            NoteClass::Addressable => self.other += 1,
        }
    }
}

/// Signs payloads and hands them to the sink.
pub struct RelayPublisher<S: NoteSink> {
    sink: S,
    signer: PriceSigner,
    tally: PublishTally,
}

impl<S: NoteSink> RelayPublisher<S> {
    #[must_use]
    pub fn new(sink: S, signer: PriceSigner) -> Self {
        Self {
            sink,
            signer,
            tally: PublishTally::default(),
        }
    }

    pub fn offer(&mut self, payload: &impl NotePayload) -> Outcome {
        let note = match self.signer.sign(payload) {
            Ok(note) => note,
            Err(error) => {
                self.tally.rejected += 1;
                return Outcome::Rejected(error);
            }
        };
        self.send(&note)
    }

    /// Deliver a note signed elsewhere, such as a deletion request.
    pub fn send(&mut self, note: &NostrNote) -> Outcome {
        if let Err(message) = self.sink.deliver(note) {
            self.tally.undelivered += 1;
            return Outcome::Undelivered(message);
        }
        let class = NoteClass::of_kind(note.kind);
        self.tally.count(class);
        Outcome::Published(class, note.id.clone().unwrap_or_default())
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

    use nostro2::NostrNote;

    use super::{NoteSink, Outcome, PublishTally, RelayPublisher};
    use crate::{NoteClass, PriceSigner, PriceUpdate};

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
            RelayPublisher::new(RecordingSink::default(), PriceSigner::generate())
        }

        fn update(price: f64) -> PriceUpdate {
            PriceUpdate::new("mempool", price, 300, 150, price * 0.99, price * 1.01)
        }
    }

    #[test]
    fn every_offer_is_signed_and_delivered_with_its_id() {
        let mut publisher = PublisherFixture::publisher();
        let first = publisher.offer(&PublisherFixture::update(77_000.0));
        let second = publisher.offer(&PublisherFixture::update(77_000.0));

        let notes = publisher.sink().notes.borrow();
        assert_eq!(notes.len(), 2);
        let Outcome::Published(class, id) = first else {
            panic!("published expected");
        };
        assert_eq!(class, NoteClass::Live);
        assert_eq!(Some(id), notes[0].id.clone());
        assert!(second.is_published());
    }

    #[test]
    fn the_tally_counts_every_outcome() {
        let mut publisher = PublisherFixture::publisher();
        publisher.offer(&PublisherFixture::update(77_000.0));
        publisher.offer(&PublisherFixture::update(f64::NAN));
        *publisher.sink().failing.borrow_mut() = true;
        let failed = publisher.offer(&PublisherFixture::update(90_000.0));

        assert!(matches!(failed, Outcome::Undelivered(_)));
        assert_eq!(
            publisher.tally(),
            PublishTally {
                audit: 0,
                live: 1,
                transactions: 0,
                other: 0,
                rejected: 1,
                undelivered: 1,
            }
        );
        assert_eq!(publisher.tally().published(), 1);
    }
}
