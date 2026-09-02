use std::net::SocketAddrV4;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mantle::network::Network;
use mantle_oracle::{MempoolWatcher, OutputFilter, PeerSources};

const HELP: &str = "\
mempool-spike — measure the live mempool's usable output rate

usage: mempool-spike [options]

options:
  --peer ADDR        Peer to subscribe to (repeatable); default: DNS seeds
  --seconds N        How long to watch (default 120)
  --help             Print this message

Subscribes to one peer's transaction announcements and reports how many
transactions arrive and how many survive the oracle's output filter. Nothing
is written to disk. Answers whether a mempool price is worth building, by
measuring the sample rate the oracle would actually get.
";

struct SpikeArgs {
    peers: Vec<SocketAddrV4>,
    seconds: u64,
}

impl SpikeArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut peers = Vec::new();
        let mut seconds = 120;
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Err(String::new());
                }
                "--peer" => {
                    index += 1;
                    let text = raw.get(index).ok_or("--peer requires ADDR")?;
                    peers.push(
                        text.parse::<SocketAddrV4>()
                            .map_err(|_| format!("invalid peer address {text}"))?,
                    );
                }
                "--seconds" => {
                    index += 1;
                    seconds = raw
                        .get(index)
                        .ok_or("--seconds requires N")?
                        .parse()
                        .map_err(|_| "invalid --seconds".to_string())?;
                }
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 1;
        }
        Ok(Self { peers, seconds })
    }
}

struct SpikeTally {
    transactions: u64,
    admitted_outputs: u64,
    admitted_transactions: u64,
    started: Instant,
}

impl SpikeTally {
    fn new() -> Self {
        Self {
            transactions: 0,
            admitted_outputs: 0,
            admitted_transactions: 0,
            started: Instant::now(),
        }
    }

    fn record(&mut self, admitted: usize) {
        self.transactions += 1;
        if admitted > 0 {
            self.admitted_transactions += 1;
            self.admitted_outputs += admitted as u64;
        }
    }

    fn elapsed(&self) -> f64 {
        self.started.elapsed().as_secs_f64().max(1e-9)
    }

    fn report(&self, watcher: &MempoolWatcher, filter: &OutputFilter) {
        let elapsed = self.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let outputs = self.admitted_outputs as f64;
        #[allow(clippy::cast_precision_loss)]
        let txs = self.transactions as f64;
        println!("\nwatched {elapsed:.0}s");
        println!("  announced          {}", watcher.announced());
        println!("  delivered          {}", watcher.delivered());
        println!("  decoded            {}", self.transactions);
        println!(
            "  admitted txs       {} ({:.1}%)",
            self.admitted_transactions,
            if txs > 0.0 {
                #[allow(clippy::cast_precision_loss)]
                let share = self.admitted_transactions as f64 / txs * 100.0;
                share
            } else {
                0.0
            }
        );
        println!("  admitted outputs   {}", self.admitted_outputs);
        println!("  distinct spends    {}", filter.distinct_spends());
        println!("\nrates");
        println!("  tx/s               {:.2}", txs / elapsed);
        println!("  admitted output/s  {:.2}", outputs / elapsed);
        println!(
            "  admitted outputs projected per 10m window: {:.0}",
            outputs / elapsed * 600.0
        );
        println!(
            "  ...per 45m window:                        {:.0}",
            outputs / elapsed * 2_700.0
        );
    }
}

struct SpikeApp;

impl SpikeApp {
    fn execute() -> ExitCode {
        let args = match SpikeArgs::parse() {
            Ok(args) => args,
            Err(error) if error.is_empty() => return ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}\n\n{HELP}");
                return ExitCode::from(2);
            }
        };
        match Self::run(&args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fatal: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(args: &SpikeArgs) -> Result<(), String> {
        let candidates = Self::candidates(args)?;
        let mut watcher = Self::connect_any(&candidates)?;
        eprintln!("subscribed; watching for {}s\n", args.seconds);

        let mut filter = OutputFilter::new();
        let mut tally = SpikeTally::new();
        let deadline = Instant::now() + Duration::from_secs(args.seconds);
        let mut next_tick = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            let seen = match watcher.poll() {
                Ok(seen) => seen,
                Err(error) => {
                    eprintln!("peer dropped: {error}");
                    break;
                }
            };
            for entry in seen {
                let admitted = filter.admit(&entry.transaction).count();
                tally.record(admitted);
            }
            if Instant::now() >= next_tick {
                eprintln!(
                    "  {:>4.0}s  {:>6} tx  {:>6} admitted outputs",
                    tally.elapsed(),
                    tally.transactions,
                    tally.admitted_outputs
                );
                next_tick += Duration::from_secs(10);
            }
        }
        tally.report(&watcher, &filter);
        Ok(())
    }

    fn candidates(args: &SpikeArgs) -> Result<Vec<SocketAddrV4>, String> {
        if !args.peers.is_empty() {
            return Ok(args.peers.clone());
        }
        let sources = PeerSources::new();
        let peers = sources.resolve(Network::Mainnet);
        if peers.is_empty() {
            return Err("no peers discovered; pass --peer ADDR".to_string());
        }
        Ok(peers)
    }

    fn connect_any(candidates: &[SocketAddrV4]) -> Result<MempoolWatcher, String> {
        for address in candidates.iter().take(16) {
            eprintln!("connecting to {address}...");
            match MempoolWatcher::connect(*address, Network::Mainnet, Duration::from_secs(5)) {
                Ok(watcher) => return Ok(watcher),
                Err(error) => eprintln!("  {error}"),
            }
        }
        Err("could not complete a handshake with any peer".to_string())
    }
}

fn main() -> ExitCode {
    SpikeApp::execute()
}
