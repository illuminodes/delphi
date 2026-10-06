use mantle::amount::Satoshi;
use mantle::difficulty::{BlockHeight, BlockTime, Locktime, Sequence, TxVersion};
use mantle::hash::Hash256;
use mantle::script::{ScriptPubKey, ScriptSig, Witness};
use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};
use mantle_oracle::AdmittedSample;

use crate::{BlockRecord, BootstrapRng};

/// Payment amounts drawn the way people pay: a third at arbitrary USD values
/// and the rest at round ones, converted to sats at a fixed price.
pub struct SyntheticMarket {
    price: f64,
    rng: BootstrapRng,
    transactions: u32,
}

impl SyntheticMarket {
    const ROUND_USD: [f64; 9] = [5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1_000.0];

    pub const fn new(price: f64, seed: u64) -> Self {
        Self {
            price,
            rng: BootstrapRng::seeded(seed),
            transactions: 0,
        }
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn amount(&mut self) -> Satoshi {
        let usd = if self.rng.below(3) == 0 {
            let fraction =
                f64::from(u32::try_from(self.rng.next_u64() % 10_000).unwrap_or(0)) / 10_000.0;
            5.0 * 400.0_f64.powf(fraction)
        } else {
            Self::ROUND_USD[self.rng.below(Self::ROUND_USD.len())]
        };
        Satoshi::new(((usd / self.price * 100_000_000.0).round() as u64).max(2_000))
    }

    pub fn record(&mut self, height: u32, outputs: usize) -> BlockRecord {
        let amounts = (0..outputs).map(|_| self.amount()).collect();
        BlockRecord::new(
            BlockHeight::new(height),
            BlockTime::new(1_700_000_000 + height),
            Hash256::hash(&height.to_le_bytes()),
            amounts,
        )
    }

    pub fn sample(&mut self, outputs: usize) -> AdmittedSample {
        let mut sample = AdmittedSample::new();
        for _ in 0..outputs {
            sample.push(self.amount(), BlockHeight::new(1), BlockTime::new(1));
        }
        sample
    }

    /// A two-output transaction with a fresh input, which the output filter
    /// admits.
    pub fn transaction(&mut self) -> Transaction {
        self.transactions += 1;
        let mut txid = [0_u8; 32];
        txid[..4].copy_from_slice(&self.transactions.to_le_bytes());
        let outputs = (0..2)
            .map(|_| TxOut {
                value: self.amount(),
                script_pubkey: ScriptPubKey::from_slice(&[0x51]),
            })
            .collect();
        Transaction {
            version: TxVersion::new(2),
            inputs: vec![TxIn {
                previous_output: OutPoint {
                    txid: Hash256::from_bytes(txid),
                    vout: 0,
                },
                script_sig: ScriptSig::from_slice(&[]),
                sequence: Sequence::new(u32::MAX),
                witness: Witness::empty(),
            }],
            outputs,
            locktime: Locktime::new(0),
        }
    }
}
