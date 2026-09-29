use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The raw block files the fetcher appends to inside the data directory.
///
/// The fetcher never deletes a block, so the daemon bounds disk use by
/// dropping the fetcher and purging these files once they pass a limit. The
/// headers file is left alone: rebuilding it means downloading every header
/// since genesis.
#[derive(Debug, Clone)]
pub struct BlockCache {
    data_dir: PathBuf,
    limit_bytes: u64,
}

impl BlockCache {
    const DATA_FILE: &'static str = "blocks.bin";
    const INDEX_FILE: &'static str = "blocks.idx";

    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>, limit_bytes: u64) -> Self {
        Self {
            data_dir: data_dir.into(),
            limit_bytes,
        }
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub const fn limit_bytes(&self) -> u64 {
        self.limit_bytes
    }

    #[must_use]
    pub fn bytes(&self) -> u64 {
        [Self::DATA_FILE, Self::INDEX_FILE]
            .iter()
            .filter_map(|name| fs::metadata(self.data_dir.join(name)).ok())
            .map(|metadata| metadata.len())
            .sum()
    }

    #[must_use]
    pub fn exceeds_limit(&self) -> bool {
        self.bytes() > self.limit_bytes
    }

    /// Delete the block data and its index. No fetcher may have them open.
    ///
    /// # Errors
    /// Returns an I/O error if a file exists but cannot be removed.
    pub fn purge(&self) -> io::Result<()> {
        for name in [Self::DATA_FILE, Self::INDEX_FILE] {
            match fs::remove_file(self.data_dir.join(name)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use super::BlockCache;

    struct CacheFixture {
        dir: PathBuf,
    }

    impl CacheFixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("oracled-cache-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn write(&self, name: &str, length: usize) {
            fs::write(self.dir.join(name), vec![0_u8; length]).expect("write");
        }
    }

    impl Drop for CacheFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn an_empty_directory_holds_no_bytes() {
        let fixture = CacheFixture::new("empty");
        let cache = BlockCache::new(&fixture.dir, 100);

        assert_eq!(cache.bytes(), 0);
        assert!(!cache.exceeds_limit());
    }

    #[test]
    fn block_data_and_index_both_count() {
        let fixture = CacheFixture::new("count");
        fixture.write("blocks.bin", 90);
        fixture.write("blocks.idx", 24);
        let cache = BlockCache::new(&fixture.dir, 100);

        assert_eq!(cache.bytes(), 114);
        assert!(cache.exceeds_limit());
    }

    #[test]
    fn purging_removes_only_the_block_files() {
        let fixture = CacheFixture::new("purge");
        fixture.write("blocks.bin", 10);
        fixture.write("blocks.idx", 10);
        fixture.write("chain_headers.bin", 10);
        let cache = BlockCache::new(&fixture.dir, 100);

        cache.purge().expect("purge");

        assert_eq!(cache.bytes(), 0);
        assert!(fixture.dir.join("chain_headers.bin").exists());
    }

    #[test]
    fn purging_an_empty_directory_succeeds() {
        let fixture = CacheFixture::new("noop");

        assert!(BlockCache::new(&fixture.dir, 1).purge().is_ok());
    }

    #[test]
    fn a_cache_at_the_limit_does_not_exceed_it() {
        let fixture = CacheFixture::new("edge");
        fixture.write("blocks.bin", 100);

        assert!(!BlockCache::new(&fixture.dir, 100).exceeds_limit());
    }
}
