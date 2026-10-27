use std::collections::HashMap;

use nostr_minions::ReadyState;

/// The relay pool's health as one state: a CSS class and a label.
pub struct ConnectionSummary;

impl ConnectionSummary {
    #[must_use]
    pub fn describe(health: &HashMap<String, ReadyState>) -> (&'static str, &'static str) {
        if health.is_empty() {
            return ("connecting", "Connecting");
        }
        if health.values().any(|state| *state == ReadyState::OPEN) {
            return ("open", "Connected");
        }
        if health.values().any(|state| *state == ReadyState::CONNECTING) {
            return ("connecting", "Connecting");
        }
        ("closed", "Disconnected")
    }
}
