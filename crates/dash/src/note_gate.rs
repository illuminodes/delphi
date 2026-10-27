use nostr_minions::{NostrEvent, NostrNote};

/// Lets through only notes from the configured publisher whose id and
/// signature check out, so a relay cannot put words in the oracle's mouth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteGate {
    publisher: String,
}

impl NoteGate {
    #[must_use]
    pub fn new(publisher_hex: impl Into<String>) -> Self {
        Self {
            publisher: publisher_hex.into(),
        }
    }

    #[must_use]
    pub fn admits(&self, note: &NostrNote) -> bool {
        note.pubkey == self.publisher && note.verify()
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;
    use nostr_minions::nostro2_signer::nostro2_traits::NostrSigner;
    use nostr_minions::{NostrKeypair, NostrNote, NostrNoteBuilder};

    use super::NoteGate;

    struct GateFixture;

    impl GateFixture {
        fn signed(keypair: &NostrKeypair, content: &str) -> NostrNote {
            let mut note = NostrNoteBuilder::new().kind(20_000).content(content).build();
            note.pubkey = keypair.public_key();
            note.sign_with(keypair).expect("sign");
            note
        }
    }

    #[test]
    fn a_note_signed_by_the_publisher_is_admitted() {
        let keypair = <NostrKeypair as KeypairTrait>::generate();
        let gate = NoteGate::new(keypair.public_key());

        assert!(gate.admits(&GateFixture::signed(&keypair, "{}")));
    }

    #[test]
    fn a_note_from_another_key_is_refused() {
        let publisher = <NostrKeypair as KeypairTrait>::generate();
        let stranger = <NostrKeypair as KeypairTrait>::generate();
        let gate = NoteGate::new(publisher.public_key());

        assert!(!gate.admits(&GateFixture::signed(&stranger, "{}")));
    }

    #[test]
    fn edited_content_breaks_the_signature() {
        let keypair = <NostrKeypair as KeypairTrait>::generate();
        let gate = NoteGate::new(keypair.public_key());
        let mut note = GateFixture::signed(&keypair, r#"{"price":100.0}"#);
        note.content = r#"{"price":1.0}"#.to_string();

        assert!(!gate.admits(&note));
    }

    #[test]
    fn a_note_claiming_the_publisher_key_without_its_signature_is_refused() {
        let publisher = <NostrKeypair as KeypairTrait>::generate();
        let stranger = <NostrKeypair as KeypairTrait>::generate();
        let gate = NoteGate::new(publisher.public_key());
        let mut note = GateFixture::signed(&stranger, "{}");
        note.pubkey = publisher.public_key();

        assert!(!gate.admits(&note));
    }
}
