use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle_oracled::{NoteSink, PriceSigner, PriceSource, PriceUpdate, RelayConnector, SeriesPoint};
use nostro2::{NostrNote, NostrRelayEvent, NostrSubscription};
use nostro2_relay::ReconnectConfig;
use nostro2_signer::NostrKeypair;
use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;

struct PublishProbe {
    url: String,
    insecure: bool,
    kind: Option<u32>,
    age_seconds: i64,
    points: u32,
    keypair: NostrKeypair,
}

impl PublishProbe {
    const WAIT: Duration = Duration::from_secs(10);

    fn parse() -> Self {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        Self {
            insecure: arguments.iter().any(|argument| argument == "--insecure"),
            kind: arguments
                .iter()
                .position(|argument| argument == "--kind")
                .and_then(|index| arguments.get(index + 1))
                .and_then(|value| value.parse().ok()),
            age_seconds: arguments
                .iter()
                .position(|argument| argument == "--age-seconds")
                .and_then(|index| arguments.get(index + 1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            points: arguments
                .iter()
                .position(|argument| argument == "--points")
                .and_then(|index| arguments.get(index + 1))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            url: arguments
                .iter()
                .find(|argument| argument.starts_with("ws"))
                .cloned()
                .unwrap_or_else(|| "wss://192.168.1.239:4848".to_string()),
            keypair: <NostrKeypair as KeypairTrait>::generate(),
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

    fn note(&self) -> Result<NostrNote, String> {
        let mut signer = PriceSigner::new(self.keypair.clone());
        let series = (0..self.points)
            .map(|index| SeriesPoint {
                price: 80_000.0 + f64::from(index) * 0.123_456_789,
                deviation: 0.171_234_567_891_234,
                samples: 100_000 + index as usize,
                stop_height: BlockHeight::new(969_000 + index),
                stop_time: BlockTime::new(1_790_000_000 + index),
            })
            .collect();
        let update = PriceUpdate::new(
            "probe",
            80_000.0,
            1,
            PriceSource::Mempool {
                transactions: 1,
                band_low: 79_000.0,
                band_high: 81_000.0,
            },
        )
        .with_series(series);
        eprintln!("note content is {} bytes", update.to_json().len());
        let mut note = signer.sign(&update).map_err(|error| error.to_string())?;
        if self.kind.is_some() || self.age_seconds != 0 {
            if let Some(kind) = self.kind {
                note.kind = kind;
            }
            note.created_at -= self.age_seconds;
            note.id = None;
            note.sig = None;
            note.sign_with(&self.keypair).map_err(|error| error.to_string())?;
        }
        Ok(note)
    }

    fn run(&self) -> Result<(), String> {
        let relay = RelayConnector::new(&self.url, self.insecure).connect(ReconnectConfig::default())?;
        let note = self.note()?;
        let id = note.id.clone().unwrap_or_default();
        let author = note.pubkey.clone();
        let mut watcher = RelayConnector::new(&self.url, self.insecure).connect(ReconnectConfig::default())?;
        watcher
            .send(NostrSubscription::new().author(&author))
            .map_err(|error| error.to_string())?;
        let (live_sender, live_receiver) = mpsc::channel();
        thread::spawn(move || {
            while let Some(event) = watcher.recv_blocking() {
                if live_sender.send(event).is_err() {
                    return;
                }
            }
        });
        thread::sleep(Duration::from_secs(1));
        eprintln!("sending note {id} kind {}", note.kind);
        relay.deliver(&note)?;

        let mut reader = relay.clone();
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            while let Some(event) = reader.recv_blocking() {
                if sender.send(event).is_err() {
                    return;
                }
            }
        });
        let accepted = Self::await_ok(&receiver, &id)?;
        eprintln!("relay OK for {id}: {accepted}");
        let live = Self::await_live(&live_receiver, &id);
        eprintln!("note delivered live to a subscriber: {live}");

        thread::sleep(Duration::from_secs(1));
        let filters = [
            ("by id", NostrSubscription::new().id(&id)),
            ("by author", NostrSubscription::new().author(&author).limit(5)),
            (
                "by author and kind",
                NostrSubscription::new().kind(note.kind).author(&author).limit(5),
            ),
        ];
        let mut stored = false;
        for (label, filter) in filters {
            relay.send(filter).map_err(|error| error.to_string())?;
            let found = Self::await_note(&receiver, &id);
            eprintln!("query {label}: {found}");
            stored |= found;
        }
        let mut second = RelayConnector::new(&self.url, self.insecure).connect(ReconnectConfig::default())?;
        second
            .send(NostrSubscription::new().id(&id))
            .map_err(|error| error.to_string())?;
        while let Some(event) = second.recv_blocking() {
            eprintln!("  second connection: {event:?}");
            match event {
                NostrRelayEvent::NewNote(_, _, note) if note.id.as_deref() == Some(&id) => {
                    stored = true;
                    break;
                }
                NostrRelayEvent::EndOfSubscription(..) => break,
                _ => {}
            }
        }
        eprintln!("note visible on a second connection: {stored}");
        second.close();
        relay.close();
        if accepted && (stored || (Self::is_ephemeral(note.kind) && live)) {
            Ok(())
        } else {
            Err("relay did not accept and return the note".to_string())
        }
    }

    fn await_ok(receiver: &mpsc::Receiver<NostrRelayEvent>, id: &str) -> Result<bool, String> {
        while let Ok(event) = receiver.recv_timeout(Self::WAIT) {
            match event {
                NostrRelayEvent::SentOk(_, event_id, accepted, message) if event_id == id => {
                    eprintln!("relay OK message: {message:?}");
                    return Ok(accepted);
                }
                NostrRelayEvent::Notice(_, message) => eprintln!("notice: {message}"),
                _ => {}
            }
        }
        Err("no OK from the relay".to_string())
    }

    const fn is_ephemeral(kind: u32) -> bool {
        kind >= 20_000 && kind < 30_000
    }

    fn await_live(receiver: &mpsc::Receiver<NostrRelayEvent>, id: &str) -> bool {
        while let Ok(event) = receiver.recv_timeout(Self::WAIT) {
            if let NostrRelayEvent::NewNote(_, _, note) = event
                && note.id.as_deref() == Some(id)
            {
                return true;
            }
        }
        false
    }

    fn await_note(receiver: &mpsc::Receiver<NostrRelayEvent>, id: &str) -> bool {
        while let Ok(event) = receiver.recv_timeout(Self::WAIT) {
            eprintln!("  event: {event:?}");
            match event {
                NostrRelayEvent::NewNote(_, _, note) if note.id.as_deref() == Some(id) => return true,
                NostrRelayEvent::EndOfSubscription(..) => return false,
                _ => {}
            }
        }
        false
    }
}

fn main() -> ExitCode {
    PublishProbe::execute()
}
