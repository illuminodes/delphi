use std::path::PathBuf;
use std::process::ExitCode;

use mantle::difficulty::BlockTime;
use delphi_harness::convergence::{ConvergenceRow, ConvergenceTrial};
use delphi_harness::intraday_reference::IntradayReference;
use delphi_harness::sample_rng::SampleRng;
use delphi_harness::span_plan::SpanPlan;
use delphi_oracle::{AdmittedSample, CachedWindows, OraclePipeline};

const HELP: &str = "\
oracle-convergence — how many admitted outputs the oracle needs for a price

usage: oracle-convergence [options]

options:
  --data-dir PATH        Cached blocks and headers (default data/oracle/mainnet)
  --cache PATH           Intraday reference cache (default intraday-prices.txt)
  --window DURATION      Span pooled per trial (default 1d)
  --sizes LIST           Comma-separated sample sizes (default 50,100,...,20000)
  --draws N              Random draws per sample size (default 200)
  --seed N               PRNG seed (default 20260416)
  --offline              Never contact CoinGecko; use the cache as-is
  -h, --help             Print this message

Subsamples the admitted outputs of already-cached blocks, so it answers how
large a sample the estimator needs without touching the network. The answer
bounds any live mempool collector: a mempool sample of N outputs cannot beat
a confirmed sample of N outputs drawn from the same window.
";

struct ConvergenceArgs {
    data_dir: PathBuf,
    cache: PathBuf,
    window: u32,
    sizes: Vec<usize>,
    draws: usize,
    seed: u64,
    offline: bool,
}

impl ConvergenceArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut parsed = Self {
            data_dir: PathBuf::from("data/oracle/mainnet"),
            cache: PathBuf::from("intraday-prices.txt"),
            window: 86_400,
            sizes: vec![
                50, 100, 200, 400, 800, 1_500, 3_000, 6_000, 12_000, 20_000,
            ],
            draws: 200,
            seed: 20_260_416,
            offline: false,
        };
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Err(String::new());
                }
                "--offline" => parsed.offline = true,
                "--data-dir" => {
                    index += 1;
                    parsed.data_dir =
                        PathBuf::from(raw.get(index).ok_or("--data-dir requires PATH")?);
                }
                "--cache" => {
                    index += 1;
                    parsed.cache = PathBuf::from(raw.get(index).ok_or("--cache requires PATH")?);
                }
                "--window" => {
                    index += 1;
                    parsed.window =
                        Self::parse_duration(raw.get(index).ok_or("--window requires DURATION")?)?;
                }
                "--sizes" => {
                    index += 1;
                    parsed.sizes = Self::parse_sizes(raw.get(index).ok_or("--sizes requires LIST")?)?;
                }
                "--draws" => {
                    index += 1;
                    parsed.draws = raw
                        .get(index)
                        .ok_or("--draws requires N")?
                        .parse()
                        .map_err(|_| "--draws must be a number".to_string())?;
                }
                "--seed" => {
                    index += 1;
                    parsed.seed = raw
                        .get(index)
                        .ok_or("--seed requires N")?
                        .parse()
                        .map_err(|_| "--seed must be a number".to_string())?;
                }
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 1;
        }
        if parsed.sizes.is_empty() {
            return Err("--sizes must name at least one size".to_string());
        }
        if parsed.draws == 0 {
            return Err("--draws must be at least one".to_string());
        }
        Ok(parsed)
    }

    fn parse_sizes(list: &str) -> Result<Vec<usize>, String> {
        list.split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| {
                token
                    .parse()
                    .map_err(|_| format!("invalid sample size {token}"))
            })
            .collect()
    }

    fn parse_duration(token: &str) -> Result<u32, String> {
        let (digits, multiplier) = match token.as_bytes().last() {
            Some(b'd') => (&token[..token.len() - 1], 86_400),
            Some(b'h') => (&token[..token.len() - 1], 3_600),
            Some(b'm') => (&token[..token.len() - 1], 60),
            Some(b's') => (&token[..token.len() - 1], 1),
            _ => (token, 1),
        };
        let count: u32 = digits
            .parse()
            .map_err(|_| format!("invalid duration {token}"))?;
        count
            .checked_mul(multiplier)
            .filter(|seconds| *seconds > 0)
            .ok_or_else(|| format!("duration {token} is out of range"))
    }
}

struct PooledWindow {
    sample: AdmittedSample,
    reference: f64,
}

struct ConvergenceApp;

impl ConvergenceApp {
    fn execute() -> ExitCode {
        let args = match ConvergenceArgs::parse() {
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

    #[allow(clippy::cast_precision_loss)]
    fn run(args: &ConvergenceArgs) -> Result<(), String> {
        let mut windows = CachedWindows::open(&args.data_dir)
            .map_err(|error| format!("open cached blocks: {error}"))?;
        let run = windows.longest_contiguous_run();
        let (Some((start_height, start_header)), Some((stop_height, stop_header))) =
            (run.first(), run.last())
        else {
            return Err("the data directory holds no contiguous cached blocks".to_string());
        };
        let first = start_header.time();
        let last = stop_header.time();
        eprintln!(
            "cached corpus: {} contiguous blocks {start_height}..={stop_height} ({:.2} days)",
            run.len(),
            f64::from(last.to_u32() - first.to_u32()) / 86_400.0,
        );

        let mut reference = IntradayReference::with_cache(&args.cache);
        reference.load_cache();
        if !args.offline {
            reference
                .ensure_span(first.to_u32(), last.to_u32())
                .map_err(|error| error.to_string())?;
            reference.store_cache();
        }
        if reference.is_empty() {
            return Err("no reference prices available; drop --offline to fetch them".to_string());
        }

        let pooled = Self::pool(&mut windows, &reference, args.window, first, last);
        if pooled.is_empty() {
            return Err("no window produced admitted outputs with a reference price".to_string());
        }
        let total: usize = pooled.iter().map(|window| window.sample.len()).sum();
        eprintln!(
            "pooled {} windows of {}s, {total} admitted outputs ({:.0} per window)",
            pooled.len(),
            args.window,
            total as f64 / pooled.len() as f64,
        );

        let rows = Self::sweep(&pooled, args);
        Self::report(&rows, &pooled);
        Ok(())
    }

    fn pool(
        windows: &mut CachedWindows,
        reference: &IntradayReference,
        duration: u32,
        first: BlockTime,
        last: BlockTime,
    ) -> Vec<PooledWindow> {
        let plan = SpanPlan::of_duration(duration);
        let mut pooled = Vec::new();
        for window in plan.tile(first.to_u32(), last.to_u32()) {
            let Some(expected) = reference.mean_over(window.start(), window.stop()) else {
                continue;
            };
            let Ok(sample) = Self::admitted_of(windows, window.selection()) else {
                continue;
            };
            if !sample.is_empty() {
                pooled.push(PooledWindow {
                    sample,
                    reference: expected,
                });
            }
        }
        pooled
    }

    fn admitted_of(
        windows: &mut CachedWindows,
        selection: delphi_oracle::RangeSelection,
    ) -> Result<AdmittedSample, String> {
        let mut pipeline = OraclePipeline::new();
        windows
            .feed(selection, &mut pipeline)
            .map_err(|error| error.to_string())?;
        Ok(pipeline.into_sample())
    }

    fn sweep(pooled: &[PooledWindow], args: &ConvergenceArgs) -> Vec<ConvergenceRow> {
        let mut rows = Vec::new();
        for size in &args.sizes {
            let mut rng = SampleRng::seeded(args.seed ^ *size as u64);
            let mut trial = ConvergenceTrial::new(*size);
            for draw in 0..args.draws {
                let window = &pooled[draw % pooled.len()];
                if window.sample.len() < *size {
                    continue;
                }
                let picks = rng.choose(window.sample.len(), *size);
                match window.sample.subset(&picks).price() {
                    Ok(report) => trial.record(report.central_price, window.reference),
                    Err(_) => trial.record_attempt(),
                }
            }
            let row = trial.finish();
            eprintln!(
                "  {:>6}: {}/{} draws priced",
                row.sample_size(),
                row.priced(),
                row.attempted()
            );
            rows.push(row);
        }
        rows
    }

    #[allow(clippy::cast_precision_loss)]
    fn report(rows: &[ConvergenceRow], pooled: &[PooledWindow]) {
        let per_window = pooled.iter().map(|window| window.sample.len()).sum::<usize>() as f64
            / pooled.len() as f64;
        println!(
            "\n{:>8} {:>10} {:>9} {:>9} {:>8} {:>8} {:>9}",
            "outputs", "priced", "median", "p90", "<1%", "<5%", "minutes"
        );
        for row in rows {
            if row.priced() == 0 {
                println!(
                    "{:>8} {:>10} {:>9} {:>9} {:>8} {:>8} {:>9}",
                    row.sample_size(),
                    format!("0/{}", row.attempted()),
                    "-",
                    "-",
                    "-",
                    "-",
                    "-"
                );
                continue;
            }
            println!(
                "{:>8} {:>10} {:>8.3}% {:>8.3}% {:>7.1}% {:>7.1}% {:>9.1}",
                row.sample_size(),
                format!("{}/{}", row.priced(), row.attempted()),
                row.median_error() * 100.0,
                row.p90_error() * 100.0,
                row.within_one_percent() * 100.0,
                row.within_five_percent() * 100.0,
                Self::mempool_minutes(row.sample_size()),
            );
        }
        println!(
            "\nper-window admitted outputs: {per_window:.0}; \
             minutes = sample size at the measured mempool rate of {:.2} admitted outputs/s",
            Self::MEMPOOL_OUTPUTS_PER_SECOND
        );
    }

    const MEMPOOL_OUTPUTS_PER_SECOND: f64 = 4.01;

    #[allow(clippy::cast_precision_loss)]
    fn mempool_minutes(sample_size: usize) -> f64 {
        sample_size as f64 / Self::MEMPOOL_OUTPUTS_PER_SECOND / 60.0
    }
}

fn main() -> ExitCode {
    ConvergenceApp::execute()
}
