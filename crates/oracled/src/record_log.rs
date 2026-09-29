use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use mantle::difficulty::BlockHeight;
use mantle_oracle::AdmittedSample;

use crate::BlockRecord;

/// The newest `capacity` block records, contiguous by height, backed by an
/// append-only file that is rewritten once it holds twice that many.
///
/// Appending a height at or below the tip drops the records from that height
/// up, which is how a reorg replaces blocks. Appending past a gap discards
/// everything held, because a window must be contiguous.
pub struct RecordLog {
    path: PathBuf,
    capacity: usize,
    records: VecDeque<BlockRecord>,
    file: File,
    file_records: usize,
}

impl RecordLog {
    /// # Errors
    /// Returns an I/O error if the file cannot be read or opened for append.
    pub fn open(path: impl Into<PathBuf>, capacity: usize) -> io::Result<Self> {
        let path = path.into();
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error),
        };
        let mut records = VecDeque::new();
        let mut used = 0;
        let mut replayed = 0;
        while let Some((record, consumed)) = BlockRecord::decode_prefix(&bytes[used..]) {
            Self::place(&mut records, capacity, record);
            used += consumed;
            replayed += 1;
        }
        if used < bytes.len() {
            OpenOptions::new().write(true).open(&path)?.set_len(used as u64)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            capacity,
            records,
            file,
            file_records: replayed,
        })
    }

    /// # Errors
    /// Returns an I/O error if the record cannot be written or the file
    /// cannot be compacted.
    pub fn append(&mut self, record: BlockRecord) -> io::Result<()> {
        let mut bytes = Vec::new();
        record.encode_into(&mut bytes);
        self.file.write_all(&bytes)?;
        self.file.flush()?;
        self.file_records += 1;
        Self::place(&mut self.records, self.capacity, record);
        if self.file_records > self.capacity.saturating_mul(2) {
            self.compact()?;
        }
        Ok(())
    }

    fn place(records: &mut VecDeque<BlockRecord>, capacity: usize, record: BlockRecord) {
        if let Some(tip) = records.back().map(BlockRecord::height) {
            if record.height() <= tip {
                while records
                    .back()
                    .is_some_and(|held| held.height() >= record.height())
                {
                    records.pop_back();
                }
            } else if record.height().to_u32() != tip.to_u32() + 1 {
                records.clear();
            }
        }
        records.push_back(record);
        while records.len() > capacity {
            records.pop_front();
        }
    }

    fn compact(&mut self) -> io::Result<()> {
        let temporary = self.path.with_extension("tmp");
        let mut bytes = Vec::new();
        for record in &self.records {
            record.encode_into(&mut bytes);
        }
        let mut rewritten = File::create(&temporary)?;
        rewritten.write_all(&bytes)?;
        rewritten.sync_data()?;
        fs::rename(&temporary, &self.path)?;
        self.file = OpenOptions::new().append(true).open(&self.path)?;
        self.file_records = self.records.len();
        Ok(())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn tip(&self) -> Option<&BlockRecord> {
        self.records.back()
    }

    #[must_use]
    pub fn oldest_height(&self) -> Option<BlockHeight> {
        self.records.front().map(BlockRecord::height)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The outputs of the newest `blocks` records, or `None` while fewer are held.
    #[must_use]
    pub fn sample(&self, blocks: usize) -> Option<AdmittedSample> {
        if blocks == 0 || self.records.len() < blocks {
            return None;
        }
        let mut sample = AdmittedSample::new();
        for record in self.records.iter().skip(self.records.len() - blocks) {
            record.push_into(&mut sample);
        }
        Some(sample)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockHeight, BlockTime};

    use super::RecordLog;
    use crate::BlockRecord;

    struct LogFixture {
        dir: PathBuf,
    }

    impl LogFixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("oracled-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn path(&self) -> PathBuf {
            self.dir.join("records.log")
        }

        fn record(height: u32) -> BlockRecord {
            BlockRecord::new(
                BlockHeight::new(height),
                BlockTime::new(1_700_000_000 + height),
                vec![Satoshi::new(10_000 + u64::from(height))],
            )
        }

        fn heights(log: &RecordLog) -> Vec<u32> {
            log.sample(log.len()).map_or_else(Vec::new, |sample| {
                sample
                    .outputs()
                    .iter()
                    .map(|output| output.height.to_u32())
                    .collect()
            })
        }
    }

    impl Drop for LogFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn keeps_only_the_newest_capacity_records() {
        let fixture = LogFixture::new("cap");
        let mut log = RecordLog::open(fixture.path(), 3).expect("open");
        for height in 1..=5 {
            log.append(LogFixture::record(height)).expect("append");
        }

        assert_eq!(log.len(), 3);
        assert_eq!(log.oldest_height(), Some(BlockHeight::new(3)));
        assert_eq!(log.tip().map(BlockRecord::height), Some(BlockHeight::new(5)));
    }

    #[test]
    fn a_reopened_log_holds_the_same_records() {
        let fixture = LogFixture::new("reopen");
        {
            let mut log = RecordLog::open(fixture.path(), 4).expect("open");
            for height in 10..=13 {
                log.append(LogFixture::record(height)).expect("append");
            }
        }
        let log = RecordLog::open(fixture.path(), 4).expect("reopen");

        assert_eq!(LogFixture::heights(&log), vec![10, 11, 12, 13]);
    }

    #[test]
    fn a_torn_tail_is_dropped_and_the_log_stays_appendable() {
        let fixture = LogFixture::new("torn");
        {
            let mut log = RecordLog::open(fixture.path(), 4).expect("open");
            log.append(LogFixture::record(1)).expect("append");
            log.append(LogFixture::record(2)).expect("append");
        }
        let mut bytes = fs::read(fixture.path()).expect("read");
        bytes.extend_from_slice(&[9, 9, 9, 9, 9]);
        fs::write(fixture.path(), &bytes).expect("write");

        let mut log = RecordLog::open(fixture.path(), 4).expect("reopen");
        assert_eq!(log.len(), 2);
        log.append(LogFixture::record(3)).expect("append");
        drop(log);
        let log = RecordLog::open(fixture.path(), 4).expect("reopen again");

        assert_eq!(LogFixture::heights(&log), vec![1, 2, 3]);
    }

    #[test]
    fn a_reorg_replaces_the_blocks_from_the_new_height() {
        let fixture = LogFixture::new("reorg");
        let mut log = RecordLog::open(fixture.path(), 10).expect("open");
        for height in 1..=5 {
            log.append(LogFixture::record(height)).expect("append");
        }
        let replacement = BlockRecord::new(
            BlockHeight::new(4),
            BlockTime::new(9),
            vec![Satoshi::new(77_000)],
        );
        log.append(replacement).expect("append");

        assert_eq!(log.len(), 4);
        assert_eq!(log.tip().map(BlockRecord::height), Some(BlockHeight::new(4)));
        assert_eq!(log.tip().map(|record| record.amounts()[0]), Some(Satoshi::new(77_000)));
    }

    #[test]
    fn a_gap_discards_the_older_records() {
        let fixture = LogFixture::new("gap");
        let mut log = RecordLog::open(fixture.path(), 10).expect("open");
        log.append(LogFixture::record(1)).expect("append");
        log.append(LogFixture::record(2)).expect("append");
        log.append(LogFixture::record(9)).expect("append");

        assert_eq!(log.len(), 1);
        assert_eq!(log.oldest_height(), Some(BlockHeight::new(9)));
    }

    #[test]
    fn the_file_stays_bounded_across_many_appends() {
        let fixture = LogFixture::new("bounded");
        let mut log = RecordLog::open(fixture.path(), 5).expect("open");
        for height in 1..=200 {
            log.append(LogFixture::record(height)).expect("append");
        }
        let one_record = 12 + 8;
        let size = fs::metadata(fixture.path()).expect("stat").len();

        assert!(size <= (5 * 2 + 1) * one_record, "file grew to {size}");
        drop(log);
        let log = RecordLog::open(fixture.path(), 5).expect("reopen");
        assert_eq!(log.oldest_height(), Some(BlockHeight::new(196)));
        assert_eq!(log.len(), 5);
    }

    #[test]
    fn a_sample_needs_the_full_block_count() {
        let fixture = LogFixture::new("sample");
        let mut log = RecordLog::open(fixture.path(), 10).expect("open");
        for height in 1..=4 {
            log.append(LogFixture::record(height)).expect("append");
        }

        assert!(log.sample(5).is_none());
        assert!(log.sample(0).is_none());
        let sample = log.sample(2).expect("two blocks held");
        assert_eq!(sample.len(), 2);
        assert_eq!(sample.outputs()[0].height, BlockHeight::new(3));
    }
}
