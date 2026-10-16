use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mantle::difficulty::BlockTime;
use mantle::network::Network;
use delphi_oracle::PeerFanout;

use crate::Event;

/// Streams unconfirmed transactions from a set of peers, reconnecting when
/// every peer has dropped.
pub struct MempoolWorker {
    candidates: Vec<SocketAddrV4>,
    want: usize,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
}

impl MempoolWorker {
    pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    pub const POLL_TIMEOUT: Duration = Duration::from_millis(50);
    pub const RETRY: Duration = Duration::from_secs(15);

    #[must_use]
    pub const fn new(
        candidates: Vec<SocketAddrV4>,
        want: usize,
        stop: Arc<AtomicBool>,
        events: Sender<Event>,
    ) -> Self {
        Self {
            candidates,
            want,
            stop,
            events,
        }
    }

    pub fn run(self) {
        while !self.stop.load(Ordering::Relaxed) {
            let mut fanout = PeerFanout::connect(
                &self.candidates,
                Network::Mainnet,
                self.want,
                Self::CONNECT_TIMEOUT,
                Self::POLL_TIMEOUT,
            );
            if fanout.is_empty() {
                if !self.notice("mempool: no peer completed a handshake".to_string()) {
                    return;
                }
                self.pause(Self::RETRY);
                continue;
            }
            if !self.notice(format!("mempool: watching {} peers", fanout.peer_count())) {
                return;
            }
            while !self.stop.load(Ordering::Relaxed) && fanout.live_peers() > 0 {
                for seen in fanout.poll() {
                    if self
                        .events
                        .send(Event::Transaction(seen.transaction, Self::now()))
                        .is_err()
                    {
                        return;
                    }
                }
            }
            if !self.notice("mempool: every peer dropped, reconnecting".to_string()) {
                return;
            }
        }
    }

    fn notice(&self, message: String) -> bool {
        self.events.send(Event::Notice(message)).is_ok()
    }

    fn pause(&self, total: Duration) {
        let step = Duration::from_millis(500);
        let mut waited = Duration::ZERO;
        while waited < total && !self.stop.load(Ordering::Relaxed) {
            thread::sleep(step);
            waited += step;
        }
    }

    fn now() -> BlockTime {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        BlockTime::new(u32::try_from(seconds).unwrap_or(u32::MAX))
    }
}
