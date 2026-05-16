use std::fmt;

use mantle::primitives::{BlockHeight, OutPoint};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UtxoError {
    OutputAlreadyExists(OutPoint),
    OutputNotFound(OutPoint),
    TooManyOutputs { txid_prefix: [u8; 8], count: usize },
    CoinbaseMaturity { outpoint: OutPoint, created: BlockHeight, spend: BlockHeight },
}

impl fmt::Display for UtxoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputAlreadyExists(op) => write!(f, "output already exists: {op:?}"),
            Self::OutputNotFound(op) => write!(f, "output not found: {op:?}"),
            Self::TooManyOutputs { count, .. } => {
                write!(f, "transaction has {count} outputs, max 256")
            }
            Self::CoinbaseMaturity {
                outpoint,
                created,
                spend,
            } => write!(
                f,
                "coinbase output {outpoint:?} created at {created} not mature at {spend}"
            ),
        }
    }
}

impl std::error::Error for UtxoError {}
