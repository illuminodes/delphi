use std::collections::HashMap;

use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::RingBuffer as MpscRing;
use quetzalcoatl::spmc::RingBuffer as SpmcRing;
use ququmatz::types::{AF_INET, MsgFlags, SockAddrIn, SOCK_NONBLOCK, SOCK_STREAM};
use ququmatz::{IoUring, Socket, Sqe};

use crate::hash::checksum;
use crate::network::{Command, Magic, MessageHeader, VersionMsg};
use crate::codec::{Decode, Encode};

// ── PeerId ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PeerId(pub u32);

// ── user_data tag encoding ────────────────────────────────────────────────────
//
// Layout: upper 32 bits = peer_id, lower 32 bits = op type.

const OP_CONNECT: u32 = 0;
const OP_SEND: u32 = 1;
const OP_RECV: u32 = 2;

#[inline]
fn make_tag(peer: PeerId, op: u32) -> u64 {
    ((peer.0 as u64) << 32) | (op as u64)
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

/// Receive chunk size per io_uring recv SQE.
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
    pub fn new(magic: Magic, ring_size: u32, our_version: VersionMsg) -> std::io::Result<Self> {
        let ring = IoUring::new(ring_size).map_err(|e| {
            std::io::Error::other(format!("io_uring: {e:?}"))
        })?;

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
    pub fn connect(&mut self, addr: std::net::SocketAddrV4) -> std::io::Result<PeerId> {
        let id = PeerId(self.next_id);
        self.next_id += 1;

        let sock = Socket::new(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0).map_err(|e| {
            std::io::Error::other(format!("socket: {e:?}"))
        })?;
        let fd = sock.into_fd();

        let sockaddr = SockAddrIn {
            sin_family: AF_INET as u16,
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
            .map_err(|e| std::io::Error::other(format!("push: {e:?}")))?;

        self.ring.submit().map_err(|e| {
            std::io::Error::other(format!("submit: {e:?}"))
        })?;

        let state = PeerState::new(id, fd);
        self.recv_chunks
            .insert(id, Box::new([0u8; RECV_CHUNK]));
        self.peers.insert(id, state);

        Ok(id)
    }

    /// Drive the event loop once — submit pending sends, drain CQEs, advance
    /// handshakes, dispatch inbound messages. Call in a loop from the I/O
    /// thread.
    /// Returns the handshake state for a peer, or `None` if it has disconnected.
    pub fn peer_handshake(&self, id: PeerId) -> Option<HandshakeState> {
        self.peers.get(&id).map(|p| p.handshake)
    }

    /// Send a framed message directly via the ring, bypassing the outbound
    /// channel. Use when ordering relative to the current poll tick matters.
    pub fn send_direct(&mut self, peer: PeerId, command: Command, payload: &[u8]) -> std::io::Result<()> {
        let fd = self.peers.get(&peer)
            .map(|p| p.fd)
            .ok_or_else(|| std::io::Error::other("peer not found"))?;

        let mut framed = Vec::new();
        crate::network::encode_message(self.magic, command, payload, &mut framed).unwrap();

        let sqe = Sqe::send(fd, framed.leak(), MsgFlags::default())
            .user_data(make_tag(peer, OP_SEND));
        self.ring.push(sqe)
            .map_err(|e| std::io::Error::other(format!("push: {e:?}")))?;
        self.ring.submit()
            .map_err(|e| std::io::Error::other(format!("submit: {e:?}")))?;
        Ok(())
    }

    pub fn poll(&mut self) -> std::io::Result<()> {
        self.flush_outbound()?;
        self.ring.submit().map_err(|e| {
            std::io::Error::other(format!("submit: {e:?}"))
        })?;

        while let Some(cqe) = self.ring.complete() {
            if cqe.result < 0 {
                let (peer_id, op) = decode_tag(cqe.user_data);
                eprintln!("peer {:?} op {op} error: {}", peer_id, cqe.result);
                self.peers.remove(&peer_id);
                continue;
            }

            let (peer_id, op) = decode_tag(cqe.user_data);
            match op {
                OP_CONNECT => self.on_connected(peer_id)?,
                OP_SEND => {} // nothing to do — buffer was borrowed from Vec
                OP_RECV => self.on_recv(peer_id, cqe.result as usize)?,
                _ => {}
            }
        }

        Ok(())
    }

    // ── Internal ──────────────────────────────────────────────────────────────

    fn flush_outbound(&mut self) -> std::io::Result<()> {
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
        Ok(())
    }

    fn on_connected(&mut self, id: PeerId) -> std::io::Result<()> {
        let fd = match self.peers.get(&id) {
            Some(p) => p.fd,
            None => return Ok(()),
        };

        // Send our version message — borrow of peers ends here.
        let mut payload = Vec::new();
        self.our_version.encode(&mut payload).unwrap();
        let mut framed = Vec::new();
        crate::network::encode_message(self.magic, Command::Version, &payload, &mut framed).unwrap();

        let sqe = Sqe::send(fd, framed.leak(), MsgFlags::default())
            .user_data(make_tag(id, OP_SEND));
        let _ = self.ring.push(sqe);

        // submit_recv takes &mut self — safe now, peers borrow is released.
        self.submit_recv(id)?;

        if let Some(peer) = self.peers.get_mut(&id) {
            peer.handshake = HandshakeState::VersionSent;
        }
        Ok(())
    }

    fn submit_recv(&mut self, id: PeerId) -> std::io::Result<()> {
        let peer = match self.peers.get(&id) {
            Some(p) => p,
            None => return Ok(()),
        };
        let fd = peer.fd;
        let chunk = self.recv_chunks.get_mut(&id).unwrap();
        let sqe = Sqe::recv(fd, chunk.as_mut_slice(), MsgFlags::default())
            .user_data(make_tag(id, OP_RECV));
        self.ring
            .push(sqe)
            .map_err(|e| std::io::Error::other(format!("push: {e:?}")))?;
        Ok(())
    }

    fn on_recv(&mut self, id: PeerId, n: usize) -> std::io::Result<()> {
        if n == 0 {
            // Peer closed the connection.
            self.peers.remove(&id);
            return Ok(());
        }

        // Append received bytes to the reassembly buffer.
        let chunk = self.recv_chunks[&id][..n].to_vec();
        let peer = match self.peers.get_mut(&id) {
            Some(p) => p,
            None => return Ok(()),
        };
        peer.recv_buf.extend_from_slice(&chunk);

        // Consume as many complete messages as possible.
        loop {
            if peer.recv_buf.len() < PeerState::HEADER_SIZE {
                break;
            }
            let header = match MessageHeader::decode(
                &mut std::io::Cursor::new(&peer.recv_buf[..PeerState::HEADER_SIZE]),
            ) {
                Ok(h) => h,
                Err(_) => break,
            };
            let total = PeerState::HEADER_SIZE + header.length as usize;
            if peer.recv_buf.len() < total {
                break;
            }

            let payload = peer.recv_buf[PeerState::HEADER_SIZE..total].to_vec();

            // Validate checksum.
            let expected = checksum(&payload);
            if expected != header.checksum {
                eprintln!("peer {:?} checksum mismatch — dropping", id);
                self.peers.remove(&id);
                return Ok(());
            }

            peer.recv_buf.drain(..total);

            // Advance handshake or dispatch to app.
            let cmd = header.command.as_str().to_owned();
            let handshake = peer.handshake;
            match (handshake, cmd.as_str()) {
                (HandshakeState::VersionSent, "version") => {
                    // Send verack.
                    let mut framed = Vec::new();
                    crate::network::encode_message(self.magic, Command::Verack, &[], &mut framed).unwrap();
                    let sqe = Sqe::send(peer.fd, framed.leak(), MsgFlags::default())
                        .user_data(make_tag(id, OP_SEND));
                    let _ = self.ring.push(sqe);
                    peer.handshake = HandshakeState::VerackSent;
                }
                (HandshakeState::VerackSent, "verack")
                | (HandshakeState::VersionSent, "verack") => {
                    peer.handshake = HandshakeState::Ready;
                }
                (HandshakeState::Ready, "ping") => {
                    // Reflect the nonce back as a pong — handled transparently,
                    // never forwarded to the app layer.
                    let mut framed = Vec::new();
                    crate::network::encode_message(self.magic, Command::Pong, &payload, &mut framed)
                        .unwrap();
                    let sqe = Sqe::send(peer.fd, framed.leak(), MsgFlags::default())
                        .user_data(make_tag(id, OP_SEND));
                    let _ = self.ring.push(sqe);
                }
                (HandshakeState::Ready, _) => {
                    let _ = self.inbound_tx.push(InboundMsg {
                        peer: id,
                        command: header.command.0,
                        payload,
                    });
                }
                _ => {}
            }
        }

        // Re-arm the recv SQE for this peer.
        self.submit_recv(id)?;
        Ok(())
    }
}
