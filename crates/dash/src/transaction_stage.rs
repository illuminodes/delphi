use serde::Deserialize;

/// How far a mempool transaction got into the price, as the daemon tagged it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransactionStage {
    Rejected,
    Dropped,
    Sampled,
    Round,
    Voting,
}

impl TransactionStage {
    pub const ALL: [Self; 5] = [Self::Voting, Self::Round, Self::Sampled, Self::Dropped, Self::Rejected];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Dropped => "dropped",
            Self::Sampled => "sampled",
            Self::Round => "near round $",
            Self::Voting => "voting",
        }
    }

    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Rejected => "a filter rule turned it away before any output was looked at",
            Self::Dropped => "passed the rules, but every output was below 1,000 sats or above 100,000 BTC",
            Self::Sampled => "its outputs entered the sample, but none sits near a round dollar amount",
            Self::Round => "an output sits within ±25% of a round dollar amount, but implies a price outside ±10%",
            Self::Voting => "an output implies a price within ±10% of the hourly price: it votes",
        }
    }

    #[must_use]
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Rejected => "stage-rejected",
            Self::Dropped => "stage-dropped",
            Self::Sampled => "stage-sampled",
            Self::Round => "stage-round",
            Self::Voting => "stage-voting",
        }
    }

    #[must_use]
    pub const fn entered_model(self) -> bool {
        matches!(self, Self::Sampled | Self::Round | Self::Voting)
    }
}

#[cfg(test)]
mod tests {
    use super::TransactionStage;

    #[test]
    fn stages_read_from_their_tag_and_only_sampled_ones_enter_the_model() {
        let stage: TransactionStage = serde_json::from_str("\"voting\"").expect("stage");

        assert_eq!(stage, TransactionStage::Voting);
        assert!(stage.entered_model());
        assert!(!TransactionStage::Dropped.entered_model());
        assert!(!TransactionStage::Rejected.entered_model());
    }

    #[test]
    fn every_stage_has_its_own_label_and_class() {
        let mut labels: Vec<&str> = TransactionStage::ALL.iter().map(|stage| stage.label()).collect();
        let mut classes: Vec<&str> = TransactionStage::ALL.iter().map(|stage| stage.css_class()).collect();
        labels.dedup();
        classes.dedup();

        assert_eq!(labels.len(), TransactionStage::ALL.len());
        assert_eq!(classes.len(), TransactionStage::ALL.len());
    }
}
