use std::thread::{self, JoinHandle};

use nostro2::NostrRelayEvent;
use nostro2_relay::NostrRelay;

use crate::{Event, EventSender};

/// Reads the relay's replies and forwards each `OK` to the hub, which
/// enters accepted audits in the ledger. Notices are printed.
pub struct AckLogger {
    relay: NostrRelay,
    events: EventSender,
}

impl AckLogger {
    #[must_use]
    pub const fn new(relay: NostrRelay, events: EventSender) -> Self {
        Self { relay, events }
    }

    /// # Errors
    /// Returns a message when the thread cannot be spawned.
    pub fn spawn(self) -> Result<JoinHandle<()>, String> {
        thread::Builder::new()
            .name("relay-acks".into())
            .spawn(move || self.run())
            .map_err(|error| format!("spawn ack logger: {error}"))
    }

    fn run(mut self) {
        while let Some(event) = self.relay.recv_blocking() {
            if !self.forward(event) {
                return;
            }
        }
    }

    fn forward(&self, event: NostrRelayEvent) -> bool {
        match event {
            NostrRelayEvent::SentOk(_, id, accepted, message) => {
                self.events.send(Event::Ack { id, accepted, message })
            }
            NostrRelayEvent::Notice(_, message) => {
                eprintln!("relay notice: {message}");
                true
            }
            _ => true,
        }
    }
}
