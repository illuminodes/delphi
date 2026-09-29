use mantle::difficulty::BlockTime;
use mantle::tx::Transaction;

use crate::ChainSnapshot;

/// What the worker threads tell the thread that owns the price hub.
pub enum Event {
    Chain(ChainSnapshot),
    Transaction(Transaction, BlockTime),
    Notice(String),
}
