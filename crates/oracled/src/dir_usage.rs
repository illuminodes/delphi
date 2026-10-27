use std::fs;
use std::path::{Path, PathBuf};

/// Bytes held by the files under a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirUsage {
    root: PathBuf,
}

impl DirUsage {
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    /// Total size of the regular files under the root, without following
    /// symlinks. Entries that vanish or cannot be read count as zero.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        Self::under(&self.root)
    }

    fn under(path: &Path) -> u64 {
        let Ok(metadata) = fs::symlink_metadata(path) else {
            return 0;
        };
        if metadata.is_file() {
            return metadata.len();
        }
        if !metadata.is_dir() {
            return 0;
        }
        fs::read_dir(path).map_or(0, |entries| {
            entries.flatten().map(|entry| Self::under(&entry.path())).sum()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::DirUsage;

    #[test]
    fn nested_files_are_summed() {
        let root = std::env::temp_dir().join(format!("dir-usage-{}", std::process::id()));
        std::fs::create_dir_all(root.join("inner")).expect("mkdir");
        std::fs::write(root.join("a"), [0_u8; 100]).expect("write");
        std::fs::write(root.join("inner/b"), [0_u8; 23]).expect("write");

        let bytes = DirUsage::new(&root).bytes();
        std::fs::remove_dir_all(&root).expect("clean up");

        assert_eq!(bytes, 123);
    }

    #[test]
    fn a_missing_directory_is_empty() {
        assert_eq!(DirUsage::new(std::path::Path::new("/no/such/dir")).bytes(), 0);
    }
}
