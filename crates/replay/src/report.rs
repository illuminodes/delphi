//! Progress reporter for the replay loop.
//!
//! Throttled stderr printer that emits one line at most every `INTERVAL`
//! showing height, % done, blocks/sec running average, and ETA. The replay
//! loop calls `tick(height)` on every block; the reporter decides when to
//! actually print.

use std::time::{Duration, Instant};

use mantle::primitives::BlockHeight;

/// Minimum gap between progress lines. Any tighter and the log dominates the
/// terminal during fast-replaying ranges (skip-script can do hundreds of
/// blocks per second on early-era blocks).
const INTERVAL: Duration = Duration::from_secs(5);

pub struct Reporter {
    start: Instant,
    last_print: Instant,
    last_height: BlockHeight,
    target_height: BlockHeight,
    blocks_since_last: u64,
}

impl Reporter {
    pub fn new(start_height: BlockHeight, target_height: BlockHeight) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last_print: now,
            last_height: start_height,
            target_height,
            blocks_since_last: 0,
        }
    }

    /// Call after every committed block. Throttles internally; only emits a
    /// line when `INTERVAL` has elapsed since the last print.
    pub fn tick(&mut self, height: BlockHeight) {
        self.blocks_since_last += 1;
        let now = Instant::now();
        let since_print = now.duration_since(self.last_print);
        if since_print < INTERVAL {
            return;
        }

        let blocks_in_window = self.blocks_since_last.max(1);
        let bps_window = f64::from(u32::try_from(blocks_in_window).unwrap_or(u32::MAX))
            / since_print.as_secs_f64().max(1e-9);
        let elapsed = now.duration_since(self.start).as_secs_f64().max(1e-9);
        let total_done = u64::from(height.0).saturating_sub(u64::from(self.last_height.0))
            + self
                .blocks_since_last
                .saturating_sub(u64::from(height.0).saturating_sub(u64::from(self.last_height.0)));
        let bps_overall =
            f64::from(u32::try_from(total_done).unwrap_or(u32::MAX)) / elapsed;
        let remaining = u64::from(self.target_height.0).saturating_sub(u64::from(height.0));
        // ETA: remaining * (window_duration / blocks_in_window), integer only.
        let eta = remaining
            .checked_mul(since_print.as_secs())
            .map(|t| t / blocks_in_window);

        eprintln!(
            "  [{:>7}/{:>7}]  {:>5.1}%  {:>6.1} bps (window) / {:>6.1} bps (avg)  ETA {}",
            height.0,
            self.target_height.0,
            100.0 * f64::from(height.0) / f64::from(self.target_height.0).max(1.0),
            bps_window,
            bps_overall,
            format_eta(eta),
        );
        self.last_print = now;
        self.last_height = height;
        self.blocks_since_last = 0;
    }

    /// Called once after the final commit. Prints a summary line.
    pub fn done(&self, final_height: BlockHeight) {
        let elapsed = self.start.elapsed().as_secs_f64();
        let bps = f64::from(final_height.0) / elapsed.max(1e-9);
        eprintln!(
            "Replay complete: {} blocks in {elapsed:.1}s ({bps:.1} blocks/s)",
            final_height.0,
        );
    }
}

fn format_eta(secs: Option<u64>) -> String {
    let Some(secs) = secs else {
        return "—".to_string();
    };
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h{m:02}m")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}
