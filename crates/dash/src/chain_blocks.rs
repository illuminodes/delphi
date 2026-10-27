use nostr_minions::NostrNote;
use serde::Deserialize;

use crate::{ChainBlock, NoteKind};

/// The newest blocks the daemon read, newest first, from its live `blocks` note.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ChainBlocks {
    blocks: Vec<ChainBlock>,
}

impl ChainBlocks {
    pub const WINDOW: &'static str = "blocks";

    #[must_use]
    pub fn from_note(note: &NostrNote) -> Option<Self> {
        if !NoteKind::carries_live(note.kind) || NoteKind::window(note) != Some(Self::WINDOW) {
            return None;
        }
        let mut blocks: Self = serde_json::from_str(&note.content).ok()?;
        blocks.blocks.sort_by_key(|block| std::cmp::Reverse(block.height));
        Some(blocks)
    }

    #[must_use]
    pub fn latest(&self) -> Option<&ChainBlock> {
        self.blocks.first()
    }

    #[must_use]
    pub fn blocks(&self) -> &[ChainBlock] {
        &self.blocks
    }

    /// Mean seconds between the blocks held, or `None` with fewer than two.
    #[must_use]
    pub fn mean_interval(&self) -> Option<f64> {
        let (newest, oldest) = (self.blocks.first()?, self.blocks.last()?);
        let gaps = self.blocks.len().checked_sub(1).filter(|gaps| *gaps > 0)?;
        Some((newest.time - oldest.time) as f64 / gaps as f64)
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::NostrNoteBuilder;

    use super::ChainBlocks;
    use crate::NoteKind;

    struct BlocksFixture;

    impl BlocksFixture {
        fn note(window: &str, content: &str) -> nostr_minions::NostrNote {
            NostrNoteBuilder::new().kind(NoteKind::LIVE).content(content).tag("w", window).build()
        }
    }

    #[test]
    fn a_blocks_note_is_read_newest_first() {
        let note = BlocksFixture::note(
            "blocks",
            r#"{"window":"blocks","blocks":[{"height":9,"hash":"a","time":1000,"transactions":2},{"height":10,"hash":"b","time":1600,"transactions":3},{"height":8,"hash":"c","time":400,"transactions":1}]}"#,
        );
        let blocks = ChainBlocks::from_note(&note).expect("blocks");

        assert_eq!(blocks.latest().map(|block| block.height), Some(10));
        assert_eq!(blocks.blocks().len(), 3);
        assert_eq!(blocks.mean_interval(), Some(600.0));
    }

    #[test]
    fn other_notes_and_a_single_block_interval_are_rejected() {
        assert!(ChainBlocks::from_note(&BlocksFixture::note("status", "{}")).is_none());
        let one = BlocksFixture::note(
            "blocks",
            r#"{"blocks":[{"height":1,"hash":"a","time":1,"transactions":1}]}"#,
        );
        assert_eq!(ChainBlocks::from_note(&one).expect("blocks").mean_interval(), None);
    }
}
