use std::collections::{BTreeMap, VecDeque};

use nostr_minions::NostrNote;

use crate::{MempoolTransaction, TransactionStage};

/// The per-transaction stream since the dashboard subscribed: how many arrived at each stage and
/// under each verdict, and the newest few in full.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MempoolStream {
    recent: VecDeque<MempoolTransaction>,
    stages: BTreeMap<TransactionStage, usize>,
    verdicts: BTreeMap<String, usize>,
    total: usize,
}

impl MempoolStream {
    pub const RECENT: usize = 600;

    pub fn accept(&mut self, note: &NostrNote) -> bool {
        let Some(transaction) = MempoolTransaction::from_note(note) else {
            return false;
        };
        if self.recent.iter().any(|seen| seen.txid == transaction.txid) {
            return true;
        }
        *self.stages.entry(transaction.stage).or_default() += 1;
        *self.verdicts.entry(transaction.verdict.clone()).or_default() += 1;
        self.total += 1;
        self.recent.push_front(transaction);
        self.recent.truncate(Self::RECENT);
        true
    }

    #[must_use]
    pub const fn total(&self) -> usize {
        self.total
    }

    #[must_use]
    pub fn stage_count(&self, stage: TransactionStage) -> usize {
        self.stages.get(&stage).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn entered_model(&self) -> usize {
        TransactionStage::ALL
            .into_iter()
            .filter(|stage| stage.entered_model())
            .map(|stage| self.stage_count(stage))
            .sum()
    }

    /// Verdicts other than acceptance, largest first.
    #[must_use]
    pub fn rejections(&self) -> Vec<(&str, usize)> {
        let mut rows: Vec<(&str, usize)> = self
            .verdicts
            .iter()
            .filter(|(verdict, _)| verdict.as_str() != crate::BlockFilter::ACCEPTED)
            .map(|(verdict, count)| (verdict.as_str(), *count))
            .collect();
        rows.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
        rows
    }

    /// The newest transactions, newest first, optionally only those at one stage.
    #[must_use]
    pub fn recent(&self, only: Option<TransactionStage>) -> Vec<&MempoolTransaction> {
        self.recent
            .iter()
            .filter(|transaction| only.is_none_or(|stage| transaction.stage == stage))
            .collect()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::MempoolStream;
    use crate::{NoteKind, TransactionStage};

    struct StreamFixture;

    impl StreamFixture {
        fn note(txid: &str, verdict: &str, stage: &str) -> NostrNote {
            NostrNoteBuilder::new()
                .kind(NoteKind::TRANSACTION)
                .content(format!(r#"{{"txid":"{txid}","verdict":"{verdict}","stage":"{stage}","price":null}}"#))
                .tag("w", "tx")
                .build()
        }
    }

    #[test]
    fn transactions_are_counted_by_stage_and_verdict_and_kept_newest_first() {
        let mut stream = MempoolStream::default();
        for (txid, verdict, stage) in [
            ("a", "accepted", "voting"),
            ("b", "not_two_outputs", "rejected"),
            ("c", "not_two_outputs", "rejected"),
            ("d", "op_return", "rejected"),
            ("e", "accepted", "sampled"),
        ] {
            assert!(stream.accept(&StreamFixture::note(txid, verdict, stage)));
        }

        assert_eq!(stream.total(), 5);
        assert_eq!(stream.stage_count(TransactionStage::Rejected), 3);
        assert_eq!(stream.entered_model(), 2);
        assert_eq!(stream.rejections(), vec![("not_two_outputs", 2), ("op_return", 1)]);
        let newest: Vec<&str> = stream.recent(None).iter().map(|tx| tx.txid.as_str()).collect();
        assert_eq!(newest, vec!["e", "d", "c", "b", "a"]);
        assert_eq!(stream.recent(Some(TransactionStage::Voting)).len(), 1);
    }

    #[test]
    fn a_repeated_note_is_counted_once_and_the_list_is_capped() {
        let mut stream = MempoolStream::default();
        stream.accept(&StreamFixture::note("a", "accepted", "voting"));
        stream.accept(&StreamFixture::note("a", "accepted", "voting"));
        for index in 0..MempoolStream::RECENT + 5 {
            stream.accept(&StreamFixture::note(&format!("x{index}"), "coinbase", "rejected"));
        }

        assert_eq!(stream.total(), MempoolStream::RECENT + 6);
        assert_eq!(stream.recent(None).len(), MempoolStream::RECENT);
    }

    #[test]
    fn other_notes_are_refused() {
        let note = NostrNoteBuilder::new().kind(NoteKind::LIVE).content("{}").tag("w", "status").build();

        assert!(!MempoolStream::default().accept(&note));
    }
}
