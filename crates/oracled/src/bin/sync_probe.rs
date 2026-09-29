use std::process::ExitCode;
use std::time::Instant;

use mantle::network::Network;
use mantle_oracle::PeerSources;
use mantle_oracled::ChainSync;

struct ProbeApp {
    data_dir: String,
    depth: usize,
}

impl ProbeApp {
    fn parse() -> Self {
        let mut arguments = std::env::args().skip(1);
        let data_dir = arguments
            .next()
            .unwrap_or_else(|| "data/oracled-probe".to_string());
        let depth = arguments
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(6);
        Self { data_dir, depth }
    }

    fn execute() -> ExitCode {
        match Self::parse().run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fatal: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(&self) -> Result<(), String> {
        let peers = PeerSources::new().resolve(Network::Mainnet);
        eprintln!("{} candidate peers, depth {}", peers.len(), self.depth);
        let mut sync = ChainSync::open(
            Network::Mainnet,
            peers,
            &self.data_dir,
            self.depth,
            1_000_000_000,
        )?;
        for pass in 1..=2 {
            let started = Instant::now();
            let report = sync.sync()?;
            eprintln!(
                "pass {pass}: synced to {} in {:.1}s: {} records appended, caught up {}, cache purged {}, block cache {} bytes",
                report.tip,
                started.elapsed().as_secs_f64(),
                report.appended,
                report.caught_up,
                report.purged_cache,
                sync.cache().bytes()
            );
        }
        for blocks in [1, self.depth] {
            let Some(sample) = sync.log().sample(blocks) else {
                println!("{blocks:>5} blocks: not enough records");
                continue;
            };
            println!(
                "{blocks:>5} blocks: {} outputs, price {}",
                sample.len(),
                sample
                    .price()
                    .map_or_else(|error| error.to_string(), |report| format!("{:.0}", report.central_price))
            );
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    ProbeApp::execute()
}
