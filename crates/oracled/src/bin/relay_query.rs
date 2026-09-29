use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use mantle_oracled::{KeyStore, RelayConnector};
use nostro2::{NostrRelayEvent, NostrSubscription};
use nostro2_relay::ReconnectConfig;

struct QueryApp {
    url: String,
    insecure: bool,
    key_file: String,
    watch: Duration,
}

impl QueryApp {
    fn parse() -> Self {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let value_of = |flag: &str| {
            arguments
                .iter()
                .position(|argument| argument == flag)
                .and_then(|index| arguments.get(index + 1))
                .cloned()
        };
        Self {
            url: arguments
                .iter()
                .find(|argument| argument.starts_with("ws"))
                .cloned()
                .unwrap_or_else(|| "wss://rof.illuminodes.com".to_string()),
            insecure: arguments.iter().any(|argument| argument == "--insecure"),
            key_file: value_of("--key-file").unwrap_or_else(|| "data/oracled/oracled.nsec".to_string()),
            watch: Duration::from_secs(
                value_of("--watch-seconds")
                    .and_then(|seconds| seconds.parse().ok())
                    .unwrap_or(15),
            ),
        }
    }

    fn execute() -> ExitCode {
        match Self::parse().run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("failed: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(&self) -> Result<(), String> {
        let signer = KeyStore::new(&self.key_file)
            .load_or_create()
            .map_err(|error| error.to_string())?;
        let author = signer.public_key();
        eprintln!("relay {} author {author}", self.url);
        let relay = RelayConnector::new(&self.url, self.insecure).connect(ReconnectConfig::default())?;
        relay
            .send(NostrSubscription::new().author(&author).limit(50))
            .map_err(|error| error.to_string())?;
        let mut reader = relay.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            while let Some(event) = reader.recv_blocking() {
                if sender.send(event).is_err() {
                    return;
                }
            }
        });
        let started = Instant::now();
        let (mut stored, mut live, mut ended) = (0_usize, 0_usize, false);
        while started.elapsed() < self.watch {
            let Ok(event) = receiver.recv_timeout(Duration::from_secs(1)) else {
                continue;
            };
            match event {
                NostrRelayEvent::NewNote(_, _, note) => {
                    if ended {
                        live += 1;
                    } else {
                        stored += 1;
                    }
                    println!(
                        "{} kind {} created_at {} d={:?} {}",
                        if ended { "LIVE  " } else { "STORED" },
                        note.kind,
                        note.created_at,
                        note.tags.find_tags_ref("d"),
                        note.content
                    );
                }
                NostrRelayEvent::EndOfSubscription(..) => ended = true,
                NostrRelayEvent::Notice(_, message) => eprintln!("notice: {message}"),
                other => eprintln!("event: {other:?}"),
            }
        }
        eprintln!("stored notes: {stored}, live notes during {}s: {live}", self.watch.as_secs());
        relay.close();
        Ok(())
    }
}

fn main() -> ExitCode {
    QueryApp::execute()
}
