use std::process::ExitCode;
use std::time::Duration;

use delphi_oracled::RelayConnector;
use nostro2_relay::ReconnectConfig;

struct ProbeApp;

impl ProbeApp {
    fn execute() -> ExitCode {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let insecure = arguments.iter().any(|argument| argument == "--insecure");
        let url = arguments
            .iter()
            .find(|argument| !argument.starts_with("--"))
            .cloned()
            .unwrap_or_else(|| "wss://192.168.1.239:4848".to_string());
        eprintln!("connecting to {url} (certificate checks {})", if insecure { "off" } else { "on" });
        let reconnect = ReconnectConfig {
            max_retries: 1,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_millis(200),
            backoff_multiplier: 1.0,
        };
        match RelayConnector::new(&url, insecure).connect(reconnect) {
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
