use serde::Deserialize;

/// One output of a streamed mempool transaction, and, when the filter kept it, what it reads as
/// in dollars at the hourly price.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TransactionOutput {
    pub sats: u64,
    #[serde(default)]
    pub admitted: bool,
    pub usd: Option<u32>,
    pub implied: Option<f64>,
    #[serde(default)]
    pub inside_band: bool,
}

impl TransactionOutput {
    const SATS_PER_BTC: f64 = 100_000_000.0;

    #[must_use]
    pub fn btc(&self) -> f64 {
        self.sats as f64 / Self::SATS_PER_BTC
    }
}

#[cfg(test)]
mod tests {
    use super::TransactionOutput;

    #[test]
    fn a_bare_output_is_not_admitted_and_reads_nothing() {
        let output: TransactionOutput = serde_json::from_str(r#"{"sats":150000000,"admitted":false}"#).expect("output");

        assert!(!output.admitted);
        assert_eq!(output.usd, None);
        assert!(!output.inside_band);
        assert!((output.btc() - 1.5).abs() < f64::EPSILON);
    }
}
