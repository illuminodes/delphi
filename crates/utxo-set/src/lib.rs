#![forbid(unsafe_code)]

mod coin_entry;
mod error;
mod set;
mod tx_entry;

pub use coin_entry::CoinEntry;
pub use error::UtxoError;
pub use set::{ConnectScratch, UtxoSet};
pub use tx_entry::TxEntry;
