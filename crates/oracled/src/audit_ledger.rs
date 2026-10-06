use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use mantle::difficulty::BlockHeight;
use mantle::hash::Hash256;

use crate::ChainWindow;

/// What the ledger knows about one window at one height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuditStatus {
    /// Never published.
    New,
    /// Published for this very block.
    Done,
    /// Published for another block at this height, as note `id`.
    Replaces(String),
}

/// The audit notes already published, one per window and height, so a
/// restart resumes where it stopped and a reorg can point at what it replaces.
///
/// Each publication is appended as one line; the file is rewritten without
/// superseded or excess lines when opened.
pub struct AuditLedger {
    path: Option<PathBuf>,
    entries: BTreeMap<(ChainWindow, u32), (Hash256, String)>,
    file: Option<File>,
}

impl AuditLedger {
    pub const MAX_ENTRIES: usize = 8_192;

    /// A ledger that forgets everything when dropped.
    #[must_use]
    pub const fn volatile() -> Self {
        Self {
            path: None,
            entries: BTreeMap::new(),
            file: None,
        }
    }

    /// # Errors
    /// Returns an I/O error if the file cannot be read or rewritten.
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error),
        };
        let mut ledger = Self {
            path: Some(path),
            entries: BTreeMap::new(),
            file: None,
        };
        for line in text.lines() {
            if let Some((window, height, hash, id)) = Self::parse(line) {
                ledger.entries.insert((window, height), (hash, id));
            }
        }
        ledger.trim();
        ledger.rewrite()?;
        Ok(ledger)
    }

    fn parse(line: &str) -> Option<(ChainWindow, u32, Hash256, String)> {
        let mut fields = line.split_whitespace();
        let window = ChainWindow::from_tag(fields.next()?)?;
        let height = fields.next()?.parse().ok()?;
        let hash = Hash256::from_display_hex(fields.next()?).ok()?;
        let id = fields.next()?.to_string();
        Some((window, height, hash, id))
    }

    fn line(window: ChainWindow, height: u32, hash: Hash256, id: &str) -> String {
        format!("{} {height} {} {id}\n", window.tag(), hash.to_display_hex())
    }

    fn rewrite(&mut self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let temporary = path.with_extension("tmp");
        let mut text = String::new();
        for ((window, height), (hash, id)) in &self.entries {
            text.push_str(&Self::line(*window, *height, *hash, id));
        }
        fs::write(&temporary, text)?;
        fs::rename(&temporary, path)?;
        self.file = Some(OpenOptions::new().append(true).open(path)?);
        Ok(())
    }

    fn trim(&mut self) {
        while self.entries.len() > Self::MAX_ENTRIES {
            let oldest = self
                .entries
                .keys()
                .min_by_key(|(_, height)| *height)
                .copied();
            if let Some(key) = oldest {
                self.entries.remove(&key);
            }
        }
    }

    #[must_use]
    pub fn status(&self, window: ChainWindow, height: BlockHeight, hash: Hash256) -> AuditStatus {
        match self.entries.get(&(window, height.to_u32())) {
            None => AuditStatus::New,
            Some((held, _)) if *held == hash => AuditStatus::Done,
            Some((_, id)) => AuditStatus::Replaces(id.clone()),
        }
    }

    /// # Errors
    /// Returns an I/O error if the line cannot be appended.
    pub fn record(
        &mut self,
        window: ChainWindow,
        height: BlockHeight,
        hash: Hash256,
        id: &str,
    ) -> io::Result<()> {
        if let Some(file) = &mut self.file {
            file.write_all(Self::line(window, height.to_u32(), hash, id).as_bytes())?;
        }
        self.entries
            .insert((window, height.to_u32()), (hash, id.to_string()));
        self.trim();
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use mantle::difficulty::BlockHeight;
    use mantle::hash::Hash256;

    use super::{AuditLedger, AuditStatus};
    use crate::ChainWindow;

    struct LedgerFixture {
        dir: PathBuf,
    }

    impl LedgerFixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("oracled-ledger-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn path(&self) -> PathBuf {
            self.dir.join("audit.ledger")
        }

        fn hash(seed: u8) -> Hash256 {
            Hash256::hash(&[seed])
        }
    }

    impl Drop for LedgerFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_publication_is_done_for_its_block_and_replaced_for_another() {
        let mut ledger = AuditLedger::volatile();
        let height = BlockHeight::new(10);
        ledger
            .record(ChainWindow::Hourly, height, LedgerFixture::hash(1), "id1")
            .expect("record");

        assert_eq!(ledger.status(ChainWindow::Hourly, height, LedgerFixture::hash(1)), AuditStatus::Done);
        assert_eq!(
            ledger.status(ChainWindow::Hourly, height, LedgerFixture::hash(2)),
            AuditStatus::Replaces("id1".to_string())
        );
        assert_eq!(ledger.status(ChainWindow::Daily, height, LedgerFixture::hash(1)), AuditStatus::New);
    }

    #[test]
    fn a_reopened_ledger_keeps_the_newest_entry_per_height() {
        let fixture = LedgerFixture::new("reopen");
        {
            let mut ledger = AuditLedger::open(fixture.path()).expect("open");
            let height = BlockHeight::new(5);
            ledger.record(ChainWindow::Daily, height, LedgerFixture::hash(1), "old").expect("record");
            ledger.record(ChainWindow::Daily, height, LedgerFixture::hash(2), "new").expect("record");
            ledger
                .record(ChainWindow::Hourly, BlockHeight::new(6), LedgerFixture::hash(3), "other")
                .expect("record");
        }
        let ledger = AuditLedger::open(fixture.path()).expect("reopen");

        assert_eq!(ledger.len(), 2);
        assert_eq!(
            ledger.status(ChainWindow::Daily, BlockHeight::new(5), LedgerFixture::hash(2)),
            AuditStatus::Done
        );
        assert_eq!(fs::read_to_string(fixture.path()).expect("read").lines().count(), 2);
    }

    #[test]
    fn unreadable_lines_are_skipped() {
        let fixture = LedgerFixture::new("garbage");
        fs::write(
            fixture.path(),
            format!("junk\nchain-1w 1 {} x\nchain-1h 7 {} id\n", LedgerFixture::hash(1), LedgerFixture::hash(2)),
        )
        .expect("write");

        let ledger = AuditLedger::open(fixture.path()).expect("open");

        assert_eq!(ledger.len(), 1);
        assert_eq!(
            ledger.status(ChainWindow::Hourly, BlockHeight::new(7), LedgerFixture::hash(2)),
            AuditStatus::Done
        );
    }

    #[test]
    fn the_oldest_heights_are_dropped_past_the_limit() {
        let mut ledger = AuditLedger::volatile();
        let total = u32::try_from(AuditLedger::MAX_ENTRIES).expect("fits") + 3;
        for height in 0..total {
            ledger
                .record(ChainWindow::Hourly, BlockHeight::new(height), LedgerFixture::hash(0), "id")
                .expect("record");
        }

        assert_eq!(ledger.len(), AuditLedger::MAX_ENTRIES);
        assert_eq!(
            ledger.status(ChainWindow::Hourly, BlockHeight::new(2), LedgerFixture::hash(0)),
            AuditStatus::New
        );
    }
}
