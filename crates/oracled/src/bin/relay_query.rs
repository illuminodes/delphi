use std::fmt::Write;
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use delphi_oracled::{KeyStore, RelayConnector};
use nostro2::{NostrRelayEvent, NostrSubscription};
use nostro2_signer::nostro2_traits::bech32::Bech32Crypto;
use nostro2_relay::ReconnectConfig;

struct QueryApp {
    url: String,
    insecure: bool,
    key_file: String,
    author: Option<String>,
    watch: Duration,
    kind: Option<u32>,
    window: Option<String>,
    since_seconds: Option<u64>,
    limit: u32,
    quiet: bool,
    full: bool,
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
            author: value_of("--author"),
            watch: Duration::from_secs(
                value_of("--watch-seconds")
                    .and_then(|seconds| seconds.parse().ok())
                    .unwrap_or(15),
            ),
            kind: value_of("--kind").and_then(|kind| kind.parse().ok()),
            window: value_of("--window"),
            since_seconds: value_of("--since-seconds").and_then(|seconds| seconds.parse().ok()),
            limit: value_of("--limit").and_then(|limit| limit.parse().ok()).unwrap_or(50),
            quiet: arguments.iter().any(|argument| argument == "--quiet"),
            full: arguments.iter().any(|argument| argument == "--full"),
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
        let author = self.author()?;
        eprintln!("relay {} author {author}", self.url);
        let relay = RelayConnector::new(&self.url, self.insecure).connect(ReconnectConfig::default())?;
        let mut filter = NostrSubscription::new().author(&author).limit(self.limit);
        if let Some(kind) = self.kind {
            filter = filter.kind(kind);
        }
        if let Some(window) = &self.window {
            filter.add_tag("w", window);
        }
        if let Some(seconds) = self.since_seconds {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs());
            filter.since = Some(now.saturating_sub(seconds));
        }
        relay.send(filter).map_err(|error| error.to_string())?;
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
                    let head: String = note.content.chars().take(90).collect();
                    let tail: String = note.content.chars().rev().take(110).collect::<Vec<_>>().into_iter().rev().collect();
                    let preview = if note.content.len() > 200 && !self.full {
                        format!("{head} … {tail}")
                    } else {
                        note.content.clone()
                    };
                    if self.quiet {
                        continue;
                    }
                    println!(
                        "{} id {} kind {} created_at {} d={:?} w={:?} bytes={} {}",
                        if ended { "LIVE  " } else { "STORED" },
                        note.id.as_deref().unwrap_or("?"),
                        note.kind,
                        note.created_at,
                        note.tags.find_tags_ref("d"),
                        note.tags.find_tags_ref("w"),
                        note.content.len(),
                        preview
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

impl QueryApp {
    /// The `--author` key (npub or hex), else the key in `--key-file`.
    fn author(&self) -> Result<String, String> {
        let Some(author) = &self.author else {
            let signer = KeyStore::new(&self.key_file)
                .load_or_create()
                .map_err(|error| error.to_string())?;
            return Ok(signer.public_key());
        };
        if !author.starts_with("npub1") {
            return Ok(author.clone());
        }
        let (_, bytes) = Bech32Crypto::decode(author).map_err(|error| format!("bad npub: {error:?}"))?;
        Ok(bytes.iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        }))
    }
}

fn main() -> ExitCode {
    QueryApp::execute()
}
