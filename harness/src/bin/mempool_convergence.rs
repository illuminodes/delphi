use std::net::SocketAddrV4;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::network::Network;
use delphi_harness::amount_dump::AmountDump;
use delphi_harness::intraday_reference::IntradayReference;
use delphi_harness::sample_rng::SampleRng;
use delphi_oracle::{
    AdmittedSample, CachedWindows, MempoolCollector, OraclePipeline, PeerFanout, PeerSources,
};

const HELP: &str = "\
mempool-convergence — check the sample-size curve against live mempool traffic

usage: mempool-convergence [options]

options:
  --peer ADDR        Peer to subscribe to (repeatable); default: DNS seeds
  --peers N          How many peers to fan out across (default 16)
  --checkpoints LIST Output counts to price at (default 100,200,...,6000)
  --minutes N        Give up after this long (default 30)
  --data-dir PATH    Cached blocks, for the confirmed comparison
  --cache PATH       Intraday reference cache (default intraday-prices.txt)
  --seed N           PRNG seed for the confirmed subsample (default 20260416)
  --offline          Never contact CoinGecko; use the cache as-is
  --dump PATH        Save every admitted output as `satoshis unix_seconds`
  -h, --help         Print this message

Collects admitted outputs from the live mempool and prices the running sample
each time it crosses a checkpoint, so one run yields the whole size/accuracy
curve. Each checkpoint is also compared against an equal-sized subsample of
confirmed outputs, which is what the offline sweep measured.
";

struct LiveArgs {
    peers: Vec<SocketAddrV4>,
    peer_count: usize,
    checkpoints: Vec<usize>,
    minutes: u64,
    data_dir: PathBuf,
    cache: PathBuf,
    seed: u64,
    offline: bool,
    dump: Option<PathBuf>,
}

impl LiveArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut parsed = Self {
            peers: Vec::new(),
            peer_count: 16,
            checkpoints: vec![100, 200, 300, 400, 800, 1_500, 3_000, 6_000],
            minutes: 30,
            data_dir: PathBuf::from("data/oracle/mainnet"),
            cache: PathBuf::from("intraday-prices.txt"),
            seed: 20_260_416,
            offline: false,
            dump: None,
        };
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Err(String::new());
                }
                "--offline" => parsed.offline = true,
                "--dump" => {
                    index += 1;
                    parsed.dump = Some(PathBuf::from(
                        raw.get(index).ok_or("--dump requires PATH")?,
                    ));
                }
                "--peer" => {
                    index += 1;
                    let text = raw.get(index).ok_or("--peer requires ADDR")?;
                    parsed.peers.push(
                        text.parse::<SocketAddrV4>()
                            .map_err(|_| format!("invalid peer address {text}"))?,
                    );
                }
                "--peers" => {
                    index += 1;
                    parsed.peer_count = raw
                        .get(index)
                        .ok_or("--peers requires N")?
                        .parse()
                        .map_err(|_| "invalid --peers".to_string())?;
                }
                "--checkpoints" => {
                    index += 1;
                    parsed.checkpoints = Self::parse_sizes(
                        raw.get(index).ok_or("--checkpoints requires LIST")?,
                    )?;
                }
                "--minutes" => {
                    index += 1;
                    parsed.minutes = raw
                        .get(index)
                        .ok_or("--minutes requires N")?
                        .parse()
                        .map_err(|_| "invalid --minutes".to_string())?;
                }
                "--data-dir" => {
                    index += 1;
                    parsed.data_dir =
                        PathBuf::from(raw.get(index).ok_or("--data-dir requires PATH")?);
                }
                "--cache" => {
                    index += 1;
                    parsed.cache = PathBuf::from(raw.get(index).ok_or("--cache requires PATH")?);
                }
                "--seed" => {
                    index += 1;
                    parsed.seed = raw
                        .get(index)
                        .ok_or("--seed requires N")?
                        .parse()
                        .map_err(|_| "invalid --seed".to_string())?;
                }
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 1;
        }
        if parsed.checkpoints.is_empty() {
            return Err("--checkpoints must name at least one size".to_string());
        }
        parsed.checkpoints.sort_unstable();
        Ok(parsed)
    }

    fn parse_sizes(list: &str) -> Result<Vec<usize>, String> {
        list.split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| {
                token
                    .parse()
                    .map_err(|_| format!("invalid checkpoint {token}"))
            })
            .collect()
    }
}

/// One checkpoint: the live estimate, and a confirmed draw of equal size.
struct LiveCheckpoint {
    outputs: usize,
    seconds: f64,
    live_price: Option<f64>,
    confirmed_price: Option<f64>,
}

impl LiveCheckpoint {
    fn error_against(estimate: Option<f64>, reference: f64) -> Option<f64> {
        estimate
            .filter(|price| price.is_finite() && *price > 0.0)
            .map(|price| (price - reference).abs() / reference)
    }

    fn live_error(&self, reference: f64) -> Option<f64> {
        Self::error_against(self.live_price, reference)
    }

    fn confirmed_error(&self, reference: f64) -> Option<f64> {
        Self::error_against(self.confirmed_price, reference)
    }
}

struct LiveApp;

impl LiveApp {
    fn execute() -> ExitCode {
        let args = match LiveArgs::parse() {
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

    fn run(args: &LiveArgs) -> Result<(), String> {
        let confirmed = Self::confirmed_pool(args);
        let reference = Self::reference_price(args)?;
        eprintln!("reference price: ${reference:.2}");

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
            "subscribed to {} peers; collecting up to {} outputs or {} minutes\n",
            fanout.peer_count(),
            args.checkpoints.last().copied().unwrap_or_default(),
            args.minutes
        );

        let (checkpoints, sample) =
            Self::collect(args, &mut fanout, confirmed.as_ref(), reference);
        Self::report(&checkpoints, reference, confirmed.as_ref());
        if let Some(path) = &args.dump {
            let dump = AmountDump::at(path);
            dump.write(&sample)
                .map_err(|error| format!("write {}: {error}", dump.path().display()))?;
            eprintln!("saved {} outputs to {}", sample.len(), dump.path().display());
        }
        Ok(())
    }

    #[allow(clippy::cast_precision_loss)]
    fn collect(
        args: &LiveArgs,
        fanout: &mut PeerFanout,
        confirmed: Option<&AdmittedSample>,
        reference: f64,
    ) -> (Vec<LiveCheckpoint>, AdmittedSample) {
        let mut collector = MempoolCollector::new(BlockHeight::new(0));
        let mut rng = SampleRng::seeded(args.seed);
        let mut results = Vec::new();
        let mut pending = args.checkpoints.iter().copied().peekable();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(args.minutes * 60);
        let mut next_tick = Instant::now() + Duration::from_secs(30);

        while Instant::now() < deadline {
            for entry in fanout.poll() {
                collector.absorb(&entry.transaction, Self::now());
            }
            while let Some(target) = pending.peek().copied() {
                if collector.sample().len() < target {
                    break;
                }
                pending.next();
                let seconds = started.elapsed().as_secs_f64();
                let live_price = collector.sample().price().ok().map(|r| r.central_price);
                let confirmed_price = confirmed
                    .filter(|pool| pool.len() >= target)
                    .and_then(|pool| {
                        let picks = rng.choose(pool.len(), target);
                        pool.subset(&picks).price().ok().map(|r| r.central_price)
                    });
                let checkpoint = LiveCheckpoint {
                    outputs: collector.sample().len(),
                    seconds,
                    live_price,
                    confirmed_price,
                };
                eprintln!(
                    "  {:>5} outputs at {:>5.0}s  live {}  confirmed {}",
                    checkpoint.outputs,
                    seconds,
                    Self::format_error(checkpoint.live_error(reference)),
                    Self::format_error(checkpoint.confirmed_error(reference)),
                );
                results.push(checkpoint);
            }
            if pending.peek().is_none() {
                break;
            }
            if fanout.live_peers() == 0 {
                eprintln!("all peers dropped");
                break;
            }
            if Instant::now() >= next_tick {
                eprintln!(
                    "  {:>4.0}s  {:>6} tx  {:>6} outputs  {:>5.1}% admitted  {} peers",
                    started.elapsed().as_secs_f64(),
                    collector.transactions(),
                    collector.sample().len(),
                    collector.admission_rate() * 100.0,
                    fanout.live_peers()
                );
                next_tick += Duration::from_secs(30);
            }
        }
        eprintln!(
            "\ncollected {} outputs from {} transactions ({:.1}% admitted)",
            collector.sample().len(),
            collector.transactions(),
            collector.admission_rate() * 100.0
        );
        (results, collector.into_sample())
    }

    fn confirmed_pool(args: &LiveArgs) -> Option<AdmittedSample> {
        let Ok(mut windows) = CachedWindows::open(&args.data_dir) else {
            eprintln!("no cached blocks; skipping the confirmed comparison");
            return None;
        };
        let run = windows.longest_contiguous_run();
        let (Some((start_height, _)), Some((stop_height, _))) = (run.first(), run.last()) else {
            eprintln!("no contiguous cached blocks; skipping the confirmed comparison");
            return None;
        };
        let selection = delphi_oracle::RangeSelection::Heights {
            start: *start_height,
            stop: *stop_height,
        };
        let mut pipeline = OraclePipeline::new();
        if windows.feed(selection, &mut pipeline).is_err() {
            eprintln!("could not read cached blocks; skipping the confirmed comparison");
            return None;
        }
        let sample = pipeline.into_sample();
        eprintln!("confirmed pool: {} admitted outputs", sample.len());
        Some(sample)
    }

    fn reference_price(args: &LiveArgs) -> Result<f64, String> {
        let now = Self::now().to_u32();
        let mut reference = IntradayReference::with_cache(&args.cache);
        reference.load_cache();
        if !args.offline {
            reference
                .ensure_span(now.saturating_sub(7_200), now)
                .map_err(|error| error.to_string())?;
            reference.store_cache();
        }
        reference
            .price_at(now)
            .or_else(|| {
                reference
                    .span()
                    .and_then(|(_, last)| reference.price_at(last))
            })
            .ok_or_else(|| "no reference price available for now".to_string())
    }

    fn format_error(error: Option<f64>) -> String {
        error.map_or_else(|| "    -   ".to_string(), |value| format!("{:>7.3}%", value * 100.0))
    }

    #[allow(clippy::cast_precision_loss)]
    fn report(checkpoints: &[LiveCheckpoint], reference: f64, confirmed: Option<&AdmittedSample>) {
        if checkpoints.is_empty() {
            println!("\nno checkpoint was reached");
            return;
        }
        println!(
            "\n{:>8} {:>8} {:>12} {:>10} {:>12} {:>10}",
            "outputs", "seconds", "live price", "live err", "confirmed", "conf err"
        );
        for checkpoint in checkpoints {
            println!(
                "{:>8} {:>8.0} {:>12} {:>10} {:>12} {:>10}",
                checkpoint.outputs,
                checkpoint.seconds,
                checkpoint
                    .live_price
                    .map_or_else(|| "-".to_string(), |price| format!("{price:.2}")),
                Self::format_error(checkpoint.live_error(reference)),
                checkpoint
                    .confirmed_price
                    .map_or_else(|| "-".to_string(), |price| format!("{price:.2}")),
                Self::format_error(checkpoint.confirmed_error(reference)),
            );
        }
        println!("\nreference ${reference:.2}");
        if confirmed.is_none() {
            println!("confirmed column empty: no cached blocks were available");
        }
        if let Some(rate) = Self::observed_rate(checkpoints) {
            println!("observed mempool rate: {rate:.2} admitted outputs/s");
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn observed_rate(checkpoints: &[LiveCheckpoint]) -> Option<f64> {
        let last = checkpoints.last()?;
        (last.seconds > 0.0).then(|| last.outputs as f64 / last.seconds)
    }

    fn now() -> BlockTime {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        BlockTime::new(u32::try_from(seconds).unwrap_or(u32::MAX))
    }

    fn candidates(args: &LiveArgs) -> Result<Vec<SocketAddrV4>, String> {
        if !args.peers.is_empty() {
            return Ok(args.peers.clone());
        }
        let peers = PeerSources::new().resolve(Network::Mainnet);
        if peers.is_empty() {
            return Err("no peers discovered; pass --peer ADDR".to_string());
        }
        Ok(peers)
    }
}

fn main() -> ExitCode {
    LiveApp::execute()
}
