use std::net::SocketAddrV4;
use std::path::PathBuf;

use mantle::network::Network;
use delphi_oracle::UtcDate;

pub const HELP: &str = "\
oracle-harness — validate the price oracle across N daily windows

Usage:
    oracle-harness [OPTIONS]

Options:
    --windows N            Number of consecutive daily windows to score
                           (default: 3).
    --tolerance PCT        Maximum relative deviation from the reference
                           price, in percent (default: 2.0).
    --network NAME         mainnet | testnet3 | testnet4 | signet | regtest
                           (default: mainnet; references only exist for
                           mainnet).
    --peer HOST:PORT       Add a P2P peer; may be repeated.
    --peer-file PATH       Add peers from a file, one IPv4 HOST:PORT per line.
    --data-dir DIR         Header and block cache root
                           (default: ./data/oracle/<network>).
    --reference-cache PATH
                           Reference price cache file
                           (default: <data-dir>/reference-cache.txt).
    --offline              Use only the reference cache; never fetch.
    --last-date YYYY-MM-DD
                           Score windows ending on this date instead of the
                           most recent complete UTC day.
    --help                 Print this message.

The harness scores each window against the average of the surrounding
midnight reference snapshots (CoinGecko daily data). The run passes when
every window's central price lands within the tolerance of its reference.
Exit status is nonzero when any window misses or fails to fetch.
";

#[derive(Debug)]
pub struct Args {
    pub windows: usize,
    pub tolerance: f64,
    pub network: Network,
    pub peers: Vec<SocketAddrV4>,
    pub peer_files: Vec<String>,
    pub data_dir: PathBuf,
    reference_cache: PathBuf,
    pub offline: bool,
    pub last_date: Option<UtcDate>,
}

impl Args {
    #[allow(clippy::missing_errors_doc)]
    pub fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut windows = 3_usize;
        let mut tolerance = 2.0_f64;
        let mut network = Network::Mainnet;
        let mut peers = Vec::new();
        let mut peer_files = Vec::new();
        let mut data_dir = None;
        let mut reference_cache = None;
        let mut offline = false;
        let mut last_date = None;
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "--windows" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--windows requires N")?;
                    windows = value
                        .parse::<usize>()
                        .map_err(|error| format!("invalid --windows {value}: {error}"))?;
                    if windows == 0 {
                        return Err("--windows must be at least 1".to_string());
                    }
                }
                "--tolerance" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--tolerance requires PCT")?;
                    tolerance = value
                        .parse::<f64>()
                        .map_err(|error| format!("invalid --tolerance {value}: {error}"))?;
                    if !(tolerance.is_finite() && tolerance > 0.0) {
                        return Err("--tolerance must be a positive percentage".to_string());
                    }
                }
                "--network" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--network requires a name")?;
                    network =
                        Network::parse(value).ok_or_else(|| format!("unknown network: {value}"))?;
                }
                "--peer" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--peer requires HOST:PORT")?;
                    peers.push(
                        value
                            .parse()
                            .map_err(|error| format!("invalid --peer {value}: {error}"))?,
                    );
                }
                "--peer-file" => {
                    index += 1;
                    let path = raw.get(index).ok_or("--peer-file requires a path")?.clone();
                    peer_files.push(path);
                }
                "--data-dir" => {
                    index += 1;
                    data_dir = Some(PathBuf::from(
                        raw.get(index).ok_or("--data-dir requires a path")?,
                    ));
                }
                "--reference-cache" => {
                    index += 1;
                    reference_cache = Some(PathBuf::from(
                        raw.get(index).ok_or("--reference-cache requires a path")?,
                    ));
                }
                "--offline" => offline = true,
                "--last-date" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--last-date requires YYYY-MM-DD")?;
                    last_date = Some(
                        UtcDate::parse(value)
                            .map_err(|error| format!("invalid --last-date: {error}"))?,
                    );
                }
                "--help" | "-h" => {
                    print!("{HELP}");
                    return Err(String::new());
                }
                other => return Err(format!("unknown argument: {other}")),
            }
            index += 1;
        }
        tolerance /= 100.0;
        let data_dir =
            data_dir.unwrap_or_else(|| PathBuf::from(format!("./data/oracle/{}", network.name())));
        let reference_cache =
            reference_cache.unwrap_or_else(|| data_dir.join("reference-cache.txt"));
        Ok(Self {
            windows,
            tolerance,
            network,
            peers,
            peer_files,
            data_dir,
            reference_cache,
            offline,
            last_date,
        })
    }

    #[must_use]
    pub fn reference_cache_path(&self) -> &std::path::Path {
        &self.reference_cache
    }
}
