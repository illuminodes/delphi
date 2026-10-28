use crate::{ChainBlock, ChainBlocks};

/// One block in the chain view: what it held, how deep it is and whether the oracle has settled it.
#[derive(Debug, Clone, PartialEq)]
pub struct StripBlock {
    pub height: u32,
    pub confirmations: u32,
    pub settled: bool,
    pub fee_rate: Option<f64>,
    pub fullness: f64,
    pub time: i64,
    pub transactions: usize,
    pub fees: u64,
    pub votes: Option<usize>,
}

impl StripBlock {
    /// Blocks with this many votes or fewer told the oracle almost nothing.
    pub const FEW_VOTES: f64 = 10.0;
    /// Blocks with this many votes or more are as good as blocks get.
    pub const MANY_VOTES: f64 = 500.0;

    /// How much the block told the oracle, from 0 to 1: its votes (outputs whose implied price
    /// agreed with the hourly price) on a log scale from [`Self::FEW_VOTES`] to
    /// [`Self::MANY_VOTES`]. `None` when the daemon did not report votes.
    #[must_use]
    pub fn vote_quality(&self) -> Option<f64> {
        let votes = self.votes? as f64;
        let span = (Self::MANY_VOTES / Self::FEW_VOTES).ln();
        Some(((votes / Self::FEW_VOTES).max(1.0).ln() / span).clamp(0.0, 1.0))
    }
}

/// The newest blocks the oracle read, newest first, the way a block explorer lines them up.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStrip {
    blocks: Vec<StripBlock>,
}

impl BlockStrip {
    /// Confirmations the oracle waits for before a block counts toward a settled price.
    pub const SETTLE_DEPTH: u32 = 6;

    /// `tip` and `settled` come from the oracle's status note; without them the tip is the
    /// newest block listed and a block is settled once it is `SETTLE_DEPTH` deep.
    #[must_use]
    pub fn new(blocks: &ChainBlocks, tip: Option<u32>, settled: Option<u32>) -> Self {
        let newest = blocks.latest().map_or(0, |block| block.height);
        let tip = tip.unwrap_or(newest).max(newest);
        let settled = settled.unwrap_or_else(|| tip.saturating_sub(Self::SETTLE_DEPTH - 1));
        let mut strip: Vec<StripBlock> = blocks
            .blocks()
            .iter()
            .map(|block| Self::entry(block, tip, settled))
            .collect();
        strip.sort_by_key(|block| std::cmp::Reverse(block.height));
        Self { blocks: strip }
    }

    fn entry(block: &ChainBlock, tip: u32, settled: u32) -> StripBlock {
        StripBlock {
            height: block.height,
            confirmations: tip.saturating_sub(block.height) + 1,
            settled: block.height <= settled,
            fee_rate: block.fee_rate(),
            fullness: block.fullness().clamp(0.0, 1.0),
            time: block.time,
            transactions: block.transactions,
            fees: block.fees,
            votes: block.filter.inside_band,
        }
    }

    #[must_use]
    pub fn blocks(&self) -> &[StripBlock] {
        &self.blocks
    }

    /// How many blocks in the strip still wait to settle.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.blocks.iter().filter(|block| !block.settled).count()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNoteBuilder;

    use super::BlockStrip;
    use crate::{ChainBlocks, NoteKind};

    struct StripFixture;

    impl StripFixture {
        fn blocks(heights: &[u32]) -> ChainBlocks {
            let list: Vec<String> = heights
                .iter()
                .map(|height| {
                    format!(
                        r#"{{"height":{height},"hash":"00ab","time":{},"transactions":2,"weight":400000,"fees":10000}}"#,
                        1_780_000_000 + i64::from(*height) * 600
                    )
                })
                .collect();
            let note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .content(format!(r#"{{"blocks":[{}]}}"#, list.join(",")))
                .tag("w", ChainBlocks::WINDOW)
                .build();
            ChainBlocks::from_note(&note).expect("blocks")
        }
    }

    #[test]
    fn the_strip_runs_newest_to_oldest_and_counts_confirmations_from_the_tip() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[101, 100, 102]), Some(103), None);
        let heights: Vec<u32> = strip.blocks().iter().map(|block| block.height).collect();
        let depths: Vec<u32> = strip.blocks().iter().map(|block| block.confirmations).collect();

        assert_eq!(heights, vec![102, 101, 100]);
        assert_eq!(depths, vec![2, 3, 4]);
    }

    #[test]
    fn the_reported_settled_height_decides_what_is_settled() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[100, 101, 102]), Some(102), Some(100));
        let settled: Vec<bool> = strip.blocks().iter().map(|block| block.settled).collect();

        assert_eq!(settled, vec![false, false, true]);
        assert_eq!(strip.pending(), 2);
    }

    #[test]
    fn without_a_status_a_block_settles_at_six_confirmations() {
        let heights: Vec<u32> = (100..=107).collect();
        let strip = BlockStrip::new(&StripFixture::blocks(&heights), None, None);
        let mut settled: Vec<u32> = strip.blocks().iter().filter(|block| block.settled).map(|block| block.height).collect();
        settled.sort_unstable();

        assert_eq!(settled, vec![100, 101, 102]);
        assert!(strip.blocks().iter().all(|block| block.settled == (block.confirmations >= BlockStrip::SETTLE_DEPTH)));
    }

    #[test]
    fn a_stale_status_tip_never_hides_a_newer_block() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[100, 101]), Some(99), None);

        assert_eq!(strip.blocks().first().map(|block| block.confirmations), Some(1));
    }

    #[test]
    fn vote_quality_rises_on_a_log_scale_between_few_and_many_votes() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[100]), None, None);
        let mut block = strip.blocks()[0].clone();
        let quality = |block: &super::StripBlock| block.vote_quality().expect("quality");

        block.votes = Some(3);
        assert!(quality(&block).abs() < f64::EPSILON);
        block.votes = Some(5_000);
        assert!((quality(&block) - 1.0).abs() < f64::EPSILON);
        block.votes = Some(70);
        assert!((quality(&block) - 0.5).abs() < 0.01);
        block.votes = None;
        assert!(block.vote_quality().is_none());
    }
}
