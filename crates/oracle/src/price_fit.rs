use crate::{DenominationFit, ImpliedHistogram, OracleReport, PriceBand, ScoreCurve};

/// A priced sample together with the evidence the price was read from: the
/// rough stage's score at every candidate price, the round-USD implied
/// prices around the result, and how each round amount agrees with it.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceFit {
    pub report: OracleReport,
    pub rough_price: f64,
    pub band: Option<PriceBand>,
    pub octave_ratio: Option<f64>,
    pub curve: ScoreCurve,
    pub implied: ImpliedHistogram,
    pub denominations: Vec<DenominationFit>,
}
