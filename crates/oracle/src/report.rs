use std::fmt;

use mantle::difficulty::{BlockHeight, BlockTime};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OracleReport {
    pub central_price: f64,
    pub deviation: f64,
    pub sample_count: usize,
    pub start_height: BlockHeight,
    pub stop_height: BlockHeight,
    pub start_time: BlockTime,
    pub stop_time: BlockTime,
}

impl fmt::Display for OracleReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "BTC price: ${:.0}", self.central_price)?;
        writeln!(formatter, "Deviation: {:.4}%", self.deviation * 100.0)?;
        writeln!(formatter, "Samples: {}", self.sample_count)?;
        writeln!(
            formatter,
            "Blocks: {}-{}",
            self.start_height, self.stop_height
        )?;
        write!(formatter, "Times: {}-{}", self.start_time, self.stop_time)
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};

    use super::OracleReport;

    #[test]
    fn formats_stdout_report() {
        let report = OracleReport {
            central_price: 100_123.4,
            deviation: 0.012_345,
            sample_count: 42,
            start_height: BlockHeight::new(800_000),
            stop_height: BlockHeight::new(800_143),
            start_time: BlockTime::new(1_700_000_000),
            stop_time: BlockTime::new(1_700_086_400),
        };

        let rendered = report.to_string();
        assert!(rendered.contains("BTC price: $100123"));
        assert!(rendered.contains("Samples: 42"));
        assert!(rendered.contains("Blocks: 800000-800143"));
    }
}
