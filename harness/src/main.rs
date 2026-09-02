mod cli;

use std::process::ExitCode;

use mantle::network::Network;
use mantle_harness::reference_prices::{ReferenceError, ReferencePrices};
use mantle_harness::score::Scorecard;
use mantle_harness::window_plan::WindowPlan;
use mantle_oracle::{PeerSources, UtcDate, WindowRunner};

use crate::cli::{Args, HELP};

struct HarnessApp;

impl HarnessApp {
    fn execute() -> ExitCode {
        let args = match Args::parse() {
            Ok(args) => args,
            Err(error) if error.is_empty() => return ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: {error}\n\n{HELP}");
                return ExitCode::from(2);
            }
        };
        match Self::run(&args) {
            Ok(passed) if passed => ExitCode::SUCCESS,
            Ok(_) => ExitCode::FAILURE,
            Err(error) => {
                eprintln!("fatal: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(args: &Args) -> Result<bool, String> {
        if args.network != Network::Mainnet {
            return Err(format!(
                "reference prices only exist for mainnet; {} cannot be scored",
                args.network.name()
            ));
        }
        let peers = Self::resolve_peers(args);
        if peers.is_empty() {
            return Err(format!(
                "no peers available for {}; supply --peer or --peer-file",
                args.network.name()
            ));
        }
        let windows = Self::plan_windows(args);
        let mut references = Self::load_references(args, &windows)?;
        let mut scorecard = Scorecard::new(args.tolerance);
        let runner = WindowRunner::new(args.network, &args.data_dir, peers);
        for window in &windows {
            eprintln!("scoring window {}", window.label());
            let Some(reference) = references.day_average(window.date) else {
                scorecard.record_failure(&window.label(), "missing reference price");
                continue;
            };
            match runner.run(window.selection()) {
                Ok(report) => {
                    scorecard.record_success(&window.label(), &report, reference);
                }
                Err(error) => {
                    scorecard.record_failure(&window.label(), &error);
                }
            }
            references.store_cache();
        }
        Self::render(&scorecard);
        Ok(scorecard.passed())
    }

    fn resolve_peers(args: &Args) -> Vec<std::net::SocketAddrV4> {
        let mut sources = PeerSources::new();
        for peer in &args.peers {
            sources.add_explicit(*peer);
        }
        for path in &args.peer_files {
            sources.add_file(path);
        }
        sources.resolve(args.network)
    }

    fn plan_windows(args: &Args) -> Vec<WindowPlan> {
        let last = match args.last_date {
            Some(date) => date,
            None => UtcDate::containing(mantle::difficulty::BlockTime::new(
                u32::try_from(Self::now_seconds().saturating_sub(86_400)).expect("timestamp fits"),
            )),
        };
        WindowPlan::ending_on(last, args.windows)
    }

    fn now_seconds() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default()
    }

    fn load_references(args: &Args, windows: &[WindowPlan]) -> Result<ReferencePrices, String> {
        let dates: Vec<UtcDate> = windows.iter().map(|window| window.date).collect();
        let mut references = ReferencePrices::with_cache(args.reference_cache_path());
        references.load_cache();
        if args.offline {
            return Ok(references);
        }
        references
            .ensure_daily_snapshots(&dates, args.network)
            .map_err(|error| match error {
                ReferenceError::MissingSnapshot(date) => {
                    format!("reference data unavailable for {date}; use --offline to skip fetching")
                }
                other => other.to_string(),
            })?;
        references.store_cache();
        Ok(references)
    }

    fn render(scorecard: &Scorecard) {
        println!("{:<12} {:>12} {:>12} {:>10} {:>9}", "date", "estimate", "reference", "deviation", "samples");
        for score in scorecard.scores() {
            println!(
                "{:<12} {:>12.2} {:>12.2} {:>9.3}% {:>9}",
                score.date,
                score.estimated,
                score.reference,
                score.deviation * 100.0,
                score.samples
            );
        }
        match (scorecard.median_absolute_deviation(), scorecard.worst_deviation()) {
            (Some(median), Some(worst)) => println!(
                "median |deviation| {:.3}%  worst {:.3}%  tolerance {:.2}%  → {}",
                median * 100.0,
                worst * 100.0,
                scorecard.tolerance() * 100.0,
                if scorecard.passed() { "PASS" } else { "FAIL" }
            ),
            _ => println!("no windows were scored"),
        }
    }
}

fn main() -> ExitCode {
    HarnessApp::execute()
}
