use mantle::difficulty::BlockHeight;

use crate::{BlockDigest, NoteClass, NotePayload};

/// The newest blocks the daemon has read, newest first, streamed live under the window `blocks`.
/// A block read again at a height already held replaces it, so a reorg leaves no stale entry.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecentBlocks {
    digests: Vec<BlockDigest>,
}

impl RecentBlocks {
    pub const WINDOW: &'static str = "blocks";
    pub const KEEP: usize = 6;

    pub fn record(&mut self, digest: BlockDigest) {
        self.digests.retain(|held| held.height != digest.height);
        let at = self.digests.partition_point(|held| held.height > digest.height);
        self.digests.insert(at, digest);
        self.digests.truncate(Self::KEEP);
    }

    #[must_use]
    pub fn digests(&self) -> &[BlockDigest] {
        &self.digests
    }

    #[must_use]
    pub fn tip(&self) -> Option<BlockHeight> {
        self.digests.first().map(|digest| digest.height)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.digests.is_empty()
    }
}

impl NotePayload for RecentBlocks {
    fn window(&self) -> &str {
        Self::WINDOW
    }

    fn label(&self) -> &str {
        Self::WINDOW
    }

    fn price(&self) -> f64 {
        f64::NAN
    }

    fn class(&self) -> NoteClass {
        NoteClass::Live
    }

    fn is_publishable(&self) -> bool {
        !self.digests.is_empty()
    }

    fn to_json(&self) -> String {
        let blocks: Vec<String> = self.digests.iter().map(BlockDigest::to_json).collect();
        format!("{{\"window\":\"{}\",\"blocks\":[{}]}}", Self::WINDOW, blocks.join(","))
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::BlockHeight;

    use super::RecentBlocks;
    use crate::{BlockDigest, NoteClass, NotePayload};

    struct RecentFixture;

    impl RecentFixture {
        fn digest(height: u32, hash: &str) -> BlockDigest {
            BlockDigest {
                height: BlockHeight::new(height),
                hash: hash.to_string(),
                previous: String::new(),
                time: height,
                version: 1,
                difficulty: 1.0,
                transactions: 1,
                outputs: 1,
                size: 100,
                weight: 400,
                output_value: Satoshi::new(0),
                coinbase_value: Satoshi::new(0),
                subsidy: Satoshi::new(0),
            }
        }
    }

    #[test]
    fn the_newest_blocks_are_kept_newest_first() {
        let mut recent = RecentBlocks::default();
        for height in [5, 3, 9, 4, 8, 6, 7, 1] {
            recent.record(RecentFixture::digest(height, "x"));
        }

        let heights: Vec<u32> = recent.digests().iter().map(|digest| digest.height.to_u32()).collect();
        assert_eq!(heights, vec![9, 8, 7, 6, 5, 4]);
        assert_eq!(recent.tip(), Some(BlockHeight::new(9)));
    }

    #[test]
    fn a_block_read_again_replaces_the_one_at_its_height() {
        let mut recent = RecentBlocks::default();
        recent.record(RecentFixture::digest(10, "old"));
        recent.record(RecentFixture::digest(10, "new"));

        assert_eq!(recent.digests().len(), 1);
        assert_eq!(recent.digests()[0].hash, "new");
    }

    #[test]
    fn the_note_is_live_and_lists_every_block() {
        let mut recent = RecentBlocks::default();
        assert!(!recent.is_publishable());
        recent.record(RecentFixture::digest(2, "b"));
        recent.record(RecentFixture::digest(1, "a"));
        let json: serde_json::Value = serde_json::from_str(&recent.to_json()).expect("json");

        assert_eq!(recent.class(), NoteClass::Live);
        assert_eq!(json["blocks"][0]["hash"], "b");
        assert_eq!(json["blocks"].as_array().map(Vec::len), Some(2));
    }
}
