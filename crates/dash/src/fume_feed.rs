use crate::{SplitMix, TransactionStage};

#[derive(Debug, Clone)]
pub struct FumeFeed {
    random: SplitMix,
    until_next: f64,
}

impl FumeFeed {
    pub const RATE: f64 = 2.5;
    const MIX: [(TransactionStage, f64); 5] = [
        (TransactionStage::Rejected, 0.42),
        (TransactionStage::Dropped, 0.10),
        (TransactionStage::Sampled, 0.30),
        (TransactionStage::Round, 0.10),
        (TransactionStage::Voting, 0.08),
    ];

    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut random = SplitMix::new(seed);
        let until_next = Self::gap(&mut random);
        Self { random, until_next }
    }

    pub fn advance(&mut self, seconds: f64) -> Vec<TransactionStage> {
        let mut arrivals = Vec::new();
        let mut left = seconds.max(0.0);
        while self.until_next <= left {
            left -= self.until_next;
            arrivals.push(self.pick());
            self.until_next = Self::gap(&mut self.random);
        }
        self.until_next -= left;
        arrivals
    }

    fn gap(random: &mut SplitMix) -> f64 {
        -random.unit().ln() / Self::RATE
    }

    fn pick(&mut self) -> TransactionStage {
        let mut draw = self.random.unit();
        for (stage, share) in Self::MIX {
            if draw <= share {
                return stage;
            }
            draw -= share;
        }
        TransactionStage::Voting
    }
}

#[cfg(test)]
mod tests {
    use super::FumeFeed;
    use crate::TransactionStage;

    #[test]
    fn arrivals_come_at_the_configured_rate() {
        let mut feed = FumeFeed::new(3);
        let arrivals: usize = (0..60_000).map(|_| feed.advance(1.0 / 60.0).len()).sum();
        let expected = FumeFeed::RATE * 1000.0;

        assert!((arrivals as f64 - expected).abs() < expected * 0.05, "{arrivals}");
    }

    #[test]
    fn rejections_are_common_and_votes_are_rare() {
        let stages = FumeFeed::new(5).advance(4000.0);
        let share = |stage| stages.iter().filter(|seen| **seen == stage).count() as f64 / stages.len() as f64;

        assert!((share(TransactionStage::Rejected) - 0.42).abs() < 0.03);
        assert!((share(TransactionStage::Voting) - 0.08).abs() < 0.02);
    }

    #[test]
    fn the_same_seed_gives_the_same_stream() {
        assert_eq!(FumeFeed::new(9).advance(30.0), FumeFeed::new(9).advance(30.0));
    }

    #[test]
    fn no_time_brings_no_arrivals() {
        assert!(FumeFeed::new(1).advance(0.0).is_empty());
        assert!(FumeFeed::new(1).advance(-4.0).is_empty());
    }
}
