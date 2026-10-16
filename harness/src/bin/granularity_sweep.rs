use std::path::PathBuf;
use std::process::ExitCode;

use mantle::difficulty::BlockTime;
use delphi_harness::granularity::{GranularityRow, GranularitySweep};
use delphi_harness::intraday_reference::IntradayReference;
use delphi_harness::span_plan::SpanPlan;
use delphi_oracle::{BlockRange, CachedWindows, RangeSelection};

const HELP: &str = "\
oracle-granularity — find the shortest window the oracle can still price

usage: oracle-granularity [options]

options:
  --data-dir PATH        Cached blocks and headers (default data/oracle/mainnet)
  --cache PATH           Intraday reference cache (default intraday-prices.txt)
  --durations LIST       Comma-separated window lengths (default 1d,12h,6h,3h,1h,30m,15m)
  --offline              Never contact CoinGecko; use the cache as-is
  -h, --help             Print this message

Reads only blocks already cached in the data directory, so it never touches the
Bitcoin network. Each duration tiles the cached span with non-overlapping
windows and scores every window against the interpolated reference price.
";

struct SweepArgs {
    data_dir: PathBuf,
    cache: PathBuf,
    durations: Vec<u32>,
    offline: bool,
}

impl SweepArgs {
    fn parse() -> Result<Self, String> {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        let mut data_dir = PathBuf::from("data/oracle/mainnet");
        let mut cache = PathBuf::from("intraday-prices.txt");
        let mut durations = vec![86_400, 43_200, 21_600, 10_800, 3_600, 1_800, 900];
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
                "--durations" => {
                    index += 1;
                    let list = raw.get(index).ok_or("--durations requires LIST")?;
                    durations = Self::parse_durations(list)?;
                }
                other => return Err(format!("unrecognized argument {other}")),
            }
            index += 1;
        }
        if durations.is_empty() {
            return Err("--durations must name at least one window length".to_string());
        }
        Ok(Self {
            data_dir,
            cache,
            durations,
            offline,
        })
    }

    fn parse_durations(list: &str) -> Result<Vec<u32>, String> {
        list.split(',')
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(Self::parse_duration)
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

struct SweepApp;

impl SweepApp {
    fn execute() -> ExitCode {
        let args = match SweepArgs::parse() {
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

    fn run(args: &SweepArgs) -> Result<(), String> {
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
        eprintln!("reference: {} price points cached", reference.len());

        let rows = Self::sweep(&mut windows, &reference, &args.durations, first, last);
        Self::report(&rows);
        Ok(())
    }

    fn sweep(
        windows: &mut CachedWindows,
        reference: &IntradayReference,
        durations: &[u32],
        first: BlockTime,
        last: BlockTime,
    ) -> Vec<GranularityRow> {
        let mut rows = Vec::new();
        for duration in durations {
            let plan = SpanPlan::of_duration(*duration);
            let mut sweep = GranularitySweep::new(plan.label(), *duration);
            for window in plan.tile(first.to_u32(), last.to_u32()) {
                let Some(expected) = reference.mean_over(window.start(), window.stop()) else {
                    sweep.record_attempt();
                    continue;
                };
                let selection = window.selection();
                let blocks = Self::block_count(windows, selection);
                match windows.run(selection) {
                    Ok(report) => sweep.record(
                        report.central_price,
                        expected,
                        report.sample_count,
                        blocks,
                    ),
                    Err(_) => sweep.record_attempt(),
                }
            }
            let row = sweep.finish();
            eprintln!(
                "  {:>4}: {}/{} windows scored",
                row.label(),
                row.scored(),
                row.attempted()
            );
            rows.push(row);
        }
        rows
    }

    fn block_count(windows: &CachedWindows, selection: RangeSelection) -> usize {
        BlockRange::select(windows.headers(), selection)
            .map(|range| range.headers(windows.headers()).len())
            .unwrap_or_default()
    }

    fn report(rows: &[GranularityRow]) {
        println!(
            "{:<7} {:>9} {:>8} {:>8} {:>9} {:>8} {:>7} {:>8} {:>7}",
            "window", "scored", "median", "p90", "worst", "<1%", "bad", "samples", "blocks"
        );
        for row in rows {
            if row.scored() == 0 {
                println!(
                    "{:<7} {:>9} {:>8} {:>8} {:>9} {:>8} {:>7} {:>8} {:>7}",
                    row.label(),
                    format!("0/{}", row.attempted()),
                    "-",
                    "-",
                    "-",
                    "-",
                    "-",
                    "-",
                    "-"
                );
                continue;
            }
            println!(
                "{:<7} {:>9} {:>7.3}% {:>7.3}% {:>8.2}% {:>7.1}% {:>7} {:>8.0} {:>7.1}",
                row.label(),
                format!("{}/{}", row.scored(), row.attempted()),
                row.median_deviation() * 100.0,
                row.p90_deviation() * 100.0,
                row.worst_deviation() * 100.0,
                row.within_one_percent() * 100.0,
                row.blowups(),
                row.mean_samples(),
                row.mean_blocks(),
            );
        }
    }
}

fn main() -> ExitCode {
    SweepApp::execute()
}
