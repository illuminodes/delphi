use std::time::Instant;

use mantle::difficulty::BlockTime;
use mantle::tx::Transaction;

use crate::ChainSnapshot;

/// What the worker threads tell the thread that owns the price hub.
pub enum Event {
    Chain(ChainSnapshot),
    Transaction(Transaction, BlockTime),
    Notice(String),
    /// The relay's `OK` for note `id`.
    Ack { id: String, accepted: bool, message: String },
    /// The ticker's periodic wake-up.
    Tick(Instant),
    /// The deadline passed or a shutdown signal arrived.
    Stop,
}
