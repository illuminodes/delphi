//! Multi-peer P2P block fetcher with on-disk caching.
//!
//! Adapted from `crates/bench/src/fetch_fixtures.rs`. The shape is:
//!
//! 1. Dial seed peers (from `Network::default_seeds()` plus any `--peer`
//!    overrides).
//! 2. Wait for handshakes. Elect one peer to receive `getheaders`.
//! 3. Decode the `headers` response. Persist headers to disk.
//! 4. Fan blocks out across all ready peers via `getdata`, with a per-peer
//!    in-flight window. Reclaim from stalled peers.
//! 5. Write each received block to `<data-dir>/blocks/NNNNNN.bin`.
//!
//! Phase 1 simplification: headers and block downloads are *separate* phases,
//! and each phase reconnects/handshakes peers afresh. A long-running fetcher
//! that keeps peers alive across `getheaders` pages is a Phase 1.5 follow-up;
//! this version is correct, just not maximally efficient over multi-day runs.

use std::collections::HashMap;
use std::io;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mantle::codec::Encode;
use mantle::network::{
    Command, GetHeadersMsg, HeadersMsg, InvEntry, InvMsg, InvType, Network, NetworkAddr,
    VersionMsg, services,
};
use mantle::peer::{InboundMsg, Node, OutboundMsg, PeerId};
use mantle::primitives::{BlockHeader, BlockHeight, Hash256};

/// Maximum blocks in-flight per peer at once. Mirrors the bench fetcher.
const PEER_WINDOW: usize = 8;
/// Seconds without progress before a peer's in-flight blocks are reclaimed.
const STALL_SECS: u64 = 30;
/// Hard ceiling on how long a single *headers* sync phase may run. Block
/// downloads use a per-progress watchdog instead (see
/// `BLOCK_NO_PROGRESS_TIMEOUT_SECS`) since a contiguous chain of 10⁵+
/// blocks legitimately takes much longer than 10 minutes.
const PHASE_DEADLINE_SECS: u64 = 600;
/// Seconds the block-download session may sit with *zero* arrivals across
/// all peers before giving up. Resets on every successful block arrival,
/// so long-running runs are unbounded as long as bytes keep flowing.
const BLOCK_NO_PROGRESS_TIMEOUT_SECS: u64 = 120;
/// Seconds to wait for a peer to send its first non-handshake message before
/// giving up on it. The peer module has no disconnect signal, so we discover
/// dead/mute peers via this Tick-driven timeout.
const HANDSHAKE_TIMEOUT_SECS: u64 = 15;
/// Seconds to wait for the elected peer's `headers` response before
/// re-electing a different peer.
const HEADERS_RESPONSE_TIMEOUT_SECS: u64 = 30;

/// Errors surfaced from fetcher operations. Either the underlying P2P stack
/// returned an error, or local I/O failed, or no peer made progress.
#[derive(Debug)]
pub enum FetchError {
    Peer(mantle::peer::Error),
    Io(io::Error),
    /// Every peer disconnected before the phase completed.
    AllPeersDead,
    /// Phase deadline elapsed without all needed work finishing.
    Timeout,
    /// No peer addresses were given (after seeds + extras).
    NoPeers,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Peer(e) => write!(f, "peer: {e}"),
            Self::Io(e) => write!(f, "io: {e}"),
            Self::AllPeersDead => f.write_str("all peers disconnected before phase completed"),
            Self::Timeout => f.write_str("phase deadline exceeded"),
            Self::NoPeers => f.write_str("no peer addresses configured"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<mantle::peer::Error> for FetchError {
    fn from(e: mantle::peer::Error) -> Self {
        Self::Peer(e)
    }
}

impl From<io::Error> for FetchError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Configuration for a `Fetcher`. Caller assembles peer list from
/// `network.default_seeds()` plus any `--peer` overrides; the fetcher does not
/// know about CLI flags.
pub struct Fetcher {
    network: Network,
    peer_addrs: Vec<SocketAddrV4>,
    data_dir: PathBuf,
}

impl Fetcher {
    /// `peer_addrs` is the assembled list of `host:port` peers to dial. If
    /// empty, every method that needs the network returns
    /// [`FetchError::NoPeers`].
    pub fn new(
        network: Network,
        peer_addrs: Vec<SocketAddrV4>,
        data_dir: PathBuf,
    ) -> io::Result<Self> {
        std::fs::create_dir_all(data_dir.join("blocks"))?;
        Ok(Self {
            network,
            peer_addrs,
            data_dir,
        })
    }

    /// On-disk path for the block at `height` in this fetcher's data dir.
    #[must_use]
    pub fn block_path(&self, height: BlockHeight) -> PathBuf {
        self.data_dir
            .join("blocks")
            .join(format!("{:07}.bin", height.0))
    }

    /// On-disk path for the headers cache. Distinct from the storage
    /// engine's `headers.bin` (which is a 112-byte-per-record file the
    /// engine owns). The fetcher writes raw 80-byte BIP144 headers.
    #[must_use]
    pub fn headers_path(&self) -> PathBuf {
        self.data_dir.join("chain_headers.bin")
    }

    /// Read a block's wire bytes from the on-disk cache. Errors with `NotFound`
    /// if the block has not been fetched.
    pub fn read_block_bytes(&self, height: BlockHeight) -> io::Result<Vec<u8>> {
        std::fs::read(self.block_path(height))
    }

    /// Sync headers covering `[start..=end]` (inclusive) starting from a known
    /// `locator` hash. Loops `getheaders` calls — each response carries up to
    /// 2000 headers — until enough are collected, then writes them to
    /// `<data-dir>/headers.bin` keyed by height.
    ///
    /// Returns the list of `(height, header)` pairs in ascending order.
    pub fn sync_headers(
        &self,
        start: BlockHeight,
        end: BlockHeight,
        locator: Hash256,
    ) -> Result<Vec<(BlockHeight, BlockHeader)>, FetchError> {
        if self.peer_addrs.is_empty() {
            return Err(FetchError::NoPeers);
        }
        let want = u64::from(end.0)
            .saturating_sub(u64::from(start.0))
            .saturating_add(1);
        let want_usize = usize::try_from(want).unwrap_or(usize::MAX);

        let mut collected: Vec<(BlockHeight, BlockHeader)> = Vec::with_capacity(want_usize);
        let mut current_locator = locator;
        let mut next_height = start.0;

        // Page until we have enough or a paged getheaders returns 0 headers
        // (peer signals "no more"). One handshake-and-getheaders session per
        // page; this is wasteful but keeps the state machine straightforward.
        loop {
            let page = run_getheaders_session(self.network, &self.peer_addrs, current_locator)?;
            if page.is_empty() {
                break;
            }
            for hdr in page {
                let h = BlockHeight(next_height);
                if h > end {
                    break;
                }
                collected.push((h, hdr));
                next_height = next_height.wrapping_add(1);
            }
            if collected.last().is_some_and(|(h, _)| *h >= end) {
                break;
            }
            // Continue from the last header we accepted.
            let Some(last) = collected.last() else {
                break;
            };
            current_locator = last.1.hash();
        }

        write_headers_bin(&self.headers_path(), start, &collected)?;
        Ok(collected)
    }

    /// Ensure every block in `headers` is on disk under
    /// `<data-dir>/blocks/NNNNNN.bin`. Already-present blocks are skipped.
    /// Downloads missing ones via a single multi-peer session.
    pub fn fetch_blocks(&self, headers: &[(BlockHeight, BlockHeader)]) -> Result<(), FetchError> {
        if self.peer_addrs.is_empty() {
            return Err(FetchError::NoPeers);
        }
        let needed: Vec<(BlockHeight, Hash256)> = headers
            .iter()
            .filter(|(h, _)| !self.block_path(*h).exists())
            .map(|(h, hdr)| (*h, hdr.hash()))
            .collect();
        if needed.is_empty() {
            return Ok(());
        }
        run_block_download_session(self.network, &self.peer_addrs, &self.data_dir, &needed)
    }
}

// ── headers session ──────────────────────────────────────────────────────────

/// Run one handshake-then-getheaders cycle and return the decoded headers.
///
/// Failover model: only one peer is "elected" to receive `getheaders` at a
/// time (the protocol returns headers serially from a locator, so racing
/// duplicate requests at multiple peers is wasted bandwidth). But if the
/// elected peer goes silent — never speaks the protocol after TCP connect,
/// or accepts `getheaders` and never replies — we time it out via Tick and
/// fall over to another `Ready` (handshaked) peer. If we run out of peers,
/// surface `AllPeersDead` instead of waiting for the global phase deadline.
fn run_getheaders_session(
    network: Network,
    peer_addrs: &[SocketAddrV4],
    locator: Hash256,
) -> Result<Vec<BlockHeader>, FetchError> {
    let node = Node::new(network.magic(), our_version())?;
    let inbound = node.inbound_rx.clone();
    let mut peers: Vec<PeerSlot> = Vec::new();
    let connect_t0 = Instant::now();
    for addr in peer_addrs {
        match node.connect(*addr) {
            Ok((id, fd)) => peers.push(PeerSlot {
                id,
                fd,
                phase: PeerPhase::WaitingHandshake,
                connected_at: connect_t0,
                in_flight: Vec::new(),
            }),
            Err(e) => eprintln!("  connect {addr}: {e}"),
        }
    }
    if peers.is_empty() {
        return Err(FetchError::AllPeersDead);
    }

    let payload = GetHeadersMsg::single_locator(locator).encode_to_vec();
    let phase_deadline = Instant::now() + Duration::from_secs(PHASE_DEADLINE_SECS);
    let handshake_timeout = Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);
    let response_timeout = Duration::from_secs(HEADERS_RESPONSE_TIMEOUT_SECS);

    // Index of the peer currently expected to deliver `headers`, plus the
    // instant at which we sent it `getheaders` (so we can time the response).
    let mut elected: Option<(usize, Instant)> = None;

    loop {
        let Some(msg) = inbound.pop_block() else {
            return Err(FetchError::AllPeersDead);
        };

        match msg {
            InboundMsg::Tick => {
                if Instant::now() > phase_deadline {
                    return Err(FetchError::Timeout);
                }
                let now = Instant::now();

                // Mute peers in WaitingHandshake → mark Dead.
                for peer in &mut peers {
                    if peer.phase == PeerPhase::WaitingHandshake
                        && now.duration_since(peer.connected_at) > handshake_timeout
                    {
                        eprintln!("  peer {:?} handshake timeout — marking dead", peer.id);
                        peer.phase = PeerPhase::Dead;
                    }
                }

                // Elected peer that hasn't replied → demote and re-elect.
                if let Some((idx, sent_at)) = elected
                    && now.duration_since(sent_at) > response_timeout
                {
                    eprintln!(
                        "  peer {:?} headers-response timeout — failing over",
                        peers[idx].id
                    );
                    peers[idx].phase = PeerPhase::Dead;
                    elected = None;
                }

                // No election → try to elect from a Ready peer.
                if elected.is_none()
                    && let Some(idx) = peers.iter().position(|p| p.phase == PeerPhase::Ready) {
                        let (pid, fd) = (peers[idx].id, peers[idx].fd);
                        peers[idx].phase = PeerPhase::WaitingHeaders;
                        eprintln!("  peer {pid:?} elected → getheaders");
                        frame_and_send(&node, network, pid, fd, Command::GetHeaders, &payload);
                        elected = Some((idx, now));
                    }

                if peers.iter().all(|p| matches!(p.phase, PeerPhase::Dead)) {
                    return Err(FetchError::AllPeersDead);
                }
            }
            InboundMsg::Network {
                peer: peer_id,
                command,
                payload: pl,
            } => {
                let Some(idx) = peers.iter().position(|p| p.id == peer_id) else {
                    continue;
                };
                let cmd = Command::from_command_name(mantle::network::CommandName(command));

                // First non-handshake message: handshake done. Either this is
                // our first such peer (elect immediately) or a later one
                // (park as Ready for failover).
                if peers[idx].phase == PeerPhase::WaitingHandshake {
                    if elected.is_none() {
                        let (pid, fd) = (peers[idx].id, peers[idx].fd);
                        peers[idx].phase = PeerPhase::WaitingHeaders;
                        eprintln!("  peer {pid:?} elected → getheaders");
                        frame_and_send(&node, network, pid, fd, Command::GetHeaders, &payload);
                        elected = Some((idx, Instant::now()));
                    } else {
                        peers[idx].phase = PeerPhase::Ready;
                    }
                    continue;
                }

                if cmd == Some(Command::Headers) && peers[idx].phase == PeerPhase::WaitingHeaders {
                    let mut headers: Vec<BlockHeader> = Vec::new();
                    HeadersMsg::decode_into_limited::<2_000>(
                        &mut std::io::Cursor::new(&pl),
                        &mut headers,
                    )?;
                    return Ok(headers);
                }
            }
        }
    }
}

// ── block download session ───────────────────────────────────────────────────

/// Fan `needed` blocks out across all ready peers and write each to disk.
fn run_block_download_session(
    network: Network,
    peer_addrs: &[SocketAddrV4],
    data_dir: &Path,
    needed: &[(BlockHeight, Hash256)],
) -> Result<(), FetchError> {
    let node = Node::new(network.magic(), our_version())?;
    let inbound = node.inbound_rx.clone();
    let mut peers: Vec<PeerSlot> = Vec::new();
    let connect_t0 = Instant::now();
    for addr in peer_addrs {
        match node.connect(*addr) {
            Ok((id, fd)) => peers.push(PeerSlot {
                id,
                fd,
                phase: PeerPhase::WaitingHandshake,
                connected_at: connect_t0,
                in_flight: Vec::new(),
            }),
            Err(e) => eprintln!("  connect {addr}: {e}"),
        }
    }
    if peers.is_empty() {
        return Err(FetchError::AllPeersDead);
    }

    // hash → height for arrival routing. `handle_block` takes-and-removes,
    // so duplicate deliveries (from a retried getdata that both peers
    // eventually answer) become no-ops instead of double-counting `saved`.
    let mut pending: HashMap<Hash256, BlockHeight> =
        needed.iter().map(|(h, hash)| (*hash, *h)).collect();
    // Sequential cursor: next index in `needed` to assign.
    let mut next_assign = 0usize;
    // Reclaimed-from-stalled-peer queue (drained before next_assign advances).
    let mut retry: Vec<Hash256> = Vec::new();

    let mut saved = 0usize;
    let total = needed.len();
    // Per-progress watchdog: as long as *some* block keeps arriving across
    // all peers, the session is healthy and may run indefinitely. Reset on
    // every successful arrival (in `handle_block`'s caller) below.
    let mut last_progress = Instant::now();
    let no_progress_limit = Duration::from_secs(BLOCK_NO_PROGRESS_TIMEOUT_SECS);

    loop {
        let Some(msg) = inbound.pop_block() else {
            return Err(FetchError::AllPeersDead);
        };

        match msg {
            InboundMsg::Tick => {
                if last_progress.elapsed() > no_progress_limit {
                    return Err(FetchError::Timeout);
                }
                on_block_tick(
                    &node,
                    network,
                    needed,
                    &mut peers,
                    &mut retry,
                    &mut next_assign,
                )?;
            }
            InboundMsg::Network { peer: peer_id, command, payload } => {
                if let Some(done) = on_block_network_msg(
                    &node,
                    network,
                    data_dir,
                    needed,
                    peer_id,
                    command,
                    &payload,
                    &mut peers,
                    &mut pending,
                    &mut retry,
                    &mut next_assign,
                    &mut saved,
                    &mut last_progress,
                    total,
                )? {
                    return Ok(done);
                }
            }
        }
    }
}

fn on_block_tick(
    node: &Node,
    network: Network,
    needed: &[(BlockHeight, Hash256)],
    peers: &mut [PeerSlot],
    retry: &mut Vec<Hash256>,
    next_assign: &mut usize,
) -> Result<(), FetchError> {
    let stall = Duration::from_secs(STALL_SECS);
    for peer in peers.iter_mut() {
        if peer.phase != PeerPhase::Ready {
            continue;
        }
        let stalled: Vec<Hash256> = peer
            .in_flight
            .iter()
            .filter(|(_, t)| t.elapsed() > stall)
            .map(|(h, _)| *h)
            .collect();
        if !stalled.is_empty() {
            peer.in_flight.retain(|(h, _)| !stalled.contains(h));
            retry.extend(stalled);
        }
    }
    for i in 0..peers.len() {
        if peers[i].phase == PeerPhase::Ready && peers[i].in_flight.len() < PEER_WINDOW {
            topup(node, network, peers, i, needed, next_assign, retry);
        }
    }
    if peers.iter().all(|p| p.phase == PeerPhase::Dead) {
        return Err(FetchError::AllPeersDead);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn on_block_network_msg(
    node: &Node,
    network: Network,
    data_dir: &Path,
    needed: &[(BlockHeight, Hash256)],
    peer_id: PeerId,
    command: [u8; 12],
    payload: &[u8],
    peers: &mut [PeerSlot],
    pending: &mut HashMap<Hash256, BlockHeight>,
    retry: &mut Vec<Hash256>,
    next_assign: &mut usize,
    saved: &mut usize,
    last_progress: &mut Instant,
    total: usize,
) -> Result<Option<()>, FetchError> {
    let Some(idx) = peers.iter().position(|p| p.id == peer_id) else {
        return Ok(None);
    };
    let cmd = Command::from_command_name(mantle::network::CommandName(command));

    if peers[idx].phase == PeerPhase::WaitingHandshake {
        peers[idx].phase = PeerPhase::Ready;
        topup(node, network, peers, idx, needed, next_assign, retry);
        return Ok(None);
    }

    if cmd == Some(Command::Block)
        && let Some(height) = handle_block(payload, pending, data_dir)?
    {
        if let Some(pos) = peers[idx]
            .in_flight
            .iter()
            .position(|(h, _)| pending.get(h) == Some(&height))
        {
            peers[idx].in_flight.swap_remove(pos);
        }
        *saved += 1;
        *last_progress = Instant::now();
        if *saved == total {
            return Ok(Some(()));
        }
        topup(node, network, peers, idx, needed, next_assign, retry);
    }
    Ok(None)
}

fn topup(
    node: &Node,
    network: Network,
    peers: &mut [PeerSlot],
    peer_idx: usize,
    needed: &[(BlockHeight, Hash256)],
    next_assign: &mut usize,
    retry: &mut Vec<Hash256>,
) {
    let want = PEER_WINDOW.saturating_sub(peers[peer_idx].in_flight.len());
    if want == 0 {
        return;
    }
    let mut hashes: Vec<Hash256> = Vec::with_capacity(want);
    while hashes.len() < want && !retry.is_empty() {
        hashes.push(retry.swap_remove(0));
    }
    while hashes.len() < want && *next_assign < needed.len() {
        hashes.push(needed[*next_assign].1);
        *next_assign += 1;
    }
    if hashes.is_empty() {
        return;
    }
    let (pid, fd) = (peers[peer_idx].id, peers[peer_idx].fd);
    frame_and_send(
        node,
        network,
        pid,
        fd,
        Command::GetData,
        &encode_getdata(&hashes),
    );
    let now = Instant::now();
    peers[peer_idx]
        .in_flight
        .extend(hashes.iter().map(|&h| (h, now)));
}

fn handle_block(
    payload: &[u8],
    pending: &mut HashMap<Hash256, BlockHeight>,
    data_dir: &Path,
) -> Result<Option<BlockHeight>, FetchError> {
    if payload.len() < 80 {
        return Ok(None);
    }
    let hdr_bytes: &[u8; 80] = payload[..80].try_into().expect("slice is 80 bytes");
    let hdr = BlockHeader::from_bytes(hdr_bytes).map_err(mantle::peer::Error::Codec)?;
    let hash = hdr.hash();
    // Take instead of get: a block can arrive twice (we sometimes re-issue a
    // getdata to a different peer when the original peer stalls; if both
    // peers eventually deliver, the second arrival is a duplicate). Removing
    // from `pending` here makes the duplicate return `None` so the caller's
    // `saved` counter doesn't double-count and prematurely declare success
    // before every height is on disk.
    let Some(height) = pending.remove(&hash) else {
        return Ok(None);
    };
    let path = data_dir.join("blocks").join(format!("{:07}.bin", height.0));
    std::fs::write(&path, payload)?;
    Ok(Some(height))
}

// ── peer state ───────────────────────────────────────────────────────────────

#[derive(PartialEq, Eq)]
enum PeerPhase {
    WaitingHandshake,
    WaitingHeaders,
    Ready,
    Dead,
}

struct PeerSlot {
    id: PeerId,
    fd: i32,
    phase: PeerPhase,
    /// When we issued the `connect` for this peer. Used to time out peers
    /// that are TCP-reachable but never speak our protocol — `WaitingHandshake`
    /// is left silent until we get the first non-handshake message. The peer
    /// module has no disconnect signal, so this Tick-driven check is how we
    /// notice mute peers.
    connected_at: Instant,
    in_flight: Vec<(Hash256, Instant)>,
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn our_version() -> VersionMsg {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs(),
    )
    .unwrap_or(i64::MAX);
    let empty_addr = NetworkAddr {
        services: 0,
        ip: [0u8; 16],
        port: 0,
    };
    VersionMsg {
        version: 70015,
        services: services::NODE_NETWORK,
        timestamp: now,
        receiver: empty_addr,
        sender: empty_addr,
        nonce: 0xdead_beef_cafe_1234,
        user_agent: "/mantle-replay:0.1.0/".to_string(),
        start_height: 0,
        relay: false,
    }
}

fn encode_getdata(hashes: &[Hash256]) -> Vec<u8> {
    let entries: Vec<InvEntry> = hashes
        .iter()
        .map(|&hash| InvEntry {
            kind: InvType::WitnessBlock,
            hash,
        })
        .collect();
    let mut buf = Vec::new();
    InvMsg(entries)
        .encode(&mut buf)
        .expect("Vec<u8> writes never fail");
    buf
}

fn frame_and_send(
    node: &Node,
    network: Network,
    peer: PeerId,
    fd: i32,
    cmd: Command,
    payload: &[u8],
) {
    let mut framed = Vec::new();
    network
        .magic()
        .frame(cmd, payload, &mut framed)
        .expect("frame cannot fail");
    let _ = node.outbound_tx.push(OutboundMsg {
        peer,
        fd,
        bytes: framed.into_boxed_slice(),
    });
}

fn write_headers_bin(
    path: &Path,
    start: BlockHeight,
    headers: &[(BlockHeight, BlockHeader)],
) -> io::Result<()> {
    // Sorted by ascending height, written as raw 80-byte serialisation per
    // header. Same on-disk layout `bench/fetch-fixtures` produces, so its
    // fixtures and ours are interchangeable.
    let mut sorted: Vec<&(BlockHeight, BlockHeader)> = headers.iter().collect();
    sorted.sort_by_key(|(h, _)| h.0);
    let mut out = Vec::with_capacity(sorted.len() * 80);
    for (_, hdr) in &sorted {
        out.extend_from_slice(&hdr.to_bytes());
    }
    std::fs::write(path, &out)?;
    // Sidecar metadata.bin records the start height (matches fetch-fixtures).
    let metadata = path.parent().unwrap_or(Path::new(".")).join("metadata.bin");
    std::fs::write(metadata, start.0.to_le_bytes())?;
    Ok(())
}
