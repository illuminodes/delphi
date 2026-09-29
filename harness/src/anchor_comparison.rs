use mantle_oracle::PriceBand;

use crate::anchor_spec::AnchorSpec;
use crate::convergence::{ConvergenceRow, ConvergenceTrial};
use crate::subsample_sweep::{DrawMode, SubsampleSweep};

/// The same draws priced three ways: by the chain anchor alone, by the
/// mempool sample with no constraint, and by the mempool sample limited to a
/// band around the anchor.
pub struct AnchorRow {
    pub chain_only: ConvergenceRow,
    pub unanchored: ConvergenceRow,
    pub anchored: ConvergenceRow,
}

pub struct AnchorComparison<'a> {
    sweep: &'a SubsampleSweep<'a>,
}

impl<'a> AnchorComparison<'a> {
    #[must_use]
    pub const fn new(sweep: &'a SubsampleSweep<'a>) -> Self {
        Self { sweep }
    }

    /// Draws from pools without the anchor `spec` are skipped in all three
    /// columns, so they always cover the same draws.
    #[must_use]
    pub fn row(&self, size: usize, mode: DrawMode, spec: AnchorSpec, tolerance: f64) -> AnchorRow {
        let mut chain_only = ConvergenceTrial::new(size);
        let mut unanchored = ConvergenceTrial::new(size);
        let mut anchored = ConvergenceTrial::new(size);
        let estimator = self.sweep.estimator();
        self.sweep.each_pool_draw(size, mode, |subset, pool| {
            let Some(anchor) = pool.anchor(spec) else {
                return;
            };
            let Some(band) = PriceBand::around(anchor, tolerance) else {
                return;
            };
            let reference = pool.reference();
            chain_only.record(anchor, reference);
            match subset.price_with(estimator) {
                Ok(report) => unanchored.record(report.central_price, reference),
                Err(_) => unanchored.record_attempt(),
            }
            match subset.price_within(estimator, band) {
                Ok(report) => anchored.record(report.central_price, reference),
                Err(_) => anchored.record_attempt(),
            }
        });
        AnchorRow {
            chain_only: chain_only.finish(),
            unanchored: unanchored.finish(),
            anchored: anchored.finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle_oracle::AdmittedSample;

    use super::AnchorComparison;
    use crate::anchor_spec::AnchorSpec;
    use crate::subsample_sweep::{DrawMode, PricedPool, SubsampleSweep};

    fn signal_pool() -> AdmittedSample {
        let mut sample = AdmittedSample::new();
        for step in 1..200 {
            sample.push(
                Satoshi::new(99_000 + step * 30),
                BlockHeight::new(1),
                BlockTime::new(1),
            );
        }
        sample
    }

    #[test]
    fn pools_without_an_anchor_are_skipped_in_every_column() {
        let pools = [PricedPool::new(signal_pool(), 100_000.0)];
        let sweep = SubsampleSweep::new(&pools, 10, 1);
        let row = AnchorComparison::new(&sweep).row(100, DrawMode::Random, AnchorSpec::new(6, 0), 0.25);
        assert_eq!(row.chain_only.attempted(), 0);
        assert_eq!(row.unanchored.attempted(), 0);
        assert_eq!(row.anchored.attempted(), 0);
    }

    #[test]
    fn an_anchored_pool_is_priced_in_all_three_columns() {
        let spec = AnchorSpec::new(6, 0);
        let pools = [PricedPool::new(signal_pool(), 100_000.0).with_anchor(spec, 100_000.0)];
        let sweep = SubsampleSweep::new(&pools, 10, 1);
        let row = AnchorComparison::new(&sweep).row(100, DrawMode::Random, spec, 0.25);
        assert_eq!(row.chain_only.attempted(), 10);
        assert_eq!(row.unanchored.attempted(), 10);
        assert_eq!(row.anchored.priced(), 10);
    }

    #[test]
    fn an_anchor_far_from_the_signal_leaves_the_anchored_column_unpriced() {
        let spec = AnchorSpec::new(6, 0);
        let pools = [PricedPool::new(signal_pool(), 100_000.0).with_anchor(spec, 1.0e9)];
        let sweep = SubsampleSweep::new(&pools, 10, 1);
        let row = AnchorComparison::new(&sweep).row(100, DrawMode::Random, spec, 0.25);
        assert_eq!(row.anchored.attempted(), 10);
        assert_eq!(row.anchored.priced(), 0);
    }
}
