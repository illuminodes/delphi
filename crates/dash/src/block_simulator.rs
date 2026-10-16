use crate::{ChainBlock, SplitMix};

#[derive(Debug, Clone)]
pub struct BlockSimulator {
    random: SplitMix,
    height: u32,
    hash: String,
    time: i64,
}

impl BlockSimulator {
    const START_HEIGHT: u32 = 903_400;
    const START_TIME: i64 = 1_780_000_000;
    const SUBSIDY: u64 = 312_500_000;
    const TARGET: f64 = 600.0;

    #[must_use]
    pub fn new(seed: u64) -> Self {
        let mut random = SplitMix::new(seed);
        let hash = Self::hash(&mut random);
        Self { random, height: Self::START_HEIGHT, hash, time: Self::START_TIME }
    }

    fn hash(random: &mut SplitMix) -> String {
        let zeros = 18 + (random.next_u64() % 5) as usize;
        let tail: String = (0..64 - zeros).map(|_| char::from_digit((random.next_u64() % 16) as u32, 16).unwrap_or('f')).collect();
        format!("{}{tail}", "0".repeat(zeros))
    }

    pub fn next_block(&mut self) -> ChainBlock {
        let weight = if self.random.unit() < 0.15 { 1_000_000 + (self.random.unit() * 2_000_000.0) as u64 } else { 3_850_000 + (self.random.unit() * 149_000.0) as u64 };
        let transactions = (weight as f64 / 4_000_000.0 * (2_200.0 + self.random.unit() * 2_400.0)) as usize + 1;
        let fee_rate = (1.5 + (-self.random.unit().ln()) * 6.0).min(80.0);
        let fees = (fee_rate * weight as f64 / 4.0) as u64;
        let interval = (-self.random.unit().ln() * Self::TARGET) as i64;
        let previous = std::mem::replace(&mut self.hash, Self::hash(&mut self.random));
        self.height += 1;
        self.time += interval.max(1);
        let json = format!(
            r#"{{"height":{},"hash":"{}","previous":"{previous}","time":{},"difficulty":null,"transactions":{transactions},"weight":{weight},"subsidy":{},"fees":{fees}}}"#,
            self.height,
            self.hash,
            self.time,
            Self::SUBSIDY
        );
        serde_json::from_str(&json).unwrap_or_else(|_| unreachable!("the simulator writes valid block json"))
    }
}

#[cfg(test)]
mod tests {
    use super::BlockSimulator;

    #[test]
    fn each_block_names_the_previous_one_and_counts_up() {
        let mut chain = BlockSimulator::new(4);
        let blocks: Vec<_> = (0..20).map(|_| chain.next_block()).collect();

        assert!(blocks.windows(2).all(|pair| pair[1].previous == pair[0].hash && pair[1].height == pair[0].height + 1 && pair[1].time > pair[0].time));
    }

    #[test]
    fn blocks_look_like_mainnet_blocks() {
        let mut chain = BlockSimulator::new(4);
        let blocks: Vec<_> = (0..200).map(|_| chain.next_block()).collect();

        assert!(blocks.iter().all(|block| block.hash.len() == 64 && block.hash.starts_with(&"0".repeat(18))));
        assert!(blocks.iter().all(|block| block.fullness() > 0.2 && block.fullness() < 1.0));
        assert!(blocks.iter().filter(|block| block.fullness() > 0.95).count() > 140);
        assert!(blocks.iter().all(|block| block.fee_rate().is_some_and(|rate| rate > 1.0 && rate <= 80.5)));
    }
}
