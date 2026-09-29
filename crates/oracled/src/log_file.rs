use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

/// Append-mode destination for everything the process writes to stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogFile {
    path: PathBuf,
}

impl LogFile {
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// # Errors
    /// Returns the I/O error if the file cannot be opened or stderr cannot be
    /// pointed at it.
    pub fn redirect_stderr(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        rustix::stdio::dup2_stderr(&file)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::LogFile;

    #[test]
    fn an_unwritable_location_is_an_error() {
        let log = LogFile::new(&PathBuf::from("/proc/no-such-directory/oracled.log"));

        assert!(log.redirect_stderr().is_err());
    }
}
