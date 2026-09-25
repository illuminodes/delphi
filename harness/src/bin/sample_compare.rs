use std::path::PathBuf;
use std::process::ExitCode;

use mantle_harness::amount_dump::AmountDump;
use mantle_harness::confirmed_pools::ConfirmedPools;
use mantle_harness::intraday_reference::IntradayReference;
use mantle_harness::octave_separation::OctaveSeparation;
use mantle_harness::signal_density::SignalDensity;
use mantle_harness::stencil_variant::StencilVariant;
use mantle_harness::span_plan::SpanPlan;
use mantle_harness::subsample_sweep::{DrawMode, PricedPool, SubsampleSweep};
use mantle_oracle::{AdmittedSample, CachedWindows, RoughPriceEstimator};

const HELP: &str = "\
oracle-sample-compare — bootstrap live mempool dumps against confirmed windows

usage: oracle-sample-compare [options] DUMP...

options:
  --data-dir PATH    Cached blocks, for the confirmed pools (default data/oracle/mainnet)
  --cache PATH       Intraday reference cache (default intraday-prices.txt)
  --window DURATION  Confirmed pool width (default 20m)
  --sizes LIST       Sample sizes (default 50,100,150,200,300,400,800)
  --draws N          Draws per size and mode (default 240)
  --seed N           PRNG seed (default 20260416)
  --offline          Never contact CoinGecko; use the cache as-is
  --weights LIST     Smooth-stencil weights to grid over (enables grid mode)
  --smooth-below LIST  Slides below which the smooth term applies (default 150)
  --octave           Also report whether the rough estimator's octave ratio
                     separates octave locks from sound draws
  -h, --help         Print this message

Each DUMP is a file written by `mempool-convergence --dump`. The dumps form
the mempool pools, each priced against the mean reference over its own time
span. Both populations are resampled at every size, with random and with
contiguous (arrival-order) draws, and their round-USD signal density is
compared at the true price.
";

struct CompareArgs {
    dumps: Vec<PathBuf>,
    data_dir: PathBuf,
    cache: PathBuf,
    window: u32,
    sizes: Vec<usize>,
    draws: usize,
    seed: u64,
    offline: bool,
    octave: bool,
    weights: Vec<f64>,
    smooth_below: Vec<i32>,
}

impl CompareArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut parsed = Self {
            dumps: Vec::new(),
            data_dir: PathBuf::from("data/oracle/mainnet"),
            cache: PathBuf::from("intraday-prices.txt"),
            window: 1_200,
            sizes: vec![50, 100, 150, 200, 300, 400, 800],
            draws: 240,
            seed: 20_260_416,
            offline: false,
            octave: false,
            weights: Vec::new(),
            smooth_below: vec![RoughPriceEstimator::DEFAULT_SMOOTH_BELOW],
        };
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Err(String::new());
                }
                "--offline" => parsed.offline = true,
                "--octave" => parsed.octave = true,
                "--weights" => {
                    index += 1;
                    parsed.weights =
                        Self::parse_list(raw.get(index).ok_or("--weights requires LIST")?)?;
                }
                "--smooth-below" => {
                    index += 1;
                    parsed.smooth_below =
                        Self::parse_list(raw.get(index).ok_or("--smooth-below requires LIST")?)?;
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
                    parsed.draws = Self::parse_number(raw.get(index), "--draws")?;
                }
                "--seed" => {
                    index += 1;
                    parsed.seed = Self::parse_number(raw.get(index), "--seed")?;
                }
                flag if flag.starts_with("--") => {
                    return Err(format!("unrecognized argument {flag}"));
                }
                path => parsed.dumps.push(PathBuf::from(path)),
            }
            index += 1;
        }
        if parsed.dumps.is_empty() {
            return Err("name at least one DUMP file".to_string());
        }
        if parsed.sizes.is_empty() || parsed.draws == 0 {
            return Err("--sizes and --draws must be non-empty".to_string());
        }
        Ok(parsed)
    }

    fn parse_number<T: std::str::FromStr>(token: Option<&String>, flag: &str) -> Result<T, String> {
        token
            .ok_or_else(|| format!("{flag} requires N"))?
            .parse()
            .map_err(|_| format!("invalid {flag}"))
    }

    fn parse_sizes(list: &str) -> Result<Vec<usize>, String> {
        Self::parse_list(list)
    }

    fn parse_list<T: std::str::FromStr>(list: &str) -> Result<Vec<T>, String> {
        list.split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| token.parse().map_err(|_| format!("invalid list entry {token}")))
            .collect()
    }

    fn parse_duration(token: &str) -> Result<u32, String> {
        let (digits, multiplier) = match token.as_bytes().last() {
            Some(b'h') => (&token[..token.len() - 1], 3_600),
            Some(b'm') => (&token[..token.len() - 1], 60),
            Some(b's') => (&token[..token.len() - 1], 1),
            _ => (token, 1),
        };
        digits
            .parse::<u32>()
            .ok()
            .and_then(|count| count.checked_mul(multiplier))
            .filter(|seconds| *seconds > 0)
            .ok_or_else(|| format!("invalid duration {token}"))
    }
}

struct Population {
    label: &'static str,
    pools: Vec<PricedPool>,
}

impl Population {
    fn density(&self) -> SignalDensity {
        let mut density = SignalDensity::new();
        for pool in &self.pools {
            for output in pool.sample().outputs() {
                density.absorb(output.amount, pool.reference());
            }
        }
        density
    }

    fn outputs(&self) -> usize {
        self.pools.iter().map(|pool| pool.sample().len()).sum()
    }
}

struct CompareApp;

impl CompareApp {
    const REFERENCE_TOLERANCE: u32 = 3_600;

    fn execute() -> ExitCode {
        let args = match CompareArgs::parse() {
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

    fn run(args: &CompareArgs) -> Result<(), String> {
        let dumps = Self::read_dumps(args)?;
        let mut windows = CachedWindows::open(&args.data_dir)
            .map_err(|error| format!("open cached blocks: {error}"))?;
        let run = windows.longest_contiguous_run();
        let (Some((_, first)), Some((_, last))) = (run.first(), run.last()) else {
            return Err("the data directory holds no contiguous cached blocks".to_string());
        };
        let (confirmed_start, confirmed_stop) = (first.time().to_u32(), last.time().to_u32());

        let mut reference = IntradayReference::with_cache(&args.cache);
        reference.load_cache();
        if !args.offline {
            let mut spans = vec![(confirmed_start, confirmed_stop)];
            spans.extend(dumps.iter().filter_map(|(_, sample)| Self::time_span(sample)));
            for (start, stop) in spans {
                reference
                    .ensure_span(start.saturating_sub(3_600), stop + 3_600)
                    .map_err(|error| error.to_string())?;
            }
            reference.store_cache();
        }

        let mempool = Population {
            label: "mempool",
            pools: Self::mempool_pools(dumps, &reference),
        };
        if mempool.pools.is_empty() {
            return Err("no dump has a reference price over its time span".to_string());
        }
        let confirmed = Population {
            label: "confirmed",
            pools: ConfirmedPools::of_duration(args.window).gather(
                &mut windows,
                &reference,
                confirmed_start,
                confirmed_stop,
            ),
        };
        eprintln!(
            "mempool: {} dumps, {} outputs; confirmed: {} windows of {}, {} outputs",
            mempool.pools.len(),
            mempool.outputs(),
            confirmed.pools.len(),
            SpanPlan::of_duration(args.window).label(),
            confirmed.outputs(),
        );

        if !args.weights.is_empty() {
            Self::report_grid(args, &[&mempool, &confirmed]);
            return Ok(());
        }
        Self::report_density(&[&mempool, &confirmed]);
        Self::report_curves(args, &[&mempool, &confirmed]);
        if args.octave {
            Self::report_octave(args, &[&mempool, &confirmed]);
        }
        Ok(())
    }

    fn read_dumps(args: &CompareArgs) -> Result<Vec<(PathBuf, AdmittedSample)>, String> {
        args.dumps
            .iter()
            .map(|path| {
                AmountDump::at(path)
                    .read()
                    .map(|sample| (path.clone(), sample))
                    .map_err(|error| format!("read {}: {error}", path.display()))
            })
            .collect()
    }

    fn time_span(sample: &AdmittedSample) -> Option<(u32, u32)> {
        let times = sample.outputs().iter().map(|output| output.time.to_u32());
        Some((times.clone().min()?, times.max()?))
    }

    fn mempool_pools(
        dumps: Vec<(PathBuf, AdmittedSample)>,
        reference: &IntradayReference,
    ) -> Vec<PricedPool> {
        let mut pools = Vec::new();
        for (path, sample) in dumps {
            let Some(price) = Self::time_span(&sample)
                .and_then(|(start, stop)| {
                    reference.mean_near(start, stop, Self::REFERENCE_TOLERANCE)
                })
            else {
                eprintln!("skipping {}: no reference over its span", path.display());
                continue;
            };
            eprintln!(
                "  {}: {} outputs, reference ${price:.2}",
                path.display(),
                sample.len()
            );
            pools.push(PricedPool::new(sample, price));
        }
        pools
    }

    fn report_density(populations: &[&Population]) {
        println!(
            "\nround-USD signal at the true price (tight = within {:.0}%, uniform baseline {:.1}%)",
            SignalDensity::TIGHT_BAND * 100.0,
            SignalDensity::UNIFORM_TIGHT_SHARE * 100.0
        );
        println!(
            "{:>10} {:>9} {:>11} {:>11} {:>11} {:>11}",
            "population", "outputs", "cand/out", "tight/out", "tight share", "enrichment"
        );
        for population in populations {
            let density = population.density();
            println!(
                "{:>10} {:>9} {:>11.3} {:>11.4} {:>10.2}% {:>10.2}x",
                population.label,
                density.outputs(),
                density.candidates_per_output(),
                density.tight_per_output(),
                density.tight_share() * 100.0,
                density.enrichment(),
            );
        }
    }

    fn report_grid(args: &CompareArgs, populations: &[&Population]) {
        println!("\nrandom draws; each cell is p90 error / share within 1%");
        print!("{:>10} {:>14}", "population", "variant");
        for size in &args.sizes {
            print!(" {size:>15}");
        }
        println!();
        for population in populations {
            for variant in StencilVariant::grid(&args.weights, &args.smooth_below) {
                let sweep = SubsampleSweep::new(&population.pools, args.draws, args.seed)
                    .with_estimator(variant.estimator());
                print!("{:>10} {:>14}", population.label, variant.label());
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

    fn report_octave(args: &CompareArgs, populations: &[&Population]) {
        const THRESHOLDS: [f64; 5] = [0.5, 0.6, 0.7, 0.8, 0.9];
        println!(
            "\noctave ratio (rival lead / best lead); locked = error > {:.0}%, sound = error < {:.0}%",
            OctaveSeparation::LOCK_ERROR * 100.0,
            OctaveSeparation::SOUND_ERROR * 100.0
        );
        print!(
            "{:>10} {:>7} {:>6} {:>6} {:>6} {:>12} {:>12}",
            "population", "size", "sound", "locked", "AUC", "sound p50/90", "lock p10/50"
        );
        for threshold in THRESHOLDS {
            print!(" {:>11}", format!("flag>={threshold:.1}"));
        }
        println!();
        for population in populations {
            let sweep = SubsampleSweep::new(&population.pools, args.draws, args.seed);
            let mut pooled = OctaveSeparation::new();
            for size in &args.sizes {
                let mut separation = OctaveSeparation::new();
                sweep.separate(*size, DrawMode::Random, &mut separation);
                sweep.separate(*size, DrawMode::Random, &mut pooled);
                Self::print_separation(population.label, &size.to_string(), &separation, &THRESHOLDS);
            }
            Self::print_separation(population.label, "all", &pooled, &THRESHOLDS);
        }
        println!("flag>=t columns: share of sound / share of locked draws a ratio >= t rule rejects");
    }

    fn print_separation(label: &str, size: &str, separation: &OctaveSeparation, thresholds: &[f64]) {
        let pair = |low: Option<f64>, high: Option<f64>| match (low, high) {
            (Some(low), Some(high)) => format!("{low:.2}/{high:.2}"),
            _ => "-".to_string(),
        };
        print!(
            "{:>10} {:>7} {:>6} {:>6} {:>6} {:>12} {:>12}",
            label,
            size,
            separation.sound(),
            separation.locked(),
            separation
                .auc()
                .map_or_else(|| "-".to_string(), |auc| format!("{auc:.3}")),
            pair(separation.sound_quantile(0.5), separation.sound_quantile(0.9)),
            pair(separation.locked_quantile(0.1), separation.locked_quantile(0.5)),
        );
        for threshold in thresholds {
            let (sound, locked) = separation.flagged_at(*threshold);
            let cell = if sound.is_nan() || locked.is_nan() {
                "-".to_string()
            } else {
                format!("{:.0}/{:.0}", sound * 100.0, locked * 100.0)
            };
            print!(" {cell:>11}");
        }
        println!();
    }

    fn report_curves(args: &CompareArgs, populations: &[&Population]) {
        println!(
            "\n{:>10} {:>10} {:>7} {:>9} {:>9} {:>9} {:>7}",
            "population", "draw", "size", "priced", "median", "p90", "<1%"
        );
        for population in populations {
            let sweep = SubsampleSweep::new(&population.pools, args.draws, args.seed);
            for mode in [DrawMode::Random, DrawMode::Contiguous] {
                for size in &args.sizes {
                    let row = sweep.row(*size, mode);
                    if row.priced() == 0 {
                        println!(
                            "{:>10} {:>10} {:>7} {:>9} {:>9} {:>9} {:>7}",
                            population.label,
                            mode.label(),
                            size,
                            format!("0/{}", row.attempted()),
                            "-",
                            "-",
                            "-"
                        );
                        continue;
                    }
                    println!(
                        "{:>10} {:>10} {:>7} {:>9} {:>8.3}% {:>8.2}% {:>6.1}%",
                        population.label,
                        mode.label(),
                        size,
                        format!("{}/{}", row.priced(), row.attempted()),
                        row.median_error() * 100.0,
                        row.p90_error() * 100.0,
                        row.within_one_percent() * 100.0,
                    );
                }
            }
        }
    }
}

fn main() -> ExitCode {
    CompareApp::execute()
}
