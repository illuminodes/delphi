use std::net::SocketAddrV4;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mantle::difficulty::BlockTime;
use mantle::network::Network;
use delphi_oracle::mempool_ring::UringMempoolFanout;

use crate::{Event, EventSender};

/// Streams unconfirmed transactions from a set of peers on one `io_uring`,
/// reconnecting when every peer has dropped or none has sent a transaction
/// for [`Self::QUIET_LIMIT`]: a peer whose connection died without a reset
/// still counts as live, and mainnet is never quiet that long.
pub struct MempoolWorker {
    candidates: Vec<SocketAddrV4>,
    want: usize,
    stop: Arc<AtomicBool>,
    events: EventSender,
}

impl MempoolWorker {
    pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
    pub const POLL_TIMEOUT: Duration = Duration::from_millis(250);
    pub const RETRY: Duration = Duration::from_secs(15);
    pub const QUIET_LIMIT: Duration = Duration::from_mins(2);

    #[must_use]
    pub const fn new(
        candidates: Vec<SocketAddrV4>,
        want: usize,
        stop: Arc<AtomicBool>,
        events: EventSender,
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
            let mut fanout = match UringMempoolFanout::connect(
                &self.candidates,
                Network::Mainnet,
                self.want,
                Self::CONNECT_TIMEOUT,
                Self::POLL_TIMEOUT,
            ) {
                Ok(fanout) => fanout,
                Err(error) => {
                    if !self.notice(format!("mempool: ring setup failed: {error}")) {
                        return;
                    }
                    self.pause(Self::RETRY);
                    continue;
                }
            };
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
            let mut heard = Instant::now();
            while !self.stop.load(Ordering::Relaxed) && fanout.live_peers() > 0 && heard.elapsed() < Self::QUIET_LIMIT {
                for seen in fanout.poll() {
                    heard = Instant::now();
                    if !self.events.send(Event::Transaction(seen.transaction, Self::now())) {
                        return;
                    }
                }
            }
            let reason = if fanout.live_peers() == 0 {
                "mempool: every peer dropped, reconnecting".to_string()
            } else {
                format!("mempool: no transaction from {} peers in {}s, reconnecting", fanout.live_peers(), Self::QUIET_LIMIT.as_secs())
            };
            if !self.notice(reason) {
                return;
            }
        }
    }

    fn notice(&self, message: String) -> bool {
        self.events.send(Event::Notice(message))
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
