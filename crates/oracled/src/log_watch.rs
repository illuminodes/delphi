use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Follows the daemon's log file and counts the lines that show progress.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogWatch {
    path: PathBuf,
    offset: u64,
    partial: String,
    pub last_chain: Option<String>,
    pub last_headers: Option<String>,
    pub chain_passes: u64,
    pub sync_failures: u64,
    pub published: u64,
    pub rejected: u64,
    pub undelivered: u64,
    pub mempool_reconnects: u64,
    pub fatal: Option<String>,
}

impl LogWatch {
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            ..Self::default()
        }
    }

    /// Read whatever the daemon appended since the last call.
    ///
    /// # Errors
    /// Returns the I/O error if the log exists but cannot be read.
    pub fn poll(&mut self) -> io::Result<()> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        file.seek(SeekFrom::Start(self.offset))?;
        let mut fresh = Vec::new();
        self.offset += file.read_to_end(&mut fresh)? as u64;
        self.partial.push_str(&String::from_utf8_lossy(&fresh));
        let complete = self.partial.rfind('\n').map_or(0, |end| end + 1);
        let lines: String = self.partial.drain(..complete).collect();
        lines.lines().for_each(|line| self.observe(line));
        Ok(())
    }

    /// Count one log line.
    pub fn observe(&mut self, line: &str) {
        let line = line.trim_start();
        if line.starts_with("headers: got ") {
            self.last_headers = Some(line.to_string());
        } else if line.starts_with("chain tip ") && line.contains(", settled ") {
            self.chain_passes += 1;
            self.last_chain = Some(line.to_string());
        } else if line.starts_with("chain sync failed") || line.starts_with("chain sync cannot start") {
            self.sync_failures += 1;
        } else if line.starts_with("published ") {
            self.published += 1;
        } else if line.starts_with("rejected ") {
            self.rejected += 1;
        } else if line.starts_with("undelivered ") {
            self.undelivered += 1;
        } else if line.starts_with("mempool: every peer dropped") {
            self.mempool_reconnects += 1;
        } else if line.starts_with("fatal: ") {
            self.fatal = Some(line.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LogWatch;

    #[test]
    fn progress_lines_are_counted() {
        let mut watch = LogWatch::default();
        for line in [
            "chain tip 970000, settled nothing yet (1008 blocks read, 1008 held)",
            "chain tip 970001, settled 969995 (1 blocks read, 1008 held)",
            "chain sync failed: survey peers: timed out",
            "published hourly@969995 (Audit abc)",
            "rejected daily: no samples",
            "undelivered weekly: closed",
            "mempool: every peer dropped, reconnecting",
            "mempool: watching 8 peers",
            "fatal: lock data directory",
            "  headers: got 2000 in page, 4000 durable total (height 1..=4000)",
        ] {
            watch.observe(line);
        }

        assert_eq!(watch.chain_passes, 2);
        assert_eq!(
            watch.last_chain.as_deref(),
            Some("chain tip 970001, settled 969995 (1 blocks read, 1008 held)")
        );
        assert_eq!(watch.sync_failures, 1);
        assert_eq!(watch.published, 1);
        assert_eq!(watch.rejected, 1);
        assert_eq!(watch.undelivered, 1);
        assert_eq!(watch.mempool_reconnects, 1);
        assert_eq!(watch.fatal.as_deref(), Some("fatal: lock data directory"));
        assert_eq!(
            watch.last_headers.as_deref(),
            Some("headers: got 2000 in page, 4000 durable total (height 1..=4000)")
        );
    }

    #[test]
    fn a_line_split_across_polls_is_counted_once() {
        let path = std::env::temp_dir().join(format!("log-watch-{}.log", std::process::id()));
        std::fs::write(&path, "published a (Audit 1)\npubli").expect("write");
        let mut watch = LogWatch::new(&path);
        watch.poll().expect("poll");
        std::fs::write(&path, "published a (Audit 1)\npublished b (Audit 2)\n").expect("write");
        watch.poll().expect("poll");
        std::fs::remove_file(&path).expect("clean up");

        assert_eq!(watch.published, 2);
    }

    #[test]
    fn a_log_not_yet_written_is_not_an_error() {
        let mut watch = LogWatch::new(std::path::Path::new("/no/such/oracled.log"));

        assert!(watch.poll().is_ok());
        assert_eq!(watch.chain_passes, 0);
    }
}
