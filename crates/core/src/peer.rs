use std::collections::HashMap;

use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::RingBuffer as MpscRing;
use quetzalcoatl::spmc::RingBuffer as SpmcRing;
use ququmatz::types::{AF_INET, MsgFlags, SOCK_NONBLOCK, SOCK_STREAM, SockAddrIn, TimeoutFlags, Timespec};
use ququmatz::{IoUring, Socket, Sqe};

use crate::codec::{Decode, Encode};
use crate::hash::checksum;
use crate::network::{Command, Magic, MessageHeader, VersionMsg};

// ── Error ─────────────────────────────────────────────────────────────────────

/// All failure modes that can occur in the peer I/O layer.
#[derive(Debug)]
pub enum Error {
    /// The `io_uring` instance could not be created.
    RingCreate(String),
    /// A non-blocking TCP socket could not be created.
    SocketCreate(String),
    /// A submission queue entry could not be pushed to the ring.
    RingPush(String),
    /// The ring's submission queue could not be submitted to the kernel.
    RingSubmit(String),
    /// An operation was requested for a peer that is not tracked by this node.
    PeerNotFound(PeerId),
    /// The kernel returned a negative result code for a completed I/O operation.
    CqeError {
        peer: PeerId,
        op: u32,
        /// Negated `errno` value returned by the kernel.
        code: i32,
    },
    /// The received message's checksum did not match its payload.
    ChecksumMismatch { peer: PeerId },
    /// An encode/decode step inside the peer layer failed.
    Codec(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RingCreate(msg) => write!(f, "io_uring create failed: {msg}"),
            Self::SocketCreate(msg) => write!(f, "socket create failed: {msg}"),
            Self::RingPush(msg) => write!(f, "ring push failed: {msg}"),
            Self::RingSubmit(msg) => write!(f, "ring submit failed: {msg}"),
            Self::PeerNotFound(id) => write!(f, "peer {id:?} not found"),
            Self::CqeError { peer, op, code } => {
                write!(f, "peer {peer:?} op {op} kernel error: {code}")
            }
            Self::ChecksumMismatch { peer } => write!(f, "checksum mismatch from peer {peer:?}"),
            Self::Codec(e) => write!(f, "codec error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Codec(e) => Some(e),
            _ => None,
        }
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

// ── user_data tag encoding ────────────────────────────────────────────────────
//
// Layout: upper 32 bits = peer_id, lower 32 bits = op type.

const OP_CONNECT: u32 = 0;
const OP_SEND: u32 = 1;
const OP_RECV: u32 = 2;

// Sentinel user_data for timeout SQEs — no valid peer tag can equal this.
const TAG_TIMEOUT: u64 = u64::MAX;

#[inline]
fn make_tag(peer: PeerId, op: u32) -> u64 {
    (u64::from(peer.0) << 32) | u64::from(op)
}

#[inline]
fn decode_tag(tag: u64) -> (PeerId, u32) {
    (PeerId((tag >> 32) as u32), (tag & 0xffff_ffff) as u32)
}

// ── Handshake state ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandshakeState {
    /// connect SQE submitted, waiting for CQE.
    Connecting,
    /// Connected; version sent, waiting for peer's version.
    VersionSent,
    /// Received peer's version; verack sent, waiting for peer's verack.
    VerackSent,
    /// Handshake complete.
    Ready,
}

// ── PeerState ─────────────────────────────────────────────────────────────────

pub struct PeerState {
    pub id: PeerId,
    pub fd: i32,
    pub handshake: HandshakeState,
    /// Reassembly buffer for incoming bytes.
    recv_buf: Vec<u8>,
}

impl PeerState {
    const RECV_BUF_SIZE: usize = 1024 * 64;
    const HEADER_SIZE: usize = MessageHeader::ENCODED_LEN;

    fn new(id: PeerId, fd: i32) -> Self {
        Self {
            id,
            fd,
            handshake: HandshakeState::Connecting,
            recv_buf: Vec::with_capacity(Self::RECV_BUF_SIZE),
        }
    }
}

// ── Outbound / inbound message types ─────────────────────────────────────────

/// A fully framed message (header + payload) ready to send.
pub struct OutboundMsg {
    pub peer: PeerId,
    pub bytes: Vec<u8>,
}

/// A decoded inbound message from a peer.
pub struct InboundMsg {
    pub peer: PeerId,
    pub command: [u8; 12],
    pub payload: Vec<u8>,
}

// ── Node ──────────────────────────────────────────────────────────────────────

/// Channel capacity for the outbound and inbound queues.
const CHANNEL_CAP: usize = 256;

/// Receive chunk size per `io_uring` recv SQE.
const RECV_CHUNK: usize = 4096;

pub struct Node {
    magic: Magic,
    ring: IoUring,
    peers: HashMap<PeerId, PeerState>,
    next_id: u32,

    /// App threads push messages here; the I/O thread drains and sends.
    pub outbound_tx: quetzalcoatl::mpsc::Producer<OutboundMsg>,
    outbound_rx: quetzalcoatl::mpsc::Consumer<OutboundMsg>,

    /// I/O thread pushes completed receives here; app threads consume.
    pub inbound_rx: quetzalcoatl::spmc::Consumer<InboundMsg>,
    inbound_tx: quetzalcoatl::spmc::Producer<InboundMsg>,

    /// Per-peer recv scratch buffers (fd → chunk buf).
    recv_chunks: HashMap<PeerId, Box<[u8; RECV_CHUNK]>>,

    our_version: VersionMsg,
}

impl Node {
    /// # Errors
    /// Returns [`Error::RingCreate`] if the `io_uring` instance cannot be created.
    pub fn new(magic: Magic, ring_size: u32, our_version: VersionMsg) -> Result<Self, Error> {
        let ring = IoUring::new(ring_size)
            .map_err(|e| Error::RingCreate(format!("{e:?}")))?;

        let (outbound_tx, outbound_rx) = MpscRing::new(Capacity::exact(CHANNEL_CAP)).split();
        let (inbound_tx, inbound_rx) = SpmcRing::new(Capacity::exact(CHANNEL_CAP)).split();

        Ok(Self {
            magic,
            ring,
            peers: HashMap::new(),
            next_id: 0,
            outbound_tx,
            outbound_rx,
            inbound_tx,
            inbound_rx,
            recv_chunks: HashMap::new(),
            our_version,
        })
    }

    /// Dial a peer. Submits the connect SQE; the handshake completes on the
    /// next call(s) to [`Node::poll`].
    ///
    /// # Errors
    /// Returns [`Error::SocketCreate`], [`Error::RingPush`], or [`Error::RingSubmit`]
    /// if the underlying system calls fail.
    ///
    /// # Panics
    /// Never — `AF_INET` is the POSIX constant `2` and always fits in `u16`.
    pub fn connect(&mut self, addr: std::net::SocketAddrV4) -> Result<PeerId, Error> {
        let id = PeerId(self.next_id);
        self.next_id += 1;

        let sock = Socket::new(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0)
            .map_err(|e| Error::SocketCreate(format!("{e:?}")))?;
        let fd = sock.into_fd();

        let sockaddr = SockAddrIn {
            // AF_INET is defined as 2 by POSIX and always fits in u16.
            sin_family: u16::try_from(AF_INET).expect("AF_INET fits in u16"),
            sin_port: addr.port().to_be(),
            sin_addr: u32::from_ne_bytes(addr.ip().octets()),
            sin_zero: [0; 8],
        };
        let addr_bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(
                (&raw const sockaddr).cast(),
                std::mem::size_of::<SockAddrIn>(),
            )
        };

        self.ring
            .push(Sqe::connect(fd, addr_bytes).user_data(make_tag(id, OP_CONNECT)))
            .map_err(|e| Error::RingPush(format!("{e:?}")))?;

        self.ring
            .submit()
            .map_err(|e| Error::RingSubmit(format!("{e:?}")))?;

        self.recv_chunks.insert(id, Box::new([0u8; RECV_CHUNK]));
        self.peers.insert(id, PeerState::new(id, fd));

        Ok(id)
    }

    /// Returns the handshake state for a peer, or `None` if it has disconnected.
    pub fn peer_handshake(&self, id: PeerId) -> Option<HandshakeState> {
        self.peers.get(&id).map(|p| p.handshake)
    }

    /// Send a framed message directly via the ring, bypassing the outbound
    /// channel. Use when ordering relative to the current poll tick matters.
    ///
    /// # Errors
    /// Returns [`Error::PeerNotFound`], [`Error::Codec`], [`Error::RingPush`],
    /// or [`Error::RingSubmit`] on failure.
    pub fn send_direct(
        &mut self,
        peer: PeerId,
        command: Command,
        payload: &[u8],
    ) -> Result<(), Error> {
        let fd = self
            .peers
            .get(&peer)
            .map(|p| p.fd)
            .ok_or(Error::PeerNotFound(peer))?;

        let mut framed = Vec::new();
        crate::network::encode_message(self.magic, command, payload, &mut framed)
            .map_err(Error::Codec)?;

        self.ring
            .push(Sqe::send(fd, framed.leak(), MsgFlags::default()).user_data(make_tag(peer, OP_SEND)))
            .map_err(|e| Error::RingPush(format!("{e:?}")))?;
        self.ring
            .submit()
            .map_err(|e| Error::RingSubmit(format!("{e:?}")))?;
        Ok(())
    }

    /// Drive the event loop once — submit pending sends, drain CQEs, advance
    /// handshakes, and dispatch inbound messages. Call in a loop from the I/O thread.
    ///
    /// # Errors
    /// Returns [`Error::RingSubmit`] if the submission queue cannot be flushed,
    /// or propagates errors from [`on_connected`](Self::on_connected) /
    /// [`on_recv`](Self::on_recv).
    pub fn poll(&mut self) -> Result<(), Error> {
        self.flush_outbound();
        self.ring
            .submit()
            .map_err(|e| Error::RingSubmit(format!("{e:?}")))?;

        while let Some(cqe) = self.ring.complete() {
            self.dispatch_cqe(cqe)?;
        }

        Ok(())
    }

    /// Block until at least one I/O event arrives or `timeout_ms` elapses,
    /// then drain all pending CQEs.
    ///
    /// Uses a `Sqe::timeout(count=1)` submitted alongside any pending SQEs so
    /// the kernel wakes us on whichever comes first — no `thread::sleep`, no
    /// `Instant::now` poll loop.
    ///
    /// Returns `true` if at least one real I/O CQE was processed, `false` if
    /// the call returned solely due to the timeout expiring.
    ///
    /// # Errors
    /// Returns [`Error::RingPush`] or [`Error::RingSubmit`] on ring failures,
    /// or propagates errors from CQE dispatch.
    pub fn poll_or_timeout(&mut self, timeout_ms: u64) -> Result<bool, Error> {
        self.flush_outbound();

        let ts = Timespec::from_millis(timeout_ms);
        self.ring
            .push(Sqe::timeout(&ts, 1, TimeoutFlags::default()).user_data(TAG_TIMEOUT))
            .map_err(|e| Error::RingPush(format!("{e:?}")))?;

        // Block until ≥1 CQE (the timeout itself counts as one).
        self.ring
            .submit_and_wait(1)
            .map_err(|e| Error::RingSubmit(format!("{e:?}")))?;

        let mut io_work = false;
        while let Some(cqe) = self.ring.complete() {
            if cqe.user_data == TAG_TIMEOUT {
                // -ETIME on normal expiry, 0 if count was satisfied first.
                // Either way just skip — not an I/O error.
                continue;
            }
            io_work = true;
            self.dispatch_cqe(cqe)?;
        }

        Ok(io_work)
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn dispatch_cqe(&mut self, cqe: ququmatz::Completion) -> Result<(), Error> {
        let (peer_id, op) = decode_tag(cqe.user_data);
        if cqe.result < 0 {
            eprintln!("{}", Error::CqeError { peer: peer_id, op, code: cqe.result });
            self.peers.remove(&peer_id);
            return Ok(());
        }
        match op {
            OP_CONNECT => self.on_connected(peer_id)?,
            OP_RECV => {
                #[allow(clippy::cast_sign_loss)]
                self.on_recv(peer_id, cqe.result as usize)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn flush_outbound(&mut self) {
        while let Some(msg) = self.outbound_rx.pop() {
            if let Some(peer) = self.peers.get(&msg.peer) {
                let fd = peer.fd;
                // Safety: bytes live in the OutboundMsg heap allocation; we
                // keep msg alive until the send completes by leaking it here.
                // A production implementation would track in-flight sends.
                let bytes = msg.bytes.leak();
                let sqe = Sqe::send(fd, bytes, MsgFlags::default())
                    .user_data(make_tag(msg.peer, OP_SEND));
                let _ = self.ring.push(sqe);
            }
        }
    }

    fn on_connected(&mut self, id: PeerId) -> Result<(), Error> {
        let fd = match self.peers.get(&id) {
            Some(p) => p.fd,
            None => return Ok(()),
        };

        // Send our version message — borrow of peers ends here.
        let mut payload = Vec::new();
        self.our_version.encode(&mut payload).map_err(Error::Codec)?;
        let mut framed = Vec::new();
        crate::network::encode_message(self.magic, Command::Version, &payload, &mut framed)
            .map_err(Error::Codec)?;

        let _ = self.ring.push(
            Sqe::send(fd, framed.leak(), MsgFlags::default()).user_data(make_tag(id, OP_SEND)),
        );

        // submit_recv takes &mut self — safe now, peers borrow is released.
        self.submit_recv(id)?;

        if let Some(peer) = self.peers.get_mut(&id) {
            peer.handshake = HandshakeState::VersionSent;
        }
        Ok(())
    }

    fn submit_recv(&mut self, id: PeerId) -> Result<(), Error> {
        let Some(peer) = self.peers.get(&id) else {
            return Ok(());
        };
        let fd = peer.fd;
        let chunk = self.recv_chunks.get_mut(&id).unwrap();
        let sqe = Sqe::recv(fd, chunk.as_mut_slice(), MsgFlags::default())
            .user_data(make_tag(id, OP_RECV));
        self.ring
            .push(sqe)
            .map_err(|e| Error::RingPush(format!("{e:?}")))?;
        Ok(())
    }

    fn on_recv(&mut self, id: PeerId, n: usize) -> Result<(), Error> {
        if n == 0 {
            // Peer closed the connection.
            self.peers.remove(&id);
            return Ok(());
        }

        // Append received bytes to the reassembly buffer directly from the
        // kernel chunk — no intermediate Vec.
        let Some(peer) = self.peers.get_mut(&id) else {
            return Ok(());
        };
        let chunk_bytes = &self.recv_chunks[&id][..n];
        peer.recv_buf.extend_from_slice(chunk_bytes);

        // Consume as many complete messages as possible.
        loop {
            if peer.recv_buf.len() < PeerState::HEADER_SIZE {
                break;
            }
            let Ok(header) = MessageHeader::decode(&mut std::io::Cursor::new(
                &peer.recv_buf[..PeerState::HEADER_SIZE],
            )) else {
                break;
            };
            let total = PeerState::HEADER_SIZE + header.length as usize;
            if peer.recv_buf.len() < total {
                break;
            }

            // Validate checksum directly against recv_buf — no copy needed.
            let payload_slice = &peer.recv_buf[PeerState::HEADER_SIZE..total];
            if checksum(payload_slice) != header.checksum {
                eprintln!("{}", Error::ChecksumMismatch { peer: id });
                self.peers.remove(&id);
                return Ok(());
            }

            // Advance handshake or dispatch to app.
            let handshake = peer.handshake;
            let cmd = header.command.as_str();
            match (handshake, cmd) {
                (HandshakeState::VersionSent, "version") => {
                    peer.recv_buf.drain(..total);
                    // Send verack.
                    let mut framed = Vec::new();
                    crate::network::encode_message(self.magic, Command::Verack, &[], &mut framed)
                        .map_err(Error::Codec)?;
                    let _ = self.ring.push(
                        Sqe::send(peer.fd, framed.leak(), MsgFlags::default())
                            .user_data(make_tag(id, OP_SEND)),
                    );
                    peer.handshake = HandshakeState::VerackSent;
                }
                (HandshakeState::VerackSent | HandshakeState::VersionSent, "verack") => {
                    peer.recv_buf.drain(..total);
                    peer.handshake = HandshakeState::Ready;
                }
                (HandshakeState::Ready, "ping") => {
                    // Reflect the nonce back as a pong — borrow payload_slice
                    // before draining so we don't need to copy it.
                    let mut framed = Vec::new();
                    crate::network::encode_message(self.magic, Command::Pong, payload_slice, &mut framed)
                        .map_err(Error::Codec)?;
                    peer.recv_buf.drain(..total);
                    let _ = self.ring.push(
                        Sqe::send(peer.fd, framed.leak(), MsgFlags::default())
                            .user_data(make_tag(id, OP_SEND)),
                    );
                }
                (HandshakeState::Ready, _) => {
                    // Hand the payload bytes to InboundMsg without copying:
                    // 1. drop the 24-byte framing header in-place
                    // 2. split_off at payload_len — recv_buf becomes the payload,
                    //    remainder owns everything after it
                    // 3. replace recv_buf with remainder so it stays ready for
                    //    the next message
                    // Net result: zero copies, one pointer-swap.
                    let payload_len = total - PeerState::HEADER_SIZE;
                    peer.recv_buf.drain(..PeerState::HEADER_SIZE);
                    let remainder = peer.recv_buf.split_off(payload_len);
                    let payload = std::mem::replace(&mut peer.recv_buf, remainder);
                    if let Some(slot) = self.inbound_tx.reserve() {
                        slot.write(InboundMsg {
                            peer: id,
                            command: header.command.0,
                            payload,
                        }).commit();
                    }
                    // If the ring is full the payload is dropped here — same
                    // behaviour as before (message silently discarded).
                }
                _ => {
                    peer.recv_buf.drain(..total);
                }
            }
        }

        // Re-arm the recv SQE for this peer.
        self.submit_recv(id)?;
        Ok(())
    }
}
