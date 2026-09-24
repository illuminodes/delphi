use mantle_oracle::AdmittedSample;

use crate::convergence::{ConvergenceRow, ConvergenceTrial};
use crate::sample_rng::SampleRng;

/// A sample of admitted outputs together with the price it should recover.
pub struct PricedPool {
    sample: AdmittedSample,
    reference: f64,
}

impl PricedPool {
    #[must_use]
    pub const fn new(sample: AdmittedSample, reference: f64) -> Self {
        Self { sample, reference }
    }

    #[must_use]
    pub const fn sample(&self) -> &AdmittedSample {
        &self.sample
    }

    #[must_use]
    pub const fn reference(&self) -> f64 {
        self.reference
    }
}

/// How a draw of `size` outputs is taken from a pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawMode {
    /// `size` distinct outputs chosen uniformly from the whole pool.
    Random,
    /// `size` consecutive outputs in arrival order, from a random offset.
    Contiguous,
}

impl DrawMode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Random => "random",
            Self::Contiguous => "contiguous",
        }
    }

    #[must_use]
    pub fn indices(self, rng: &mut SampleRng, pool: usize, size: usize) -> Vec<usize> {
        match self {
            Self::Random => rng.choose(pool, size),
            Self::Contiguous => {
                let take = size.min(pool);
                let offset = rng.below(pool - take + 1);
                (offset..offset + take).collect()
            }
        }
    }
}

/// Prices repeated draws of each size across a set of pools.
pub struct SubsampleSweep<'a> {
    pools: &'a [PricedPool],
    draws: usize,
    seed: u64,
}

impl<'a> SubsampleSweep<'a> {
    #[must_use]
    pub const fn new(pools: &'a [PricedPool], draws: usize, seed: u64) -> Self {
        Self { pools, draws, seed }
    }

    /// Draws cycle through pools that hold at least `size` outputs, so a size
    /// larger than every pool yields an empty row rather than a skewed one.
    #[must_use]
    pub fn row(&self, size: usize, mode: DrawMode) -> ConvergenceRow {
        let eligible: Vec<&PricedPool> = self
            .pools
            .iter()
            .filter(|pool| pool.sample.len() >= size)
            .collect();
        let mut trial = ConvergenceTrial::new(size);
        if eligible.is_empty() {
            return trial.finish();
        }
        let mut rng = SampleRng::seeded(self.seed ^ size as u64);
        for draw in 0..self.draws {
            let pool = eligible[draw % eligible.len()];
            let picks = mode.indices(&mut rng, pool.sample.len(), size);
            match pool.sample.subset(&picks).price() {
                Ok(report) => trial.record(report.central_price, pool.reference),
                Err(_) => trial.record_attempt(),
            }
        }
        trial.finish()
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle_oracle::AdmittedSample;

    use super::{DrawMode, PricedPool, SubsampleSweep};
    use crate::sample_rng::SampleRng;

    #[test]
    fn contiguous_draws_are_consecutive_and_in_range() {
        let mut rng = SampleRng::seeded(5);
        for _ in 0..50 {
            let picks = DrawMode::Contiguous.indices(&mut rng, 40, 10);
            assert_eq!(picks.len(), 10);
            assert!(picks.windows(2).all(|pair| pair[1] == pair[0] + 1));
            assert!(*picks.last().expect("non-empty") < 40);
        }
    }

    #[test]
    fn a_contiguous_draw_of_the_whole_pool_takes_everything() {
        let picks = DrawMode::Contiguous.indices(&mut SampleRng::seeded(1), 7, 7);
        assert_eq!(picks, (0..7).collect::<Vec<_>>());
    }

    #[test]
    fn a_size_beyond_every_pool_attempts_nothing() {
        let mut sample = AdmittedSample::new();
        sample.push(Satoshi::new(10_000), BlockHeight::new(1), BlockTime::new(1));
        let pools = [PricedPool::new(sample, 80_000.0)];
        let row = SubsampleSweep::new(&pools, 10, 1).row(5, DrawMode::Random);
        assert_eq!(row.attempted(), 0);
    }

    #[test]
    fn every_draw_is_attempted_when_a_pool_is_large_enough() {
        let mut sample = AdmittedSample::new();
        for step in 0..20 {
            sample.push(Satoshi::new(10_000 + step), BlockHeight::new(1), BlockTime::new(1));
        }
        let pools = [PricedPool::new(sample, 80_000.0)];
        let row = SubsampleSweep::new(&pools, 12, 1).row(10, DrawMode::Contiguous);
        assert_eq!(row.attempted(), 12);
    }
}
