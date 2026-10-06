use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};

use crate::{
    CentralPriceFinder, DenominationFit, ImpliedHistogram, IntradayPricePoints, OracleError,
    OracleReport, OutputHistogram, PriceBand, PriceFit, RoughPriceEstimator, ScoreCurve,
    SlideProfile,
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
        self.bounds = Some(Self::widen(self.bounds, height, time));
    }

    fn widen(
        bounds: Option<(BlockHeight, BlockHeight, BlockTime, BlockTime)>,
        height: BlockHeight,
        time: BlockTime,
    ) -> (BlockHeight, BlockHeight, BlockTime, BlockTime) {
        match bounds {
            None => (height, height, time, time),
            Some((start_height, stop_height, start_time, stop_time)) => (
                start_height.min(height),
                stop_height.max(height),
                start_time.min(time),
                stop_time.max(time),
            ),
        }
    }

    pub fn evict_oldest(&mut self, count: usize) {
        self.outputs.drain(..count.min(self.outputs.len()));
        self.bounds = self
            .outputs
            .iter()
            .fold(None, |bounds, output| Some(Self::widen(bounds, output.height, output.time)));
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
        self.bounds.ok_or(OracleError::NoTransactions)?;
        let rough_price = estimator
            .estimate(&self.histogram()?)
            .ok_or(OracleError::RoughPriceUnavailable)?;
        self.report_from(rough_price)
    }

    /// [`Self::price_with`] with the rough search limited to `band`, so a
    /// price the chain already rules out cannot win.
    ///
    /// # Errors
    /// As [`Self::price`], and [`OracleError::RoughPriceUnavailable`] when
    /// the band lies outside the searched range.
    pub fn price_within(
        &self,
        estimator: &RoughPriceEstimator,
        band: PriceBand,
    ) -> Result<OracleReport, OracleError> {
        self.bounds.ok_or(OracleError::NoTransactions)?;
        let rough_price = estimator
            .estimate_within(&self.histogram()?, band)
            .ok_or(OracleError::RoughPriceUnavailable)?;
        self.report_from(rough_price)
    }

    /// The price and the evidence behind it, with the rough search limited
    /// to `band` when one is given.
    ///
    /// # Errors
    /// As [`Self::price_within`].
    pub fn fit(
        &self,
        estimator: &RoughPriceEstimator,
        band: Option<PriceBand>,
    ) -> Result<PriceFit, OracleError> {
        self.bounds.ok_or(OracleError::NoTransactions)?;
        let histogram = self.histogram()?;
        let profile = estimator
            .profile(&histogram)
            .ok_or(OracleError::RoughPriceUnavailable)?;
        let rough_price = estimator
            .estimate_bounded(&histogram, &profile, band)
            .ok_or(OracleError::RoughPriceUnavailable)?;
        let points = self.implied_points(rough_price);
        let report = self.report_from(rough_price)?;
        let curve = ScoreCurve::from_profile(&histogram, &profile)
            .ok_or(OracleError::RoughPriceUnavailable)?;
        let implied = ImpliedHistogram::build(
            points.as_slice(),
            report.central_price,
            ImpliedHistogram::DEFAULT_REACH,
            ImpliedHistogram::DEFAULT_BINS,
        )
        .ok_or(OracleError::CentralPriceUnavailable)?;
        Ok(PriceFit {
            rough_price,
            band,
            octave_ratio: profile.octave_ratio(),
            curve,
            implied,
            denominations: DenominationFit::per_amount(points.as_slice(), report.central_price),
            report,
        })
    }

    fn implied_points(&self, rough_price: f64) -> IntradayPricePoints {
        let mut points = IntradayPricePoints::new();
        for output in &self.outputs {
            points.insert(output.amount, rough_price, output.height, output.time);
        }
        points
    }

    fn report_from(&self, rough_price: f64) -> Result<OracleReport, OracleError> {
        let (start_height, stop_height, start_time, stop_time) =
            self.bounds.ok_or(OracleError::NoTransactions)?;
        let points = self.implied_points(rough_price);
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
    fn evicting_drops_the_oldest_outputs() {
        let mut sample = sample_of(&[10, 20, 30, 40]);
        sample.evict_oldest(2);
        assert_eq!(sample.len(), 2);
        assert_eq!(sample.outputs()[0].amount, Satoshi::new(30));
    }

    #[test]
    fn evicting_recomputes_the_bounds() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let mut sample = sample_of(&amounts);
        sample.evict_oldest(10);
        let report = sample.price().expect("remaining outputs carry a signal");
        assert_eq!(report.start_height, BlockHeight::new(900_010));
    }

    #[test]
    fn evicting_more_than_held_empties_the_sample() {
        let mut sample = sample_of(&[10, 20]);
        sample.evict_oldest(9);
        assert!(sample.is_empty());
        assert_eq!(sample.price().unwrap_err(), OracleError::NoTransactions);
    }

    #[test]
    fn a_subset_ignores_out_of_range_indices() {
        assert_eq!(sample_of(&[10, 20]).subset(&[0, 9]).len(), 1);
    }

    #[test]
    fn a_fit_reports_the_same_price_as_pricing() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let sample = sample_of(&amounts);
        let fit = sample
            .fit(&crate::RoughPriceEstimator::new(), None)
            .expect("outputs carry a signal");

        assert_eq!(fit.report, sample.price().expect("price"));
        assert!(fit.implied.total() > 0);
        assert_eq!(
            fit.denominations.iter().map(|row| row.inside).sum::<usize>(),
            fit.report.sample_count
        );
    }

    #[test]
    fn a_banded_fit_matches_banded_pricing() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let sample = sample_of(&amounts);
        let estimator = crate::RoughPriceEstimator::new();
        let band = crate::PriceBand::around(100_000.0, 0.15).expect("band");
        let fit = sample.fit(&estimator, Some(band)).expect("fit");

        assert_eq!(fit.report, sample.price_within(&estimator, band).expect("price"));
        assert_eq!(fit.band, Some(band));
    }

    #[test]
    fn bounds_span_every_pushed_output() {
        let amounts: Vec<u64> = (1..100).map(|step| 99_000 + step * 30).collect();
        let report = sample_of(&amounts).price().expect("outputs carry a signal");
        assert_eq!(report.start_height, BlockHeight::new(900_000));
        assert_eq!(report.stop_height, BlockHeight::new(900_098));
    }
}
