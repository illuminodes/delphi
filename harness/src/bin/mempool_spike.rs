use std::net::SocketAddrV4;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mantle::network::Network;
use delphi_oracle::{OutputFilter, PeerFanout, PeerSources};

const HELP: &str = "\
mempool-spike — measure the live mempool's usable output rate

usage: mempool-spike [options]

options:
  --peer ADDR        Peer to subscribe to (repeatable); default: DNS seeds
  --peers N          How many peers to fan out across (default 16)
  --seconds N        How long to watch (default 120)
  --help             Print this message

Subscribes to several peers' transaction announcements and reports how many
transactions arrive and how many survive the oracle's output filter. Each
transaction is requested from the first peer that announced it, so the
per-peer first-sighting counts show what each peer added. Nothing is written
to disk.
";

struct SpikeArgs {
    peers: Vec<SocketAddrV4>,
    peer_count: usize,
    seconds: u64,
}

impl SpikeArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut peers = Vec::new();
        let mut peer_count = 16;
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
                "--peers" => {
                    index += 1;
                    peer_count = raw
                        .get(index)
                        .ok_or("--peers requires N")?
                        .parse()
                        .map_err(|_| "invalid --peers".to_string())?;
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
        Ok(Self {
            peers,
            peer_count,
            seconds,
        })
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

    fn report(&self, fanout: &PeerFanout, filter: &OutputFilter) {
        let elapsed = self.elapsed();
        #[allow(clippy::cast_precision_loss)]
        let outputs = self.admitted_outputs as f64;
        #[allow(clippy::cast_precision_loss)]
        let txs = self.transactions as f64;
        println!("\nwatched {elapsed:.0}s");
        println!(
            "  peers              {} of {} still live",
            fanout.live_peers(),
            fanout.peer_count()
        );
        println!("  unique announced   {}", fanout.unique_announced());
        println!(
            "  duplicate annce    {}",
            fanout.duplicate_announcements()
        );
        println!("  best single peer   {}", fanout.best_single_peer());
        println!("  decoded            {}", self.transactions);
        let (refused, undecodable) = fanout.delivery_losses();
        println!("  notfound           {refused}");
        println!("  undecodable        {undecodable}");
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
        Self::report_fanout_gain(fanout);
    }

    /// The number that decides the question: how much more the fanout saw
    /// than the single best peer would have on its own.
    fn report_fanout_gain(fanout: &PeerFanout) {
        let best = fanout.best_single_peer();
        if best == 0 {
            return;
        }
        #[allow(clippy::cast_precision_loss)]
        let gain = fanout.unique_announced() as f64 / best as f64;
        println!("\nfanout gain over best single peer: {gain:.2}x");
        println!("\nper-peer contribution");
        for stat in fanout.stats().iter().take(20) {
            println!(
                "  {:<22} announced {:>6}  first {:>6}  delivered {:>6} ({:>5.1}%)  {}",
                stat.address.to_string(),
                stat.announced,
                stat.first_sightings,
                stat.delivered,
                stat.delivery_rate() * 100.0,
                if stat.live { "live" } else { "dropped" }
            );
        }
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
        eprintln!("connecting to up to {} peers...", args.peer_count);
        let mut fanout = PeerFanout::connect(
            &candidates,
            Network::Mainnet,
            args.peer_count,
            Duration::from_secs(5),
            Duration::from_millis(50),
        );
        if fanout.is_empty() {
            return Err("could not complete a handshake with any peer".to_string());
        }
        eprintln!(
            "subscribed to {} peers; watching for {}s\n",
            fanout.peer_count(),
            args.seconds
        );

        let mut filter = OutputFilter::new();
        let mut tally = SpikeTally::new();
        let deadline = Instant::now() + Duration::from_secs(args.seconds);
        let mut next_tick = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            for entry in fanout.poll() {
                let admitted = filter.admit(&entry.transaction).count();
                tally.record(admitted);
            }
            if fanout.live_peers() == 0 {
                eprintln!("all peers dropped");
                break;
            }
            if Instant::now() >= next_tick {
                eprintln!(
                    "  {:>4.0}s  {:>6} tx  {:>6} admitted outputs  {} peers",
                    tally.elapsed(),
                    tally.transactions,
                    tally.admitted_outputs,
                    fanout.live_peers()
                );
                next_tick += Duration::from_secs(10);
            }
        }
        tally.report(&fanout, &filter);
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
}

fn main() -> ExitCode {
    SpikeApp::execute()
}
