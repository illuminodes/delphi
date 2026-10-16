use delphi_oracle::OracleReport;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowScore {
    pub date: String,
    pub estimated: f64,
    pub reference: f64,
    pub deviation: f64,
    pub within_tolerance: bool,
    pub samples: usize,
}

pub struct Scorecard {
    tolerance: f64,
    scores: Vec<WindowScore>,
    failures: Vec<String>,
}

impl Scorecard {
    #[must_use]
    pub fn new(tolerance: f64) -> Self {
        Self {
            tolerance,
            scores: Vec::new(),
            failures: Vec::new(),
        }
    }

    #[must_use]
    pub const fn tolerance(&self) -> f64 {
        self.tolerance
    }

    #[must_use]
    pub fn scores(&self) -> &[WindowScore] {
        &self.scores
    }

    #[must_use]
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }

    pub fn record_success(&mut self, date: &str, report: &OracleReport, reference: f64) {
        let deviation = Self::relative_deviation(report.central_price, reference);
        let score = WindowScore {
            date: date.to_string(),
            estimated: report.central_price,
            reference,
            deviation,
            within_tolerance: deviation <= self.tolerance,
            samples: report.sample_count,
        };
        if score.within_tolerance {
            self.scores.push(score);
        } else {
            self.failures.push(format!(
                "{date}: deviation {:.2}% exceeds tolerance {:.2}%",
                deviation * 100.0,
                self.tolerance * 100.0
            ));
            self.scores.push(score);
        }
    }

    pub fn record_failure(&mut self, date: &str, error: &str) {
        self.failures.push(format!("{date}: {error}"));
    }

    #[must_use]
    pub fn median_absolute_deviation(&self) -> Option<f64> {
        let mut deviations: Vec<f64> = self
            .scores
            .iter()
            .map(|score| score.deviation.abs())
            .collect();
        if deviations.is_empty() {
            return None;
        }
        deviations.sort_by(f64::total_cmp);
        Some(deviations[deviations.len() / 2])
    }

    #[must_use]
    pub fn worst_deviation(&self) -> Option<f64> {
        self.scores
            .iter()
            .map(|score| score.deviation.abs())
            .fold(None, |worst, current| match worst {
                Some(value) if value >= current => Some(value),
                _ => Some(current),
            })
    }

    fn relative_deviation(estimated: f64, reference: f64) -> f64 {
        (estimated - reference).abs() / reference
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime};
    use delphi_oracle::OracleReport;

    use super::{Scorecard, WindowScore};

    struct ReportFixture;

    impl ReportFixture {
        fn price(price: f64) -> OracleReport {
            OracleReport {
                central_price: price,
                deviation: 0.05,
                sample_count: 30_000,
                start_height: BlockHeight::new(900_000),
                stop_height: BlockHeight::new(900_143),
                start_time: BlockTime::new(1),
                stop_time: BlockTime::new(2),
            }
        }
    }

    #[test]
    fn scores_windows_against_the_tolerance() {
        let mut scorecard = Scorecard::new(0.02);
        scorecard.record_success("2026-08-27", &ReportFixture::price(77_000.0), 77_370.0);
        scorecard.record_success("2026-08-28", &ReportFixture::price(60_000.0), 77_370.0);
        assert!(!scorecard.passed());
        let scores = scorecard.scores();
        assert!((*scores)[0].within_tolerance);
        assert!(!(*scores)[1].within_tolerance);
        assert!((*scores)[0].deviation > 0.0 && (*scores)[0].deviation < 0.01);
    }

    #[test]
    fn median_and_worst_track_all_scored_windows() {
        let mut scorecard = Scorecard::new(1.0);
        scorecard.record_success("a", &ReportFixture::price(100.0), 100.0);
        scorecard.record_success("b", &ReportFixture::price(110.0), 100.0);
        scorecard.record_success("c", &ReportFixture::price(90.0), 100.0);
        assert_eq!(scorecard.median_absolute_deviation(), Some(0.1));
        assert_eq!(scorecard.worst_deviation(), Some(0.1));
        assert!(scorecard.passed());
    }

    #[test]
    fn failures_mark_the_run_failed() {
        let mut scorecard = Scorecard::new(0.01);
        scorecard.record_failure("2026-08-27", "fetch failed");
        assert!(!scorecard.passed());
        assert_eq!(scorecard.median_absolute_deviation(), None);
    }

    #[test]
    fn window_scores_are_plain_data() {
        let score = WindowScore {
            date: "2026-08-27".to_string(),
            estimated: 77_000.0,
            reference: 77_370.0,
            deviation: 0.0048,
            within_tolerance: true,
            samples: 36_000,
        };
        assert_eq!(score.date, "2026-08-27");
    }
}
