use crate::{ChainBlock, ChainBlocks};

/// One block in the strip: how deep it is and whether the oracle has settled it.
#[derive(Debug, Clone, PartialEq)]
pub struct StripBlock {
    pub height: u32,
    pub confirmations: u32,
    pub settled: bool,
    pub fee_rate: Option<f64>,
    pub fullness: f64,
    pub time: i64,
}

/// The newest blocks the oracle read, oldest first, so the tip sits at the right edge.
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
        strip.sort_by_key(|block| block.height);
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
    fn the_strip_runs_oldest_to_newest_and_counts_confirmations_from_the_tip() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[102, 100, 101]), Some(103), None);
        let heights: Vec<u32> = strip.blocks().iter().map(|block| block.height).collect();
        let depths: Vec<u32> = strip.blocks().iter().map(|block| block.confirmations).collect();

        assert_eq!(heights, vec![100, 101, 102]);
        assert_eq!(depths, vec![4, 3, 2]);
    }

    #[test]
    fn the_reported_settled_height_decides_what_is_settled() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[100, 101, 102]), Some(102), Some(100));
        let settled: Vec<bool> = strip.blocks().iter().map(|block| block.settled).collect();

        assert_eq!(settled, vec![true, false, false]);
        assert_eq!(strip.pending(), 2);
    }

    #[test]
    fn without_a_status_a_block_settles_at_six_confirmations() {
        let heights: Vec<u32> = (100..=107).collect();
        let strip = BlockStrip::new(&StripFixture::blocks(&heights), None, None);
        let settled: Vec<u32> = strip.blocks().iter().filter(|block| block.settled).map(|block| block.height).collect();

        assert_eq!(settled, vec![100, 101, 102]);
        assert!(strip.blocks().iter().all(|block| block.settled == (block.confirmations >= BlockStrip::SETTLE_DEPTH)));
    }

    #[test]
    fn a_stale_status_tip_never_hides_a_newer_block() {
        let strip = BlockStrip::new(&StripFixture::blocks(&[100, 101]), Some(99), None);

        assert_eq!(strip.blocks().last().map(|block| block.confirmations), Some(1));
    }
}
