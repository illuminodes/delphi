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
    /// Fee rates at or above this many sat/vB get the last colour of the scale.
    const TOP_RATE: f64 = 300.0;

    /// The colour hue for the block's fee rate on a log scale: teal at 1 sat/vB, through yellow and
    /// red, to purple at [`Self::TOP_RATE`]. `None` when the block has no fee rate.
    #[must_use]
    pub fn fee_hue(&self) -> Option<f64> {
        let rate = self.fee_rate?;
        let step = (rate.max(1.0).ln() / Self::TOP_RATE.ln()).clamp(0.0, 1.0);
        Some((165.0 - step * 225.0).rem_euclid(360.0))
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
    fn the_fee_colour_runs_from_teal_through_red_to_purple() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[100]), None, None);
        let mut block = strip.blocks()[0].clone();
        let hue = |block: &super::StripBlock| block.fee_hue().expect("hue");

        block.fee_rate = Some(0.5);
        assert!((hue(&block) - 165.0).abs() < 1e-9);
        block.fee_rate = Some(1_000.0);
        assert!((hue(&block) - 300.0).abs() < 1e-9);
        block.fee_rate = Some(20.0);
        assert!(hue(&block) > 0.0 && hue(&block) < 165.0);
        block.fee_rate = None;
        assert!(block.fee_hue().is_none());
    }
}
