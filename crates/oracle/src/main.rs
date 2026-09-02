mod cli;
mod data_dir_lock;

use std::process::ExitCode;

use mantle::difficulty::BlockHeight;
use mantle_fetcher::Fetcher;
use mantle_oracle::{BlockRange, OraclePipeline};

use crate::cli::{Args, HELP};
use crate::data_dir_lock::DataDirLock;

struct OracleApp;

impl OracleApp {
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
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fatal: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(args: &Args) -> Result<(), String> {
        std::fs::create_dir_all(&args.data_dir)
            .map_err(|error| format!("create {}: {error}", args.data_dir.display()))?;
        let _data_dir_lock = DataDirLock::acquire(&args.data_dir)
            .map_err(|error| format!("lock data directory: {error}"))?;
        let peers = args.resolve_peers();
        if peers.is_empty() {
            return Err(format!(
                "no peers available for {}; supply --peer or --peer-file",
                args.network.name()
            ));
        }
        eprintln!(
            "syncing {} headers with {} candidate peers",
            args.network.name(),
            peers.len()
        );
        let mut fetcher = Fetcher::new(args.network, peers, &args.data_dir)
            .map_err(|error| format!("initialize fetcher: {error}"))?;
        let headers = fetcher
            .sync_headers(
                BlockHeight::new(1),
                BlockHeight::new(u32::MAX),
                args.network.genesis_hash(),
            )
            .map_err(|error| format!("synchronize headers: {error}"))?;
        let range = BlockRange::select(&headers, args.selection)
            .map_err(|error| format!("select block range: {error}"))?;
        let selected_headers = range.headers(&headers);
        eprintln!(
            "fetching {} blocks ({}..={})",
            selected_headers.len(),
            range.start(),
            range.stop()
        );
        fetcher
            .fetch_blocks(selected_headers)
            .map_err(|error| format!("fetch blocks: {error}"))?;
        let mut pipeline = OraclePipeline::new();
        for (height, header) in selected_headers {
            let block = fetcher
                .read_block_decoded(*height, header.hash())
                .map_err(|error| format!("decode block {height}: {error}"))?;
            for transaction in &block.transactions {
                pipeline.process(*height, header.time(), transaction);
            }
        }
        let report = pipeline
            .finish()
            .map_err(|error| format!("compute oracle report: {error}"))?;
        println!("{report}");
        Ok(())
    }
}

fn main() -> ExitCode {
    OracleApp::execute()
}
