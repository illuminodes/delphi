use ququmatz::types::RawFd;

use crate::network::MessageHeader;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Error {
    /// `io_uring` setup syscall failed.
    RingCreate(ququmatz::Error),
    /// Non-blocking TCP socket creation failed.
    SocketCreate(ququmatz::Error),
    /// SQE could not be pushed (SQ full = EAGAIN).
    RingPush(ququmatz::Error),
    /// `io_uring_enter` submission failed.
    RingSubmit(ququmatz::Error),
    /// Operation referenced a peer id not tracked by this node.
    PeerNotFound(PeerId),
    /// The kernel returned a negative errno for a CQE.
    Cqe {
        peer: PeerId,
        op: Op,
        errno: ququmatz::Error,
    },
    /// Received message checksum did not match payload.
    ChecksumMismatch { peer: PeerId },
    /// Frame encode / decode failed.
    Codec(crate::codec::Error),
    /// OS thread could not be spawned.
    ThreadSpawn(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RingCreate(e) => write!(f, "io_uring create: {e}"),
            Self::SocketCreate(e) => write!(f, "socket create: {e}"),
            Self::RingPush(e) => write!(f, "ring push: {e}"),
            Self::RingSubmit(e) => write!(f, "ring submit: {e}"),
            Self::PeerNotFound(id) => write!(f, "peer {id:?} not found"),
            Self::Cqe { peer, op, errno } => {
                write!(f, "peer {peer:?} op {op:?} cqe error: {errno}")
            }
            Self::ChecksumMismatch { peer } => write!(f, "checksum mismatch peer {peer:?}"),
            Self::Codec(e) => write!(f, "codec: {e}"),
            Self::ThreadSpawn(e) => write!(f, "thread spawn: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            Self::ThreadSpawn(e) => Some(e),
            _ => None,
        }
    }
}

impl From<crate::codec::Error> for Error {
    fn from(e: crate::codec::Error) -> Self {
        Self::Codec(e)
    }
}

impl From<Error> for std::io::Error {
    fn from(e: Error) -> Self {
        std::io::Error::other(e.to_string())
    }
}

// ── PeerId ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerId(pub u32);

// ── Op / UserDataTag ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Connect,
    Send,
    Recv,
    Unknown(u32),
}

impl Op {
    #[must_use]
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Connect => 0,
            Self::Send => 1,
            Self::Recv => 2,
            Self::Unknown(n) => n,
        }
    }

    #[must_use]
    pub fn from_u32(n: u32) -> Self {
        match n {
            0 => Self::Connect,
            1 => Self::Send,
            2 => Self::Recv,
            n => Self::Unknown(n),
        }
    }
}

/// Packed `io_uring` `user_data`: upper 32 bits = `PeerId`, lower 32 = Op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserDataTag(u64);

impl UserDataTag {
    pub const TIMEOUT: Self = Self(u64::MAX);

    #[inline]
    #[must_use]
    pub fn new(peer: PeerId, op: Op) -> Self {
        Self((u64::from(peer.0) << 32) | u64::from(op.as_u32()))
    }

    #[inline]
    #[must_use]
    pub fn parts(self) -> (PeerId, Op) {
        #[allow(clippy::cast_possible_truncation)]
        (PeerId((self.0 >> 32) as u32), Op::from_u32(self.0 as u32))
    }

    #[inline]
    #[must_use]
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl From<u64> for UserDataTag {
    #[inline]
    fn from(raw: u64) -> Self {
        Self(raw)
    }
}

// ── HandshakeState / PeerState ────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeState {
    Connecting,
    VersionSent,
    VerackSent,
    Ready,
}

pub struct PeerState {
    pub id: PeerId,
    pub fd: RawFd,
    pub handshake: HandshakeState,
    /// Byte reassembly buffer for this peer's inbound stream.
    pub recv_buf: Vec<u8>,
}

impl PeerState {
    pub const RECV_BUF_SIZE: usize = 64 * 1024;
    pub const HEADER_SIZE: usize = MessageHeader::ENCODED_LEN;

    #[must_use]
    pub fn new(id: PeerId, fd: RawFd) -> Self {
        Self {
            id,
            fd,
            handshake: HandshakeState::Connecting,
            recv_buf: Vec::with_capacity(Self::RECV_BUF_SIZE),
        }
    }
}

// ── Channel message types ─────────────────────────────────────────────────────

/// A fully framed message enqueued by a worker for the TX thread to send.
/// Bytes are written directly into the mpsc slot via the zero-copy reserve API;
/// no additional allocation happens on the hot path.
pub struct OutboundMsg {
    pub peer: PeerId,
    /// File descriptor to send on — stored here so the TX thread does not need
    /// to look it up from `PeerState` (which lives on the RX thread).
    pub fd: RawFd,
    /// Raw wire bytes (header + payload), already framed by the caller.
    pub bytes: Box<[u8]>,
}

/// A decoded inbound message delivered from the RX thread to a worker.
pub enum InboundMsg {
    /// A fully decoded P2P message from a peer. Payload bytes are moved out of
    /// the reassembly buffer via `split_off` + `mem::replace`; no copy occurs.
    Network {
        peer: PeerId,
        command: [u8; 12],
        payload: Vec<u8>,
    },
    /// Periodic heartbeat injected by the RX thread on its `io_uring` timeout.
    /// Consumers can use this to run housekeeping (stall checks, deadline
    /// checks, etc.) without a separate timer thread or spin loop.
    Tick,
}

/// Sent to the RX thread to request a new outbound TCP connection.
/// The reply channel is a bounded spsc — exactly one response expected.
/// Reply carries `(PeerId, fd)` so the caller can construct `OutboundMsg`.
pub struct ConnectCmd {
    pub addr: std::net::SocketAddrV4,
    pub reply: quetzalcoatl::spsc::Producer<Result<(PeerId, RawFd), Error>>,
}
