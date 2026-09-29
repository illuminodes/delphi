use std::thread::{self, JoinHandle};

use nostro2::NostrRelayEvent;
use nostro2_relay::NostrRelay;

pub struct AckLogger {
    relay: NostrRelay,
}

impl AckLogger {
    #[must_use]
    pub const fn new(relay: NostrRelay) -> Self {
        Self { relay }
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
            match event {
                NostrRelayEvent::SentOk(_, id, accepted, message) => {
                    eprintln!("relay ack {id}: accepted={accepted} {message}");
                }
                NostrRelayEvent::Notice(_, message) => eprintln!("relay notice: {message}"),
                _ => {}
            }
        }
    }
}
