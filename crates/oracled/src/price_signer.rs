use std::collections::HashMap;
use std::fmt;

use nostro2::{NostrNote, NostrNoteBuilder};
use nostro2_signer::NostrKeypair;
use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;
use nostro2_signer::nostro2_traits::NostrSigner as SignerTrait;

use crate::NotePayload;

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

/// Turns price updates into signed addressable notes.
///
/// A relay keeps one addressable event per author, kind and `d` tag, and
/// decides which is newer by `created_at`. Two updates to one window inside
/// the same second would tie, so each window's timestamp is forced to rise.
pub struct PriceSigner {
    keypair: NostrKeypair,
    last_created: HashMap<String, i64>,
}

impl PriceSigner {
    pub const KIND: u32 = 30_078;

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
        let mut note = NostrNoteBuilder::new()
            .kind(Self::KIND)
            .content(payload.to_json())
            .tag("d", payload.window())
            .tag("t", payload.label())
            .build();
        note.created_at = self.next_timestamp(payload.window(), note.created_at);
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

    use super::{PriceSigner, PriceSignerError};
    use crate::{PriceSource, PriceUpdate};

    struct SignerFixture;

    impl SignerFixture {
        fn signer() -> PriceSigner {
            PriceSigner::new(<NostrKeypair as KeypairTrait>::generate())
        }

        fn update(window: &str, price: f64) -> PriceUpdate {
            PriceUpdate::new(
                window,
                price,
                300,
                PriceSource::Mempool {
                    transactions: 150,
                    band_low: price * 0.99,
                    band_high: price * 1.01,
                },
            )
        }
    }

    #[test]
    fn a_signed_note_verifies_and_is_addressable() {
        let mut signer = SignerFixture::signer();
        let note = signer
            .sign(&SignerFixture::update("mempool", 77_000.0))
            .expect("signs");

        assert!(note.verify());
        assert_eq!(note.kind, PriceSigner::KIND);
        assert_eq!(note.tags.find_tags_ref("d"), vec!["mempool"]);
        assert_eq!(note.tags.find_tags_ref("t"), vec!["mempool"]);
        assert!(note.content.contains("\"price\":77000"));
    }

    #[test]
    fn repeated_updates_to_one_window_get_rising_timestamps() {
        let mut signer = SignerFixture::signer();
        let stamps: Vec<i64> = (0..5)
            .map(|step| {
                signer
                    .sign(&SignerFixture::update("mempool", 77_000.0 + f64::from(step)))
                    .expect("signs")
                    .created_at
            })
            .collect();

        assert!(stamps.windows(2).all(|pair| pair[1] > pair[0]), "{stamps:?}");
    }

    #[test]
    fn windows_keep_independent_timestamps() {
        let mut signer = SignerFixture::signer();
        for _ in 0..3 {
            signer
                .sign(&SignerFixture::update("chain-1h", 77_000.0))
                .expect("signs");
        }
        let other = signer
            .sign(&SignerFixture::update("chain-1d", 77_000.0))
            .expect("signs");
        let busy = signer
            .sign(&SignerFixture::update("chain-1h", 77_000.0))
            .expect("signs");

        assert!(busy.created_at > other.created_at);
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
