use std::fs;
use std::path::{Path, PathBuf};

/// The size of the raw block files the fetcher appends to inside the data
/// directory.
///
/// The fetcher never deletes a block on its own, so the daemon bounds disk
/// use by asking it to discard them once they pass a limit. The headers file
/// is kept: rebuilding it means downloading every header since genesis.
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
    fn a_cache_at_the_limit_does_not_exceed_it() {
        let fixture = CacheFixture::new("edge");
        fixture.write("blocks.bin", 100);

        assert!(!BlockCache::new(&fixture.dir, 100).exceeds_limit());
    }
}
