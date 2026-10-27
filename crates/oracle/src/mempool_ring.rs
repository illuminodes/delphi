use std::collections::HashSet;
use std::io;
use std::net::SocketAddrV4;
use std::time::{Duration, Instant};

use mantle::hash::Hash256;
use mantle::network::{Command, Network};
use mantle_transport::ququmatz::types::{AddressFamily, CqeFlags, SockAddrIn, SocketFlags, SocketType, Timespec};
use mantle_transport::ququmatz::{self, IoUring, ProvidedBufferRing, Socket};
use mantle_transport::{
    CqeData, IoUringExt, Op, Peer, PeerPool, PeerState, SendArena, decode_cqe_buffer_id, make_ring,
};

use crate::mempool_session::{MempoolSession, SessionOutput};
use crate::mempool_watch::SeenTransaction;

type MempoolPeer = Peer<MempoolSession>;
type MempoolPool = PeerPool<MempoolSession, 16>;

/// The ring, its receive buffers and send slots, shared by every peer.
struct RingIo {
    network: Network,
    ring: IoUring,
    buffers: ProvidedBufferRing,
    arena: SendArena,
    version: Vec<u8>,
    connect_timeout: Timespec,
    next_generation: u16,
}

/// Mempool peers on one `io_uring` with multishot receives, deduplicated by
/// txid: each announced transaction is requested from the first peer that
/// offered it. One [`Self::poll`] waits at most the poll timeout for any
/// peer, instead of reading each peer in turn.
pub struct UringMempoolFanout {
    io: RingIo,
    peers: MempoolPool,
    seen: HashSet<Hash256>,
    poll_timeout: Duration,
}

impl RingIo {
    const BUFFER_GROUP: u16 = 1;
    const BUFFER_COUNT: u32 = 256;
    const BUFFER_SIZE: u32 = 64 * 1024;

    fn open(network: Network, connect_timeout: Duration) -> io::Result<Self> {
        let mut ring = make_ring()?;
        let buffers = ring
            .register_provided_buffers(Self::BUFFER_GROUP, Self::BUFFER_COUNT, Self::BUFFER_SIZE)
            .map_err(|error| io::Error::other(format!("register buffers: {error:?}")))?;
        Ok(Self {
            network,
            ring,
            buffers,
            arena: SendArena::new(),
            version: MempoolSession::version_payload()?,
            connect_timeout: Timespec::from_millis(u64::try_from(connect_timeout.as_millis()).unwrap_or(u64::MAX)),
            next_generation: 1,
        })
    }

    fn dial(&mut self, slot: u16, address: SocketAddrV4) -> io::Result<MempoolPeer> {
        let socket = Socket::with_typed_flags(AddressFamily::Inet, SocketType::Stream, 0, SocketFlags::NONBLOCK)
            .map_err(|error| io::Error::other(format!("socket: {error:?}")))?;
        let sockaddr = SockAddrIn {
            sin_family: u16::try_from(AddressFamily::Inet.as_raw()).map_err(io::Error::other)?,
            sin_port: address.port().to_be(),
            sin_addr: u32::from_ne_bytes(address.ip().octets()),
            sin_zero: [0; 8],
        };
        let generation = self.next_generation;
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.ring
            .push_connect(slot, generation, socket.fd(), &sockaddr, &self.connect_timeout)?;
        Ok(Peer {
            addr: address,
            sock: Some(socket),
            sockaddr,
            state: PeerState::Connecting,
            recv_buf: Vec::new(),
            recv_armed: false,
            connected_at: Instant::now(),
            generation,
            pending_send_seqs: Vec::new(),
            app: MempoolSession::new(self.network.magic()),
        })
    }

    /// A signal during the wait makes `io_uring_enter` return `EINTR`; that
    /// only ends the wait early, so the completions are drained as usual.
    fn wait(&mut self) -> io::Result<()> {
        const EINTR: i32 = 4;
        match self.ring.submit_and_wait(1) {
            Ok(_) => Ok(()),
            Err(
                ququmatz::Error::Syscall(errno) | ququmatz::Error::Submit(ququmatz::SubmitError::Syscall(errno)),
            ) if errno.raw() == EINTR => Ok(()),
            Err(error) => Err(io::Error::other(format!("{error:?}"))),
        }
    }

    fn kill(&mut self, peer: &mut MempoolPeer) {
        peer.state = PeerState::Dead;
        peer.close(&mut self.ring, &mut self.arena);
    }

    fn send(&mut self, slot: u16, peer: &mut MempoolPeer, command: Command, payload: &[u8]) -> io::Result<()> {
        let fd = peer.fd().ok_or_else(|| io::Error::other("peer has no socket"))?;
        let sequence = self
            .ring
            .push_send(&mut self.arena, self.network, command, payload, slot, peer.generation, fd)?;
        peer.pending_send_seqs.push(sequence);
        Ok(())
    }

    fn arm(&mut self, slot: u16, peer: &mut MempoolPeer) -> io::Result<()> {
        let fd = peer.fd().ok_or_else(|| io::Error::other("peer has no socket"))?;
        self.ring
            .push_recv_multishot(slot, peer.generation, fd, Self::BUFFER_GROUP)?;
        peer.recv_armed = true;
        Ok(())
    }

    fn connected(&mut self, slot: u16, peer: &mut MempoolPeer, result: i32) -> io::Result<()> {
        if result < 0 {
            self.kill(peer);
            return Ok(());
        }
        self.arm(slot, peer)?;
        let version = std::mem::take(&mut self.version);
        let sent = self.send(slot, peer, Command::Version, &version);
        self.version = version;
        sent?;
        peer.state = PeerState::HandshakeVersionSent;
        Ok(())
    }

    fn received(&mut self, peer: &mut MempoolPeer, result: i32, flags: CqeFlags) -> Option<SessionOutput> {
        peer.ingest_recv_cqe(&mut self.buffers, &mut self.arena, flags, result, &mut self.ring);
        if peer.state == PeerState::Dead || peer.recv_buf.is_empty() {
            return None;
        }
        let bytes = std::mem::take(&mut peer.recv_buf);
        let Ok(output) = peer.app.receive(&bytes) else {
            self.kill(peer);
            return None;
        };
        if peer.app.is_ready() {
            peer.state = PeerState::Ready;
        }
        Some(output)
    }

    fn recycle(&mut self, op: Op, flags: CqeFlags) {
        if op == Op::Recv
            && let Some(buffer) = decode_cqe_buffer_id(flags)
        {
            self.buffers.recycle(buffer);
        }
    }
}

impl UringMempoolFanout {
    pub const CONNECT_ATTEMPTS: usize = 48;
    pub const SEEN_LIMIT: usize = 500_000;
    const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

    /// Dial candidates until `want` peers finish the handshake or `timeout`
    /// passes for the last attempt, trying at most
    /// [`Self::CONNECT_ATTEMPTS`] addresses.
    ///
    /// # Errors
    /// Returns an I/O error if the ring cannot be set up.
    pub fn connect(
        candidates: &[SocketAddrV4],
        network: Network,
        want: usize,
        timeout: Duration,
        poll_timeout: Duration,
    ) -> io::Result<Self> {
        let mut fanout = Self {
            io: RingIo::open(network, timeout)?,
            peers: MempoolPool::new(),
            seen: HashSet::new(),
            poll_timeout,
        };
        let want = want.clamp(1, 16);
        let mut untried = candidates.iter().copied().take(Self::CONNECT_ATTEMPTS);
        let mut deadline = Instant::now() + timeout + Self::HANDSHAKE_TIMEOUT;
        loop {
            while fanout.peers.live_count() < want {
                let (Some(slot), Some(address)) = (fanout.peers.free_slot(), untried.next()) else {
                    break;
                };
                let peer = fanout.io.dial(slot, address)?;
                fanout.peers.insert(slot, peer);
                deadline = Instant::now() + timeout + Self::HANDSHAKE_TIMEOUT;
            }
            if fanout.ready_peers() >= want || fanout.peers.live_count() == 0 || Instant::now() >= deadline {
                break;
            }
            fanout.pump(Duration::from_millis(100), &mut Vec::new())?;
        }
        fanout.drop_unready();
        Ok(fanout)
    }

    /// Wait up to the poll timeout and return the transactions that arrived.
    /// A ring failure drops every peer, so the caller reconnects.
    pub fn poll(&mut self) -> Vec<SeenTransaction> {
        if self.seen.len() > Self::SEEN_LIMIT {
            self.seen.clear();
        }
        let mut delivered = Vec::new();
        if self.pump(self.poll_timeout, &mut delivered).is_err() {
            for peer in self.peers.values_mut() {
                self.io.kill(peer);
            }
            self.remove_dead();
        }
        delivered
    }

    #[must_use]
    pub fn live_peers(&self) -> usize {
        self.peers.live_count()
    }

    #[must_use]
    pub fn peer_count(&self) -> usize {
        self.peers.values().count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    #[must_use]
    pub fn unique_announced(&self) -> usize {
        self.seen.len()
    }

    fn ready_peers(&self) -> usize {
        self.peers
            .values()
            .filter(|peer| peer.state == PeerState::Ready)
            .count()
    }

    fn drop_unready(&mut self) {
        for peer in self.peers.values_mut() {
            if peer.state != PeerState::Ready {
                self.io.kill(peer);
            }
        }
        self.remove_dead();
    }

    fn pump(&mut self, wait: Duration, delivered: &mut Vec<SeenTransaction>) -> io::Result<()> {
        self.io.ring.push_tick(wait)?;
        self.io.wait()?;
        while let Some(cqe) = self.io.ring.complete() {
            let user_data = cqe.user_data();
            self.complete(user_data, cqe.raw_result(), cqe.flags(), delivered)?;
        }
        self.sweep()?;
        self.io.buffers.commit();
        Ok(())
    }

    fn complete(
        &mut self,
        user_data: u64,
        result: i32,
        flags: CqeFlags,
        delivered: &mut Vec<SeenTransaction>,
    ) -> io::Result<()> {
        let data = CqeData::decode(user_data);
        if matches!(data.op, Op::Tick | Op::LinkTimeout | Op::Close) {
            return Ok(());
        }
        if data.op == Op::Send {
            let sequence = CqeData::decode_send_seq(user_data);
            self.io.arena.release(sequence);
            if let Some(peer) = self.peers.get_mut(data.slot) {
                peer.pending_send_seqs.retain(|held| *held != sequence);
            }
        }
        let Some(peer) = self
            .peers
            .get_mut(data.slot)
            .filter(|peer| peer.generation == data.generation)
        else {
            self.io.recycle(data.op, flags);
            return Ok(());
        };
        match data.op {
            Op::Connect => self.io.connected(data.slot, peer, result),
            Op::Send if result < 0 => {
                self.io.kill(peer);
                Ok(())
            }
            Op::Recv => {
                let Some(output) = self.io.received(peer, result, flags) else {
                    return Ok(());
                };
                Self::answer(&mut self.io, &mut self.seen, data.slot, peer, output, delivered)
            }
            _ => Ok(()),
        }
    }

    fn answer(
        io: &mut RingIo,
        seen: &mut HashSet<Hash256>,
        slot: u16,
        peer: &mut MempoolPeer,
        output: SessionOutput,
        delivered: &mut Vec<SeenTransaction>,
    ) -> io::Result<()> {
        for (command, payload) in output.replies {
            if io.send(slot, peer, command, &payload).is_err() {
                io.kill(peer);
                return Ok(());
            }
        }
        let wanted: Vec<Hash256> = output.announced.into_iter().filter(|hash| seen.insert(*hash)).collect();
        for payload in MempoolSession::getdata_payloads(&wanted)? {
            if io.send(slot, peer, Command::GetData, &payload).is_err() {
                io.kill(peer);
                return Ok(());
            }
        }
        let now = Instant::now();
        delivered.extend(
            output
                .transactions
                .into_iter()
                .map(|transaction| SeenTransaction { transaction, at: now }),
        );
        Ok(())
    }

    fn sweep(&mut self) -> io::Result<()> {
        let now = Instant::now();
        for (slot, peer) in self.peers.iter_mut() {
            let stalled = peer.state != PeerState::Ready
                && peer.state != PeerState::Dead
                && now.duration_since(peer.connected_at) > Self::HANDSHAKE_TIMEOUT;
            if stalled {
                self.io.kill(peer);
                continue;
            }
            let unarmed = !peer.recv_armed
                && matches!(
                    peer.state,
                    PeerState::HandshakeVersionSent | PeerState::HandshakeRecving | PeerState::Ready
                );
            if unarmed {
                self.io.arm(slot, peer)?;
            }
        }
        self.remove_dead();
        Ok(())
    }

    fn remove_dead(&mut self) {
        let dead = self.peers.dead_slots().to_vec();
        for slot in dead {
            self.peers.remove(slot);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{SocketAddrV4, TcpListener, TcpStream};
    use std::thread;
    use std::time::Duration;

    use mantle::amount::Satoshi;
    use mantle::codec::Encode;
    use mantle::difficulty::{Locktime, Sequence, TxVersion};
    use mantle::hash::Hash256;
    use mantle::network::{Command, InvEntry, InvMsg, InvType, Network, encode_message};
    use mantle::script::{ScriptPubKey, ScriptSig, Witness};
    use mantle::tx::{OutPoint, Transaction, TxIn, TxOut};

    use super::UringMempoolFanout;
    use crate::frame_buffer::FrameBuffer;
    use crate::mempool_session::MempoolSession;

    /// A local node that completes the handshake, announces one
    /// transaction and serves it when asked.
    struct FakeNode {
        stream: TcpStream,
        frames: FrameBuffer,
    }

    impl FakeNode {
        fn transaction() -> Transaction {
            Transaction {
                version: TxVersion::new(2),
                inputs: vec![TxIn {
                    previous_output: OutPoint {
                        txid: Hash256::from_bytes([3; 32]),
                        vout: 0,
                    },
                    script_sig: ScriptSig::from_slice(&[]),
                    sequence: Sequence::new(u32::MAX),
                    witness: Witness::empty(),
                }],
                outputs: vec![TxOut {
                    value: Satoshi::new(50_000),
                    script_pubkey: ScriptPubKey::from_slice(&[0x51]),
                }],
                locktime: Locktime::new(0),
            }
        }

        fn send(&mut self, command: Command, payload: &[u8]) {
            let mut bytes = Vec::new();
            encode_message(Network::Mainnet.magic(), command, payload, &mut bytes).expect("encode");
            self.stream.write_all(&bytes).expect("write");
        }

        fn wait_for(&mut self, wanted: Command) -> Vec<u8> {
            let mut chunk = [0u8; 4096];
            loop {
                if let Some((command, payload)) = self.frames.next_frame().expect("framing") {
                    if command == wanted {
                        return payload;
                    }
                    continue;
                }
                let read = self.stream.read(&mut chunk).expect("read");
                assert!(read > 0, "client hung up");
                self.frames.extend(&chunk[..read]);
            }
        }

        fn serve(listener: &TcpListener) {
            let (stream, _) = listener.accept().expect("accept");
            let mut node = Self {
                stream,
                frames: FrameBuffer::new(Network::Mainnet.magic(), 4096),
            };
            node.wait_for(Command::Version);
            node.send(Command::Version, &MempoolSession::version_payload().expect("version"));
            node.send(Command::Verack, &[]);
            node.wait_for(Command::Verack);
            let mut inv = Vec::new();
            InvMsg(vec![InvEntry::new(InvType::Tx, Hash256::from_bytes([9; 32]))])
                .encode(&mut inv)
                .expect("inv");
            node.send(Command::Inv, &inv);
            node.wait_for(Command::GetData);
            let mut body = Vec::new();
            Self::transaction().encode(&mut body).expect("tx");
            node.send(Command::Tx, &body);
            let _ = node.stream.read(&mut [0u8; 1]);
        }
    }

    #[test]
    fn a_peer_on_the_ring_handshakes_announces_and_delivers_a_transaction() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let std::net::SocketAddr::V4(address) = listener.local_addr().expect("address") else {
            panic!("ipv4 expected");
        };
        let node = thread::spawn(move || FakeNode::serve(&listener));

        let mut fanout = UringMempoolFanout::connect(
            &[address],
            Network::Mainnet,
            1,
            Duration::from_secs(2),
            Duration::from_millis(50),
        )
        .expect("ring");
        assert_eq!(fanout.live_peers(), 1);
        let mut delivered = Vec::new();
        for _ in 0..100 {
            delivered.extend(fanout.poll());
            if !delivered.is_empty() {
                break;
            }
        }
        drop(fanout);
        node.join().expect("node");

        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].transaction.outputs[0].value, Satoshi::new(50_000));
    }

    #[test]
    fn an_address_nobody_listens_on_leaves_the_fanout_empty() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let std::net::SocketAddr::V4(address) = listener.local_addr().expect("address") else {
            panic!("ipv4 expected");
        };
        drop(listener);
        let closed = SocketAddrV4::new(*address.ip(), address.port());

        let fanout = UringMempoolFanout::connect(
            &[closed],
            Network::Mainnet,
            1,
            Duration::from_millis(500),
            Duration::from_millis(50),
        )
        .expect("ring");

        assert!(fanout.is_empty());
    }
}
