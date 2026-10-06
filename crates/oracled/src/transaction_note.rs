use mantle::amount::Satoshi;
use mantle::difficulty::BlockTime;
use mantle::hash::Hash256;
use mantle::tx::{Transaction, TxHash};
use mantle_oracle::TransactionVerdict;

use crate::{JsonObject, NoteClass, NotePayload, OutputReading};

/// One unconfirmed transaction as the oracle judged it, streamed live under the window `tx`.
///
/// The `v` tag carries the filter verdict and the `s` tag the stage it reached, so a relay can
/// filter on either:
/// `rejected` (a filter rule turned it away), `dropped` (accepted, but every output fell outside
/// the amount limits), `sampled` (its outputs entered the sample), `round` (one sits near a round
/// dollar amount) and `voting` (one implies a price inside the central band).
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionNote {
    txid: String,
    seen: BlockTime,
    verdict: TransactionVerdict,
    inputs: usize,
    size: usize,
    outputs: Vec<Satoshi>,
    admitted: Vec<Satoshi>,
    price: Option<f64>,
    readings: Vec<OutputReading>,
}

impl TransactionNote {
    pub const WINDOW: &'static str = "tx";
    pub const VERDICT_TAG: &'static str = "v";
    pub const STAGE_TAG: &'static str = "s";

    #[must_use]
    pub fn new(
        transaction: &Transaction,
        seen: BlockTime,
        verdict: TransactionVerdict,
        admitted: Vec<Satoshi>,
        price: Option<f64>,
    ) -> Self {
        let price = price.filter(|price| price.is_finite() && *price > 0.0);
        let readings = price.map_or_else(Vec::new, |price| {
            admitted.iter().map(|amount| OutputReading::of(*amount, price)).collect()
        });
        Self {
            txid: transaction.txid().map(Hash256::to_display_hex).unwrap_or_default(),
            seen,
            verdict,
            inputs: transaction.inputs.len(),
            size: transaction.encoded_len(),
            outputs: transaction.outputs.iter().map(|output| output.value).collect(),
            admitted,
            price,
            readings,
        }
    }

    #[must_use]
    pub const fn verdict(&self) -> TransactionVerdict {
        self.verdict
    }

    #[must_use]
    pub fn stage(&self) -> &'static str {
        if !self.verdict.is_accepted() {
            "rejected"
        } else if self.admitted.is_empty() {
            "dropped"
        } else if self.readings.iter().any(|reading| reading.inside_band) {
            "voting"
        } else if self.readings.iter().any(OutputReading::near_round_usd) {
            "round"
        } else {
            "sampled"
        }
    }

    fn outputs_json(&self) -> String {
        let mut cursor = 0;
        let rows: Vec<String> = self
            .outputs
            .iter()
            .map(|amount| {
                let mut object = JsonObject::new();
                object.raw("sats", &amount.to_u64().to_string());
                let index = (self.admitted.get(cursor) == Some(amount)).then_some(cursor);
                cursor += usize::from(index.is_some());
                object.raw("admitted", if index.is_some() { "true" } else { "false" });
                if let Some(reading) = index.and_then(|index| self.readings.get(index)) {
                    match reading.usd {
                        Some(usd) => object.count("usd", usd as usize),
                        None => object.raw("usd", "null"),
                    }
                    object.fixed("implied", reading.implied.unwrap_or(f64::NAN), 2);
                    object.raw("inside_band", if reading.inside_band { "true" } else { "false" });
                }
                object.finish()
            })
            .collect();
        format!("[{}]", rows.join(","))
    }
}

impl NotePayload for TransactionNote {
    fn window(&self) -> &str {
        Self::WINDOW
    }

    fn label(&self) -> &str {
        Self::WINDOW
    }

    fn price(&self) -> f64 {
        self.price.unwrap_or(f64::NAN)
    }

    fn class(&self) -> NoteClass {
        NoteClass::Transaction
    }

    fn extra_tags(&self) -> Vec<(&'static str, String)> {
        vec![
            (Self::VERDICT_TAG, self.verdict.key().to_string()),
            (Self::STAGE_TAG, self.stage().to_string()),
        ]
    }

    fn is_publishable(&self) -> bool {
        !self.txid.is_empty()
    }

    fn to_json(&self) -> String {
        let mut object = JsonObject::new();
        object.text("window", Self::WINDOW);
        object.text("txid", &self.txid);
        object.raw("seen", &self.seen.to_u32().to_string());
        object.text("verdict", self.verdict.key());
        object.text("stage", self.stage());
        object.count("inputs", self.inputs);
        object.count("size", self.size);
        object.fixed("price", self.price.unwrap_or(f64::NAN), 2);
        object.raw("outputs", &self.outputs_json());
        object.finish()
    }
}

#[cfg(test)]
mod tests {
    use mantle::amount::Satoshi;
    use mantle::difficulty::{BlockTime, Locktime, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::script::{ScriptPubKey, ScriptSig, Witness};
    use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};
    use mantle_oracle::TransactionVerdict;

    use super::TransactionNote;
    use crate::{NoteClass, NotePayload, PriceSigner};

    struct NoteFixture;

    impl NoteFixture {
        fn transaction(values: &[u64]) -> Transaction {
            Transaction {
                version: TxVersion::new(2),
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([9; 32]),
                        vout: 0,
                    },
                    script_sig: ScriptSig::from_slice(&[]),
                    sequence: Sequence::new(0xffff_ffff),
                    witness: Witness::default(),
                }],
                outputs: values
                    .iter()
                    .map(|value| TxOut {
                        value: Satoshi::new(*value),
                        script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                    })
                    .collect(),
                locktime: Locktime::new(0),
            }
        }

        fn note(values: &[u64], verdict: TransactionVerdict, admitted: &[u64], price: Option<f64>) -> TransactionNote {
            TransactionNote::new(
                &Self::transaction(values),
                BlockTime::new(1_780_000_000),
                verdict,
                admitted.iter().copied().map(Satoshi::new).collect(),
                price,
            )
        }
    }

    #[test]
    fn each_stage_follows_how_far_the_transaction_got() {
        let price = Some(100_000.0);
        let stage = |verdict, admitted: &[u64]| NoteFixture::note(&[100_300, 50_000_000], verdict, admitted, price).stage();

        assert_eq!(stage(TransactionVerdict::OpReturn, &[]), "rejected");
        assert_eq!(stage(TransactionVerdict::Accepted, &[]), "dropped");
        assert_eq!(stage(TransactionVerdict::Accepted, &[50_000_000]), "sampled");
        assert_eq!(stage(TransactionVerdict::Accepted, &[118_000]), "round");
        assert_eq!(stage(TransactionVerdict::Accepted, &[100_300, 50_000_000]), "voting");
    }

    #[test]
    fn without_a_price_an_admitted_transaction_stops_at_sampled() {
        let note = NoteFixture::note(&[100_300, 50_000_000], TransactionVerdict::Accepted, &[100_300], None);

        assert_eq!(note.stage(), "sampled");
    }

    #[test]
    fn the_note_carries_its_verdict_and_stage_as_tags_and_reads_every_output() {
        let note = NoteFixture::note(&[100_300, 500], TransactionVerdict::Accepted, &[100_300], Some(100_000.0));
        let signed = PriceSigner::generate().sign(&note).expect("signs");
        let json: serde_json::Value = serde_json::from_str(&signed.content).expect("json");

        assert_eq!(signed.kind, NoteClass::Transaction.kind());
        assert_eq!(signed.tags.find_tags_ref("w"), vec!["tx"]);
        assert_eq!(signed.tags.find_tags_ref("v"), vec!["accepted"]);
        assert_eq!(signed.tags.find_tags_ref("s"), vec!["voting"]);
        assert_eq!(json["txid"].as_str().map(str::len), Some(64));
        assert_eq!(json["outputs"][0]["usd"], 100);
        assert_eq!(json["outputs"][0]["inside_band"], true);
        assert_eq!(json["outputs"][1]["admitted"], false);
        assert!(json["outputs"][1].get("usd").is_none());
    }

    #[test]
    fn twin_outputs_are_each_matched_once() {
        let note = NoteFixture::note(&[500, 100_300, 100_300], TransactionVerdict::Accepted, &[100_300], Some(100_000.0));
        let json: serde_json::Value = serde_json::from_str(&note.to_json()).expect("json");

        assert_eq!(json["outputs"][0]["admitted"], false);
        assert_eq!(json["outputs"][1]["admitted"], true);
        assert_eq!(json["outputs"][2]["admitted"], false);
    }

    #[test]
    fn a_rejected_note_names_its_rule() {
        let note = NoteFixture::note(&[1, 2, 3], TransactionVerdict::NotTwoOutputs, &[], Some(100_000.0));

        assert_eq!(
            note.extra_tags(),
            vec![("v", "not_two_outputs".to_string()), ("s", "rejected".to_string())]
        );
        assert!(note.is_publishable());
    }
}
