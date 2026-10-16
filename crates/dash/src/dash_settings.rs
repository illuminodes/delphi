use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{PublisherKey, PublisherKeyError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsError {
    RelayUrl,
    Publisher(PublisherKeyError),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelayUrl => formatter.write_str("relay address must start with wss:// or ws://"),
            Self::Publisher(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SettingsError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashSettings {
    pub relay: String,
    pub publisher: String,
}

impl Default for DashSettings {
    fn default() -> Self {
        Self {
            relay: Self::DEFAULT_RELAY.to_string(),
            publisher: String::new(),
        }
    }
}

impl DashSettings {
    pub const DEFAULT_RELAY: &'static str = "wss://rof.illuminodes.com";

    pub fn relay_url(&self) -> Result<String, SettingsError> {
        let relay = self.relay.trim();
        relay
            .strip_prefix("wss://")
            .or_else(|| relay.strip_prefix("ws://"))
            .filter(|host| !host.is_empty() && !host.contains(char::is_whitespace))
            .map(|_| relay.to_string())
            .ok_or(SettingsError::RelayUrl)
    }

    pub fn publisher_key(&self) -> Result<PublisherKey, SettingsError> {
        PublisherKey::parse(&self.publisher).map_err(SettingsError::Publisher)
    }

    pub fn validate(&self) -> Result<(String, PublisherKey), SettingsError> {
        Ok((self.relay_url()?, self.publisher_key()?))
    }

    #[must_use]
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_default()
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::{DashSettings, SettingsError};
    use crate::PublisherKeyError;

    struct SettingsFixture;

    impl SettingsFixture {
        fn settings(relay: &str, publisher: &str) -> DashSettings {
            DashSettings {
                relay: relay.to_string(),
                publisher: publisher.to_string(),
            }
        }
    }

    use SettingsFixture as Fixture;

    #[test]
    fn the_default_points_at_the_production_relay_without_a_publisher() {
        let defaults = DashSettings::default();

        assert_eq!(defaults.relay, "wss://rof.illuminodes.com");
        assert_eq!(
            defaults.validate(),
            Err(SettingsError::Publisher(PublisherKeyError::Empty))
        );
    }

    #[test]
    fn secure_and_plain_websocket_relays_are_accepted() {
        assert!(Fixture::settings("wss://192.168.1.239:4848", "").relay_url().is_ok());
        assert!(Fixture::settings(" ws://localhost:7000 ", "").relay_url().is_ok());
    }

    #[test]
    fn other_schemes_and_empty_hosts_are_refused() {
        for relay in ["https://relay.example", "relay.example", "wss://", "wss://a b", ""] {
            assert_eq!(
                Fixture::settings(relay, "").relay_url(),
                Err(SettingsError::RelayUrl),
                "{relay}"
            );
        }
    }

    #[test]
    fn valid_settings_yield_the_relay_and_key() {
        let (relay, key) = Fixture::settings("wss://r.example", &"ab".repeat(32))
            .validate()
            .expect("valid");

        assert_eq!(relay, "wss://r.example");
        assert_eq!(key.hex(), "ab".repeat(32));
    }

    #[test]
    fn settings_survive_a_json_round_trip() {
        let original = Fixture::settings("wss://r.example", "npub1abc");

        assert_eq!(DashSettings::from_json(&original.to_json()), original);
    }

    #[test]
    fn unreadable_stored_settings_fall_back_to_the_defaults() {
        assert_eq!(DashSettings::from_json("{not json"), DashSettings::default());
        assert_eq!(DashSettings::from_json(""), DashSettings::default());
    }
}
