use std::fmt;

use nostr_minions::nostro2_signer::nostro2_traits::bech32::Bech32Crypto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublisherKeyError {
    Empty,
    NotAKey,
    WrongLength(usize),
}

impl fmt::Display for PublisherKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("enter the publisher key"),
            Self::NotAKey => formatter.write_str("not an npub or 64-digit hex key"),
            Self::WrongLength(bytes) => write!(formatter, "key holds {bytes} bytes, not 32"),
        }
    }
}

impl std::error::Error for PublisherKeyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherKey {
    hex: String,
}

impl PublisherKey {
    const KEY_BYTES: usize = 32;

    pub fn parse(text: &str) -> Result<Self, PublisherKeyError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(PublisherKeyError::Empty);
        }
        if text.starts_with("npub1") {
            return Self::from_npub(text);
        }
        Self::from_hex(text)
    }

    fn from_hex(text: &str) -> Result<Self, PublisherKeyError> {
        if !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PublisherKeyError::NotAKey);
        }
        if text.len() != Self::KEY_BYTES * 2 {
            return Err(PublisherKeyError::WrongLength(text.len() / 2));
        }
        Ok(Self {
            hex: text.to_ascii_lowercase(),
        })
    }

    fn from_npub(text: &str) -> Result<Self, PublisherKeyError> {
        let (prefix, bytes) =
            Bech32Crypto::decode(text).map_err(|_| PublisherKeyError::NotAKey)?;
        if prefix != "npub" {
            return Err(PublisherKeyError::NotAKey);
        }
        if bytes.len() != Self::KEY_BYTES {
            return Err(PublisherKeyError::WrongLength(bytes.len()));
        }
        let hex = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Self { hex })
    }

    #[must_use]
    pub fn hex(&self) -> &str {
        &self.hex
    }

    #[must_use]
    pub fn short(&self) -> String {
        format!("{}…{}", &self.hex[..8], &self.hex[self.hex.len() - 8..])
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;
    use nostr_minions::nostro2_signer::nostro2_traits::NostrSigner;
    use nostr_minions::NostrKeypair;

    use super::{PublisherKey, PublisherKeyError};

    #[test]
    fn a_hex_key_is_kept_in_lower_case() {
        let key = PublisherKey::parse(&"AB".repeat(32)).expect("hex key");

        assert_eq!(key.hex(), "ab".repeat(32));
    }

    #[test]
    fn an_npub_decodes_to_the_same_key_as_its_hex() {
        let keypair = <NostrKeypair as KeypairTrait>::generate();
        let npub = keypair.npub().expect("npub");

        let key = PublisherKey::parse(&npub).expect("npub key");

        assert_eq!(key.hex(), keypair.public_key());
    }

    #[test]
    fn surrounding_whitespace_is_ignored() {
        let key = PublisherKey::parse(&format!("  {}\n", "0f".repeat(32))).expect("key");

        assert_eq!(key.hex(), "0f".repeat(32));
    }

    #[test]
    fn empty_input_is_reported_as_empty() {
        assert_eq!(PublisherKey::parse("   "), Err(PublisherKeyError::Empty));
    }

    #[test]
    fn text_that_is_not_hex_is_refused() {
        assert_eq!(PublisherKey::parse(&"zz".repeat(32)), Err(PublisherKeyError::NotAKey));
    }

    #[test]
    fn a_short_hex_key_reports_its_length() {
        assert_eq!(PublisherKey::parse("abcd"), Err(PublisherKeyError::WrongLength(2)));
    }

    #[test]
    fn a_corrupted_npub_is_refused() {
        let keypair = <NostrKeypair as KeypairTrait>::generate();
        let mut npub = keypair.npub().expect("npub");
        let last = npub.pop().expect("char");
        npub.push(if last == 'q' { 'p' } else { 'q' });

        assert_eq!(PublisherKey::parse(&npub), Err(PublisherKeyError::NotAKey));
    }

    #[test]
    fn a_secret_key_is_not_accepted_as_a_publisher() {
        let keypair = <NostrKeypair as KeypairTrait>::generate();
        let nsec = keypair.nsec().expect("nsec");

        assert_eq!(PublisherKey::parse(&nsec), Err(PublisherKeyError::NotAKey));
    }

    #[test]
    fn the_short_form_keeps_both_ends() {
        let key = PublisherKey::parse(&format!("{}{}", "11111111", "2".repeat(48) + "33333333")).expect("key");

        assert_eq!(key.short(), "11111111…33333333");
    }
}
