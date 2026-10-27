use crate::{NoteClass, NotePayload};

/// A live payload republished as an addressable note named `latest-<window>`, so relays keep the
/// newest one and a subscriber that arrives between repeats still finds it. The `w` tag still
/// names the live window, so readers treat both copies alike.
pub struct StoredCopy<'a, P: NotePayload> {
    payload: &'a P,
    name: String,
}

impl<'a, P: NotePayload> StoredCopy<'a, P> {
    pub const PREFIX: &'static str = "latest-";

    #[must_use]
    pub fn new(payload: &'a P) -> Self {
        Self {
            name: format!("{}{}", Self::PREFIX, payload.window()),
            payload,
        }
    }
}

impl<P: NotePayload> NotePayload for StoredCopy<'_, P> {
    fn window(&self) -> &str {
        &self.name
    }

    fn label(&self) -> &str {
        self.payload.label()
    }

    fn price(&self) -> f64 {
        self.payload.price()
    }

    fn class(&self) -> NoteClass {
        NoteClass::Addressable
    }

    fn extra_tags(&self) -> Vec<(&'static str, String)> {
        let mut tags = self.payload.extra_tags();
        tags.push(("w", self.payload.window().to_string()));
        tags
    }

    fn is_publishable(&self) -> bool {
        self.payload.is_publishable()
    }

    fn to_json(&self) -> String {
        self.payload.to_json()
    }
}

#[cfg(test)]
mod tests {
    use super::StoredCopy;
    use crate::{NoteClass, PriceSigner, RecentBlocks};

    #[test]
    fn a_stored_copy_is_addressable_and_names_its_live_window() {
        let mut blocks = RecentBlocks::default();
        blocks.record(crate::BlockDigest {
            height: mantle::difficulty::BlockHeight::new(7),
            hash: "h".to_string(),
            previous: String::new(),
            time: 1,
            version: 1,
            difficulty: 1.0,
            transactions: 1,
            outputs: 1,
            size: 100,
            weight: 400,
            output_value: mantle::amount::Satoshi::new(0),
            coinbase_value: mantle::amount::Satoshi::new(0),
            subsidy: mantle::amount::Satoshi::new(0),
            tally: delphi_oracle::TransactionTally::default(),
            admitted: Vec::new(),
            funnel: None,
        });
        let note = PriceSigner::generate().sign(&StoredCopy::new(&blocks)).expect("signs");

        assert_eq!(note.kind, NoteClass::Addressable.kind());
        assert_eq!(note.tags.find_tags_ref("d"), ["latest-blocks"]);
        assert_eq!(note.tags.find_tags_ref("w"), ["blocks"]);
        assert!(note.content.contains("\"blocks\":["));
    }
}
