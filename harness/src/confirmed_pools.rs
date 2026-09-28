use mantle::difficulty::BlockHeight;
use mantle::primitives::BlockHeader;
use mantle_oracle::{CachedWindows, OraclePipeline};

use crate::chain_anchor::ChainAnchor;
use crate::intraday_reference::IntradayReference;
use crate::span_plan::SpanPlan;
use crate::subsample_sweep::PricedPool;

/// Tiles a cached time span into fixed-width windows and gathers each
/// window's admitted outputs with its mean reference price.
pub struct ConfirmedPools {
    plan: SpanPlan,
}

impl ConfirmedPools {
    #[must_use]
    pub const fn of_duration(seconds: u32) -> Self {
        Self {
            plan: SpanPlan::of_duration(seconds),
        }
    }

    #[must_use]
    pub const fn plan(&self) -> &SpanPlan {
        &self.plan
    }

    /// Windows without a reference price, with a missing block, or with no
    /// admitted outputs are skipped.
    #[must_use]
    pub fn gather(
        &self,
        windows: &mut CachedWindows,
        reference: &IntradayReference,
        first: u32,
        last: u32,
    ) -> Vec<PricedPool> {
        self.gather_anchored(windows, reference, first, last, &[], &[])
    }

    /// [`Self::gather`], also pricing each window's chain anchor from the
    /// `chain_blocks` blocks of `run` that precede it. A window whose anchor
    /// cannot be priced simply lacks that anchor.
    #[must_use]
    pub fn gather_anchored(
        &self,
        windows: &mut CachedWindows,
        reference: &IntradayReference,
        first: u32,
        last: u32,
        run: &[(BlockHeight, BlockHeader)],
        chain_blocks: &[usize],
    ) -> Vec<PricedPool> {
        let mut pools = Vec::new();
        for window in self.plan.tile(first, last) {
            let Some(price) = reference.mean_over(window.start(), window.stop()) else {
                continue;
            };
            let mut pipeline = OraclePipeline::new();
            if windows.feed(window.selection(), &mut pipeline).is_err() {
                continue;
            }
            let sample = pipeline.into_sample();
            if sample.is_empty() {
                continue;
            }
            let mut pool = PricedPool::new(sample, price);
            for blocks in chain_blocks {
                if let Some(anchor) = ChainAnchor::price(windows, run, window.start(), *blocks) {
                    pool = pool.with_anchor(*blocks, anchor);
                }
            }
            pools.push(pool);
        }
        pools
    }
}
