use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::hash::Hash256;

use crate::chain_sync::ChainSyncSpec;
use crate::{AuditBackfill, BlockRecord, ChainPricer, ChainSnapshot, ChainSync, ChainWindow, Event, EventSender, TipAge};

/// Follows the chain tip and reports what to publish after each pass.
///
/// A snapshot is sent only once the record log is level with the chain tip,
/// so a half-filled log never publishes old prices as current. Prices come
/// from the settled height, the newest block enough peers agree on, so a
/// reorg near the tip never reaches a signed audit; the newest blocks are
/// still streamed as they arrive. Audit records come newest first in batches
/// of [`Self::AUDIT_BATCH`]; while a backlog remains the next batch follows
/// after [`Self::BACKFILL_PAUSE`].
pub struct ChainWorker {
    spec: ChainSyncSpec,
    published: HashSet<(ChainWindow, u32, Hash256)>,
    stop: Arc<AtomicBool>,
    events: EventSender,
}

struct ChainPass {
    sync: ChainSync,
    pricer: ChainPricer,
    backfill: AuditBackfill,
    stop: Arc<AtomicBool>,
    events: EventSender,
    priced_tip: Option<BlockHeight>,
    settled: Option<BlockHeight>,
}

impl ChainWorker {
    pub const POLL: Duration = Duration::from_secs(30);
    pub const RETRY: Duration = Duration::from_secs(20);
    pub const AUDIT_BATCH: usize = 24;
    pub const BACKFILL_PAUSE: Duration = Duration::from_secs(2);

    #[must_use]
    pub fn new(
        spec: ChainSyncSpec,
        published: HashSet<(ChainWindow, u32, Hash256)>,
        stop: Arc<AtomicBool>,
        events: EventSender,
    ) -> Self {
        Self {
            spec,
            published,
            stop,
            events,
        }
    }

    /// Open the sync on this thread and follow the chain until told to stop.
    /// A sync that cannot open stops the whole daemon.
    pub fn run(self) {
        let Self {
            spec,
            published,
            stop,
            events,
        } = self;
        let sync = match spec.open() {
            Ok(sync) => sync,
            Err(message) => {
                let _ = events.send(Event::Notice(format!("chain sync cannot start: {message}")));
                stop.store(true, Ordering::Relaxed);
                let _ = events.send(Event::Stop);
                return;
            }
        };
        ChainPass {
            sync,
            pricer: ChainPricer::new(),
            backfill: AuditBackfill::new(published),
            stop,
            events,
            priced_tip: None,
            settled: None,
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
            "chain tip {}, settled {} ({} blocks read, {} held)",
            report.tip,
            report.settled.map_or_else(|| "nothing yet".to_string(), |height| height.to_string()),
            report.appended,
            self.sync.log().len(),
        );
        if report.settled.is_some() {
            self.settled = report.settled;
        }
        if !self.notice(notice) {
            return Ok(Duration::ZERO);
        }
        let age = TipAge::now(report.tip_time);
        if age.is_stale()
            && !self.notice(format!(
                "chain tip {} is {} minutes old; peers may be behind or withholding blocks",
                report.tip,
                age.seconds() / 60
            ))
        {
            return Ok(Duration::ZERO);
        }
        if !report.caught_up {
            return Ok(Duration::ZERO);
        }
        let snapshot = self.snapshot(report.tip_time);
        let backlog = snapshot.as_ref().is_some_and(|snapshot| snapshot.audits.len() >= ChainWorker::AUDIT_BATCH);
        if let Some(snapshot) = snapshot
            && !self.events.send(Event::Chain(snapshot))
        {
            self.stop.store(true, Ordering::Relaxed);
        }
        Ok(if backlog {
            ChainWorker::BACKFILL_PAUSE
        } else {
            ChainWorker::POLL
        })
    }

    /// The next audit batch up to the settled height, with the fits at that
    /// height when the tip has moved. `None` when there is nothing new to
    /// say. Before anything has settled, only the newest blocks go out.
    fn snapshot(&mut self, tip_time: BlockTime) -> Option<ChainSnapshot> {
        let log = self.sync.log();
        let tip = log.tip().map(BlockRecord::height)?;
        self.backfill.prune(log);
        let audits = self.settled.map_or_else(Vec::new, |settled| {
            self.backfill.next_batch(log, ChainWorker::AUDIT_BATCH, settled)
        });
        let moved = self.priced_tip != Some(tip);
        if !moved && audits.is_empty() {
            return None;
        }
        self.priced_tip = Some(tip);
        let anchor = self.settled.and_then(|settled| self.pricer.anchor(log, settled));
        let fits = match self.settled {
            Some(settled) if moved => self.pricer.fits(log, settled),
            _ => Vec::new(),
        };
        Some(ChainSnapshot {
            tip,
            tip_time,
            settled: self.settled,
            anchor,
            audits,
            fits,
            blocks: self.sync.recent().measured(anchor.map(|anchor| anchor.price)),
        })
    }

    fn notice(&self, message: String) -> bool {
        self.events.send(Event::Notice(message))
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
