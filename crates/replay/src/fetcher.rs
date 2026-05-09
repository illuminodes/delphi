// `Fetcher`'s public API is consumed by `main.rs`, which lands in the next
// commit. Until then every item below looks unused; the allow keeps that one
// transitional commit warning-free.
#![allow(dead_code)]

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
/// Hard ceiling on how long a single sync phase (headers or blocks) may run.
const PHASE_DEADLINE_SECS: u64 = 600;

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
        self.data_dir.join("blocks").join(format!("{:07}.bin", height.0))
    }

    /// On-disk path for the headers index.
    #[must_use]
    pub fn headers_path(&self) -> PathBuf {
        self.data_dir.join("headers.bin")
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
        let want = u64::from(end.0).saturating_sub(u64::from(start.0)).saturating_add(1);
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
    pub fn fetch_blocks(
        &self,
        headers: &[(BlockHeight, BlockHeader)],
    ) -> Result<(), FetchError> {
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
/// Tries every configured peer until one delivers a `headers` response.
fn run_getheaders_session(
    network: Network,
    peer_addrs: &[SocketAddrV4],
    locator: Hash256,
) -> Result<Vec<BlockHeader>, FetchError> {
    let node = Node::new(network.magic(), our_version())?;
    let inbound = node.inbound_rx.clone();
    let mut peers: Vec<PeerSlot> = Vec::new();
    for addr in peer_addrs {
        match node.connect(*addr) {
            Ok((id, fd)) => peers.push(PeerSlot {
                id,
                fd,
                phase: PeerPhase::WaitingHandshake,
                in_flight: Vec::new(),
            }),
            Err(e) => eprintln!("  connect {addr}: {e}"),
        }
    }
    if peers.is_empty() {
        return Err(FetchError::AllPeersDead);
    }

    let mut headers_peer_elected = false;
    let payload = GetHeadersMsg::single_locator(locator).encode_to_vec();
    let deadline = Instant::now() + Duration::from_secs(PHASE_DEADLINE_SECS);

    loop {
        let Some(msg) = inbound.pop_block() else {
            return Err(FetchError::AllPeersDead);
        };

        match msg {
            InboundMsg::Tick => {
                if Instant::now() > deadline {
                    return Err(FetchError::Timeout);
                }
                if peers.iter().all(|p| p.phase == PeerPhase::Dead) {
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

                if peers[idx].phase == PeerPhase::WaitingHandshake {
                    if headers_peer_elected {
                        peers[idx].phase = PeerPhase::Ready;
                    } else {
                        headers_peer_elected = true;
                        peers[idx].phase = PeerPhase::WaitingHeaders;
                        let (pid, fd) = (peers[idx].id, peers[idx].fd);
                        frame_and_send(
                            &node,
                            network,
                            pid,
                            fd,
                            Command::GetHeaders,
                            &payload,
                        );
                    }
                    continue;
                }

                if cmd == Some(Command::Headers)
                    && peers[idx].phase == PeerPhase::WaitingHeaders
                {
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
    for addr in peer_addrs {
        match node.connect(*addr) {
            Ok((id, fd)) => peers.push(PeerSlot {
                id,
                fd,
                phase: PeerPhase::WaitingHandshake,
                in_flight: Vec::new(),
            }),
            Err(e) => eprintln!("  connect {addr}: {e}"),
        }
    }
    if peers.is_empty() {
        return Err(FetchError::AllPeersDead);
    }

    // hash → height for arrival routing.
    let pending: HashMap<Hash256, BlockHeight> =
        needed.iter().map(|(h, hash)| (*hash, *h)).collect();
    // Sequential cursor: next index in `needed` to assign.
    let mut next_assign = 0usize;
    // Reclaimed-from-stalled-peer queue (drained before next_assign advances).
    let mut retry: Vec<Hash256> = Vec::new();

    let mut saved = 0usize;
    let total = needed.len();
    let deadline = Instant::now() + Duration::from_secs(PHASE_DEADLINE_SECS);

    loop {
        let Some(msg) = inbound.pop_block() else {
            return Err(FetchError::AllPeersDead);
        };

        match msg {
            InboundMsg::Tick => {
                if Instant::now() > deadline {
                    return Err(FetchError::Timeout);
                }
                let stall = Duration::from_secs(STALL_SECS);
                for peer in &mut peers {
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
                    if peers[i].phase == PeerPhase::Ready
                        && peers[i].in_flight.len() < PEER_WINDOW
                    {
                        topup(&node, network, &mut peers,i, needed, &mut next_assign, &mut retry);
                    }
                }
                if peers.iter().all(|p| p.phase == PeerPhase::Dead) {
                    return Err(FetchError::AllPeersDead);
                }
            }

            InboundMsg::Network {
                peer: peer_id,
                command,
                payload,
            } => {
                let Some(idx) = peers.iter().position(|p| p.id == peer_id) else {
                    continue;
                };
                let cmd = Command::from_command_name(mantle::network::CommandName(command));

                // First application message → handshake done; promote to Ready.
                if peers[idx].phase == PeerPhase::WaitingHandshake {
                    peers[idx].phase = PeerPhase::Ready;
                    topup(&node, network, &mut peers,idx, needed, &mut next_assign, &mut retry);
                    continue;
                }

                if cmd == Some(Command::Block)
                    && let Some(height) = handle_block(&payload, &pending, data_dir)?
                {
                    if let Some(pos) = peers[idx]
                        .in_flight
                        .iter()
                        .position(|(h, _)| pending.get(h) == Some(&height))
                    {
                        peers[idx].in_flight.swap_remove(pos);
                    }
                    saved += 1;
                    if saved == total {
                        return Ok(());
                    }
                    topup(&node, network, &mut peers,idx, needed, &mut next_assign, &mut retry);
                }
            }
        }
    }
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
    frame_and_send(node, network, pid, fd, Command::GetData, &encode_getdata(&hashes));
    let now = Instant::now();
    peers[peer_idx].in_flight.extend(hashes.iter().map(|&h| (h, now)));
}

fn handle_block(
    payload: &[u8],
    pending: &HashMap<Hash256, BlockHeight>,
    data_dir: &Path,
) -> Result<Option<BlockHeight>, FetchError> {
    if payload.len() < 80 {
        return Ok(None);
    }
    let hdr_bytes: &[u8; 80] = payload[..80].try_into().expect("slice is 80 bytes");
    let hdr = BlockHeader::from_bytes(hdr_bytes).map_err(mantle::peer::Error::Codec)?;
    let hash = hdr.hash();
    let Some(&height) = pending.get(&hash) else {
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

fn frame_and_send(node: &Node, network: Network, peer: PeerId, fd: i32, cmd: Command, payload: &[u8]) {
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
