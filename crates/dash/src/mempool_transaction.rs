use nostr_minions::NostrNote;
use serde::Deserialize;

use crate::{BlockFilter, NoteKind, TransactionOutput, TransactionStage};

/// One unconfirmed transaction the daemon saw, with the filter's verdict on it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct MempoolTransaction {
    pub txid: String,
    #[serde(default)]
    pub seen: i64,
    pub verdict: String,
    pub stage: TransactionStage,
    #[serde(default)]
    pub inputs: usize,
    #[serde(default)]
    pub size: usize,
    pub price: Option<f64>,
    #[serde(default)]
    pub outputs: Vec<TransactionOutput>,
    #[serde(skip)]
    pub received_at: i64,
}

impl MempoolTransaction {
    pub const WINDOW: &'static str = "tx";

    #[must_use]
    pub fn from_note(note: &NostrNote) -> Option<Self> {
        if note.kind != NoteKind::TRANSACTION || NoteKind::window(note) != Some(Self::WINDOW) {
            return None;
        }
        let mut transaction: Self = serde_json::from_str(&note.content).ok()?;
        transaction.received_at = note.created_at;
        Some(transaction)
    }

    #[must_use]
    pub fn short_txid(&self) -> String {
        let head: String = self.txid.chars().take(8).collect();
        let tail: String = self.txid.chars().skip(self.txid.len().saturating_sub(4)).collect();
        format!("{head}…{tail}")
    }

    /// Why the filter turned it away, as `(label, explanation)`, or `None` if it was accepted.
    #[must_use]
    pub fn rejection(&self) -> Option<(&'static str, &'static str)> {
        BlockFilter::rejection(&self.verdict)
    }

    /// What all its outputs are worth at the hourly price the oracle used, or `None` without one.
    #[must_use]
    pub fn usd_value(&self) -> Option<f64> {
        let price = self.price.filter(|price| price.is_finite() && *price > 0.0)?;
        let btc: f64 = self.outputs.iter().map(TransactionOutput::btc).sum();
        (btc > 0.0).then_some(btc * price)
    }

    /// The output that carried it furthest: one inside the band, else one near a round amount.
    #[must_use]
    pub fn best_output(&self) -> Option<&TransactionOutput> {
        self.outputs
            .iter()
            .find(|output| output.inside_band)
            .or_else(|| self.outputs.iter().find(|output| output.usd.is_some()))
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::MempoolTransaction;
    use crate::{NoteKind, TransactionStage};

    struct TransactionFixture;

    impl TransactionFixture {
        const VOTING: &'static str = r#"{"window":"tx","txid":"00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff","seen":1780000000,"verdict":"accepted","stage":"voting","inputs":1,"size":222,"price":100000.00,"outputs":[{"sats":100300,"admitted":true,"usd":100,"implied":99700.90,"inside_band":true},{"sats":500,"admitted":false}]}"#;

        fn note(kind: u32, window: &str, content: &str) -> NostrNote {
            let mut note = NostrNoteBuilder::new().kind(kind).content(content).tag("w", window).build();
            note.created_at = 42;
            note
        }
    }

    #[test]
    fn a_voting_transaction_is_read_with_its_best_output() {
        let transaction = MempoolTransaction::from_note(&TransactionFixture::note(
            NoteKind::TRANSACTION,
            "tx",
            TransactionFixture::VOTING,
        ))
        .expect("transaction");

        assert_eq!(transaction.stage, TransactionStage::Voting);
        assert_eq!(transaction.received_at, 42);
        assert_eq!(transaction.short_txid(), "00112233…eeff");
        assert_eq!(transaction.best_output().and_then(|output| output.usd), Some(100));
        assert!(transaction.rejection().is_none());
        assert!((transaction.usd_value().expect("value") - 0.001_008 * 100_000.0).abs() < 1e-6);
    }

    #[test]
    fn a_rejected_transaction_names_its_rule_and_a_null_price_is_read() {
        let transaction = MempoolTransaction::from_note(&TransactionFixture::note(
            NoteKind::TRANSACTION,
            "tx",
            r#"{"txid":"ab","verdict":"op_return","stage":"rejected","price":null,"outputs":[]}"#,
        ))
        .expect("transaction");

        assert_eq!(transaction.rejection().map(|(label, _)| label), Some("OP_RETURN"));
        assert_eq!(transaction.price, None);
        assert!(transaction.best_output().is_none());
        assert_eq!(transaction.usd_value(), None);
    }

    #[test]
    fn notes_of_another_kind_or_window_are_not_transactions() {
        assert!(MempoolTransaction::from_note(&TransactionFixture::note(NoteKind::LIVE, "tx", TransactionFixture::VOTING)).is_none());
        assert!(MempoolTransaction::from_note(&TransactionFixture::note(
            NoteKind::TRANSACTION,
            "status",
            TransactionFixture::VOTING
        ))
        .is_none());
    }
}
