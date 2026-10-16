use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::{Consumer, Producer, RingBuffer};

use crate::Event;

/// The bounded ring the worker threads feed the hub through.
///
/// A mempool transaction is dropped and counted when the ring is full, so a
/// flood of them cannot grow memory or hold up the rest. Every other event
/// waits for room, so a block, an ack or a tick is never lost. The hub's
/// loop ends once every [`EventSender`] is dropped and the ring is drained.
pub struct EventBus;

/// One producer handle; clone it for each thread.
pub struct EventSender {
    producer: Producer<Event>,
    dropped: Arc<AtomicU64>,
}

/// The hub's end of the ring.
pub struct EventReceiver {
    consumer: Consumer<Event>,
    dropped: Arc<AtomicU64>,
}

impl EventBus {
    pub const CAPACITY: usize = 16_384;

    #[must_use]
    pub fn open(capacity: usize) -> (EventSender, EventReceiver) {
        let (producer, consumer) = RingBuffer::new(Capacity::exact(capacity)).split();
        let dropped = Arc::new(AtomicU64::new(0));
        (
            EventSender {
                producer,
                dropped: Arc::clone(&dropped),
            },
            EventReceiver { consumer, dropped },
        )
    }
}

impl EventSender {
    /// Queue `event`. Returns `false` once the hub has gone.
    pub fn send(&self, event: Event) -> bool {
        match event {
            Event::Transaction(..) => {
                if self.producer.push(event).is_err() {
                    self.dropped.fetch_add(1, Ordering::Relaxed);
                }
                true
            }
            other => self.producer.push_block(other).is_ok(),
        }
    }
}

impl Clone for EventSender {
    fn clone(&self) -> Self {
        Self {
            producer: self.producer.clone(),
            dropped: Arc::clone(&self.dropped),
        }
    }
}

impl EventReceiver {
    /// The next event, waiting for one; `None` once every sender is gone and
    /// the ring is empty.
    pub fn recv(&mut self) -> Option<Event> {
        self.consumer.pop_block()
    }

    /// Transactions dropped because the ring was full.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockTime;

    use super::EventBus;
    use crate::Event;
    use crate::synthetic_market::SyntheticMarket;

    #[test]
    fn transactions_past_capacity_are_dropped_and_counted_but_notices_wait() {
        let (sender, mut receiver) = EventBus::open(4);
        let mut market = SyntheticMarket::new(80_000.0, 1);
        for _ in 0..6 {
            assert!(sender.send(Event::Transaction(market.transaction(), BlockTime::new(1))));
        }

        assert_eq!(receiver.dropped(), 2);
        let waiting = std::thread::spawn(move || sender.send(Event::Notice("late".to_string())));
        let mut transactions = 0;
        let mut notice = None;
        while notice.is_none() {
            match receiver.recv() {
                Some(Event::Transaction(..)) => transactions += 1,
                Some(Event::Notice(message)) => notice = Some(message),
                _ => panic!("unexpected event"),
            }
        }

        assert!(waiting.join().expect("join"));
        assert_eq!(transactions, 4);
        assert_eq!(notice.as_deref(), Some("late"));
    }

    #[test]
    fn the_receiver_ends_once_every_sender_is_gone() {
        let (sender, mut receiver) = EventBus::open(4);
        let second = sender.clone();
        assert!(second.send(Event::Notice("one".to_string())));
        drop(sender);
        drop(second);

        assert!(matches!(receiver.recv(), Some(Event::Notice(_))));
        assert!(receiver.recv().is_none());
    }

    #[test]
    fn sending_after_the_receiver_is_gone_reports_it() {
        let (sender, receiver) = EventBus::open(4);
        drop(receiver);

        assert!(!sender.send(Event::Notice("nobody".to_string())));
    }
}
