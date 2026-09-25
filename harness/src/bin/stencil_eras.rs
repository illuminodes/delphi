use std::path::PathBuf;
use std::process::ExitCode;

use mantle_harness::confirmed_pools::ConfirmedPools;
use mantle_harness::intraday_reference::IntradayReference;
use mantle_harness::stencil_variant::StencilVariant;
use mantle_harness::subsample_sweep::{DrawMode, PricedPool, SubsampleSweep};
use mantle_oracle::{CachedWindows, RoughPriceEstimator};

const HELP: &str = "\
oracle-stencil-eras — grid rough-estimator stencil variants per cached era

usage: oracle-stencil-eras [options]

options:
  --data-dir PATH      Cached blocks (default data/oracle/mainnet)
  --cache PATH         Intraday reference cache (default intraday-prices.txt)
  --min-blocks N       Ignore cached runs shorter than N blocks (default 100)
  --window SECONDS     Confirmed pool width (default 1200)
  --sizes LIST         Sample sizes (default 150,200,300,400,800)
  --weights LIST       Smooth-stencil weights (default 0.65,1.5,3,6)
  --smooth-below LIST  Slides below which the smooth term applies (default 150)
  --draws N            Draws per size (default 240)
  --seed N             PRNG seed (default 20260416)
  --offline            Never contact CoinGecko; use the cache as-is
  -h, --help           Print this message

Every contiguous run of cached blocks is one era. Each era is tiled into
windows, and every stencil variant is scored on random draws from them.
CoinGecko is used only to score.
";

struct ErasArgs {
    data_dir: PathBuf,
    cache: PathBuf,
    min_blocks: usize,
    window: u32,
    sizes: Vec<usize>,
    weights: Vec<f64>,
    smooth_below: Vec<i32>,
    draws: usize,
    seed: u64,
    offline: bool,
}

impl ErasArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut parsed = Self {
            data_dir: PathBuf::from("data/oracle/mainnet"),
            cache: PathBuf::from("intraday-prices.txt"),
            min_blocks: 100,
            window: 1_200,
            sizes: vec![150, 200, 300, 400, 800],
            weights: vec![0.65, 1.5, 3.0, 6.0],
            smooth_below: vec![RoughPriceEstimator::DEFAULT_SMOOTH_BELOW],
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
                "--weights" => parsed.weights = Self::list(value()?)?,
                "--smooth-below" => parsed.smooth_below = Self::list(value()?)?,
                "--draws" => parsed.draws = Self::number(value()?, flag)?,
                "--seed" => parsed.seed = Self::number(value()?, flag)?,
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 2;
        }
        if parsed.sizes.is_empty() || parsed.weights.is_empty() || parsed.draws == 0 {
            return Err("--sizes, --weights and --draws must be non-empty".to_string());
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
    first: u32,
    last: u32,
    blocks: usize,
    pools: Vec<PricedPool>,
}

impl Era {
    fn mean_reference(&self) -> f64 {
        let total: f64 = self.pools.iter().map(PricedPool::reference).sum();
        total / f64::from(u32::try_from(self.pools.len()).unwrap_or(u32::MAX).max(1))
    }

    fn outputs(&self) -> usize {
        self.pools.iter().map(|pool| pool.sample().len()).sum()
    }
}

struct ErasApp;

impl ErasApp {
    fn execute() -> ExitCode {
        let args = match ErasArgs::parse() {
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

    fn run(args: &ErasArgs) -> Result<(), String> {
        let mut windows = CachedWindows::open(&args.data_dir)
            .map_err(|error| format!("open cached blocks: {error}"))?;
        let spans: Vec<(u32, u32, usize)> = windows
            .contiguous_runs(args.min_blocks)
            .iter()
            .filter_map(|run| {
                let (_, first) = run.first()?;
                let (_, last) = run.last()?;
                Some((first.time().to_u32(), last.time().to_u32(), run.len()))
            })
            .collect();
        if spans.is_empty() {
            return Err(format!("no cached run holds {} blocks", args.min_blocks));
        }

        let mut reference = IntradayReference::with_cache(&args.cache);
        reference.load_cache();
        if !args.offline {
            for (first, last, _) in &spans {
                if let Err(error) = reference.ensure_span(first.saturating_sub(3_600), last + 3_600)
                {
                    eprintln!("skipping era {first}..{last}: {error}");
                }
            }
            reference.store_cache();
        }

        let gatherer = ConfirmedPools::of_duration(args.window);
        let eras: Vec<Era> = spans
            .into_iter()
            .map(|(first, last, blocks)| Era {
                first,
                last,
                blocks,
                pools: gatherer.gather(&mut windows, &reference, first, last),
            })
            .filter(|era| !era.pools.is_empty())
            .collect();
        for era in &eras {
            eprintln!(
                "era {}..{}: {} blocks, {} windows of {}, {} outputs, mean reference ${:.0}",
                era.first,
                era.last,
                era.blocks,
                era.pools.len(),
                gatherer.plan().label(),
                era.outputs(),
                era.mean_reference()
            );
        }
        Self::report(args, &eras);
        Ok(())
    }

    fn report(args: &ErasArgs, eras: &[Era]) {
        println!("\nrandom draws; each cell is p90 error / share within 1%");
        print!("{:>9} {:>14}", "era", "variant");
        for size in &args.sizes {
            print!(" {size:>15}");
        }
        println!();
        for era in eras {
            let label = format!("${:.0}k", era.mean_reference() / 1_000.0);
            for variant in StencilVariant::grid(&args.weights, &args.smooth_below) {
                let sweep = SubsampleSweep::new(&era.pools, args.draws, args.seed)
                    .with_estimator(variant.estimator());
                print!("{label:>9} {:>14}", variant.label());
                for size in &args.sizes {
                    let row = sweep.row(*size, DrawMode::Random);
                    let cell = if row.priced() == 0 {
                        "-".to_string()
                    } else {
                        format!(
                            "{:.2}%/{:.0}%",
                            row.p90_error() * 100.0,
                            row.within_one_percent() * 100.0
                        )
                    };
                    print!(" {cell:>15}");
                }
                println!();
            }
        }
    }
}

fn main() -> ExitCode {
    ErasApp::execute()
}
