use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::{PriceSigner, PriceSignerError};

pub struct KeyStore {
    path: PathBuf,
}

impl KeyStore {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    const SHARED_BITS: u32 = 0o077;

    /// # Errors
    /// Returns [`PriceSignerError::Key`] when the file exists but holds no
    /// usable key or is open to its group or other users, or when a new key
    /// cannot be written.
    pub fn load_or_create(&self) -> Result<PriceSigner, PriceSignerError> {
        match fs::metadata(&self.path) {
            Ok(metadata) => {
                let mode = metadata.permissions().mode();
                if mode & Self::SHARED_BITS != 0 {
                    return Err(PriceSignerError::Key(format!(
                        "{} has mode {:o}; run chmod 600 on it",
                        Self::shown(&self.path),
                        mode & 0o777
                    )));
                }
                let secret = fs::read_to_string(&self.path).map_err(|error| self.failure(&error))?;
                PriceSigner::from_secret(&secret)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => self.create(),
            Err(error) => Err(self.failure(&error)),
        }
    }

    fn create(&self) -> Result<PriceSigner, PriceSignerError> {
        if let Some(parent) = self.path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|error| self.failure(&error))?;
        }
        let signer = PriceSigner::generate();
        let nsec = signer.nsec()?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&self.path)
            .map_err(|error| self.failure(&error))?;
        writeln!(file, "{nsec}").map_err(|error| self.failure(&error))?;
        Ok(signer)
    }

    fn failure(&self, error: &std::io::Error) -> PriceSignerError {
        PriceSignerError::Key(format!("{}: {error}", Self::shown(&self.path)))
    }

    fn shown(path: &Path) -> String {
        path.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::KeyStore;

    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("oracled-keys-{}-{nanos}", std::process::id()));
            Self { root }
        }

        fn key_path(&self) -> PathBuf {
            self.root.join("nested").join("oracled.nsec")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_missing_file_gets_a_fresh_key_that_is_reused() {
        let scratch = Scratch::new();
        let store = KeyStore::new(scratch.key_path());

        let first = store.load_or_create().expect("created");
        let second = store.load_or_create().expect("loaded");

        assert_eq!(first.npub().expect("npub"), second.npub().expect("npub"));
        assert!(first.npub().expect("npub").starts_with("npub1"));
    }

    #[test]
    fn the_new_file_is_readable_by_its_owner_only() {
        let scratch = Scratch::new();
        KeyStore::new(scratch.key_path()).load_or_create().expect("created");

        let mode = std::fs::metadata(scratch.key_path()).expect("metadata").permissions().mode();

        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_file_holding_garbage_is_refused_and_left_alone() {
        let scratch = Scratch::new();
        std::fs::create_dir_all(scratch.key_path().parent().expect("parent")).expect("dir");
        std::fs::write(scratch.key_path(), "not a key\n").expect("write");
        std::fs::set_permissions(scratch.key_path(), std::fs::Permissions::from_mode(0o600)).expect("chmod");

        let outcome = KeyStore::new(scratch.key_path()).load_or_create();

        assert!(outcome.is_err());
        assert_eq!(std::fs::read_to_string(scratch.key_path()).expect("read"), "not a key\n");
    }

    #[test]
    fn a_key_readable_by_others_is_refused() {
        let scratch = Scratch::new();
        let store = KeyStore::new(scratch.key_path());
        store.load_or_create().expect("created");
        std::fs::set_permissions(scratch.key_path(), std::fs::Permissions::from_mode(0o644)).expect("chmod");

        let refused = store.load_or_create().err().map(|error| error.to_string());

        assert!(refused.is_some_and(|message| message.contains("644")));
    }

    #[test]
    fn two_stores_on_different_files_hold_different_keys() {
        let scratch = Scratch::new();
        let first = KeyStore::new(scratch.root.join("a.nsec")).load_or_create().expect("a");
        let second = KeyStore::new(scratch.root.join("b.nsec")).load_or_create().expect("b");

        assert_ne!(first.npub().expect("npub"), second.npub().expect("npub"));
    }
}
