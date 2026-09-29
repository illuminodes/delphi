use std::process::ExitCode;
use std::time::Duration;

use nostro2_relay::{NostrRelay, ReconnectConfig};

struct ProbeApp;

impl ProbeApp {
    fn execute() -> ExitCode {
        let url = std::env::args()
            .nth(1)
            .unwrap_or_else(|| "wss://192.168.1.239:4848".to_string());
        eprintln!("connecting to {url}");
        let reconnect = ReconnectConfig {
            max_retries: 1,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_millis(200),
            backoff_multiplier: 1.0,
        };
        match NostrRelay::connect_blocking_with(&url, reconnect) {
            Ok(relay) => {
                eprintln!("connected: {}", relay.url());
                relay.close();
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("failed: {error}");
                ExitCode::FAILURE
            }
        }
    }
}

fn main() -> ExitCode {
    ProbeApp::execute()
}
