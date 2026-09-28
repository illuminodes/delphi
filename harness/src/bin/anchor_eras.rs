use std::path::PathBuf;
use std::process::ExitCode;

use mantle_harness::anchor_comparison::AnchorComparison;
use mantle_harness::confirmed_pools::ConfirmedPools;
use mantle_harness::convergence::ConvergenceRow;
use mantle_harness::intraday_reference::IntradayReference;
use mantle_harness::subsample_sweep::{DrawMode, PricedPool, SubsampleSweep};
use mantle_oracle::CachedWindows;

const HELP: &str = "\
oracle-anchor — price small mempool-sized samples with and without a chain anchor

usage: oracle-anchor [options]

options:
  --data-dir PATH      Cached blocks (default data/oracle/mainnet)
  --cache PATH         Intraday reference cache (default intraday-prices.txt)
  --min-blocks N       Ignore cached runs shorter than N blocks (default 100)
  --window SECONDS     Sample pool width (default 1200)
  --sizes LIST         Sample sizes in outputs (default 100,150,200,300,400)
  --anchor-blocks LIST Chain window sizes in blocks (default 6,24)
  --tolerances LIST    Anchor band half-widths as ratios (default 0.25)
  --draws N            Draws per size (default 240)
  --seed N             PRNG seed (default 20260416)
  --offline            Never contact CoinGecko; use the cache as-is
  -h, --help           Print this message

Each pool's anchor is the oracle's price from the N confirmed blocks before
the pool. A sample is then priced unconstrained and constrained to a band
around the anchor. CoinGecko is used only to score.
";

struct AnchorArgs {
    data_dir: PathBuf,
    cache: PathBuf,
    min_blocks: usize,
    window: u32,
    sizes: Vec<usize>,
    anchor_blocks: Vec<usize>,
    tolerances: Vec<f64>,
    draws: usize,
    seed: u64,
    offline: bool,
}

impl AnchorArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut parsed = Self {
            data_dir: PathBuf::from("data/oracle/mainnet"),
            cache: PathBuf::from("intraday-prices.txt"),
            min_blocks: 100,
            window: 1_200,
            sizes: vec![100, 150, 200, 300, 400],
            anchor_blocks: vec![6, 24],
            tolerances: vec![0.25],
            draws: 240,
            seed: 20_260_416,
            offline: false,
        };
        let mut index = 0;
        while index < raw.len() {
            let flag = raw[index].as_str();
            let value = || raw.get(index + 1).ok_or(format!("{flag} requires a value"));
            match flag {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Err(String::new());
                }
                "--offline" => {
                    parsed.offline = true;
                    index += 1;
                    continue;
                }
                "--data-dir" => parsed.data_dir = PathBuf::from(value()?),
                "--cache" => parsed.cache = PathBuf::from(value()?),
                "--min-blocks" => parsed.min_blocks = Self::number(value()?, flag)?,
                "--window" => parsed.window = Self::number(value()?, flag)?,
                "--sizes" => parsed.sizes = Self::list(value()?)?,
                "--anchor-blocks" => parsed.anchor_blocks = Self::list(value()?)?,
                "--tolerances" => parsed.tolerances = Self::list(value()?)?,
                "--draws" => parsed.draws = Self::number(value()?, flag)?,
                "--seed" => parsed.seed = Self::number(value()?, flag)?,
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 2;
        }
        if parsed.sizes.is_empty()
            || parsed.anchor_blocks.is_empty()
            || parsed.tolerances.is_empty()
            || parsed.draws == 0
        {
            return Err("--sizes, --anchor-blocks, --tolerances and --draws must be non-empty".to_string());
        }
        Ok(parsed)
    }

    fn number<T: std::str::FromStr>(token: &str, flag: &str) -> Result<T, String> {
        token.parse().map_err(|_| format!("invalid {flag} {token}"))
    }

    fn list<T: std::str::FromStr>(list: &str) -> Result<Vec<T>, String> {
        list.split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| token.parse().map_err(|_| format!("invalid list entry {token}")))
            .collect()
    }
}

struct Era {
    label: String,
    pools: Vec<PricedPool>,
}

struct MeanReference;

impl MeanReference {
    fn of(pools: &[PricedPool]) -> f64 {
        let total: f64 = pools.iter().map(PricedPool::reference).sum();
        total / f64::from(u32::try_from(pools.len()).unwrap_or(u32::MAX).max(1))
    }
}

struct CellText;

impl CellText {
    fn of(row: &ConvergenceRow) -> String {
        if row.priced() == 0 {
            return format!("-/{}", row.attempted());
        }
        format!(
            "{:.2}%/{:.0}%",
            row.p90_error() * 100.0,
            row.within_one_percent() * 100.0
        )
    }

    fn coverage(row: &ConvergenceRow) -> String {
        format!("{}/{}", row.priced(), row.attempted())
    }
}

struct AnchorApp;

impl AnchorApp {
    fn execute() -> ExitCode {
        let args = match AnchorArgs::parse() {
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

    fn run(args: &AnchorArgs) -> Result<(), String> {
        let mut windows = CachedWindows::open(&args.data_dir)
            .map_err(|error| format!("open cached blocks: {error}"))?;
        let runs = windows.contiguous_runs(args.min_blocks);
        if runs.is_empty() {
            return Err(format!("no cached run holds {} blocks", args.min_blocks));
        }

        let mut reference = IntradayReference::with_cache(&args.cache);
        reference.load_cache();
        if !args.offline {
            for run in &runs {
                let (Some((_, first)), Some((_, last))) = (run.first(), run.last()) else {
                    continue;
                };
                if let Err(error) = reference.ensure_span(
                    first.time().to_u32().saturating_sub(3_600),
                    last.time().to_u32() + 3_600,
                ) {
                    eprintln!("skipping era: {error}");
                }
            }
            reference.store_cache();
        }

        let gatherer = ConfirmedPools::of_duration(args.window);
        let mut eras = Vec::new();
        for run in &runs {
            let (Some((_, first)), Some((_, last))) = (run.first(), run.last()) else {
                continue;
            };
            let (first, last) = (first.time().to_u32(), last.time().to_u32());
            let pools = gatherer.gather_anchored(
                &mut windows,
                &reference,
                first,
                last,
                run,
                &args.anchor_blocks,
            );
            if pools.is_empty() {
                continue;
            }
            let mean = MeanReference::of(&pools);
            eprintln!("era {first}..{last}: {} blocks, {} pools, ${mean:.0}", run.len(), pools.len());
            eras.push(Era {
                label: format!("${:.0}k", mean / 1_000.0),
                pools,
            });
        }
        if eras.is_empty() {
            return Err("no era could be priced against the reference".to_string());
        }
        for era in &eras {
            Self::report(args, &era.label, &era.pools);
        }
        let all: Vec<PricedPool> = eras.into_iter().flat_map(|era| era.pools).collect();
        Self::report(args, "all", &all);
        Ok(())
    }

    fn report(args: &AnchorArgs, label: &str, pools: &[PricedPool]) {
        let sweep = SubsampleSweep::new(pools, args.draws, args.seed);
        let comparison = AnchorComparison::new(&sweep);
        for blocks in &args.anchor_blocks {
            for tolerance in &args.tolerances {
                println!(
                    "\n{label}: anchor {blocks} blocks, band +-{:.0}%; cells are p90 error / share within 1%",
                    tolerance * 100.0
                );
                println!(
                    "{:>6} {:>15} {:>15} {:>15} {:>9}",
                    "size", "chain only", "unanchored", "anchored", "priced"
                );
                for size in &args.sizes {
                    let row = comparison.row(*size, DrawMode::Random, *blocks, *tolerance);
                    println!(
                        "{:>6} {:>15} {:>15} {:>15} {:>9}",
                        size,
                        CellText::of(&row.chain_only),
                        CellText::of(&row.unanchored),
                        CellText::of(&row.anchored),
                        CellText::coverage(&row.anchored),
                    );
                }
            }
        }
    }
}

fn main() -> ExitCode {
    AnchorApp::execute()
}
