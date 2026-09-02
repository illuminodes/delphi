use std::net::SocketAddrV4;
use std::path::PathBuf;

use mantle::difficulty::BlockHeight;
use mantle::network::Network;
use mantle_oracle::{RangeSelection, UtcDate};

pub const HELP: &str = "\
mantle-oracle — estimate the BTC/USD price from Bitcoin transaction outputs

Usage:
    mantle-oracle [OPTIONS]

Range options (choose at most one):
    --date YYYY-MM-DD      Analyze one UTC calendar day.
    --recent-blocks N      Analyze the latest N blocks.
    --start-height N --stop-height N
                           Analyze an inclusive explicit block range.

Network options:
    --network NAME         mainnet | testnet3 | testnet4 | signet | regtest
                           (default: mainnet).
    --peer HOST:PORT       Add a P2P peer; may be repeated.
    --peer-file PATH       Add peers from a file, one IPv4 HOST:PORT per line.
    --data-dir DIR         Header and block cache root
                           (default: ./data/oracle/<network>).
    --help                 Print this message.

Without a range option, the previous complete UTC day is analyzed.
";

#[derive(Debug)]
pub struct Args {
    pub data_dir: PathBuf,
    pub network: Network,
    pub selection: RangeSelection,
    pub peers: Vec<SocketAddrV4>,
    pub peer_files: Vec<String>,
}

impl Args {
    pub fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut network = Network::Mainnet;
        let mut data_dir = None;
        let mut peers = Vec::new();
        let mut peer_files = Vec::new();
        let mut date = None;
        let mut recent = None;
        let mut start = None;
        let mut stop = None;
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "--help" | "-h" => {
                    print!("{HELP}");
                    return Err(String::new());
                }
                "--network" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--network requires a name")?;
                    network =
                        Network::parse(value).ok_or_else(|| format!("unknown network: {value}"))?;
                }
                "--data-dir" => {
                    index += 1;
                    data_dir = Some(PathBuf::from(
                        raw.get(index).ok_or("--data-dir requires a path")?,
                    ));
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
                    let path =
                        raw.get(index).ok_or("--peer-file requires a path")?.clone();
                    peer_files.push(path);
                }
                "--date" => {
                    index += 1;
                    date = Some(
                        UtcDate::parse(raw.get(index).ok_or("--date requires YYYY-MM-DD")?)
                            .map_err(|error| format!("invalid --date: {error}"))?,
                    );
                }
                "--recent-blocks" => {
                    index += 1;
                    let value = raw.get(index).ok_or("--recent-blocks requires N")?;
                    recent =
                        Some(value.parse::<usize>().map_err(|error| {
                            format!("invalid --recent-blocks {value}: {error}")
                        })?);
                }
                "--start-height" => {
                    index += 1;
                    start = Some(Self::parse_height(
                        raw.get(index).ok_or("--start-height requires N")?,
                        "--start-height",
                    )?);
                }
                "--stop-height" => {
                    index += 1;
                    stop = Some(Self::parse_height(
                        raw.get(index).ok_or("--stop-height requires N")?,
                        "--stop-height",
                    )?);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
            index += 1;
        }
        let selection = Self::selection(date, recent, start, stop)?;
        let data_dir =
            data_dir.unwrap_or_else(|| PathBuf::from(format!("./data/oracle/{}", network.name())));
        Ok(Self {
            data_dir,
            network,
            selection,
            peers,
            peer_files,
        })
    }

    fn selection(
        date: Option<UtcDate>,
        recent: Option<usize>,
        start: Option<BlockHeight>,
        stop: Option<BlockHeight>,
    ) -> Result<RangeSelection, String> {
        let explicit_heights = match (start, stop) {
            (Some(start), Some(stop)) => Some(RangeSelection::Heights { start, stop }),
            (None, None) => None,
            _ => return Err("--start-height and --stop-height must be used together".to_string()),
        };
        let mode_count = usize::from(date.is_some())
            + usize::from(recent.is_some())
            + usize::from(explicit_heights.is_some());
        if mode_count > 1 {
            return Err("choose only one range mode".to_string());
        }
        if let Some(date) = date {
            return date
                .midnight()
                .map(RangeSelection::UtcDay)
                .map_err(|error| format!("invalid --date: {error}"));
        }
        Ok(recent
            .map(RangeSelection::RecentBlocks)
            .or(explicit_heights)
            .unwrap_or(RangeSelection::PreviousUtcDay))
    }

    fn parse_height(value: &str, option: &str) -> Result<BlockHeight, String> {
        value
            .parse::<u32>()
            .map(BlockHeight::new)
            .map_err(|error| format!("invalid {option} {value}: {error}"))
    }
}
