use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::{Event, EventSender};

/// Wakes the hub at a fixed period, and stops it at the deadline or when
/// the stop flag is set, through the same ring as every other event.
pub struct Ticker {
    period: Duration,
    deadline: Option<Instant>,
    stop: Arc<AtomicBool>,
    events: EventSender,
}

impl Ticker {
    #[must_use]
    pub const fn new(
        period: Duration,
        deadline: Option<Instant>,
        stop: Arc<AtomicBool>,
        events: EventSender,
    ) -> Self {
        Self {
            period,
            deadline,
            stop,
            events,
        }
    }

    /// # Errors
    /// Returns a message when the thread cannot be spawned.
    pub fn spawn(self) -> Result<JoinHandle<()>, String> {
        thread::Builder::new()
            .name("ticker".into())
            .spawn(move || self.run())
            .map_err(|error| format!("spawn ticker: {error}"))
    }

    fn run(self) {
        loop {
            thread::sleep(self.period);
            let now = Instant::now();
            if self.stop.load(Ordering::Relaxed) || self.deadline.is_some_and(|limit| now >= limit) {
                self.stop.store(true, Ordering::Relaxed);
                let _ = self.events.send(Event::Stop);
                return;
            }
            if !self.events.send(Event::Tick(now)) {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    use super::Ticker;
    use crate::{Event, EventBus};

    #[test]
    fn ticks_arrive_until_the_deadline_then_a_stop() {
        let (sender, mut receiver) = EventBus::open(64);
        let stop = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + Duration::from_millis(60);
        let handle = Ticker::new(Duration::from_millis(10), Some(deadline), Arc::clone(&stop), sender)
            .spawn()
            .expect("spawn");
        let mut ticks = 0;
        loop {
            match receiver.recv() {
                Some(Event::Tick(_)) => ticks += 1,
                Some(Event::Stop) => break,
                other => panic!("unexpected {:?}", other.is_some()),
            }
        }
        handle.join().expect("join");

        assert!(ticks >= 2, "{ticks}");
        assert!(stop.load(std::sync::atomic::Ordering::Relaxed));
    }
}
