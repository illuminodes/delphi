use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::chain_sync::ChainSyncSpec;
use crate::{ChainPricer, ChainSnapshot, ChainSync, Event};

/// Follows the chain tip and reports the priced windows after each pass.
///
/// A snapshot is sent only once the record log is level with the chain tip,
/// so a half-filled log never publishes old prices as current. It is priced
/// again only when the tip has moved, and sent on every caught-up pass so the
/// publisher's heartbeat keeps running.
pub struct ChainWorker {
    spec: ChainSyncSpec,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
}

struct ChainPass {
    sync: ChainSync,
    pricer: ChainPricer,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
    cached: Option<ChainSnapshot>,
}

impl ChainWorker {
    pub const POLL: Duration = Duration::from_secs(30);
    pub const RETRY: Duration = Duration::from_secs(20);

    #[must_use]
    pub const fn new(spec: ChainSyncSpec, stop: Arc<AtomicBool>, events: Sender<Event>) -> Self {
        Self { spec, stop, events }
    }

    /// Open the sync on this thread and follow the chain until told to stop.
    /// A sync that cannot open stops the whole daemon.
    pub fn run(self) {
        let Self { spec, stop, events } = self;
        let sync = match spec.open() {
            Ok(sync) => sync,
            Err(message) => {
                let _ = events.send(Event::Notice(format!("chain sync cannot start: {message}")));
                stop.store(true, Ordering::Relaxed);
                return;
            }
        };
        ChainPass {
            sync,
            pricer: ChainPricer::new(),
            stop,
            events,
            cached: None,
        }
        .run();
    }
}

impl ChainPass {
    fn run(mut self) {
        while !self.stop.load(Ordering::Relaxed) {
            let pause = match self.pass() {
                Ok(true) => ChainWorker::POLL,
                Ok(false) => Duration::ZERO,
                Err(message) => {
                    if !self.notice(format!("chain sync failed: {message}")) {
                        return;
                    }
                    ChainWorker::RETRY
                }
            };
            if !self.sleep(pause) {
                return;
            }
        }
    }

    /// One synchronization, then a snapshot if any window can be priced.
    /// Returns whether the log is level with the tip.
    fn pass(&mut self) -> Result<bool, String> {
        let report = self.sync.sync()?;
        let notice = format!(
            "chain tip {} ({} blocks read, {} held, block cache {} MB)",
            report.tip,
            report.appended,
            self.sync.log().len(),
            self.sync.cache().bytes() / 1_000_000
        );
        if !self.notice(notice) {
            return Ok(true);
        }
        if !report.caught_up {
            return Ok(false);
        }
        let stale = self
            .cached
            .as_ref()
            .is_none_or(|snapshot| self.sync.log().tip().map(crate::BlockRecord::height) != Some(snapshot.tip));
        if stale {
            self.cached = self.pricer.snapshot(self.sync.log());
        }
        if let Some(snapshot) = self.cached.clone()
            && self.events.send(Event::Chain(snapshot)).is_err()
        {
            self.stop.store(true, Ordering::Relaxed);
        }
        Ok(report.caught_up)
    }

    fn notice(&self, message: String) -> bool {
        self.events.send(Event::Notice(message)).is_ok()
    }

    fn sleep(&self, total: Duration) -> bool {
        let step = Duration::from_millis(500);
        let mut waited = Duration::ZERO;
        while waited < total {
            if self.stop.load(Ordering::Relaxed) {
                return false;
            }
            thread::sleep(step);
            waited += step;
        }
        !self.stop.load(Ordering::Relaxed)
    }
}
