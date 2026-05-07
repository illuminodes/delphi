/// All consensus validation failure modes.
///
/// Variants are structured by validation stage so callers can distinguish
/// stateless failures (Stage 1) from contextual ones (Stage 2) without
/// inspecting error strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    // ── Stage 1: structural ───────────────────────────────────────────────────
    /// Block contains no transactions.
    EmptyBlock,
    /// First transaction is not a coinbase (no inputs spending the null outpoint).
    FirstTxNotCoinbase,
    /// A non-coinbase transaction has a coinbase-style input.
    UnexpectedCoinbaseInput { tx_index: usize },
    /// A transaction has no inputs.
    EmptyInputs { tx_index: usize },
    /// A transaction has no outputs.
    EmptyOutputs { tx_index: usize },
    /// An output value is out of range (> 21M BTC in satoshis).
    OutputValueOutOfRange { tx_index: usize, output_index: usize },
    /// Sum of output values overflows or exceeds `MAX_MONEY`.
    TotalOutputValueOverflow { tx_index: usize },
    /// A transaction contains duplicate inputs (same outpoint appears twice).
    DuplicateInput { tx_index: usize },
    /// Two or more transactions in the block share the same txid (CVE-2012-2459).
    DuplicateTransaction,
    /// Coinbase scriptSig length is outside [2, 100].
    CoinbaseScriptSigLength { len: usize },
    /// Block weight exceeds the 4,000,000 weight-unit limit (BIP141).
    BlockWeightExceeded { weight: u64 },
    /// Block contains more transactions than the practical consensus limit.
    TooManyTransactions { count: usize },
    /// A transaction's wire encoding is truncated or otherwise malformed.
    MalformedTransaction { tx_index: usize },
    /// Merkle root in the header does not match the computed root of the tx list.
    MerkleRootMismatch,
    /// Header proof-of-work does not satisfy the claimed nBits target.
    InsufficientProofOfWork,
    /// nBits field encodes a target of zero or has invalid format.
    InvalidBits,

    // ── Stage 2: contextual ───────────────────────────────────────────────────
    /// nBits does not match the expected difficulty for this block's height.
    UnexpectedDifficulty,
    /// Block timestamp is not greater than the median of the past 11 blocks (BIP113).
    TimestampTooEarly,
    /// Block timestamp is more than 2 hours in the future.
    TimestampTooFarFuture,
    /// BIP34: coinbase scriptSig does not begin with the encoded block height
    /// (required after BIP34 activation height 227,931).
    Bip34HeightMissing,
    /// Block version is below the minimum required for the current deployment state.
    BlockVersionTooLow { version: i32, required: i32 },
    /// The requested block's parent is not in the chain view.
    UnknownParent,
    /// `ChainView` returned an error.
    ChainView(ChainViewError),
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for ValidationError {}

impl From<ChainViewError> for ValidationError {
    fn from(e: ChainViewError) -> Self {
        Self::ChainView(e)
    }
}

/// Errors returned by a `ChainView` implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainViewError {
    /// The requested item was not found (distinct from a storage failure).
    NotFound,
    /// An underlying storage or I/O failure.
    Storage(String),
}

impl std::fmt::Display for ChainViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Storage(s) => write!(f, "storage error: {s}"),
        }
    }
}

impl std::error::Error for ChainViewError {}
