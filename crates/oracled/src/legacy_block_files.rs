use std::fs;
use std::io;
use std::path::Path;

/// The raw block files earlier versions staged in the data directory before
/// turning blocks into records. Blocks now stay in memory, so these are
/// removed once on startup to give the space back.
#[derive(Debug, Clone, Copy)]
pub struct LegacyBlockFiles;

impl LegacyBlockFiles {
    const FILES: [&'static str; 2] = ["blocks.bin", "blocks.idx"];

    /// Delete the files that exist, returning the bytes freed.
    ///
    /// # Errors
    /// Returns an I/O error if a file exists but cannot be removed.
    pub fn remove(data_dir: &Path) -> io::Result<u64> {
        let mut freed = 0;
        for name in Self::FILES {
            let path = data_dir.join(name);
            match fs::metadata(&path) {
                Ok(metadata) => {
                    fs::remove_file(&path)?;
                    freed += metadata.len();
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
        }
        Ok(freed)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::LegacyBlockFiles;

    #[test]
    fn old_block_files_are_removed_and_counted_and_a_clean_directory_is_left_alone() {
        let dir = std::env::temp_dir().join(format!("oracled-legacy-blocks-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("blocks.bin"), [0; 90]).expect("write");
        fs::write(dir.join("blocks.idx"), [0; 12]).expect("write");
        fs::write(dir.join("records.log"), [0; 5]).expect("write");

        assert_eq!(LegacyBlockFiles::remove(&dir).expect("remove"), 102);
        assert_eq!(LegacyBlockFiles::remove(&dir).expect("remove again"), 0);
        assert!(dir.join("records.log").exists());
        fs::remove_dir_all(&dir).expect("clean up");
    }
}
