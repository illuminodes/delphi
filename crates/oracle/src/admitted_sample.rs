use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};

use crate::{
    CentralPriceFinder, IntradayPricePoints, OracleError, OracleReport, OutputHistogram,
    RoughPriceEstimator, SlideProfile,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmittedOutput {
    pub amount: Satoshi,
    pub height: BlockHeight,
    pub time: BlockTime,
}

/// Outputs that cleared [`crate::OutputFilter`], held so a caller can price
/// them repeatedly or price a subset of them.
#[derive(Debug, Clone, Default)]
pub struct AdmittedSample {
    outputs: Vec<AdmittedOutput>,
    bounds: Option<(BlockHeight, BlockHeight, BlockTime, BlockTime)>,
}

impl AdmittedSample {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, amount: Satoshi, height: BlockHeight, time: BlockTime) {
        self.outputs.push(AdmittedOutput {
            amount,
            height,
            time,
        });
        self.bounds = Some(match self.bounds {
            None => (height, height, time, time),
            Some((start_height, stop_height, start_time, stop_time)) => (
                start_height.min(height),
                stop_height.max(height),
                start_time.min(time),
                stop_time.max(time),
            ),
        });
    }

    #[must_use]
    pub fn outputs(&self) -> &[AdmittedOutput] {
        &self.outputs
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.outputs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    #[must_use]
    pub fn subset(&self, indices: &[usize]) -> Self {
        let mut subset = Self::new();
        for index in indices {
            if let Some(output) = self.outputs.get(*index) {
                subset.push(output.amount, output.height, output.time);
            }
        }
        subset
    }

    /// Estimate the price from the outputs held here.
    ///
    /// # Errors
    /// Returns [`OracleError`] when the sample is empty or too thin for the
    /// histogram, rough-price, or central-price stages to resolve.
    pub fn price(&self) -> Result<OracleReport, OracleError> {
        self.price_with(&RoughPriceEstimator::new())
    }

    /// [`Self::price`] with a caller-chosen rough estimator.
    ///
    /// # Errors
    /// As [`Self::price`].
    pub fn price_with(&self, estimator: &RoughPriceEstimator) -> Result<OracleReport, OracleError> {
        let (start_height, stop_height, start_time, stop_time) =
            self.bounds.ok_or(OracleError::NoTransactions)?;
        let rough_price = estimator
            .estimate(&self.histogram()?)
            .ok_or(OracleError::RoughPriceUnavailable)?;
        let mut points = IntradayPricePoints::new();
        for output in &self.outputs {
            points.insert(output.amount, rough_price, output.height, output.time);
        }
        let central = CentralPriceFinder::new()
            .find(&points, rough_price)
            .ok_or(OracleError::CentralPriceUnavailable)?;
        Ok(OracleReport {
            central_price: central.price,
            deviation: central.deviation,
            sample_count: central.sample_count,
            start_height,
            stop_height,
            start_time,
            stop_time,
        })
    }

    /// The rough estimator's score at every slide it searches.
    ///
    /// # Errors
    /// Returns [`OracleError`] when the histogram is empty or too narrow.
    pub fn rough_profile(&self) -> Result<SlideProfile, OracleError> {
        RoughPriceEstimator::new()
            .profile(&self.histogram()?)
            .ok_or(OracleError::RoughPriceUnavailable)
    }

    fn histogram(&self) -> Result<OutputHistogram, OracleError> {
        let mut histogram = OutputHistogram::new();
        for output in &self.outputs {
            histogram.insert(output.amount);
        }
        histogram.smooth_round_btc_amounts();
        if !histogram.normalize_and_clip() {
            return Err(OracleError::EmptyHistogram);
        }
        Ok(histogram)
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};

    use crate::OracleError;

    use super::AdmittedSample;

    fn sample_of(amounts: &[u64]) -> AdmittedSample {
        let mut sample = AdmittedSample::new();
        for (index, amount) in amounts.iter().enumerate() {
            sample.push(
                Satoshi::new(*amount),
                BlockHeight::new(900_000 + u32::try_from(index).expect("index fits u32")),
                BlockTime::new(1_700_000_000 + u32::try_from(index).expect("index fits u32")),
            );
        }
        sample
    }

    #[test]
    fn an_empty_sample_has_no_price() {
        assert_eq!(
            AdmittedSample::new().price().unwrap_err(),
            OracleError::NoTransactions
        );
    }

    #[test]
    fn prices_round_usd_outputs() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let report = sample_of(&amounts).price().expect("outputs carry a signal");
        assert!(report.central_price > 90_000.0, "price: {}", report.central_price);
        assert!(report.central_price < 110_000.0, "price: {}", report.central_price);
    }

    #[test]
    fn a_subset_prices_the_same_as_an_equal_whole() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let whole = sample_of(&amounts);
        let all: Vec<usize> = (0..whole.len()).collect();
        assert_eq!(
            whole.subset(&all).price().map(|report| report.central_price),
            whole.price().map(|report| report.central_price)
        );
    }

    #[test]
    fn a_subset_holds_only_the_named_outputs() {
        let sample = sample_of(&[10, 20, 30, 40]);
        let subset = sample.subset(&[1, 3]);
        assert_eq!(subset.len(), 2);
        assert_eq!(subset.outputs()[0].amount, Satoshi::new(20));
        assert_eq!(subset.outputs()[1].amount, Satoshi::new(40));
    }

    #[test]
    fn a_subset_ignores_out_of_range_indices() {
        assert_eq!(sample_of(&[10, 20]).subset(&[0, 9]).len(), 1);
    }

    #[test]
    fn bounds_span_every_pushed_output() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let report = sample_of(&amounts).price().expect("outputs carry a signal");
        assert_eq!(report.start_height, BlockHeight::new(900_000));
        assert_eq!(report.stop_height, BlockHeight::new(900_098));
    }
}
