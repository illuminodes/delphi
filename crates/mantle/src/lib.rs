pub mod codec;
pub mod collections;
pub mod hash;
pub mod network;
pub mod peer;
pub mod primitives;

// ── Crate-level Error ─────────────────────────────────────────────────────────

/// The single error type that encompasses every failure mode in `mantle`.
#[derive(Debug)]
pub enum Error {
    /// A wire-format encoding or decoding failure.
    Codec(codec::Error),
    /// A peer I/O or ring operation failure.
    Peer(peer::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(e) => write!(f, "codec: {e}"),
            Self::Peer(e) => write!(f, "peer: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::Peer(e) => Some(e),
        }
    }
}

impl From<codec::Error> for Error {
    fn from(e: codec::Error) -> Self {
        Self::Codec(e)
    }
}

impl From<peer::Error> for Error {
    fn from(e: peer::Error) -> Self {
        Self::Peer(e)
    }
}
