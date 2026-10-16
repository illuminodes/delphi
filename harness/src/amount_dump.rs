use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime};
use delphi_oracle::AdmittedSample;

/// Admitted outputs saved as `satoshis unix_seconds` lines, so a live
/// collection can be analysed again without reconnecting to peers.
pub struct AmountDump {
    path: PathBuf,
}

impl AmountDump {
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// # Errors
    /// Returns the I/O error if the file cannot be written.
    pub fn write(&self, sample: &AdmittedSample) -> std::io::Result<()> {
        let mut body = String::with_capacity(sample.len() * 24);
        for output in sample.outputs() {
            let _ = writeln!(body, "{} {}", output.amount.to_u64(), output.time.to_u32());
        }
        std::fs::write(&self.path, body)
    }

    /// # Errors
    /// Returns the I/O error if the file cannot be read. Malformed lines are
    /// skipped.
    pub fn read(&self) -> std::io::Result<AdmittedSample> {
        let body = std::fs::read_to_string(&self.path)?;
        let mut sample = AdmittedSample::new();
        for (amount, time) in body.lines().filter_map(Self::parse_line) {
            sample.push(Satoshi::new(amount), BlockHeight::new(0), BlockTime::new(time));
        }
        Ok(sample)
    }

    fn parse_line(line: &str) -> Option<(u64, u32)> {
        let (amount, time) = line.trim().split_once(' ')?;
        Some((amount.parse().ok()?, time.trim().parse().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};
    use delphi_oracle::AdmittedSample;

    use super::AmountDump;

    #[test]
    fn round_trips_amounts_and_times() {
        let mut sample = AdmittedSample::new();
        sample.push(Satoshi::new(12_345), BlockHeight::new(7), BlockTime::new(1_700_000_000));
        sample.push(Satoshi::new(99_999), BlockHeight::new(7), BlockTime::new(1_700_000_050));
        let path = std::env::temp_dir().join(format!("amount-dump-{}.txt", std::process::id()));
        let dump = AmountDump::at(&path);

        dump.write(&sample).expect("write dump");
        let restored = dump.read().expect("read dump");
        let _ = std::fs::remove_file(&path);

        assert_eq!(restored.len(), 2);
        assert_eq!(restored.outputs()[1].amount, Satoshi::new(99_999));
        assert_eq!(restored.outputs()[1].time, BlockTime::new(1_700_000_050));
    }

    #[test]
    fn skips_malformed_lines() {
        assert_eq!(AmountDump::parse_line("12 34"), Some((12, 34)));
        assert_eq!(AmountDump::parse_line("garbage"), None);
        assert_eq!(AmountDump::parse_line("12 x"), None);
    }
}
