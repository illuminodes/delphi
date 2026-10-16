use mantle::difficulty::BlockHeight;
use delphi_oracle::{AdmittedSample, PriceBand, RoughPriceEstimator};

use crate::{BootstrapRng, FitUpdate, MempoolEstimator, PriceUpdate};

/// A copy of the mempool sample to price inside `band`, taken at `tip`.
#[derive(Debug, Clone)]
pub struct EstimateJob {
    pub name: String,
    pub sample: AdmittedSample,
    pub admitted_transactions: u64,
    pub band: PriceBand,
    pub tip: Option<BlockHeight>,
}

/// The price and fit for one [`EstimateJob`], and the tip it was taken at
/// so a result that a new block overtook can be dropped.
#[derive(Debug, Clone)]
pub struct EstimateResult {
    pub tip: Option<BlockHeight>,
    pub update: Option<PriceUpdate>,
    pub fit: Option<FitUpdate>,
}

impl EstimateJob {
    #[must_use]
    pub fn run(self, estimator: &MempoolEstimator, rng: &mut BootstrapRng) -> EstimateResult {
        let update = estimator.estimate(&self.sample, self.band, rng).map(|estimate| {
            PriceUpdate::new(
                self.name.clone(),
                estimate.price,
                estimate.outputs,
                usize::try_from(self.admitted_transactions).unwrap_or(usize::MAX),
                estimate.band_low,
                estimate.band_high,
            )
        });
        let fit = update
            .as_ref()
            .and_then(|_| self.sample.fit(&RoughPriceEstimator::new(), Some(self.band)).ok())
            .map(|fit| FitUpdate::new(&self.name, "mempool", fit));
        EstimateResult {
            tip: self.tip,
            update,
            fit,
        }
    }
}
