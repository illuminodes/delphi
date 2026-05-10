// Phase 1.5 work-in-progress — single-ring io_uring fetcher draft.
// Compiles but not yet wired into mantle-replay. Planned to replace the
// peer-rx/peer-tx-driven Fetcher once the headers/block download paths
// are validated end-to-end against testnet4.
#![allow(dead_code)]

//! Single-ring `io_uring` fetcher for `mantle-replay`.
//!
//! Replaces the two-ring `peer-rx`/`peer-tx`-driven `Fetcher` with a single
//! [`IoUring`] driven from the fetcher's main loop. One ring → one fd table
//! view, no cross-thread coordination, no separate `Node` abstraction. The
//! whole protocol state machine for every peer lives inline in this module,
//! which makes the dead-peer rotation that the old fetcher couldn't do
//! straightforward.
//!
//! Public surface mirrors [`Fetcher`](crate::fetcher::Fetcher) — `new`,
//! `sync_headers`, `fetch_blocks` — so `mantle-replay::main` can swap one
//! for the other behind a CLI flag without other changes.
//!
//! ## Per-peer state machine
//!
//! ```text
//! Connecting        — connect SQE in flight, link_timeout chained
//!     │ on -ECANCELED / -ECONNREFUSED                             ▶ Dead
//!     │ on success
//!     ▼
//! HandshakeSendVersion — version SQE in flight
//!     │ on send-CQE → submit recv → SendingVerack on inbound `version`
//!     ▼
//! Ready              — handshake complete; available for getheaders/getdata
//!     │ when picked for getheaders (one peer at a time)            ▶ WaitingHeaders
//!     │ when given a block to fetch                                ▶ Ready (in_flight tracks blocks)
//! ```
//!
//! Stalled or dead peers are dropped from the active set and replaced from
//! the loaded peer pool. Today's `Fetcher` commits to N peers at session
//! start and silently degrades when they mute; this fetcher cycles peers
//! aggressively so the no-progress watchdog rarely trips.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mantle::codec::{Decode, Encode};
use mantle::collections::{BuildPrehashedHasher, PrehashedMap, PrehashedSet};
use mantle::hash::checksum;
use mantle::network::{
    Command, CommandName, GetHeadersMsg, HeadersMsg, InvEntry, InvMsg, InvType, MessageHeader,
    Network, NetworkAddr, VersionMsg, services,
};
use mantle::primitives::{BlockHeader, BlockHeight, Hash256};
use ququmatz::{IoUring, Sqe};
use ququmatz::types::{
    AF_INET, MsgFlags, SOCK_NONBLOCK, SOCK_STREAM, SockAddrIn, SqeFlags, TimeoutFlags, Timespec,
};

/// Size of each peer's per-recv scratch buffer. Every recv SQE asks the
/// kernel to copy up to this many bytes; oversize messages are reassembled
/// over multiple recv CQEs by the per-peer parse buffer.
const RECV_CHUNK: usize = 64 * 1024;

/// Maximum simultaneous in-flight `getdata` requests per peer. Small on
/// purpose: with the global pull queue + hedging, a per-peer window of 2
/// is enough to keep pipelining busy without trapping work behind a slow
/// peer.
const PEER_INFLIGHT_CAP: usize = 2;

/// How long to wait for a peer's connect to complete before declaring it
/// dead. The kernel TCP stack will retransmit SYNs for ~127s by default;
/// we let it go for `CONNECT_TIMEOUT_SECS` and then move on.
const CONNECT_TIMEOUT_SECS: u64 = 5;

/// How long to wait for a single peer to deliver a `headers` message after
/// we sent it `getheaders` before re-electing a different peer.
const HEADERS_RESPONSE_TIMEOUT_SECS: u64 = 30;

/// Floor for the hedge threshold. We re-issue an outstanding hash to a
/// second peer once it has been outstanding longer than
/// `max(HEDGE_FLOOR, HEDGE_FACTOR * ewma_arrival)`. The floor stops us
/// hedging on every block during cold start when ewma is meaningless.
const HEDGE_FLOOR_MS: u64 = 5_000;

/// Multiplier applied to the EWMA of arrival latency to decide when to
/// hedge. 2× the median is the standard "tail" cutoff.
const HEDGE_FACTOR: u32 = 2;

/// Maximum peers a single hash may be assigned to simultaneously. 2 is
/// enough to bound bandwidth blow-up while still racing the slowest peer.
const HEDGE_FANOUT: usize = 2;

/// Floor for the reclaim threshold. After this long with no delivery from
/// any peer that holds the hash, we put it back at the front of the queue.
const RECLAIM_FLOOR_MS: u64 = 30_000;

/// Multiplier applied to the EWMA of arrival latency for the reclaim path.
/// Reclaim is "give up entirely and re-queue" — strictly later than hedge.
const RECLAIM_FACTOR: u32 = 4;

/// A peer that has been issued at least this many requests with zero
/// deliveries observed within `BLACK_HOLE_GRACE_SECS` is killed and
/// replaced from the pool. The dual-trigger (issued-count + grace-time)
/// avoids killing a peer that just hasn't been asked yet.
const BLACK_HOLE_ASSIGNED_THRESHOLD: u32 = 4;

/// How long a peer may go without delivering before it counts as a black
/// hole — provided it has been issued at least
/// `BLACK_HOLE_ASSIGNED_THRESHOLD` requests in that window.
const BLACK_HOLE_GRACE_SECS: u64 = 60;

/// The block-download phase sits idle for at most this long before bailing
/// — measured against the last successful block arrival across *all* peers.
/// Resets on every block save. Long-running runs are unbounded as long as
/// bytes keep flowing.
const BLOCK_NO_PROGRESS_TIMEOUT_SECS: u64 = 300;

/// Hard cap on a single header-sync phase (one `getheaders` page).
const HEADERS_PHASE_DEADLINE_SECS: u64 = 600;

/// Target number of healthy peers to keep dialed at any one time. The
/// fetcher will dial up to this many from its pool, replacing dead slots
/// from the rest of the pool. With `PEER_INFLIGHT_CAP=2`, 32 peers gives
/// us ~64 hashes in flight network-wide — wide enough that one slow peer
/// barely dents progress, narrow enough to fit in a single ring's
/// SQE/CQE budget without backpressure.
const TARGET_LIVE_PEERS: usize = 32;

/// SQ/CQ depth of the per-session `io_uring`. The kernel rounds up to
/// the next power of two; we ask for 16 384 so we have generous
/// headroom for `TARGET_LIVE_PEERS` × (recv + send + close + cancel +
/// `link_timeout`) during peer churn, plus the per-session tick.
/// Headroom matters because a full SQ ring backpressures `push()` with
/// `EAGAIN` and a full CQ ring drops completions on the kernel side.
const RING_ENTRIES: u32 = 16_384;

/// Capacity of the per-peer parse buffer (`Peer::recv_buf`). Sized to
/// hold one large mainnet block (~4 MiB witness blocks at peak heights)
/// without reallocating mid-frame.
const RECV_BUF_CAPACITY: usize = 4 * 1024 * 1024;

/// Initial guess for arrival-latency EWMA before we've seen any deliveries.
/// 1 second is a defensible cold-start; the floors keep early decisions
/// sane even if it's wildly off.
const INITIAL_EWMA_MS: u64 = 1_000;

/// Smoothing factor for the EWMA — bigger N = slower to adapt. Each new
/// sample contributes `1/N` of its value. 16 is gentle enough that one
/// outlier doesn't poison the threshold.
const EWMA_N: u32 = 16;

/// Errors surfaced from fetcher operations.
#[derive(Debug)]
pub enum FetchError {
    Io(io::Error),
    UringErr(String),
    NoPeers,
    Timeout(&'static str),
    PoolExhausted,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io: {e}"),
            Self::UringErr(s) => write!(f, "uring: {s}"),
            Self::NoPeers => f.write_str("no peer addresses configured"),
            Self::Timeout(phase) => write!(f, "{phase}: deadline exceeded"),
            Self::PoolExhausted => f.write_str("peer pool exhausted before phase finished"),
        }
    }
}

impl std::error::Error for FetchError {}

impl From<io::Error> for FetchError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Per-peer state. Slots are reused as peers die and new ones are dialed.
#[derive(Debug)]
struct Peer {
    addr: SocketAddrV4,
    fd: i32,
    /// Owned sockaddr bytes the kernel reads for the connect SQE; must
    /// outlive the SQE so we keep it on the peer struct.
    sockaddr: SockAddrIn,
    state: PeerState,
    /// Bytes accumulated from recv CQEs, awaiting frame parsing.
    recv_buf: Vec<u8>,
    /// One-shot recv scratch the kernel writes into. Kept in `Box<[u8]>`
    /// so the address is stable across submit boundaries.
    recv_scratch: Box<[u8]>,
    /// Block hashes currently requested from this peer, with the time we
    /// asked. Bounded by `PEER_INFLIGHT_CAP`; the global pull queue +
    /// hedging logic in `run_block_download_session` decides what to
    /// assign and when to re-issue.
    in_flight: Vec<(Hash256, Instant)>,
    /// When this peer became `Connecting`. Used to detect dead-on-arrival.
    connected_at: Instant,
    /// Generation counter — bumped each time the slot is reused. Stale
    /// CQEs from the previous incarnation carry the old generation in
    /// `user_data` and are silently dropped.
    generation: u16,
    /// Count of `getdata` requests issued to this peer this session. Used
    /// alongside `delivered_count` and `last_delivery` to detect black
    /// holes — peers that accept assignments but never produce blocks.
    assigned_count: u32,
    /// Count of blocks successfully delivered by this peer this session.
    delivered_count: u32,
    /// Last time this peer produced a block. Initialized to the dial time
    /// so a freshly-dialed peer gets the full grace window.
    last_delivery: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerState {
    Connecting,
    HandshakeSendVersion,
    HandshakeRecving,
    Ready,
    WaitingHeaders,
    Dead,
}

/// Per-hash bookkeeping for the global pull-queue + hedging model. The
/// same hash can be assigned to up to `HEDGE_FANOUT` peers; whichever
/// delivers first wins and the duplicates become silent no-ops via the
/// `pending.remove` dedupe in `consume_block_frames`.
#[derive(Debug, Clone)]
struct InFlight {
    /// Peer slots currently holding this hash. First entry is the
    /// original assignee; subsequent entries are hedges.
    issued_to: Vec<u16>,
    /// When the *first* assignment went out. Drives the hedge / reclaim
    /// thresholds — re-issuing doesn't reset the clock.
    first_issued: Instant,
    /// When the *most recent* assignment went out. Bookkeeping; not
    /// currently consulted by the threshold logic but useful in logs.
    #[allow(dead_code)]
    last_issued: Instant,
}

/// Exponential moving average of block-arrival latency, used to decide
/// when a hash has been outstanding "too long" relative to the typical
/// case. Stored as milliseconds so threshold math stays integer.
#[derive(Debug, Clone, Copy)]
struct EwmaLatency {
    /// Current EWMA in milliseconds.
    ms: u64,
}

impl EwmaLatency {
    const fn new() -> Self {
        Self {
            ms: INITIAL_EWMA_MS,
        }
    }

    /// Fold a new sample into the average. The smoothing weight is
    /// `1/EWMA_N`, so larger `EWMA_N` = slower to adapt.
    fn observe(&mut self, sample: Duration) {
        let s = u64::try_from(sample.as_millis()).unwrap_or(u64::MAX);
        // EWMA: new = old + (sample - old) / N, computed as a signed
        // delta to avoid arithmetic overflow on large `s` or `old`.
        let n = u64::from(EWMA_N);
        if s >= self.ms {
            self.ms = self.ms.saturating_add((s - self.ms) / n);
        } else {
            self.ms = self.ms.saturating_sub((self.ms - s) / n);
        }
    }

    /// Threshold = `max(floor_ms, factor * ewma_ms)`, returned as a
    /// `Duration`.
    fn threshold(self, floor_ms: u64, factor: u32) -> Duration {
        let scaled = self.ms.saturating_mul(u64::from(factor));
        Duration::from_millis(scaled.max(floor_ms))
    }

    const fn as_millis(self) -> u64 {
        self.ms
    }
}


/// Single-ring fetcher. Owns the `IoUring`, peer pool, and download state.
pub struct Fetcher {
    network: Network,
    /// Full unconsumed peer pool. Slots are dialed out of this list.
    peer_pool: Vec<SocketAddrV4>,
    /// Cursor into `peer_pool` — next index to dial.
    next_pool_idx: usize,
    data_dir: PathBuf,
    /// Owned timeout for the connect `link_timeout`. Reused.
    connect_timeout: Timespec,
}

impl Fetcher {
    /// `peer_addrs` is the static peer list to dial from. Order matters —
    /// earlier entries dialed first, so callers should pre-rank by latency.
    ///
    /// # Errors
    /// Returns `io::Error` if the on-disk blocks dir cannot be created.
    pub fn new(
        network: Network,
        peer_addrs: Vec<SocketAddrV4>,
        data_dir: PathBuf,
    ) -> io::Result<Self> {
        std::fs::create_dir_all(data_dir.join("blocks"))?;
        Ok(Self {
            network,
            peer_pool: peer_addrs,
            next_pool_idx: 0,
            data_dir,
            connect_timeout: Timespec::from_millis(CONNECT_TIMEOUT_SECS * 1_000),
        })
    }

    pub fn block_path(&self, height: BlockHeight) -> PathBuf {
        self.data_dir
            .join("blocks")
            .join(format!("{:07}.bin", height.0))
    }

    pub fn headers_path(&self) -> PathBuf {
        self.data_dir.join("chain_headers.bin")
    }

    /// # Errors
    /// `NotFound` if not yet on disk.
    pub fn read_block_bytes(&self, height: BlockHeight) -> io::Result<Vec<u8>> {
        std::fs::read(self.block_path(height))
    }

    /// Sync headers covering `[start..=end]` from `locator`. One getheaders
    /// page at a time; reuses the same ring across pages.
    pub fn sync_headers(
        &mut self,
        start: BlockHeight,
        end: BlockHeight,
        locator: Hash256,
    ) -> Result<Vec<(BlockHeight, BlockHeader)>, FetchError> {
        if self.peer_pool.is_empty() {
            return Err(FetchError::NoPeers);
        }
        let want = u64::from(end.0)
            .saturating_sub(u64::from(start.0))
            .saturating_add(1);
        let mut collected: Vec<(BlockHeight, BlockHeader)> =
            Vec::with_capacity(usize::try_from(want).unwrap_or(usize::MAX));
        let mut current_locator = locator;
        let mut next_height = start.0;

        loop {
            let page = self.run_getheaders_session(current_locator)?;
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
            let Some(last) = collected.last() else { break };
            current_locator = last.1.hash();
        }

        write_headers_bin(&self.headers_path(), start, &collected)?;
        Ok(collected)
    }

    /// Download every block in `headers` that isn't already on disk.
    pub fn fetch_blocks(
        &mut self,
        headers: &[(BlockHeight, BlockHeader)],
    ) -> Result<(), FetchError> {
        if self.peer_pool.is_empty() {
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
        self.run_block_download_session(&needed)
    }

    // ── headers session ──────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn run_getheaders_session(
        &mut self,
        locator: Hash256,
    ) -> Result<Vec<BlockHeader>, FetchError> {
        let mut ring = make_ring()?;
        let mut peers: HashMap<u16, Peer> = HashMap::new();
        let mut next_slot: u16 = 0;
        // Top up with TARGET_LIVE_PEERS dials.
        for _ in 0..TARGET_LIVE_PEERS.min(self.peer_pool.len()) {
            if let Some(peer) = self.dial_next(&mut ring, next_slot)? {
                peers.insert(next_slot, peer);
                next_slot = next_slot.wrapping_add(1);
            }
        }
        if peers.is_empty() {
            return Err(FetchError::PoolExhausted);
        }

        let phase_deadline = Instant::now() + Duration::from_secs(HEADERS_PHASE_DEADLINE_SECS);
        let payload = GetHeadersMsg::single_locator(locator).encode_to_vec();
        let mut elected: Option<(u16, Instant)> = None;

        loop {
            // Submit any pending SQEs and wait for at least one CQE, but
            // also poll a 1s tick so timeouts can fire even if no peer
            // produces a CQE on its own.
            push_tick(&mut ring, Duration::from_secs(1))?;
            ring.submit_and_wait(1).map_err(uring_err)?;

            // Drain CQEs.
            while let Some(cqe) = ring.complete() {
                let (slot, op, g) = decode_user_data(cqe.user_data);
                if op == OP_TICK {
                    continue;
                }
                let res = cqe.result;
                let Some(peer) = peers.get_mut(&slot) else {
                    continue;
                };
                if peer.generation != g {
                    continue; // stale CQE for a recycled slot
                }
                if let Some(headers) = handle_getheaders_cqe(
                    &mut ring,
                    self.network,
                    slot,
                    peer,
                    op,
                    res,
                    &payload,
                    &mut elected,
                )? {
                    return Ok(headers);
                }
            }

            // Timeouts and election.
            let now = Instant::now();
            if now > phase_deadline {
                return Err(FetchError::Timeout("headers"));
            }

            // Drop dead-on-arrival or response-timed-out peers.
            for (slot, peer) in &mut peers {
                if peer.state == PeerState::Dead {
                    continue;
                }
                if peer.state == PeerState::Connecting
                    && now.duration_since(peer.connected_at)
                        > Duration::from_secs(CONNECT_TIMEOUT_SECS + 2)
                {
                    eprintln!(
                        "  iouring-fetcher: peer {} ({}) connect-timeout",
                        slot, peer.addr
                    );
                    close_peer(&mut ring, peer);
                    peer.state = PeerState::Dead;
                }
            }
            if let Some((slot, sent)) = elected
                && now.duration_since(sent) > Duration::from_secs(HEADERS_RESPONSE_TIMEOUT_SECS)
            {
                eprintln!("  iouring-fetcher: elected peer {slot} headers-timeout, demoting");
                if let Some(peer) = peers.get_mut(&slot) {
                    close_peer(&mut ring, peer);
                    peer.state = PeerState::Dead;
                }
                elected = None;
            }

            // Scan for headers reply: any peer that delivered a complete
            // headers frame moves it into `peer.recv_buf` and the parse
            // step in handle_getheaders_cqe parses it. We surface the
            // result via a sentinel: peer.in_flight is reused to carry
            // the parsed headers Vec by writing them into a sidechannel.
            // For simplicity here, return as soon as the elected peer
            // hits Ready *after* sending getheaders.
            for peer in peers.values_mut() {
                if peer.state == PeerState::Ready
                    && peer.recv_buf.iter().any(|_| false)
                {
                    // Placeholder — actual parse happens in handle path.
                }
            }

            // Refill dead slots from the pool.
            let dead_slots: Vec<u16> = peers
                .iter()
                .filter_map(|(s, p)| (p.state == PeerState::Dead).then_some(*s))
                .collect();
            for slot in dead_slots {
                if let Some(p) = peers.remove(&slot) {
                    drop(p);
                }
                if let Some(peer) = self.dial_next(&mut ring, next_slot)? {
                    peers.insert(next_slot, peer);
                    next_slot = next_slot.wrapping_add(1);
                }
            }
            if peers.is_empty() {
                return Err(FetchError::PoolExhausted);
            }

            // Election: any Ready peer with no current pick → send getheaders.
            if elected.is_none()
                && let Some((&slot, peer)) = peers.iter_mut().find(|(_, p)| p.state == PeerState::Ready)
            {
                let fd = peer.fd;
                push_send(&mut ring, slot, peer.generation, fd, frame(self.network, Command::GetHeaders, &payload))?;
                peer.state = PeerState::WaitingHeaders;
                elected = Some((slot, Instant::now()));
            }

        }
    }

    // ── block download session ──────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn run_block_download_session(
        &mut self,
        needed: &[(BlockHeight, Hash256)],
    ) -> Result<(), FetchError> {
        let mut ring = make_ring()?;
        let mut peers: HashMap<u16, Peer> = HashMap::new();
        let mut next_slot: u16 = 0;
        for _ in 0..TARGET_LIVE_PEERS.min(self.peer_pool.len()) {
            if let Some(peer) = self.dial_next(&mut ring, next_slot)? {
                peers.insert(next_slot, peer);
                next_slot = next_slot.wrapping_add(1);
            }
        }
        if peers.is_empty() {
            return Err(FetchError::PoolExhausted);
        }

        // `pending` maps yet-to-arrive hashes → height. `handle_block_cqe`
        // already calls `pending.remove`, so duplicate deliveries (from
        // hedged or retried requests) are silently no-ops.
        let mut pending: PrehashedMap<Hash256, BlockHeight> =
            HashMap::with_capacity_and_hasher(needed.len(), BuildPrehashedHasher);
        for (h, hash) in needed {
            pending.insert(*hash, *h);
        }

        // Global pull queue: heights still un-issued, in chain order.
        // Re-queued (reclaimed) hashes go to the *front* so they don't
        // sit at the tail behind unstarted work.
        let mut todo: VecDeque<Hash256> = needed.iter().map(|(_, h)| *h).collect();
        // Currently outstanding hashes → which peer slots hold them and
        // when the *first* request was issued (not the most recent —
        // hedging needs the original timestamp to decide if the tail has
        // stretched too far).
        let mut in_flight: PrehashedMap<Hash256, InFlight> =
            HashMap::with_capacity_and_hasher(needed.len() / 4 + 16, BuildPrehashedHasher);
        let mut ewma = EwmaLatency::new();

        let mut saved = 0usize;
        let total = needed.len();
        let mut last_progress = Instant::now();
        let mut last_log = Instant::now();
        let mut saved_at_last_log = 0usize;
        let log_interval = Duration::from_secs(2);

        loop {
            push_tick(&mut ring, Duration::from_secs(1))?;
            ring.submit_and_wait(1).map_err(uring_err)?;

            let mut got_block_hashes: Vec<Hash256> = Vec::new();
            while let Some(cqe) = ring.complete() {
                let (slot, op, g) = decode_user_data(cqe.user_data);
                if op == OP_TICK {
                    continue;
                }
                let Some(peer) = peers.get_mut(&slot) else {
                    continue;
                };
                if peer.generation != g {
                    continue;
                }
                handle_block_cqe(
                    &mut ring,
                    self.network,
                    slot,
                    peer,
                    op,
                    cqe.result,
                    &mut pending,
                    &self.data_dir,
                    &mut saved,
                    &mut got_block_hashes,
                )?;
            }

            // For every arrived block: update EWMA from the *original*
            // request time, then drop the hash from `in_flight` and from
            // every peer that was hedged-to (not just the deliverer).
            if !got_block_hashes.is_empty() {
                last_progress = Instant::now();
                for hash in &got_block_hashes {
                    if let Some(info) = in_flight.remove(hash) {
                        let arrival_latency = info.first_issued.elapsed();
                        ewma.observe(arrival_latency);
                        for slot in &info.issued_to {
                            if let Some(peer) = peers.get_mut(slot) {
                                peer.in_flight.retain(|(h, _)| h != hash);
                            }
                        }
                    }
                }
            }

            if saved == total {
                return Ok(());
            }
            if last_progress.elapsed() > Duration::from_secs(BLOCK_NO_PROGRESS_TIMEOUT_SECS) {
                return Err(FetchError::Timeout("block-download"));
            }

            let now = Instant::now();

            // Periodic progress log.
            if now.duration_since(last_log) >= log_interval {
                let window = now.duration_since(last_log).as_secs_f64().max(1e-9);
                #[allow(clippy::cast_precision_loss)]
                let bps = (saved - saved_at_last_log) as f64 / window;
                let ready = peers.values().filter(|p| p.state == PeerState::Ready).count();
                let in_flight_count = in_flight.len();
                eprintln!(
                    "  iouring-fetcher: {saved}/{total}  {bps:.1} blk/s  ready={ready} \
                     in_flight={in_flight_count} todo={} ewma={}ms",
                    todo.len(),
                    ewma.as_millis(),
                );
                last_log = now;
                saved_at_last_log = saved;
            }

            // Hedge: any in_flight hash that has been outstanding longer
            // than `max(HEDGE_FLOOR, HEDGE_FACTOR * ewma)`, with fewer
            // than `HEDGE_FANOUT` peers holding it, gets re-issued to a
            // different peer. Reclaim: an even older hash with all its
            // hedge slots used goes back to the front of `todo`.
            let hedge_threshold = ewma.threshold(HEDGE_FLOOR_MS, HEDGE_FACTOR);
            let reclaim_threshold = ewma.threshold(RECLAIM_FLOOR_MS, RECLAIM_FACTOR);
            let mut to_hedge: Vec<Hash256> = Vec::new();
            let mut to_reclaim: Vec<Hash256> = Vec::new();
            for (hash, info) in &in_flight {
                let age = now.duration_since(info.first_issued);
                if age >= reclaim_threshold && info.issued_to.len() >= HEDGE_FANOUT {
                    to_reclaim.push(*hash);
                } else if age >= hedge_threshold && info.issued_to.len() < HEDGE_FANOUT {
                    to_hedge.push(*hash);
                }
            }
            for hash in to_reclaim {
                if let Some(info) = in_flight.remove(&hash) {
                    for slot in &info.issued_to {
                        if let Some(peer) = peers.get_mut(slot) {
                            peer.in_flight.retain(|(h, _)| h != &hash);
                        }
                    }
                }
                todo.push_front(hash);
            }

            // Connect-timeout + black-hole sweep. Both produce Dead peers
            // whose work returns to the front of `todo`.
            for (slot, peer) in &mut peers {
                if peer.state == PeerState::Connecting
                    && now.duration_since(peer.connected_at)
                        > Duration::from_secs(CONNECT_TIMEOUT_SECS + 2)
                {
                    close_peer(&mut ring, peer);
                    peer.state = PeerState::Dead;
                    continue;
                }
                if peer.state == PeerState::Ready
                    && peer.assigned_count >= BLACK_HOLE_ASSIGNED_THRESHOLD
                    && peer.delivered_count == 0
                    && now.duration_since(peer.last_delivery)
                        >= Duration::from_secs(BLACK_HOLE_GRACE_SECS)
                {
                    eprintln!(
                        "  iouring-fetcher: peer slot={slot} ({}) is a black hole \
                         (assigned={} delivered=0 grace={}s) — evicting",
                        peer.addr, peer.assigned_count, BLACK_HOLE_GRACE_SECS,
                    );
                    close_peer(&mut ring, peer);
                    peer.state = PeerState::Dead;
                }
            }

            // Refill dead slots. Their in-flight hashes need to be removed
            // from the per-hash issued_to lists; if no peer still holds a
            // hash, return it to the front of `todo` so it gets re-issued.
            let dead_slots: Vec<u16> = peers
                .iter()
                .filter_map(|(s, p)| (p.state == PeerState::Dead).then_some(*s))
                .collect();
            for slot in dead_slots {
                if let Some(p) = peers.remove(&slot) {
                    for (h, _) in p.in_flight {
                        if let Some(info) = in_flight.get_mut(&h) {
                            info.issued_to.retain(|s| *s != slot);
                            if info.issued_to.is_empty() {
                                in_flight.remove(&h);
                                todo.push_front(h);
                            }
                        }
                    }
                }
                let active: Vec<SocketAddrV4> = peers.values().map(|p| p.addr).collect();
                if let Some(peer) = self.dial_next_avoiding(&mut ring, next_slot, &active)? {
                    peers.insert(next_slot, peer);
                    next_slot = next_slot.wrapping_add(1);
                }
            }

            // Now hedge: each entry in `to_hedge` goes to a Ready peer
            // that doesn't already hold it and has capacity. We do this
            // *after* dead-slot reaping so freshly-dialed peers are
            // candidates.
            for hash in to_hedge {
                let Some(info) = in_flight.get(&hash).cloned() else {
                    continue;
                };
                if info.issued_to.len() >= HEDGE_FANOUT {
                    continue;
                }
                let already: PrehashedSet<u16> = {
                    let mut s = HashSet::with_hasher(BuildPrehashedHasher);
                    for slot in &info.issued_to {
                        s.insert(*slot);
                    }
                    s
                };
                let pick = peers.iter_mut().find(|(slot, peer)| {
                    peer.state == PeerState::Ready
                        && peer.in_flight.len() < PEER_INFLIGHT_CAP
                        && !already.contains(*slot)
                });
                if let Some((slot, peer)) = pick {
                    let pl = encode_getdata(&[hash]);
                    push_send(
                        &mut ring,
                        *slot,
                        peer.generation,
                        peer.fd,
                        frame(self.network, Command::GetData, &pl),
                    )?;
                    peer.in_flight.push((hash, now));
                    peer.assigned_count = peer.assigned_count.saturating_add(1);
                    if let Some(info) = in_flight.get_mut(&hash) {
                        info.issued_to.push(*slot);
                        info.last_issued = now;
                    }
                }
            }

            // Pull-queue assignment: each Ready peer with capacity grabs
            // the front of `todo`. No per-peer batching — slow peers
            // simply pull less often.
            for (slot, peer) in &mut peers {
                if peer.state != PeerState::Ready {
                    continue;
                }
                while peer.in_flight.len() < PEER_INFLIGHT_CAP {
                    let Some(hash) = todo.pop_front() else { break };
                    let pl = encode_getdata(&[hash]);
                    push_send(
                        &mut ring,
                        *slot,
                        peer.generation,
                        peer.fd,
                        frame(self.network, Command::GetData, &pl),
                    )?;
                    peer.in_flight.push((hash, now));
                    peer.assigned_count = peer.assigned_count.saturating_add(1);
                    in_flight.insert(
                        hash,
                        InFlight {
                            issued_to: vec![*slot],
                            first_issued: now,
                            last_issued: now,
                        },
                    );
                }
            }

            if peers.is_empty() && in_flight.is_empty() && todo.is_empty() {
                return Err(FetchError::PoolExhausted);
            }
        }
    }

    // ── peer dial ───────────────────────────────────────────────────────

    fn dial_next(&mut self, ring: &mut IoUring, slot: u16) -> Result<Option<Peer>, FetchError> {
        self.dial_next_avoiding(ring, slot, &[])
    }

    /// Dial the next peer from the pool, skipping any address in `active`.
    /// Lets the block-download loop wrap around the pool without
    /// re-dialing peers it's already connected to or just evicted.
    fn dial_next_avoiding(
        &mut self,
        ring: &mut IoUring,
        slot: u16,
        active: &[SocketAddrV4],
    ) -> Result<Option<Peer>, FetchError> {
        let Some(addr) = self.next_pool_addr(active) else {
            return Ok(None);
        };
        let sock = ququmatz::Socket::new(AF_INET, SOCK_STREAM | SOCK_NONBLOCK, 0)
            .map_err(|e| FetchError::UringErr(format!("socket: {e:?}")))?;
        let fd = sock.into_fd();
        let sockaddr = SockAddrIn {
            sin_family: u16::try_from(AF_INET).expect("AF_INET fits u16"),
            sin_port: addr.port().to_be(),
            sin_addr: u32::from_ne_bytes(addr.ip().octets()),
            sin_zero: [0; 8],
        };

        let mut peer = Peer {
            addr,
            fd,
            sockaddr,
            state: PeerState::Connecting,
            recv_buf: Vec::with_capacity(RECV_BUF_CAPACITY),
            recv_scratch: vec![0u8; RECV_CHUNK].into_boxed_slice(),
            in_flight: Vec::new(),
            connected_at: Instant::now(),
            generation: 0,
            assigned_count: 0,
            delivered_count: 0,
            last_delivery: Instant::now(),
        };

        push_connect(ring, slot, peer.generation, fd, &peer.sockaddr, &self.connect_timeout)?;
        // Submit a recv straight away — the kernel will hold it until the
        // socket is connected (then deliver the first version frame in one
        // CQE). If connect fails, the recv CQE arrives with -ECANCELED.
        push_recv(ring, slot, peer.generation, fd, &mut peer.recv_scratch)?;
        // Stage the version send for after handshake — actually send it
        // immediately via the SQE chain (kernel queues sends until the
        // socket is connected, just like recv).
        let payload = our_version().encode_to_vec();
        push_send(ring, slot, peer.generation, fd, frame(self.network, Command::Version, &payload))?;

        Ok(Some(peer))
    }

    /// Pull the next pool address, wrapping around the pool if needed
    /// and skipping anything in `avoid`. Returns `None` only when *every*
    /// pool entry is currently in use — i.e. we'd have to reuse an
    /// already-connected address. The cursor advances through the pool
    /// monotonically modulo `pool.len()` so order roughly matches the
    /// caller-supplied (latency-ranked) ordering.
    fn next_pool_addr(&mut self, avoid: &[SocketAddrV4]) -> Option<SocketAddrV4> {
        if self.peer_pool.is_empty() {
            return None;
        }
        let len = self.peer_pool.len();
        for _ in 0..len {
            let idx = self.next_pool_idx % len;
            self.next_pool_idx = idx.wrapping_add(1);
            let addr = self.peer_pool[idx];
            if !avoid.contains(&addr) {
                return Some(addr);
            }
        }
        None
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

const OP_CONNECT: u8 = 0;
const OP_LINK_TIMEOUT: u8 = 1;
const OP_SEND: u8 = 2;
const OP_RECV: u8 = 3;
const OP_CLOSE: u8 = 4;
const OP_TICK: u8 = 0xff;

#[inline]
fn make_user_data(slot: u16, g: u16, op: u8) -> u64 {
    (u64::from(slot) << 16) | (u64::from(g) << 32) | u64::from(op)
}

#[inline]
fn decode_user_data(raw: u64) -> (u16, u8, u16) {
    #[allow(clippy::cast_possible_truncation)]
    let op = (raw & 0xff) as u8;
    #[allow(clippy::cast_possible_truncation)]
    let slot = ((raw >> 16) & 0xffff) as u16;
    #[allow(clippy::cast_possible_truncation)]
    let g = ((raw >> 32) & 0xffff) as u16;
    (slot, op, g)
}

fn make_ring() -> Result<IoUring, FetchError> {
    IoUring::new(RING_ENTRIES).map_err(|e| FetchError::UringErr(format!("IoUring::new: {e:?}")))
}

fn uring_err(e: ququmatz::Error) -> FetchError {
    FetchError::UringErr(format!("{e:?}"))
}

fn push_tick(ring: &mut IoUring, dur: Duration) -> Result<(), FetchError> {
    let ts = Timespec::from_millis(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX));
    ring.push(
        Sqe::timeout(&ts, 1, TimeoutFlags::default())
            .user_data(make_user_data(0, 0, OP_TICK)),
    )
    .map_err(uring_err)
}

fn push_connect(
    ring: &mut IoUring,
    slot: u16,
    g: u16,
    fd: i32,
    sockaddr: &SockAddrIn,
    timeout: &Timespec,
) -> Result<(), FetchError> {
    let addr_bytes: &[u8] = unsafe {
        std::slice::from_raw_parts(
            std::ptr::from_ref::<SockAddrIn>(sockaddr).cast(),
            std::mem::size_of::<SockAddrIn>(),
        )
    };
    ring.push(
        Sqe::connect(fd, addr_bytes)
            .user_data(make_user_data(slot, g, OP_CONNECT))
            .flags(SqeFlags::IO_LINK),
    )
    .map_err(uring_err)?;
    ring.push(
        Sqe::link_timeout(timeout, TimeoutFlags::default())
            .user_data(make_user_data(slot, g, OP_LINK_TIMEOUT)),
    )
    .map_err(uring_err)?;
    Ok(())
}

fn push_send(
    ring: &mut IoUring,
    slot: u16,
    g: u16,
    fd: i32,
    framed: Vec<u8>,
) -> Result<(), FetchError> {
    // The kernel takes a snapshot of the buffer at send-time, but ququmatz's
    // `send` borrows the slice — we'd ideally keep the Vec alive until the
    // CQE returns. Leak it for now via Box and `into_raw`; a real impl
    // pools these. Acceptable for an initial cut.
    let leaked: &'static [u8] = Box::leak(framed.into_boxed_slice());
    ring.push(
        Sqe::send(fd, leaked, MsgFlags::default())
            .user_data(make_user_data(slot, g, OP_SEND)),
    )
    .map_err(uring_err)
}

fn push_recv(
    ring: &mut IoUring,
    slot: u16,
    g: u16,
    fd: i32,
    scratch: &mut [u8],
) -> Result<(), FetchError> {
    ring.push(
        Sqe::recv(fd, scratch, MsgFlags::default())
            .user_data(make_user_data(slot, g, OP_RECV)),
    )
    .map_err(uring_err)
}

fn close_peer(ring: &mut IoUring, peer: &mut Peer) {
    if peer.fd >= 0 {
        let _ = ring.push(
            Sqe::close(peer.fd).user_data(make_user_data(0, peer.generation, OP_CLOSE)),
        );
        peer.fd = -1;
    }
}

fn frame(network: Network, cmd: Command, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(MessageHeader::ENCODED_LEN + payload.len());
    network
        .magic()
        .frame(cmd, payload, &mut out)
        .expect("Vec write cannot fail");
    out
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
        .expect("Vec write cannot fail");
    buf
}

fn our_version() -> VersionMsg {
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs(),
    )
    .unwrap_or(i64::MAX);
    let empty = NetworkAddr {
        services: 0,
        ip: [0u8; 16],
        port: 0,
    };
    VersionMsg {
        version: 70016,
        services: services::NODE_NETWORK,
        timestamp: now,
        receiver: empty,
        sender: empty,
        nonce: 0xfeed_face_dead_beef,
        user_agent: "/mantle-replay-iouring:0.1.0/".to_string(),
        start_height: 0,
        relay: false,
    }
}

fn write_headers_bin(
    path: &Path,
    start: BlockHeight,
    headers: &[(BlockHeight, BlockHeader)],
) -> io::Result<()> {
    let mut sorted: Vec<&(BlockHeight, BlockHeader)> = headers.iter().collect();
    sorted.sort_by_key(|(h, _)| h.0);
    let mut out = Vec::with_capacity(sorted.len() * 80);
    for (_, hdr) in &sorted {
        out.extend_from_slice(&hdr.to_bytes());
    }
    fs::write(path, &out)?;
    let metadata = path.parent().unwrap_or(Path::new(".")).join("metadata.bin");
    fs::write(metadata, start.0.to_le_bytes())?;
    Ok(())
}

// ── CQE handlers ────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn handle_getheaders_cqe(
    ring: &mut IoUring,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    op: u8,
    res: i32,
    payload: &[u8],
    elected: &mut Option<(u16, Instant)>,
) -> Result<Option<Vec<BlockHeader>>, FetchError> {
    match op {
        OP_CONNECT => {
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, peer);
                return Ok(None);
            }
            // Connect succeeded; the version send was already submitted in
            // dial_next, so we're already in HandshakeSendVersion implicitly.
            peer.state = PeerState::HandshakeSendVersion;
        }
        OP_SEND => {
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, peer);
            }
        }
        OP_RECV => {
            if res <= 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, peer);
                return Ok(None);
            }
            #[allow(clippy::cast_sign_loss)]
            let n = res as usize;
            peer.recv_buf
                .extend_from_slice(&peer.recv_scratch[..n.min(peer.recv_scratch.len())]);
            // Resubmit recv to keep the pipeline running.
            push_recv(ring, slot, peer.generation, peer.fd, &mut peer.recv_scratch)?;
            // Drain any complete frames.
            return consume_handshake_frames(ring, network, slot, peer, payload, elected);
        }
        _ => {}
    }
    Ok(None)
}

fn consume_handshake_frames(
    ring: &mut IoUring,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    _payload: &[u8],
    _elected: &mut Option<(u16, Instant)>,
) -> Result<Option<Vec<BlockHeader>>, FetchError> {
    while peer.recv_buf.len() >= MessageHeader::ENCODED_LEN {
        let Ok(header) = MessageHeader::decode(&mut std::io::Cursor::new(
            &peer.recv_buf[..MessageHeader::ENCODED_LEN],
        )) else { break };
        let total = MessageHeader::ENCODED_LEN + header.length as usize;
        if peer.recv_buf.len() < total {
            break;
        }
        let payload_slice = &peer.recv_buf[MessageHeader::ENCODED_LEN..total];
        if checksum(payload_slice) != header.checksum {
            peer.state = PeerState::Dead;
            close_peer(ring, peer);
            return Ok(None);
        }
        let cmd = header.command.as_str();
        match (peer.state, cmd) {
            (PeerState::HandshakeSendVersion | PeerState::HandshakeRecving, "version") => {
                push_send(
                    ring,
                    slot,
                    peer.generation,
                    peer.fd,
                    frame(network, Command::Verack, &[]),
                )?;
                peer.state = PeerState::HandshakeRecving;
            }
            (PeerState::HandshakeRecving | PeerState::HandshakeSendVersion, "verack") => {
                peer.state = PeerState::Ready;
            }
            (PeerState::Ready | PeerState::WaitingHeaders, "ping") => {
                let pong: Vec<u8> = payload_slice.to_owned();
                push_send(
                    ring,
                    slot,
                    peer.generation,
                    peer.fd,
                    frame(network, Command::Pong, &pong),
                )?;
            }
            (PeerState::WaitingHeaders, "headers") => {
                let mut headers: Vec<BlockHeader> = Vec::new();
                let _ = HeadersMsg::decode_into_limited::<2_000>(
                    &mut std::io::Cursor::new(payload_slice),
                    &mut headers,
                );
                peer.recv_buf.drain(..total);
                return Ok(Some(headers));
            }
            _ => {}
        }
        peer.recv_buf.drain(..total);
    }
    Ok(None)
}


#[allow(clippy::too_many_arguments)]
fn handle_block_cqe(
    ring: &mut IoUring,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    op: u8,
    res: i32,
    pending: &mut PrehashedMap<Hash256, BlockHeight>,
    data_dir: &Path,
    saved: &mut usize,
    arrived: &mut Vec<Hash256>,
) -> Result<(), FetchError> {
    match op {
        OP_CONNECT => {
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, peer);
                return Ok(());
            }
            peer.state = PeerState::HandshakeSendVersion;
            Ok(())
        }
        OP_RECV => {
            if res <= 0 {
                // Peer disconnected. The coordinator will detect Dead in
                // the next sweep and reclaim this peer's in_flight hashes
                // back to the global queue.
                peer.state = PeerState::Dead;
                close_peer(ring, peer);
                return Ok(());
            }
            #[allow(clippy::cast_sign_loss)]
            let n = res as usize;
            peer.recv_buf
                .extend_from_slice(&peer.recv_scratch[..n.min(peer.recv_scratch.len())]);
            push_recv(ring, slot, peer.generation, peer.fd, &mut peer.recv_scratch)?;
            consume_block_frames(ring, network, slot, peer, pending, data_dir, saved, arrived)
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn consume_block_frames(
    ring: &mut IoUring,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    pending: &mut PrehashedMap<Hash256, BlockHeight>,
    data_dir: &Path,
    saved: &mut usize,
    arrived: &mut Vec<Hash256>,
) -> Result<(), FetchError> {
    while peer.recv_buf.len() >= MessageHeader::ENCODED_LEN {
        let Ok(header) = MessageHeader::decode(&mut std::io::Cursor::new(
            &peer.recv_buf[..MessageHeader::ENCODED_LEN],
        )) else {
            break;
        };
        let total = MessageHeader::ENCODED_LEN + header.length as usize;
        if peer.recv_buf.len() < total {
            break;
        }
        let payload_slice = &peer.recv_buf[MessageHeader::ENCODED_LEN..total];
        if checksum(payload_slice) != header.checksum {
            peer.state = PeerState::Dead;
            close_peer(ring, peer);
            return Ok(());
        }
        let cmd = header.command.as_str();
        match (peer.state, cmd) {
            (PeerState::HandshakeSendVersion | PeerState::HandshakeRecving, "version") => {
                push_send(
                    ring,
                    slot,
                    peer.generation,
                    peer.fd,
                    frame(network, Command::Verack, &[]),
                )?;
                peer.state = PeerState::HandshakeRecving;
            }
            (PeerState::HandshakeRecving | PeerState::HandshakeSendVersion, "verack") => {
                peer.state = PeerState::Ready;
            }
            (PeerState::Ready, "block") => {
                use mantle::primitives::Block;
                let Ok(block) = Block::decode(&mut std::io::Cursor::new(payload_slice)) else {
                    peer.recv_buf.drain(..total);
                    continue;
                };
                let hash = block.header.hash();
                // `pending.remove` returns `Some` only on the first
                // arrival of a hash — duplicates from hedged requests
                // become silent no-ops here.
                if let Some(height) = pending.remove(&hash) {
                    let path = data_dir.join("blocks").join(format!("{:07}.bin", height.0));
                    fs::write(&path, payload_slice)?;
                    *saved += 1;
                    peer.delivered_count = peer.delivered_count.saturating_add(1);
                    peer.last_delivery = Instant::now();
                    arrived.push(hash);
                }
            }
            (_, "ping") => {
                let pong: Vec<u8> = payload_slice.to_owned();
                push_send(
                    ring,
                    slot,
                    peer.generation,
                    peer.fd,
                    frame(network, Command::Pong, &pong),
                )?;
            }
            _ => {}
        }
        peer.recv_buf.drain(..total);
    }
    Ok(())
}

// Helper added on `GetHeadersMsg` and `VersionMsg` for ergonomic encode-to-Vec.
trait EncodeToVec {
    fn encode_to_vec(&self) -> Vec<u8>;
}

impl<T: Encode> EncodeToVec for T {
    fn encode_to_vec(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let _ = self.encode(&mut out);
        out
    }
}

// ── Helpers required by GetHeadersMsg single_locator ────────────────────

trait GetHeadersMsgExt {
    fn single_locator(locator: Hash256) -> GetHeadersMsg;
}

impl GetHeadersMsgExt for GetHeadersMsg {
    fn single_locator(locator: Hash256) -> Self {
        Self {
            version: 70016,
            locator_hashes: vec![locator],
            stop_hash: Hash256::ZERO,
        }
    }
}

// `CommandName` is unused here today but imported for future use.
#[allow(dead_code)]
type _CommandName = CommandName;
