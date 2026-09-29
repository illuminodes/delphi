use std::path::PathBuf;

use crate::{DaemonConfig, KeyStore, PriceSigner, PriceSignerError};

pub struct KeySource {
    environment: Option<String>,
    key_file: Option<PathBuf>,
    default_file: PathBuf,
    throwaway: bool,
}

impl KeySource {
    pub const DEFAULT_FILE: &'static str = "oracled.nsec";

    #[must_use]
    pub fn new(config: &DaemonConfig, environment: Option<String>) -> Self {
        Self {
            environment: environment.filter(|value| !value.trim().is_empty()),
            key_file: config.key_file.clone(),
            default_file: config.data_dir.join(Self::DEFAULT_FILE),
            throwaway: config.dry_run && !config.print_npub,
        }
    }

    /// # Errors
    /// Returns [`PriceSignerError::Key`] when the chosen source holds no
    /// usable key or a new key cannot be stored.
    pub fn resolve(&self) -> Result<PriceSigner, PriceSignerError> {
        if let Some(secret) = &self.environment {
            return PriceSigner::from_secret(secret);
        }
        if let Some(path) = &self.key_file {
            return KeyStore::new(path.clone()).load_or_create();
        }
        if self.throwaway {
            return Ok(PriceSigner::generate());
        }
        KeyStore::new(self.default_file.clone()).load_or_create()
    }

    #[must_use]
    pub fn describe(&self) -> String {
        if self.environment.is_some() {
            return "environment".to_string();
        }
        if let Some(path) = &self.key_file {
            return path.display().to_string();
        }
        if self.throwaway {
            return "throwaway dry-run key".to_string();
        }
        self.default_file.display().to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::KeySource;
    use crate::DaemonConfig;

    struct SourceFixture;

    impl SourceFixture {
        fn config(arguments: &[&str], root: &str) -> DaemonConfig {
            let mut all = vec!["--data-dir", root];
            all.extend_from_slice(arguments);
            DaemonConfig::parse(all).expect("parses")
        }

        fn root(label: &str) -> PathBuf {
            std::env::temp_dir().join(format!("oracled-source-{}-{label}", std::process::id()))
        }
    }

    const HEX_ONE: &str = "0000000000000000000000000000000000000000000000000000000000000001";

    #[test]
    fn the_environment_wins_over_every_file() {
        let root = SourceFixture::root("env");
        let config = SourceFixture::config(&["--key-file", "/nonexistent/none.nsec"], root.to_str().expect("utf8"));

        let source = KeySource::new(&config, Some(HEX_ONE.to_string()));

        assert_eq!(source.describe(), "environment");
        assert!(source.resolve().expect("key").npub().expect("npub").starts_with("npub1"));
        assert!(!root.exists());
    }

    #[test]
    fn a_blank_environment_value_is_ignored() {
        let root = SourceFixture::root("blank");
        let config = SourceFixture::config(&["--dry-run"], root.to_str().expect("utf8"));

        let source = KeySource::new(&config, Some("  \n".to_string()));

        assert_eq!(source.describe(), "throwaway dry-run key");
    }

    #[test]
    fn a_dry_run_without_a_key_source_is_throwaway_and_writes_nothing() {
        let root = SourceFixture::root("dry");
        let config = SourceFixture::config(&["--dry-run"], root.to_str().expect("utf8"));
        let source = KeySource::new(&config, None);

        let first = source.resolve().expect("first").npub().expect("npub");
        let second = source.resolve().expect("second").npub().expect("npub");

        assert_ne!(first, second);
        assert!(!root.exists());
    }

    #[test]
    fn a_live_run_keeps_one_identity_in_the_data_directory() {
        let root = SourceFixture::root("live");
        let config = SourceFixture::config(&[], root.to_str().expect("utf8"));
        let source = KeySource::new(&config, None);

        let first = source.resolve().expect("first").npub().expect("npub");
        let second = source.resolve().expect("second").npub().expect("npub");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(first, second);
    }

    #[test]
    fn printing_the_npub_uses_the_persistent_key_even_in_a_dry_run() {
        let root = SourceFixture::root("print");
        let config = SourceFixture::config(&["--dry-run", "--print-npub"], root.to_str().expect("utf8"));
        let source = KeySource::new(&config, None);

        let first = source.resolve().expect("first").npub().expect("npub");
        let second = source.resolve().expect("second").npub().expect("npub");
        let described = source.describe();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(first, second);
        assert!(described.ends_with(KeySource::DEFAULT_FILE), "{described}");
    }

    #[test]
    fn an_explicit_key_file_is_used_instead_of_the_data_directory() {
        let root = SourceFixture::root("file");
        let file = root.join("elsewhere.nsec");
        let config = SourceFixture::config(
            &["--key-file", file.to_str().expect("utf8")],
            root.join("data").to_str().expect("utf8"),
        );
        let source = KeySource::new(&config, None);

        let created = source.resolve().expect("key").npub().expect("npub");
        let exists = file.exists();
        let data_dir_untouched = !root.join("data").exists();
        let _ = std::fs::remove_dir_all(&root);

        assert!(created.starts_with("npub1"));
        assert!(exists);
        assert!(data_dir_untouched);
    }
}
