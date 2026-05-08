use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::{Coin, NewCoin, TipInfo};

// ── Tokens ────────────────────────────────────────────────────────────────────

/// Opaque request token. Each `StorageHandle` call stamps its request with a
/// monotonically-increasing token and matches the response on the same token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReqToken(pub u64);

// ── Durability ────────────────────────────────────────────────────────────────

/// How durable a committed block must be before the handle call returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Durability {
    /// Write to the engine's in-memory state only. Fast; survives restart only
    /// if the engine itself persists (it does for `FlatEngine`, which flushes
    /// at `Synced` boundaries).
    Memory,
    /// `fsync` to disk before returning. Used every 2016 blocks (difficulty
    /// epoch boundary) by the replay loop.
    Synced,
}

// ── LookupResult ─────────────────────────────────────────────────────────────

/// Result for a single outpoint in a batched UTXO lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LookupResult {
    pub outpoint: OutPoint,
    pub coin: Option<Coin>,
}

// ── Request subtypes ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct LookupCoinsReq {
    pub token: ReqToken,
    pub outpoints: Vec<OutPoint>,
}

#[derive(Debug)]
pub struct CommitBlockReq {
    pub token: ReqToken,
    pub height: BlockHeight,
    pub hash: Hash256,
    pub prev_hash: Hash256,
    pub created: Vec<NewCoin>,
    pub spent: Vec<OutPoint>,
    pub block_bytes: Option<Vec<u8>>,
    pub durability: Durability,
}

#[derive(Debug)]
pub struct ReadHeaderReq {
    pub token: ReqToken,
    pub key: HeaderKey,
}

#[derive(Debug)]
pub enum HeaderKey {
    ByHash(Hash256),
    ByHeight(BlockHeight),
}

// ── StorageReq / StorageResp ──────────────────────────────────────────────────

#[derive(Debug)]
pub enum StorageReq {
    LookupCoins(LookupCoinsReq),
    CommitBlock(CommitBlockReq),
    ReadHeader(ReadHeaderReq),
    Shutdown,
}

// ── Response subtypes ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LookupCoinsResp {
    pub token: ReqToken,
    pub results: Vec<LookupResult>,
}

#[derive(Debug, Clone)]
pub struct CommittedResp {
    pub token: ReqToken,
    pub tip: TipInfo,
}

#[derive(Debug, Clone)]
pub struct ReadHeaderResp {
    pub token: ReqToken,
    pub header: Option<BlockHeader>,
}

#[derive(Debug, Clone)]
pub struct StorageError {
    pub msg: String,
}

impl StorageError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self { msg: msg.into() }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "storage error: {}", self.msg)
    }
}

impl std::error::Error for StorageError {}

#[derive(Debug, Clone)]
pub enum StorageResp {
    LookupCoins(Result<LookupCoinsResp, StorageError>),
    CommitBlock(Result<CommittedResp, StorageError>),
    ReadHeader(Result<ReadHeaderResp, StorageError>),
    Shutdown,
}

impl StorageResp {
    #[must_use] pub fn token(&self) -> Option<ReqToken> {
        match self {
            Self::LookupCoins(Ok(r)) => Some(r.token),
            Self::CommitBlock(Ok(r)) => Some(r.token),
            Self::ReadHeader(Ok(r)) => Some(r.token),
            Self::LookupCoins(Err(_))
            | Self::CommitBlock(Err(_))
            | Self::ReadHeader(Err(_))
            | Self::Shutdown => None,
        }
    }
}

