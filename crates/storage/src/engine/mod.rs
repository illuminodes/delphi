pub mod flat;

use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::{Coin, TipInfo};

use crate::types::{CommitBlockReq, Durability, LookupResult};

/// All failure modes from the storage engine.
#[derive(Debug)]
pub enum EngineError {
    /// An underlying I/O failure.
    Io(std::io::Error),
    /// A script pubkey exceeded the maximum encodable length (`u32::MAX` bytes).
    ScriptPubKeyTooLarge,
    /// A stored script pubkey length field would overflow `usize` on this platform.
    ScriptLenOverflow,
    /// A header gap would overflow `usize` on this platform.
    HeaderGapOverflow,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "engine I/O error: {e}"),
            Self::ScriptPubKeyTooLarge => f.write_str("script_pubkey exceeds 4 GiB"),
            Self::ScriptLenOverflow => f.write_str("script_pubkey length overflows usize"),
            Self::HeaderGapOverflow => f.write_str("header gap overflows usize"),
        }
    }
}

impl std::error::Error for EngineError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

pub trait Engine {
    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn read_coin(&self, op: &OutPoint) -> Result<Option<Coin>, EngineError>;

    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn read_coins(&self, ops: &[OutPoint], out: &mut Vec<LookupResult>) -> Result<(), EngineError>;

    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn write_block(&mut self, b: &CommitBlockReq) -> Result<(), EngineError>;

    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn read_header(&self, hash: &Hash256) -> Result<Option<BlockHeader>, EngineError>;

    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn read_header_at(&self, h: BlockHeight) -> Result<Option<BlockHeader>, EngineError>;

    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn write_header(&mut self, h: BlockHeight, header: &BlockHeader) -> Result<(), EngineError>;

    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn flush(&mut self, d: Durability) -> Result<(), EngineError>;

    /// Rebuild in-memory state from on-disk files. Called once at startup.
    ///
    /// # Errors
    /// Returns `EngineError` on I/O failure.
    fn recover(&mut self) -> Result<TipInfo, EngineError>;

    fn tip(&self) -> TipInfo;
}
