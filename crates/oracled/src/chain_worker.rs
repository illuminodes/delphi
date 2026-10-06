use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mantle::difficulty::BlockHeight;

use crate::chain_sync::ChainSyncSpec;
use crate::{AuditBackfill, AuditLedger, BlockRecord, ChainPricer, ChainSnapshot, ChainSync, Event};

/// Follows the chain tip and reports what to publish after each pass.
///
/// A snapshot is sent only once the record log is level with the chain tip,
/// so a half-filled log never publishes old prices as current. Its audit
/// records come newest first in batches of [`Self::AUDIT_BATCH`]; while a
/// backlog remains the next batch follows after [`Self::BACKFILL_PAUSE`].
pub struct ChainWorker {
    spec: ChainSyncSpec,
    ledger: Arc<Mutex<AuditLedger>>,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
}

struct ChainPass {
    sync: ChainSync,
    pricer: ChainPricer,
    backfill: AuditBackfill,
    stop: Arc<AtomicBool>,
    events: Sender<Event>,
    priced_tip: Option<BlockHeight>,
}

impl ChainWorker {
    pub const POLL: Duration = Duration::from_secs(30);
    pub const RETRY: Duration = Duration::from_secs(20);
    pub const AUDIT_BATCH: usize = 24;
    pub const BACKFILL_PAUSE: Duration = Duration::from_secs(2);

    #[must_use]
    pub const fn new(
        spec: ChainSyncSpec,
        ledger: Arc<Mutex<AuditLedger>>,
        stop: Arc<AtomicBool>,
        events: Sender<Event>,
    ) -> Self {
        Self {
            spec,
            ledger,
            stop,
            events,
        }
    }

    /// Open the sync on this thread and follow the chain until told to stop.
    /// A sync that cannot open stops the whole daemon.
    pub fn run(self) {
        let Self {
            spec,
            ledger,
            stop,
            events,
        } = self;
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
            backfill: AuditBackfill::new(ledger),
            stop,
            events,
            priced_tip: None,
        }
        .run();
    }
}

impl ChainPass {
    fn run(mut self) {
        while !self.stop.load(Ordering::Relaxed) {
            let pause = match self.pass() {
                Ok(pause) => pause,
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

    /// One synchronization, then a snapshot once the log is level with the
    /// tip. Returns how long to wait before the next pass.
    fn pass(&mut self) -> Result<Duration, String> {
        let report = self.sync.sync()?;
        let notice = format!(
            "chain tip {} ({} blocks read, {} held, block cache {} MB)",
            report.tip,
            report.appended,
            self.sync.log().len(),
            self.sync.cache().bytes() / 1_000_000
        );
        if !self.notice(notice) {
            return Ok(Duration::ZERO);
        }
        if !report.caught_up {
            return Ok(Duration::ZERO);
        }
        let snapshot = self.snapshot();
        let backlog = snapshot.as_ref().is_some_and(|snapshot| snapshot.audits.len() >= ChainWorker::AUDIT_BATCH);
        if let Some(snapshot) = snapshot
            && self.events.send(Event::Chain(snapshot)).is_err()
        {
            self.stop.store(true, Ordering::Relaxed);
        }
        Ok(if backlog {
            ChainWorker::BACKFILL_PAUSE
        } else {
            ChainWorker::POLL
        })
    }

    /// The next audit batch, with the tip's fits when the tip has moved.
    /// `None` when there is nothing new to say.
    fn snapshot(&mut self) -> Option<ChainSnapshot> {
        let log = self.sync.log();
        let tip = log.tip().map(BlockRecord::height)?;
        self.backfill.prune(log);
        let audits = self.backfill.next_batch(log, ChainWorker::AUDIT_BATCH);
        let moved = self.priced_tip != Some(tip);
        if !moved && audits.is_empty() {
            return None;
        }
        self.priced_tip = Some(tip);
        let anchor = self.pricer.anchor(log);
        Some(ChainSnapshot {
            tip,
            anchor,
            audits,
            fits: if moved { self.pricer.fits(log) } else { Vec::new() },
            blocks: self.sync.recent().measured(anchor),
        })
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
