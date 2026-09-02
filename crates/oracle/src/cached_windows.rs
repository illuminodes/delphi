use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::primitives::BlockHeader;
use mantle_block_io::FileIoHandle;

use crate::data_dir_lock::DataDirLock;
use crate::{BlockRange, OraclePipeline, OracleReport, RangeSelection};

#[derive(Debug)]
pub enum CachedWindowError {
    Io(io::Error),
    Lock(String),
    NoHeaders,
    Range(crate::BlockRangeError),
    MissingBlock(BlockHeight),
    Pipeline(crate::OracleError),
}

impl fmt::Display for CachedWindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "cache io failed: {error}"),
            Self::Lock(message) => write!(formatter, "lock data directory: {message}"),
            Self::NoHeaders => formatter.write_str("no cached headers in the data directory"),
            Self::Range(error) => write!(formatter, "select block range: {error}"),
            Self::MissingBlock(height) => write!(formatter, "block {height} is not cached"),
            Self::Pipeline(error) => write!(formatter, "compute oracle report: {error}"),
        }
    }
}

impl std::error::Error for CachedWindowError {}

pub struct CachedWindows {
    data_dir: PathBuf,
    headers: Vec<(BlockHeight, BlockHeader)>,
    file_io: FileIoHandle,
    _lock: DataDirLock,
}

impl CachedWindows {
    /// Open the cached headers and blocks in `data_dir` without touching the
    /// network.
    ///
    /// # Errors
    /// Returns [`CachedWindowError`] if the directory cannot be locked, the
    /// cache cannot be opened, or it holds no headers.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, CachedWindowError> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let lock = DataDirLock::acquire(&data_dir)
            .map_err(|error| CachedWindowError::Lock(error.to_string()))?;
        let mut file_io = FileIoHandle::spawn(&data_dir).map_err(CachedWindowError::Io)?;
        let headers = file_io
            .load_cached_headers(BlockHeight::new(1))
            .map_err(CachedWindowError::Io)?;
        if headers.is_empty() {
            return Err(CachedWindowError::NoHeaders);
        }
        Ok(Self {
            data_dir,
            headers,
            file_io,
            _lock: lock,
        })
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn headers(&self) -> &[(BlockHeight, BlockHeader)] {
        &self.headers
    }

    /// The time span covered by the blocks that are actually cached on disk.
    #[must_use]
    pub fn cached_span(&mut self) -> Option<(BlockTime, BlockTime)> {
        let mut times = self
            .headers
            .iter()
            .filter(|(height, _)| self.file_io.has_block(*height))
            .map(|(_, header)| header.time());
        let first = times.next()?;
        let last = times.last().unwrap_or(first);
        Some((first, last))
    }

    #[must_use]
    pub fn cached_heights(&mut self) -> Vec<(BlockHeight, BlockHeader)> {
        self.headers
            .iter()
            .filter(|(height, _)| self.file_io.has_block(*height))
            .copied()
            .collect()
    }

    /// The longest run of consecutive cached heights.
    ///
    /// A data directory accumulates blocks from unrelated runs, so the first
    /// and last cached blocks can sit years apart with nothing in between.
    /// Only a contiguous run can be tiled into windows.
    #[must_use]
    pub fn longest_contiguous_run(&mut self) -> Vec<(BlockHeight, BlockHeader)> {
        let cached = self.cached_heights();
        let heights: Vec<BlockHeight> = cached.iter().map(|(height, _)| *height).collect();
        let longest = ContiguousRun::longest(&heights);
        let (Some(first), Some(last)) = (longest.first(), longest.last()) else {
            return Vec::new();
        };
        cached
            .into_iter()
            .filter(|(height, _)| height >= first && height <= last)
            .collect()
    }

    /// Run the oracle over a selection, reading every block from disk.
    ///
    /// # Errors
    /// Returns [`CachedWindowError`] if the selection resolves to no blocks,
    /// a block in the range is not cached, or the pipeline rejects the sample.
    pub fn run(&mut self, selection: RangeSelection) -> Result<OracleReport, CachedWindowError> {
        let range =
            BlockRange::select(&self.headers, selection).map_err(CachedWindowError::Range)?;
        let selected: Vec<(BlockHeight, BlockHeader)> =
            range.headers(&self.headers).to_vec();
        let mut pipeline = OraclePipeline::new();
        for (height, header) in &selected {
            if !self.file_io.has_block(*height) {
                return Err(CachedWindowError::MissingBlock(*height));
            }
            let block = self
                .file_io
                .read_block_decoded(*height, header.hash())
                .map_err(CachedWindowError::Io)?;
            for transaction in &block.transactions {
                pipeline.process(*height, header.time(), transaction);
            }
        }
        pipeline.finish().map_err(CachedWindowError::Pipeline)
    }
}

impl Drop for CachedWindows {
    fn drop(&mut self) {
        self.file_io.shutdown();
    }
}

/// Picks the longest consecutive run out of a sorted height list.
pub struct ContiguousRun;

impl ContiguousRun {
    #[must_use]
    pub fn longest(heights: &[BlockHeight]) -> &[BlockHeight] {
        let mut best: &[BlockHeight] = &[];
        let mut start = 0;
        for index in 1..=heights.len() {
            let broken = index == heights.len()
                || heights[index].to_u32() != heights[index - 1].to_u32() + 1;
            if broken {
                let run = &heights[start..index];
                if run.len() > best.len() {
                    best = run;
                }
                start = index;
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockHeight;

    use super::ContiguousRun;

    struct Heights;

    impl Heights {
        fn of(values: &[u32]) -> Vec<BlockHeight> {
            values.iter().copied().map(BlockHeight::new).collect()
        }
    }

    #[test]
    fn picks_the_longest_run_over_an_earlier_shorter_one() {
        let heights = Heights::of(&[10, 11, 12, 500, 900, 901, 902, 903]);
        let run = ContiguousRun::longest(&heights);

        assert_eq!(run.len(), 4);
        assert_eq!(run.first().copied(), Some(BlockHeight::new(900)));
        assert_eq!(run.last().copied(), Some(BlockHeight::new(903)));
    }

    #[test]
    fn a_fully_contiguous_list_is_its_own_run() {
        let heights = Heights::of(&[7, 8, 9]);
        assert_eq!(ContiguousRun::longest(&heights).len(), 3);
    }

    #[test]
    fn isolated_heights_yield_a_single_block_run() {
        let heights = Heights::of(&[1, 50, 100]);
        assert_eq!(ContiguousRun::longest(&heights).len(), 1);
    }

    #[test]
    fn an_empty_list_has_no_run() {
        assert!(ContiguousRun::longest(&[]).is_empty());
    }
}
