use std::path::PathBuf;
use std::process::ExitCode;

use delphi_harness::intraday_reference::IntradayReference;
use delphi_harness::span_plan::SpanPlan;
use delphi_harness::volatility::{VolatilityBand, VolatilityPoint, VolatilityStudy};
use delphi_oracle::CachedWindows;

const HELP: &str = "\
oracle-volatility — measure how the oracle tracks a sudden price move

usage: oracle-volatility [options]

options:
  --data-dir PATH        Cached blocks and headers (default data/oracle/mainnet)
  --cache PATH           Intraday reference cache (default intraday-prices.txt)
  --duration SPEC        Window length to sweep (default 1h)
  --threshold PERCENT    Move that marks a window volatile (default 0.5)
  --offline              Never contact CoinGecko; use the cache as-is
  -h, --help             Print this message

Tiles the cached span with windows, scores each against the interpolated
reference, and splits them into calm and volatile groups. Reports signed error
so a lagging estimate is visible as bias rather than hidden inside magnitude.
";

struct ProbeArgs {
    data_dir: PathBuf,
    cache: PathBuf,
    duration: u32,
    threshold: f64,
    offline: bool,
}

impl ProbeArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut data_dir = PathBuf::from("data/oracle/mainnet");
        let mut cache = PathBuf::from("intraday-prices.txt");
        let mut duration = 3_600;
        let mut threshold = 0.5;
        let mut offline = false;
        let mut index = 0;
        while index < raw.len() {
            match raw[index].as_str() {
                "-h" | "--help" => {
                    println!("{HELP}");
                    return Err(String::new());
                }
                "--offline" => offline = true,
                "--data-dir" => {
                    index += 1;
                    data_dir = PathBuf::from(raw.get(index).ok_or("--data-dir requires PATH")?);
                }
                "--cache" => {
                    index += 1;
                    cache = PathBuf::from(raw.get(index).ok_or("--cache requires PATH")?);
                }
                "--duration" => {
                    index += 1;
                    let token = raw.get(index).ok_or("--duration requires SPEC")?;
                    duration = Self::parse_duration(token)?;
                }
                "--threshold" => {
                    index += 1;
                    let token = raw.get(index).ok_or("--threshold requires PERCENT")?;
                    threshold = token
                        .parse()
                        .map_err(|_| format!("invalid --threshold {token}"))?;
                }
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 1;
        }
        Ok(Self {
            data_dir,
            cache,
            duration,
            threshold: threshold / 100.0,
            offline,
        })
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

struct ProbeApp;

impl ProbeApp {
    fn execute() -> ExitCode {
        let args = match ProbeArgs::parse() {
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

    fn run(args: &ProbeArgs) -> Result<(), String> {
        let mut windows = CachedWindows::open(&args.data_dir)
            .map_err(|error| format!("open cached blocks: {error}"))?;
        let run = windows.longest_contiguous_run();
        let (Some((start_height, start_header)), Some((stop_height, stop_header))) =
            (run.first(), run.last())
        else {
            return Err("the data directory holds no contiguous cached blocks".to_string());
        };
        let first = start_header.time().to_u32();
        let last = stop_header.time().to_u32();
        eprintln!(
            "cached corpus: {} blocks {start_height}..={stop_height} ({:.2} days)",
            run.len(),
            f64::from(last - first) / 86_400.0,
        );

        let mut reference = IntradayReference::with_cache(&args.cache);
        reference.load_cache();
        if !args.offline {
            reference
                .ensure_span(first, last)
                .map_err(|error| error.to_string())?;
            reference.store_cache();
        }
        if reference.is_empty() {
            return Err("no reference prices available; drop --offline to fetch them".to_string());
        }

        let study = Self::sweep(&mut windows, &reference, args, first, last);
        if study.is_empty() {
            return Err("no windows could be scored".to_string());
        }
        Self::report(&study, args.threshold);
        Ok(())
    }

    fn sweep(
        windows: &mut CachedWindows,
        reference: &IntradayReference,
        args: &ProbeArgs,
        first: u32,
        last: u32,
    ) -> VolatilityStudy {
        let plan = SpanPlan::of_duration(args.duration);
        let mut study = VolatilityStudy::new(args.threshold);
        for window in plan.tile(first, last) {
            let (Some(expected), Some(open), Some(close)) = (
                reference.mean_over(window.start(), window.stop()),
                reference.price_at(window.start()),
                reference.price_at(window.stop()),
            ) else {
                continue;
            };
            if let Ok(report) = windows.run(window.selection()) {
                study.record(VolatilityPoint::new(
                    window.midpoint(),
                    report.central_price,
                    expected,
                    open,
                    close,
                    report.sample_count,
                ));
            }
        }
        study
    }

    fn report(study: &VolatilityStudy, threshold: f64) {
        println!(
            "{:>12} {:>11} {:>11} {:>9} {:>9} {:>8}",
            "window utc", "estimate", "reference", "signed", "move", "samples"
        );
        for point in study.points() {
            println!(
                "{:>12} {:>11.2} {:>11.2} {:>+8.3}% {:>+8.3}% {:>8}",
                point.midpoint(),
                point.estimate(),
                point.reference(),
                point.signed_error() * 100.0,
                point.market_move() * 100.0,
                point.samples(),
            );
        }

        println!();
        println!(
            "{:<10} {:>7} {:>6} {:>10} {:>10} {:>10} {:>10}",
            "band", "windows", "bad", "median", "bias", "worst", "mean move"
        );
        for band in [study.calm(), study.volatile(), study.overall()] {
            Self::print_band(&band);
        }
        println!();
        println!(
            "volatile threshold {:.2}%  lag correlation {:+.3}",
            threshold * 100.0,
            study.lag_correlation(),
        );
    }

    fn print_band(band: &VolatilityBand) {
        if band.count() == 0 {
            println!(
                "{:<10} {:>7} {:>6} {:>10} {:>10} {:>10} {:>10}",
                band.label(),
                0,
                0,
                "-",
                "-",
                "-",
                "-"
            );
            return;
        }
        println!(
            "{:<10} {:>7} {:>6} {:>9.3}% {:>+9.3}% {:>9.3}% {:>9.3}%",
            band.label(),
            band.count(),
            band.blowups(),
            band.median_absolute() * 100.0,
            band.median_signed() * 100.0,
            band.worst_absolute() * 100.0,
            band.mean_move() * 100.0,
        );
    }
}

fn main() -> ExitCode {
    ProbeApp::execute()
}
