use std::collections::HashMap;

use quetzalcoatl::{mpsc, spmc};
use ququmatz::types::{
    AF_INET, MsgFlags, SOCK_NONBLOCK, SOCK_STREAM, SockAddrIn, TimeoutFlags, Timespec,
};
use ququmatz::{IoUring, Sqe};

use crate::codec::{Decode, Encode};
use crate::hash::checksum;
use crate::network::{Command, Magic, MessageHeader, VersionMsg};

use super::types::{
    ConnectCmd, Error, HandshakeState, InboundMsg, Op, OutboundMsg, PeerId, PeerState, UserDataTag,
};

/// Per-peer kernel recv buffer size. Sized to absorb a full max-weight
/// witness block in one shot, avoiding many round-trips for large blocks.
const RECV_CHUNK: usize = 4_100_000;

/// SQ depth for the RX ring. Covers simultaneous recv SQEs for all peers
/// plus connect SQEs plus one timeout.
const RX_RING_DEPTH: u32 = 256;

/// Drives inbound receives, connection management, and handshaking on a
/// dedicated thread.
///
/// Owns:
/// - its `IoUring` instance (RX ring) — recv + connect SQEs only
/// - all `PeerState` (reassembly buffers, handshake state, fds)
/// - per-peer kernel recv scratch buffers
/// - the spmc `Producer` end of the inbound channel
/// - a clone of the mpsc `Producer` end of the outbound channel (for
///   handshake replies: version, verack, pong)
pub struct RxThread {
    ring: IoUring,
    magic: Magic,
    our_version: VersionMsg,
    peers: HashMap<PeerId, PeerState>,
    next_id: u32,
    recv_chunks: HashMap<PeerId, Box<[u8]>>,
    /// Inbound channel — completed messages go here for workers to consume.
    inbound_tx: spmc::Producer<InboundMsg>,
    /// Outbound channel — handshake replies (version/verack/pong) go here.
    outbound_tx: mpsc::Producer<OutboundMsg>,
    /// Commands from the Node coordinator to connect new peers.
    connect_rx: std::sync::mpsc::Receiver<ConnectCmd>,
}

impl RxThread {
    /// # Errors
    /// Returns [`Error::RingCreate`] if the `io_uring` instance cannot be
    /// created.
    pub fn new(
        magic: Magic,
        our_version: VersionMsg,
        inbound_tx: spmc::Producer<InboundMsg>,
        outbound_tx: mpsc::Producer<OutboundMsg>,
        connect_rx: std::sync::mpsc::Receiver<ConnectCmd>,
    ) -> Result<Self, Error> {
        let ring = IoUring::new(RX_RING_DEPTH).map_err(Error::RingCreate)?;
        Ok(Self {
            ring,
            magic,
            our_version,
            peers: HashMap::new(),
            next_id: 0,
            recv_chunks: HashMap::new(),
            inbound_tx,
            outbound_tx,
            connect_rx,
        })
    }

    /// Run the RX event loop. Blocks until the inbound channel consumer is
    /// dropped (i.e. the node is shut down).
    ///
    /// # Errors
    /// Returns [`Error::RingSubmit`] on fatal ring failures.
    pub fn run(mut self) -> Result<(), Error> {
        loop {
            // --- drain pending connect commands (non-blocking) ---
            while let Ok(cmd) = self.connect_rx.try_recv() {
                self.handle_connect_cmd(&cmd);
            }

            // --- wait for I/O with a short timeout so we re-check connect_rx ---
            let ts = Timespec::from_millis(100);
            let timeout_sqe =
                Sqe::timeout(&ts, 1, TimeoutFlags::default()).user_data(UserDataTag::TIMEOUT.raw());
            if self.ring.push(timeout_sqe).is_err() {
                // SQ full — just submit what we have.
            }
            self.ring.submit_and_wait(1).map_err(Error::RingSubmit)?;

            // --- drain all CQEs ---
            let mut had_io = false;
            loop {
                let Some(cqe) = self.ring.complete() else {
                    break;
                };

                if cqe.user_data == UserDataTag::TIMEOUT.raw() {
                    if let Some(w) = self.inbound_tx.reserve() {
                        w.write(InboundMsg::Tick).commit();
                    }
                    continue;
                }

                had_io = true;
                let (peer_id, op) = UserDataTag::from(cqe.user_data).parts();

                match cqe.into_result() {
                    Err(e) => {
                        eprintln!(
                            "{}",
                            Error::Cqe {
                                peer: peer_id,
                                op,
                                errno: e
                            }
                        );
                        self.peers.remove(&peer_id);
                        self.recv_chunks.remove(&peer_id);
                    }
                    Ok(n) => match op {
                        Op::Connect => self.on_connected(peer_id)?,
                        Op::Recv => self.on_recv(peer_id, n as usize)?,
                        Op::Send | Op::Unknown(_) => {}
                    },
                }
            }

            // Stop when all inbound consumers are gone (no workers left).
            // reserve() returns None once consumer_closed is set.
            if !had_io && self.inbound_tx.reserve().is_none() {
                break;
            }
        }
        Ok(())
    }

    // ── connection ────────────────────────────────────────────────────────────

    fn handle_connect_cmd(&mut self, cmd: &ConnectCmd) {
        let result = self.do_connect(cmd.addr);
        let _ = cmd.reply.push(result);
    }

    fn do_connect(&mut self, addr: std::net::SocketAddrV4) -> Result<(PeerId, i32), Error> {
        let id = PeerId(self.next_id);
        self.next_id += 1;

        let sock = ququmatz::Socket::new(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0)
            .map_err(Error::SocketCreate)?;
        let fd = sock.into_fd();

        let sockaddr = SockAddrIn {
            sin_family: u16::try_from(AF_INET).expect("AF_INET fits u16"),
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
            .push(Sqe::connect(fd, addr_bytes).user_data(UserDataTag::new(id, Op::Connect).raw()))
            .map_err(Error::RingPush)?;
        self.ring.submit().map_err(Error::RingSubmit)?;

        self.recv_chunks
            .insert(id, vec![0u8; RECV_CHUNK].into_boxed_slice());
        self.peers.insert(id, PeerState::new(id, fd));

        Ok((id, fd))
    }

    // ── handshake + receive ───────────────────────────────────────────────────

    fn on_connected(&mut self, id: PeerId) -> Result<(), Error> {
        let fd = match self.peers.get(&id) {
            Some(p) => p.fd,
            None => return Ok(()),
        };

        let mut payload = Vec::new();
        self.our_version
            .encode(&mut payload)
            .map_err(Error::Codec)?;

        self.send_msg(id, fd, Command::Version, &payload)?;
        self.submit_recv(id)?;

        if let Some(p) = self.peers.get_mut(&id) {
            p.handshake = HandshakeState::VersionSent;
        }
        Ok(())
    }

    fn submit_recv(&mut self, id: PeerId) -> Result<(), Error> {
        let Some(peer) = self.peers.get(&id) else {
            return Ok(());
        };
        let fd = peer.fd;
        let chunk = self
            .recv_chunks
            .get_mut(&id)
            .expect("recv_chunks entry always exists alongside PeerState");
        self.ring
            .push(
                Sqe::recv(fd, chunk.as_mut(), MsgFlags::default())
                    .user_data(UserDataTag::new(id, Op::Recv).raw()),
            )
            .map_err(Error::RingPush)?;
        Ok(())
    }

    fn on_recv(&mut self, id: PeerId, n: usize) -> Result<(), Error> {
        if n == 0 {
            self.peers.remove(&id);
            self.recv_chunks.remove(&id);
            return Ok(());
        }

        // Append directly from the kernel scratch buffer — no intermediate copy.
        {
            let chunk = &self.recv_chunks[&id][..n];
            let Some(peer) = self.peers.get_mut(&id) else {
                return Ok(());
            };
            peer.recv_buf.extend_from_slice(chunk);
        }

        self.consume_frames(id)?;
        self.submit_recv(id)?;
        Ok(())
    }

    fn consume_frames(&mut self, id: PeerId) -> Result<(), Error> {
        while let Some(peer) = self.peers.get_mut(&id) {
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

            let payload_slice = &peer.recv_buf[PeerState::HEADER_SIZE..total];
            if checksum(payload_slice) != header.checksum {
                eprintln!("{}", Error::ChecksumMismatch { peer: id });
                self.peers.remove(&id);
                self.recv_chunks.remove(&id);
                break;
            }

            let handshake = peer.handshake;
            let cmd = header.command.as_str();

            match (handshake, cmd) {
                (HandshakeState::VersionSent, "version") => {
                    peer.recv_buf.drain(..total);
                    let fd = peer.fd;
                    self.send_msg(id, fd, Command::Verack, &[])?;
                    if let Some(p) = self.peers.get_mut(&id) {
                        p.handshake = HandshakeState::VerackSent;
                    }
                }
                (HandshakeState::VerackSent | HandshakeState::VersionSent, "verack") => {
                    peer.recv_buf.drain(..total);
                    if let Some(p) = self.peers.get_mut(&id) {
                        p.handshake = HandshakeState::Ready;
                    }
                }
                (HandshakeState::Ready, "ping") => {
                    // Capture pong payload before draining.
                    let pong: Vec<u8> = payload_slice.to_owned();
                    peer.recv_buf.drain(..total);
                    let fd = peer.fd;
                    self.send_msg(id, fd, Command::Pong, &pong)?;
                }
                (HandshakeState::Ready, _) => {
                    // Zero-copy hand-off: split_off + mem::replace moves the
                    // payload bytes out of recv_buf with no allocation.
                    let payload_len = total - PeerState::HEADER_SIZE;
                    peer.recv_buf.drain(..PeerState::HEADER_SIZE);
                    let remainder = peer.recv_buf.split_off(payload_len);
                    let payload = std::mem::replace(&mut peer.recv_buf, remainder);

                    // Write directly into the spmc slot — no clone of payload.
                    if let Some(w) = self.inbound_tx.reserve() {
                        w.write(InboundMsg::Network {
                            peer: id,
                            command: header.command.0,
                            payload,
                        })
                        .commit();
                    }
                    // If the ring is full the message is silently dropped —
                    // same back-pressure behaviour as before.
                }
                _ => {
                    peer.recv_buf.drain(..total);
                }
            }
        }
        Ok(())
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    /// Frame a message and enqueue it on the outbound mpsc channel for the
    /// TX thread to send. Uses the zero-copy reserve API: bytes are written
    /// directly into the mpsc slot.
    fn send_msg(
        &mut self,
        peer: PeerId,
        fd: i32,
        command: Command,
        payload: &[u8],
    ) -> Result<(), Error> {
        let mut framed = Vec::new();
        self.magic
            .frame(command, payload, &mut framed)
            .map_err(Error::Codec)?;

        // reserve_block parks if the outbound ring is full, but in practice
        // the TX thread drains faster than we produce handshake messages.
        if let Some(w) = self.outbound_tx.reserve() {
            w.write(OutboundMsg {
                peer,
                fd,
                bytes: framed.into_boxed_slice(),
            })
            .commit();
        }
        // Dropped if ring full — acceptable for handshake; worker messages
        // use push_block and handle backpressure explicitly.
        Ok(())
    }
}
