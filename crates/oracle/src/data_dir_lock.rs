use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

pub struct DataDirLock {
    _file: File,
}

impl DataDirLock {
    #[allow(clippy::missing_errors_doc)]
    pub fn acquire(data_dir: &Path) -> io::Result<Self> {
        let path = data_dir.join("oracle.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        file.try_lock().map_err(|error| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                format!(
                    "{} is already in use by another oracle process: {error}",
                    data_dir.display()
                ),
            )
        })?;
        Ok(Self { _file: file })
    }
}
