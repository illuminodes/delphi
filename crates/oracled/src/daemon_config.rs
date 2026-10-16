use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::time::Duration;

use crate::{ChainWindow, TransactionStream};

/// Everything the daemon takes from its command line.
///
/// The signing key is deliberately absent: it comes from the environment so
/// it never appears in a process listing.
#[derive(Debug, Clone, PartialEq)]
pub struct DaemonConfig {
    pub relay_url: String,
    pub relay_insecure: bool,
    pub data_dir: PathBuf,
    pub peers: Vec<SocketAddrV4>,
    pub tolerance: f64,
    pub min_outputs: usize,
    pub draws: usize,
    pub mempool_peers: usize,
    pub history_blocks: usize,
    pub cache_limit_bytes: u64,
    pub tx_notes_per_second: u32,
    pub dry_run: bool,
    pub print_npub: bool,
    pub key_file: Option<PathBuf>,
    pub log_file: Option<PathBuf>,
    pub run_for: Option<Duration>,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            relay_url: "wss://rof.illuminodes.com".to_string(),
            relay_insecure: false,
            data_dir: PathBuf::from("data/oracled"),
            peers: Vec::new(),
            tolerance: 0.15,
            min_outputs: 200,
            draws: 30,
            mempool_peers: 8,
            history_blocks: 1_008,
            cache_limit_bytes: 1_000_000_000,
            tx_notes_per_second: TransactionStream::DEFAULT_PER_SECOND,
            dry_run: false,
            print_npub: false,
            key_file: None,
            log_file: None,
            run_for: None,
        }
    }
}

impl DaemonConfig {
    pub const HELP: &'static str = "delphi-oracled [options]\n\
        \n\
        The signing key is read from ORACLED_NSEC (nsec, hex or mnemonic), else from\n\
        --key-file, else from $CREDENTIALS_DIRECTORY/oracled.nsec (systemd\n\
        LoadCredential=), else from oracled.nsec in the data directory. A key file\n\
        readable by its group or other users is refused.\n\
        \n\
        --key-file PATH      file holding the nsec; created with a new key if absent\n\
        --log-file PATH      append everything written to stderr to this file\n\
        --print-npub         print the public key of the configured key and exit\n\
        --relay URL          relay to publish to (default wss://rof.illuminodes.com)\n\
        --relay-insecure     skip certificate checks; private-network relay addresses only\n\
        --data-dir DIR       records and header cache (default data/oracled)\n\
        --peer ADDR          bitcoin peer ip:port, repeatable (default DNS seeds)\n\
        --tolerance F        mempool anchor band around the hourly chain price (default 0.15)\n\
        --min-outputs N      smallest mempool sample that is priced (default 200)\n\
        --draws N            bootstrap resamples per mempool price (default 30)\n\
        --mempool-peers N    peers to watch for unconfirmed transactions (default 8)\n\
        --history-blocks N   blocks of history to keep, which sets how far back each\n\
        \x20                    timeline reaches (default 1008, seven days)\n\
        --cache-mb N         cap on the fetcher's raw block files (default 1000)\n\
        --tx-notes N         per-transaction verdict notes per second, 0 for none (default 20)\n\
        --dry-run            print notes instead of publishing, with a throwaway key\n\
        --run-seconds N      stop after N seconds\n";

    /// # Errors
    /// Returns a message naming the argument that is unknown, lacks a value,
    /// or holds a value out of range.
    pub fn parse<I, S>(arguments: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut config = Self::default();
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            let flag = argument.as_ref();
            if flag == "--dry-run" {
                config.dry_run = true;
                continue;
            }
            if flag == "--print-npub" {
                config.print_npub = true;
                continue;
            }
            if flag == "--relay-insecure" {
                config.relay_insecure = true;
                continue;
            }
            let value = arguments
                .next()
                .ok_or_else(|| format!("{flag} needs a value"))?;
            config.apply(flag, value.as_ref())?;
        }
        config.validate()?;
        Ok(config)
    }

    fn apply(&mut self, flag: &str, value: &str) -> Result<(), String> {
        match flag {
            "--relay" => self.relay_url = value.to_string(),
            "--data-dir" => self.data_dir = PathBuf::from(value),
            "--key-file" => self.key_file = Some(PathBuf::from(value)),
            "--log-file" => self.log_file = Some(PathBuf::from(value)),
            "--peer" => self.peers.push(Self::number(flag, value)?),
            "--tolerance" => self.tolerance = Self::number(flag, value)?,
            "--min-outputs" => self.min_outputs = Self::number(flag, value)?,
            "--draws" => self.draws = Self::number(flag, value)?,
            "--mempool-peers" => self.mempool_peers = Self::number(flag, value)?,
            "--history-blocks" => self.history_blocks = Self::number(flag, value)?,
            "--tx-notes" => self.tx_notes_per_second = Self::number(flag, value)?,
            "--cache-mb" => {
                self.cache_limit_bytes = Self::number::<u64>(flag, value)?.saturating_mul(1_000_000);
            }
            "--run-seconds" => {
                self.run_for = Some(Duration::from_secs(Self::number(flag, value)?));
            }
            other => return Err(format!("unknown argument {other}")),
        }
        Ok(())
    }

    /// Fix the data directory to an absolute path, so later reads and the
    /// startup log name the same place whatever the working directory was.
    ///
    /// # Errors
    /// Returns a message if the working directory cannot be read.
    pub fn with_absolute_data_dir(mut self) -> Result<Self, String> {
        self.data_dir = std::path::absolute(&self.data_dir)
            .map_err(|error| format!("resolve {}: {error}", self.data_dir.display()))?;
        Ok(self)
    }

    fn number<T: std::str::FromStr>(flag: &str, value: &str) -> Result<T, String> {
        value
            .parse()
            .map_err(|_| format!("{flag}: cannot read {value:?}"))
    }

    fn validate(&self) -> Result<(), String> {
        if !(self.tolerance.is_finite() && self.tolerance > 0.0 && self.tolerance < 1.0) {
            return Err("--tolerance must be between 0 and 1".to_string());
        }
        if self.draws == 0 {
            return Err("--draws must be at least 1".to_string());
        }
        if self.mempool_peers == 0 {
            return Err("--mempool-peers must be at least 1".to_string());
        }
        let shortest = ChainWindow::Hourly.blocks();
        if self.history_blocks < shortest {
            return Err(format!("--history-blocks must be at least {shortest}"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::DaemonConfig;

    #[test]
    fn no_arguments_gives_the_defaults() {
        let config = DaemonConfig::parse(Vec::<String>::new()).expect("parses");

        assert_eq!(config, DaemonConfig::default());
        assert_eq!(config.relay_url, "wss://rof.illuminodes.com");
        assert!(!config.dry_run);
    }

    #[test]
    fn every_option_is_read() {
        let config = DaemonConfig::parse([
            "--relay", "ws://10.0.0.5:4848", "--data-dir", "/tmp/o", "--peer", "1.2.3.4:8333",
            "--peer", "5.6.7.8:8333", "--tolerance", "0.1", "--min-outputs", "300", "--draws", "50",
            "--mempool-peers", "4", "--history-blocks", "144", "--cache-mb", "500", "--tx-notes", "0", "--dry-run", "--relay-insecure", "--run-seconds", "90",
            "--print-npub", "--key-file", "/tmp/k.nsec", "--log-file", "/tmp/o.log",
        ])
        .expect("parses");

        assert_eq!(config.relay_url, "ws://10.0.0.5:4848");
        assert_eq!(config.peers.len(), 2);
        assert!((config.tolerance - 0.1).abs() < f64::EPSILON);
        assert_eq!(config.min_outputs, 300);
        assert_eq!(config.draws, 50);
        assert_eq!(config.mempool_peers, 4);
        assert_eq!(config.history_blocks, 144);
        assert_eq!(config.cache_limit_bytes, 500_000_000);
        assert_eq!(config.tx_notes_per_second, 0);
        assert!(config.dry_run);
        assert!(config.relay_insecure);
        assert!(config.print_npub);
        assert_eq!(config.key_file, Some(std::path::PathBuf::from("/tmp/k.nsec")));
        assert_eq!(config.log_file, Some(std::path::PathBuf::from("/tmp/o.log")));
        assert_eq!(config.run_for, Some(Duration::from_secs(90)));
    }

    #[test]
    fn an_unknown_flag_is_an_error() {
        let error = DaemonConfig::parse(["--bogus", "1"]).expect_err("rejected");

        assert!(error.contains("--bogus"), "{error}");
    }

    #[test]
    fn a_flag_without_a_value_is_an_error() {
        let error = DaemonConfig::parse(["--relay"]).expect_err("rejected");

        assert!(error.contains("needs a value"), "{error}");
    }

    #[test]
    fn an_unreadable_number_is_an_error() {
        assert!(DaemonConfig::parse(["--draws", "many"]).is_err());
        assert!(DaemonConfig::parse(["--peer", "not-an-address"]).is_err());
    }

    #[test]
    fn the_data_directory_is_made_absolute() {
        let config = DaemonConfig::default().with_absolute_data_dir().expect("cwd");

        assert!(config.data_dir.is_absolute());
        assert!(config.data_dir.ends_with("data/oracled"));
    }

    #[test]
    fn out_of_range_values_are_rejected() {
        assert!(DaemonConfig::parse(["--tolerance", "0"]).is_err());
        assert!(DaemonConfig::parse(["--tolerance", "1.5"]).is_err());
        assert!(DaemonConfig::parse(["--tolerance", "NaN"]).is_err());
        assert!(DaemonConfig::parse(["--draws", "0"]).is_err());
        assert!(DaemonConfig::parse(["--mempool-peers", "0"]).is_err());
        assert!(DaemonConfig::parse(["--history-blocks", "5"]).is_err());
        assert!(DaemonConfig::parse(["--history-blocks", "6"]).is_ok());
    }
}
