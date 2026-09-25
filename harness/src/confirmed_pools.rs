use mantle_oracle::{CachedWindows, OraclePipeline};

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
            if !sample.is_empty() {
                pools.push(PricedPool::new(sample, price));
            }
        }
        pools
    }
}
