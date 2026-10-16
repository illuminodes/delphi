mod cli;

use std::process::ExitCode;

use delphi_oracle::{PeerSources, WindowRunner};

use crate::cli::{Args, HELP};

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
        let mut peer_sources = PeerSources::new();
        for peer in &args.peers {
            peer_sources.add_explicit(*peer);
        }
        for path in &args.peer_files {
            peer_sources.add_file(path);
        }
        let peers = peer_sources.resolve(args.network);
        if peers.is_empty() {
            return Err(format!(
                "no peers available for {}; supply --peer or --peer-file",
                args.network.name()
            ));
        }
        let runner = WindowRunner::new(args.network, &args.data_dir, peers);
        let report = runner.run(args.selection)?;
        println!("{report}");
        Ok(())
    }
}

fn main() -> ExitCode {
    OracleApp::execute()
}
