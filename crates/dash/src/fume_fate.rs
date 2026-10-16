use crate::{HexColor, TransactionStage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FumeFate {
    Sampled,
    Round,
    Voting,
    Scattered,
}

impl FumeFate {
    pub const JARS: [Self; 3] = [Self::Sampled, Self::Round, Self::Voting];

    #[must_use]
    pub const fn of(stage: TransactionStage) -> Self {
        match stage {
            TransactionStage::Rejected | TransactionStage::Dropped => Self::Scattered,
            TransactionStage::Sampled => Self::Sampled,
            TransactionStage::Round => Self::Round,
            TransactionStage::Voting => Self::Voting,
        }
    }

    #[must_use]
    pub fn jar(self) -> Option<usize> {
        Self::JARS.iter().position(|fate| *fate == self)
    }

    #[must_use]
    pub const fn colour(self) -> HexColor {
        match self {
            Self::Sampled => HexColor::rgb([0x5a, 0xab, 0xa3]),
            Self::Round => HexColor::rgb([0xa6, 0x8f, 0xd0]),
            Self::Voting => HexColor::rgb([0xc9, 0xa2, 0x4a]),
            Self::Scattered => HexColor::rgb([0x9c, 0x8d, 0x7b]),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Sampled => "sampled",
            Self::Round => "near round $",
            Self::Voting => "voting",
            Self::Scattered => "turned away",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FumeFate;
    use crate::TransactionStage;

    #[test]
    fn rejected_and_dropped_transactions_scatter_and_the_rest_reach_a_jar() {
        assert_eq!(FumeFate::of(TransactionStage::Rejected), FumeFate::Scattered);
        assert_eq!(FumeFate::of(TransactionStage::Dropped), FumeFate::Scattered);
        assert_eq!(FumeFate::of(TransactionStage::Voting).jar(), Some(2));
        assert_eq!(FumeFate::Scattered.jar(), None);
    }

    #[test]
    fn every_fate_has_its_own_colour() {
        let colours = [FumeFate::Sampled, FumeFate::Round, FumeFate::Voting, FumeFate::Scattered].map(FumeFate::colour);

        assert!(colours.iter().enumerate().all(|(at, colour)| colours[at + 1..].iter().all(|other| other != colour)));
    }
}
