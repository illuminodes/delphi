use crate::{ChainBlock, ChainBlocks};

/// The largest transaction count, fee rate and vote count among the listed blocks, so each
/// block's bars can be drawn against its neighbours. Fullness is drawn against the weight limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockScale {
    transactions: usize,
    fee_rate: f64,
    votes: usize,
}

impl BlockScale {
    #[must_use]
    pub fn of(blocks: &ChainBlocks) -> Self {
        let blocks = blocks.blocks();
        Self {
            transactions: blocks.iter().map(|block| block.transactions).max().unwrap_or(0),
            fee_rate: blocks.iter().filter_map(ChainBlock::fee_rate).fold(0.0, f64::max),
            votes: blocks.iter().filter_map(|block| block.filter.inside_band).max().unwrap_or(0),
        }
    }

    #[must_use]
    pub fn transactions(&self, block: &ChainBlock) -> f64 {
        Self::ratio(block.transactions as f64, self.transactions as f64)
    }

    #[must_use]
    pub fn fee_rate(&self, block: &ChainBlock) -> f64 {
        block.fee_rate().map_or(0.0, |rate| Self::ratio(rate, self.fee_rate))
    }

    #[must_use]
    pub fn votes(&self, block: &ChainBlock) -> f64 {
        block.filter.inside_band.map_or(0.0, |votes| Self::ratio(votes as f64, self.votes as f64))
    }

    fn ratio(value: f64, most: f64) -> f64 {
        if most > 0.0 { (value / most).clamp(0.0, 1.0) } else { 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNoteBuilder;

    use super::BlockScale;
    use crate::{ChainBlocks, NoteKind};

    struct ScaleFixture;

    impl ScaleFixture {
        fn blocks() -> ChainBlocks {
            let note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .tag("w", ChainBlocks::WINDOW)
                .content(
                    r#"{"blocks":[
                        {"height":2,"hash":"b","time":2,"transactions":4000,"weight":4000000,"fees":4000000,"filter":{"inside_band":200}},
                        {"height":1,"hash":"a","time":1,"transactions":1000,"weight":4000000,"fees":1000000}
                    ]}"#,
                )
                .build();
            ChainBlocks::from_note(&note).expect("blocks")
        }
    }

    #[test]
    fn each_block_is_measured_against_the_largest_listed() {
        let blocks = ScaleFixture::blocks();
        let scale = BlockScale::of(&blocks);
        let [newest, oldest] = blocks.blocks() else { panic!("two blocks") };

        assert!((scale.transactions(newest) - 1.0).abs() < 1e-9);
        assert!((scale.transactions(oldest) - 0.25).abs() < 1e-9);
        assert!((scale.fee_rate(oldest) - 0.25).abs() < 1e-9);
        assert!((scale.votes(newest) - 1.0).abs() < 1e-9);
        assert!(scale.votes(oldest).abs() < f64::EPSILON);
    }
}
