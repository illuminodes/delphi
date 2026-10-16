use std::collections::HashMap;
use std::fmt;

use nostro2::{NostrNote, NostrNoteBuilder};
use nostro2_signer::NostrKeypair;
use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;
use nostro2_signer::nostro2_traits::NostrSigner as SignerTrait;

use crate::{NoteClass, NotePayload};

#[derive(Debug)]
pub enum PriceSignerError {
    Key(String),
    Unpublishable(String),
    Sign(String),
}

impl fmt::Display for PriceSignerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(message) => write!(formatter, "signing key: {message}"),
            Self::Unpublishable(window) => {
                write!(formatter, "window {window} holds a non-finite or non-positive price")
            }
            Self::Sign(message) => write!(formatter, "sign note: {message}"),
        }
    }
}

impl std::error::Error for PriceSignerError {}

/// Turns payloads into signed notes of their class.
///
/// A relay keeps one addressable event per author, kind and `d` tag, and
/// decides which is newer by `created_at`. Two addressable notes for one
/// window inside the same second would tie, so their timestamps are forced
/// to rise. A payload with its own timestamp keeps it.
pub struct PriceSigner {
    keypair: NostrKeypair,
    last_created: HashMap<String, i64>,
}

impl PriceSigner {
    pub const DELETION_KIND: u32 = 5;

    /// # Errors
    /// Returns [`PriceSignerError::Key`] when `secret` is not an nsec, hex
    /// key or mnemonic.
    pub fn from_secret(secret: &str) -> Result<Self, PriceSignerError> {
        let keypair = NostrKeypair::from_any(secret.trim())
            .map_err(|error| PriceSignerError::Key(error.to_string()))?;
        Ok(Self::new(keypair))
    }

    #[must_use]
    pub fn new(keypair: NostrKeypair) -> Self {
        Self {
            keypair,
            last_created: HashMap::new(),
        }
    }

    #[must_use]
    pub fn public_key(&self) -> String {
        SignerTrait::public_key(&self.keypair)
    }

    #[must_use]
    pub fn generate() -> Self {
        Self::new(<NostrKeypair as KeypairTrait>::generate())
    }

    /// # Errors
    /// Returns [`PriceSignerError::Key`] if the key cannot be encoded.
    pub fn nsec(&self) -> Result<String, PriceSignerError> {
        self.keypair
            .nsec()
            .map_err(|error| PriceSignerError::Key(error.to_string()))
    }

    /// # Errors
    /// Returns [`PriceSignerError::Key`] if the key cannot be encoded.
    pub fn npub(&self) -> Result<String, PriceSignerError> {
        self.keypair
            .npub()
            .map_err(|error| PriceSignerError::Key(error.to_string()))
    }

    /// # Errors
    /// Returns [`PriceSignerError::Unpublishable`] for a non-finite price and
    /// [`PriceSignerError::Sign`] if signing fails.
    pub fn sign(&mut self, payload: &impl NotePayload) -> Result<NostrNote, PriceSignerError> {
        if !payload.is_publishable() {
            return Err(PriceSignerError::Unpublishable(payload.window().to_string()));
        }
        let class = payload.class();
        let mut builder = NostrNoteBuilder::new()
            .kind(class.kind())
            .content(payload.to_json())
            .tag(class.window_tag(), payload.window())
            .tag("t", payload.label());
        for (name, value) in payload.extra_tags() {
            builder = builder.tag(name, &value);
        }
        let mut note = builder.build();
        if let Some(stamp) = payload.created_at() {
            note.created_at = stamp;
        } else if class == NoteClass::Addressable {
            note.created_at = self.next_timestamp(payload.window(), note.created_at);
        }
        note.sign_with(&self.keypair)
            .map_err(|error| PriceSignerError::Sign(error.to_string()))?;
        Ok(note)
    }

    /// A deletion request for this author's addressable notes of `kind`
    /// under each `d` tag in `windows`.
    ///
    /// # Errors
    /// Returns [`PriceSignerError::Sign`] if signing fails.
    pub fn sign_deletion(&self, kind: u32, windows: &[&str]) -> Result<NostrNote, PriceSignerError> {
        let author = self.public_key();
        let mut builder = NostrNoteBuilder::new()
            .kind(Self::DELETION_KIND)
            .content("retired by the oracle")
            .tag("k", &kind.to_string());
        for window in windows {
            builder = builder.tag("a", &format!("{kind}:{author}:{window}"));
        }
        let mut note = builder.build();
        note.sign_with(&self.keypair)
            .map_err(|error| PriceSignerError::Sign(error.to_string()))?;
        Ok(note)
    }

    fn next_timestamp(&mut self, window: &str, now: i64) -> i64 {
        let previous = self.last_created.get(window).copied().unwrap_or(i64::MIN);
        let stamp = now.max(previous.saturating_add(1));
        self.last_created.insert(window.to_string(), stamp);
        stamp
    }
}

#[cfg(test)]
mod tests {
    use nostro2::NostrEvent;
    use nostro2_signer::NostrKeypair;
    use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;

    use mantle::difficulty::{BlockHeight, BlockTime};
    use mantle::hash::Hash256;
    use delphi_oracle::OracleReport;

    use super::{PriceSigner, PriceSignerError};
    use crate::{AuditRecord, ChainWindow, NoteClass, NotePayload, PriceUpdate};

    struct SignerFixture;

    impl SignerFixture {
        fn signer() -> PriceSigner {
            PriceSigner::new(<NostrKeypair as KeypairTrait>::generate())
        }

        fn update(window: &str, price: f64) -> PriceUpdate {
            PriceUpdate::new(window, price, 300, 150, price * 0.99, price * 1.01)
        }

        fn audit() -> AuditRecord {
            AuditRecord::new(
                ChainWindow::Hourly,
                OracleReport {
                    central_price: 80_000.0,
                    deviation: 0.01,
                    sample_count: 4_000,
                    start_height: BlockHeight::new(95),
                    stop_height: BlockHeight::new(100),
                    start_time: BlockTime::new(1_780_000_000),
                    stop_time: BlockTime::new(1_780_003_000),
                },
                Hash256::hash(b"tip"),
            )
        }
    }

    struct Addressable(PriceUpdate);

    impl NotePayload for Addressable {
        fn window(&self) -> &str {
            self.0.window()
        }

        fn label(&self) -> &str {
            self.0.label()
        }

        fn price(&self) -> f64 {
            self.0.price()
        }

        fn is_publishable(&self) -> bool {
            self.0.is_publishable()
        }

        fn to_json(&self) -> String {
            self.0.to_json()
        }
    }

    #[test]
    fn a_live_note_verifies_and_carries_a_w_tag() {
        let mut signer = SignerFixture::signer();
        let note = signer
            .sign(&SignerFixture::update("mempool", 77_000.0))
            .expect("signs");

        assert!(note.verify());
        assert_eq!(note.kind, NoteClass::Live.kind());
        assert_eq!(note.tags.find_tags_ref("w"), vec!["mempool"]);
        assert!(note.tags.find_tags_ref("d").is_empty());
        assert_eq!(note.tags.find_tags_ref("t"), vec!["mempool"]);
        assert!(note.content.contains("\"price\":77000"));
    }

    #[test]
    fn an_audit_note_is_dated_at_its_block_and_signs_the_same_twice() {
        let mut signer = SignerFixture::signer();
        let first = signer.sign(&SignerFixture::audit()).expect("signs");
        let second = signer.sign(&SignerFixture::audit()).expect("signs");

        assert!(first.verify());
        assert_eq!(first.kind, NoteClass::Audit.kind());
        assert_eq!(first.created_at, 1_780_003_000);
        assert_eq!(first.tags.find_tags_ref("w"), vec!["chain-1h"]);
        assert_eq!(first.tags.find_tags_ref("h"), vec!["100"]);
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn repeated_addressable_notes_to_one_window_get_rising_timestamps() {
        let mut signer = SignerFixture::signer();
        let stamps: Vec<i64> = (0..5)
            .map(|step| {
                let note = signer
                    .sign(&Addressable(SignerFixture::update("mempool", 77_000.0 + f64::from(step))))
                    .expect("signs");
                assert_eq!(note.tags.find_tags_ref("d"), vec!["mempool"]);
                note.created_at
            })
            .collect();

        assert!(stamps.windows(2).all(|pair| pair[1] > pair[0]), "{stamps:?}");
    }

    #[test]
    fn a_deletion_names_each_retired_address() {
        let signer = SignerFixture::signer();
        let note = signer
            .sign_deletion(30_078, &["chain-1w", "fit-chain-1h"])
            .expect("signs");
        let author = signer.public_key();

        assert!(note.verify());
        assert_eq!(note.kind, PriceSigner::DELETION_KIND);
        assert_eq!(
            note.tags.find_tags_ref("a"),
            vec![format!("30078:{author}:chain-1w"), format!("30078:{author}:fit-chain-1h")]
        );
        assert_eq!(note.tags.find_tags_ref("k"), vec!["30078"]);
    }

    #[test]
    fn an_unpublishable_price_is_refused() {
        let mut signer = SignerFixture::signer();
        let outcome = signer.sign(&SignerFixture::update("mempool", f64::NAN));

        assert!(matches!(outcome, Err(PriceSignerError::Unpublishable(_))));
    }

    #[test]
    fn a_secret_is_read_from_hex_and_yields_a_stable_identity() {
        let hex = "0000000000000000000000000000000000000000000000000000000000000001";
        let first = PriceSigner::from_secret(hex).expect("hex key");
        let second = PriceSigner::from_secret(&format!("  {hex}\n")).expect("trimmed");

        assert_eq!(first.npub().expect("npub"), second.npub().expect("npub"));
    }

    #[test]
    fn a_bad_secret_is_rejected() {
        assert!(matches!(
            PriceSigner::from_secret("not a key"),
            Err(PriceSignerError::Key(_))
        ));
    }
}
