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
use ququmatz::types::{
    AddressFamily, MsgFlags, RawFd, SockAddrIn, SocketFlags, SocketType, SqeFlags, TimeoutFlags,
    Timespec,
};
use ququmatz::{IoUring, Sqe};

/// Size of each kernel-managed receive buffer in the provided-buffer
/// ring. The kernel hands one buffer to each multishot-recv arrival;
/// large frames span multiple buffers and are stitched together by
/// `Peer::recv_buf` on the userland side.
const PROVIDED_BUF_SIZE: u32 = 256 * 1024;

/// Number of buffers in the provided-buffer ring. Must be a power of
/// two (kernel ABI). Sized so 32 peers each in the middle of streaming
/// a 4 MiB block plus handshake/inv noise stays comfortably inside
/// the pool.
const PROVIDED_BUF_COUNT: u32 = 1_024;

/// `io_uring` buffer-group id we register the provided-buffer ring
/// under. The recv SQE references this same id via `buffer_select`.
const PROVIDED_BUF_GROUP_ID: u16 = 1;

/// Per-slot capacity of the send arena. Bitcoin's outbound frames the
/// fetcher emits — version, verack, pong, getheaders, getdata — all
/// fit in well under 4 KiB. Oversize frames are rejected at
/// `frame_into` time so a runaway message length can't corrupt
/// neighbouring slots.
const SEND_SLOT_SIZE: usize = 4 * 1024;

/// Number of slots in the send arena. Bounded above by `u16::MAX`
/// because the slot index rides in the upper 16 bits of `user_data`.
const SEND_SLOT_COUNT: u16 = 8_192;

/// Cap on CQEs drained per session-loop pass. Multishot recv can
/// produce a torrent of CQEs from one fast peer; if we let the inner
/// drain run to empty we never call `submit_and_wait` again, so
/// recently-pushed SEND SQEs sit in the SQ ring with no chance of
/// surfacing their CQEs. Capping the drain forces a round-trip
/// through the kernel between bursts.
const CQE_DRAIN_CAP: usize = 256;

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
const HEADERS_RESPONSE_TIMEOUT_SECS: u64 = 10;

/// How long a peer can sit in `HandshakeSendVersion` / `HandshakeRecving`
/// (TCP connected, but no `version`/`verack` exchange completed) before
/// we evict it. Silent peers are common on testnet4: they accept the
/// connection but never reply, leaving the slot — and the arena entry
/// holding our queued Version send — pinned indefinitely.
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;

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
const HEADERS_PHASE_DEADLINE_SECS: u64 = 120;

/// Target number of healthy peers to keep dialed at any one time. The
/// fetcher will dial up to this many from its pool, replacing dead slots
/// from the rest of the pool. With `PEER_INFLIGHT_CAP=2`, 32 peers gives
/// us ~64 hashes in flight network-wide — wide enough that one slow peer
/// barely dents progress, narrow enough to fit in a single ring's
/// SQE/CQE budget without backpressure.
const TARGET_LIVE_PEERS: usize = 64;

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
    fd: Option<RawFd>,
    /// Owned sockaddr bytes the kernel reads for the connect SQE; must
    /// outlive the SQE *and* sit at a stable memory address. Boxed so
    /// the address survives the `Peer` being moved into the `peers`
    /// `HashMap` after the SQE is queued — without `Box`, the kernel
    /// reads stale stack memory at submission time and rejects the
    /// connect with `-EAFNOSUPPORT`.
    sockaddr: Box<SockAddrIn>,
    state: PeerState,
    /// Bytes accumulated from recv CQEs, awaiting frame parsing. Each
    /// multishot-recv arrival hands us a kernel-owned buffer; we copy
    /// its contents in here, recycle the buffer, then parse.
    recv_buf: Vec<u8>,
    /// `true` while a multishot recv SQE is armed against this peer's
    /// fd. Cleared when the kernel terminates the multishot (CQE
    /// without `MORE`) and re-armed in the per-iteration sweep at the
    /// bottom of the session loop.
    recv_armed: bool,
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
    /// Arena slot indices for SENDs we've issued but whose CQEs haven't
    /// arrived yet. Bounded by `PEER_SEND_OUTSTANDING_CAP`.
    ///
    /// Tracking the actual slot indices (not just a count) lets
    /// `close_peer` proactively release them: with
    /// `SendRecvFlag::PollFirst` the kernel only posts the SEND CQE once
    /// the socket becomes writable, so a backpressured peer's slots
    /// would otherwise be stuck until eventual `-ECANCELED` (after fd
    /// close) drains them — and the arena can fill up before that
    /// happens if peers churn faster than CQEs arrive.
    pending_send_seqs: Vec<u16>,
}

/// Maximum SENDs a single peer may have in-flight at once. Crossing this
/// means the peer is not draining its recv buffer (TCP backpressure) —
/// the kernel won't post our SEND CQEs until it does, so further SENDs
/// just leak arena slots. Tuned to leave headroom for the handshake's
/// version+verack burst plus a couple of pongs.
const PEER_SEND_OUTSTANDING_CAP: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PeerState {
    Connecting,
    HandshakeSendVersion,
    HandshakeRecving,
    Ready,
    WaitingHeaders,
    Dead,
}

/// Single-allocation, slot-indexed staging area for outbound SQE
/// payloads.
///
/// The kernel reads the bytes pointed to by `Sqe::send` when it
/// processes the SQE during `submit_and_wait`. The buffer must stay
/// live and at a stable address until the matching SEND CQE arrives.
/// We satisfy that by partitioning a single `Box<[u8]>` into
/// fixed-size slots once at session start; framing happens in-place
/// via `Magic::frame(cmd, payload, &mut Cursor<&mut [u8]>)`, and the
/// slot index rides in the upper 16 bits of `user_data` so the SEND
/// CQE handler can free the slot.
///
/// No per-frame allocation. The free-list and the slab are both
/// owned by the arena; both have fixed capacity and never resize.
struct SendArena {
    slab: Box<[u8]>,
    /// LIFO of free slot indices. Pre-populated `[0..SEND_SLOT_COUNT)`.
    free: Vec<u16>,
    /// Length of the framed bytes in each currently-allocated slot,
    /// indexed by slot id. 0 means the slot is free.
    used_len: Box<[u32]>,
    /// Debug: cumulative successful allocations (`frame_into` -> Some).
    dbg_alloc: u64,
    /// Debug: cumulative releases (matched `OP_SEND` CQEs).
    dbg_release: u64,
}

impl SendArena {
    fn new() -> Self {
        let slot_count_usize = SEND_SLOT_COUNT as usize;
        let slab = vec![0u8; slot_count_usize * SEND_SLOT_SIZE].into_boxed_slice();
        let used_len = vec![0u32; slot_count_usize].into_boxed_slice();
        // Push indices in descending order so popping the back yields
        // ascending slots — easier to reason about in logs.
        let mut free = Vec::with_capacity(slot_count_usize);
        for i in (0..SEND_SLOT_COUNT).rev() {
            free.push(i);
        }
        Self {
            slab,
            free,
            used_len,
            dbg_alloc: 0,
            dbg_release: 0,
        }
    }

    /// Frame `(cmd, payload)` into a freshly-claimed slot. Returns the
    /// slot index on success; `None` if the arena is empty or the
    /// framed bytes wouldn't fit. Caller borrows the bytes via
    /// [`SendArena::slice`] for SQE submission.
    fn frame_into(
        &mut self,
        magic: mantle::network::Magic,
        cmd: Command,
        payload: &[u8],
    ) -> Option<u16> {
        let slot = self.free.pop()?;
        let off = (slot as usize) * SEND_SLOT_SIZE;
        let dst = &mut self.slab[off..off + SEND_SLOT_SIZE];
        let mut cursor = std::io::Cursor::new(dst);
        if let Err(e) = magic.frame(cmd, payload, &mut cursor) {
            // The only realistic cause is payload-size-too-big for the
            // slot. Surface so it doesn't masquerade as a generic
            // `arena exhausted` later.
            eprintln!(
                "  iouring-fetcher: frame_into({cmd:?}, {} bytes) failed: {e:?} \
                 (slot {slot} reclaimed)",
                payload.len()
            );
            self.free.push(slot);
            return None;
        }
        let written = u32::try_from(cursor.position()).expect("frame fits u32");
        self.used_len[slot as usize] = written;
        self.dbg_alloc += 1;
        Some(slot)
    }

    fn slice(&self, slot: u16) -> &[u8] {
        let off = (slot as usize) * SEND_SLOT_SIZE;
        let len = self.used_len[slot as usize] as usize;
        &self.slab[off..off + len]
    }

    fn release(&mut self, slot: u16) {
        if (slot as usize) >= self.used_len.len() {
            return;
        }
        if self.used_len[slot as usize] == 0 {
            return; // already released; defensive against double-release
        }
        self.used_len[slot as usize] = 0;
        self.free.push(slot);
        self.dbg_release += 1;
    }
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

// ── File I/O thread ─────────────────────────────────────────────────────────

enum FileReq {
    WriteBlock { height: BlockHeight, data: Vec<u8> },
    WriteBlockAsync { height: BlockHeight, data: Vec<u8> },
    ReadBlock { height: BlockHeight },
    HasBlock { height: BlockHeight },
    WriteHeaders { start: BlockHeight, data: Vec<u8> },
    LoadCachedHeaders { start: BlockHeight },
    FlushBlockIndex,
    Shutdown,
}

enum FileResp {
    Ok,
    Bool(bool),
    BlockData(Vec<u8>),
    Headers(Vec<(BlockHeight, BlockHeader)>),
    Err(io::Error),
}

const FILE_RING_CAP: usize = 4096;

struct FileIoHandle {
    tx: quetzalcoatl::mpsc::Producer<FileReq>,
    rx: quetzalcoatl::spsc::Consumer<FileResp>,
}

impl FileIoHandle {
    fn spawn(data_dir: &Path) -> io::Result<Self> {
        let blocks = BlockStore::open(data_dir)?;
        let headers = HeaderStore::open(data_dir)?;
        let data_dir = data_dir.to_owned();

        let req_ring = quetzalcoatl::mpsc::RingBuffer::new(
            quetzalcoatl::capacity::Capacity::exact(FILE_RING_CAP),
        );
        let resp_ring = quetzalcoatl::spsc::RingBuffer::new(
            quetzalcoatl::capacity::Capacity::exact(FILE_RING_CAP),
        );
        let (req_tx, req_rx) = req_ring.split();
        let (resp_tx, resp_rx) = resp_ring.split();

        std::thread::Builder::new()
            .name("file-io".into())
            .spawn(move || {
                file_io_worker(req_rx, resp_tx, blocks, headers, &data_dir);
            })
            .map_err(|e| io::Error::other(format!("spawn file-io: {e}")))?;

        Ok(Self { tx: req_tx, rx: resp_rx })
    }

    fn write_block(&mut self, height: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx.push(FileReq::WriteBlock { height, data })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn write_block_async(&self, height: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx.push(FileReq::WriteBlockAsync { height, data })
            .map_err(|_| io::Error::other("file-io ring full"))
    }

    fn read_block(&mut self, height: BlockHeight) -> io::Result<Vec<u8>> {
        self.tx.push(FileReq::ReadBlock { height })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::BlockData(data)) => Ok(data),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn write_headers(&mut self, start: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx.push(FileReq::WriteHeaders { start, data })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn flush_block_index(&mut self) -> io::Result<()> {
        self.tx.push(FileReq::FlushBlockIndex)
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn has_block(&mut self, height: BlockHeight) -> bool {
        if self.tx.push(FileReq::HasBlock { height }).is_err() {
            return false;
        }
        matches!(self.rx.pop_block(), Some(FileResp::Bool(true)))
    }

    fn load_cached_headers(
        &mut self,
        start: BlockHeight,
    ) -> Result<Vec<(BlockHeight, BlockHeader)>, FetchError> {
        self.tx.push(FileReq::LoadCachedHeaders { start })
            .map_err(|_| FetchError::UringErr("file-io ring full".into()))?;
        match self.rx.pop_block() {
            Some(FileResp::Headers(h)) => Ok(h),
            Some(FileResp::Err(e)) => Err(FetchError::Io(e)),
            _ => Ok(Vec::new()),
        }
    }

    fn shutdown(&mut self) {
        let _ = self.tx.push(FileReq::Shutdown);
        let _ = self.rx.pop_block();
    }
}

const BLOCK_INDEX_FLUSH_INTERVAL: u64 = 2_000;

fn file_io_worker(
    mut rx: quetzalcoatl::mpsc::Consumer<FileReq>,
    tx: quetzalcoatl::spsc::Producer<FileResp>,
    mut blocks: BlockStore,
    mut headers: HeaderStore,
    data_dir: &Path,
) {
    let mut ring = match IoUring::new(256) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("file-io: failed to create ring: {e:?}");
            return;
        }
    };

    let mut writes_since_flush: u64 = 0;

    while let Some(req) = rx.pop_block() {
        let resp = match req {
            FileReq::WriteBlock { height, data } => {
                match blocks.write(&mut ring, height, &data) {
                    Ok(()) => {
                        writes_since_flush += 1;
                        if writes_since_flush >= BLOCK_INDEX_FLUSH_INTERVAL {
                            let _ = blocks.flush_index(data_dir);
                            writes_since_flush = 0;
                        }
                        FileResp::Ok
                    }
                    Err(e) => FileResp::Err(e),
                }
            }
            FileReq::WriteBlockAsync { height, data } => {
                let _ = blocks.write(&mut ring, height, &data);
                writes_since_flush += 1;
                if writes_since_flush >= BLOCK_INDEX_FLUSH_INTERVAL {
                    let _ = blocks.flush_index(data_dir);
                    writes_since_flush = 0;
                }
                continue;
            }
            FileReq::ReadBlock { height } => {
                match blocks.read(&mut ring, height) {
                    Ok(data) => FileResp::BlockData(data),
                    Err(e) => FileResp::Err(e),
                }
            }
            FileReq::HasBlock { height } => {
                FileResp::Bool(blocks.has_block(height))
            }
            FileReq::WriteHeaders { start, data } => {
                match headers.write_raw(&mut ring, start, &data) {
                    Ok(()) => FileResp::Ok,
                    Err(e) => FileResp::Err(e),
                }
            }
            FileReq::LoadCachedHeaders { start } => {
                match headers.load_cached(&mut ring, start) {
                    Ok(h) => FileResp::Headers(h),
                    Err(e) => FileResp::Err(io::Error::other(format!("{e}"))),
                }
            }
            FileReq::FlushBlockIndex => {
                match blocks.flush_index(data_dir) {
                    Ok(()) => FileResp::Ok,
                    Err(e) => FileResp::Err(e),
                }
            }
            FileReq::Shutdown => {
                let _ = tx.push(FileResp::Ok);
                return;
            }
        };
        let _ = tx.push(resp);
    }
}

// ── File-backed store ────────────────────────────────────────────────────────

struct FileStore {
    #[allow(dead_code)]
    file: fs::File,
    fd: RawFd,
    write_offset: u64,
}

impl FileStore {
    fn open(path: &Path) -> io::Result<Self> {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        let write_offset = file.metadata()?.len();
        let fd = file_to_rawfd(&file);
        Ok(Self { file, fd, write_offset })
    }

    #[allow(clippy::cast_possible_truncation)]
    fn uring_read(&self, ring: &mut IoUring, offset: u64, len: u32) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len as usize];
        let sqe = unsafe {
            Sqe::read_ptr(self.fd, buf.as_mut_ptr(), len, offset)
        }.user_data(u64::MAX);
        ring.push(sqe).map_err(|e| io::Error::other(format!("{e:?}")))?;
        loop {
            ring.submit_and_wait(1).map_err(|e| io::Error::other(format!("{e:?}")))?;
            while let Some(cqe) = ring.complete() {
                if cqe.user_data == u64::MAX {
                    if cqe.result < 0 {
                        return Err(io::Error::from_raw_os_error(-cqe.result));
                    }
                    return Ok(buf);
                }
            }
        }
    }

    #[allow(clippy::cast_possible_truncation)]
    fn uring_write(&self, ring: &mut IoUring, data: &[u8], offset: u64) -> io::Result<()> {
        let sqe = unsafe {
            Sqe::write_ptr(self.fd, data.as_ptr(), data.len() as u32, offset)
        }.user_data(u64::MAX);
        ring.push(sqe).map_err(|e| io::Error::other(format!("{e:?}")))?;
        loop {
            ring.submit_and_wait(1).map_err(|e| io::Error::other(format!("{e:?}")))?;
            while let Some(cqe) = ring.complete() {
                if cqe.user_data == u64::MAX {
                    if cqe.result < 0 {
                        return Err(io::Error::from_raw_os_error(-cqe.result));
                    }
                    return Ok(());
                }
            }
        }
    }

    fn append(&mut self, ring: &mut IoUring, data: &[u8]) -> io::Result<u64> {
        let offset = self.write_offset;
        self.uring_write(ring, data, offset)?;
        self.write_offset += data.len() as u64;
        Ok(offset)
    }

    fn truncate_and_write(&mut self, ring: &mut IoUring, data: &[u8]) -> io::Result<()> {
        self.file.set_len(0)?;
        self.write_offset = 0;
        self.append(ring, data)?;
        Ok(())
    }
}

// ── Block store ─────────────────────────────────────────────────────────────

const BLOCK_INDEX_ENTRY_SIZE: usize = 12;

#[derive(Clone, Copy, Default)]
struct BlockEntry {
    offset: u64,
    length: u32,
}

impl BlockEntry {
    fn is_present(self) -> bool {
        self.length > 0
    }
}

struct BlockStore {
    store: FileStore,
    index: Vec<BlockEntry>,
}

impl BlockStore {
    fn open(data_dir: &Path) -> io::Result<Self> {
        let store = FileStore::open(&data_dir.join("blocks.bin"))?;
        let index = load_blocks_index(&data_dir.join("blocks.idx"))?;
        Ok(Self { store, index })
    }

    fn has_block(&self, height: BlockHeight) -> bool {
        self.index
            .get(height.0 as usize)
            .copied()
            .unwrap_or_default()
            .is_present()
    }

    fn read(&self, ring: &mut IoUring, height: BlockHeight) -> io::Result<Vec<u8>> {
        let entry = self.index
            .get(height.0 as usize)
            .copied()
            .unwrap_or_default();
        if !entry.is_present() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("block {} not in index", height.0),
            ));
        }
        self.store.uring_read(ring, entry.offset, entry.length)
    }

    #[allow(clippy::cast_possible_truncation)]
    #[allow(dead_code)]
    fn write(&mut self, ring: &mut IoUring, height: BlockHeight, data: &[u8]) -> io::Result<()> {
        let offset = self.store.append(ring, data)?;
        let h = height.0 as usize;
        if h >= self.index.len() {
            self.index.resize(h + 1, BlockEntry::default());
        }
        self.index[h] = BlockEntry {
            offset,
            length: data.len() as u32,
        };
        Ok(())
    }

    fn flush_index(&self, data_dir: &Path) -> io::Result<()> {
        save_blocks_index(&data_dir.join("blocks.idx"), &self.index)
    }
}

// ── Header store ────────────────────────────────────────────────────────────

const HEADER_RECORD_SIZE: usize = 80;

struct HeaderStore {
    store: FileStore,
    metadata: FileStore,
}

impl HeaderStore {
    fn open(data_dir: &Path) -> io::Result<Self> {
        let store = FileStore::open(&data_dir.join("chain_headers.bin"))?;
        let metadata = FileStore::open(&data_dir.join("metadata.bin"))?;
        Ok(Self { store, metadata })
    }

    #[allow(clippy::cast_possible_truncation)]
    fn load_cached(
        &self,
        ring: &mut IoUring,
        start: BlockHeight,
    ) -> Result<Vec<(BlockHeight, BlockHeader)>, FetchError> {
        if self.metadata.write_offset < 4 || self.store.write_offset == 0 {
            return Ok(Vec::new());
        }
        let meta_bytes = self.metadata.uring_read(ring, 0, 4)?;
        let cached_start = u32::from_le_bytes(meta_bytes[..4].try_into().unwrap());
        if cached_start != start.0 {
            return Ok(Vec::new());
        }
        let count = self.store.write_offset as usize / HEADER_RECORD_SIZE;
        if count == 0 {
            return Ok(Vec::new());
        }
        #[allow(clippy::cast_possible_truncation)]
        let data = self.store.uring_read(ring, 0, self.store.write_offset as u32)?;
        let mut headers = Vec::with_capacity(count);
        for (i, chunk) in data.chunks_exact(HEADER_RECORD_SIZE).enumerate() {
            let arr: &[u8; 80] = chunk.try_into().unwrap();
            let hdr = BlockHeader::from_bytes(arr).map_err(|e| {
                FetchError::UringErr(format!("cached header {i} decode: {e}"))
            })?;
            headers.push((BlockHeight(start.0 + i as u32), hdr));
        }
        Ok(headers)
    }

    fn write_raw(
        &mut self,
        ring: &mut IoUring,
        start: BlockHeight,
        data: &[u8],
    ) -> io::Result<()> {
        self.store.truncate_and_write(ring, data)?;
        self.metadata.truncate_and_write(ring, &start.0.to_le_bytes())
    }

    fn write_all(
        &mut self,
        ring: &mut IoUring,
        start: BlockHeight,
        headers: &[(BlockHeight, BlockHeader)],
    ) -> io::Result<()> {
        let mut sorted: Vec<&(BlockHeight, BlockHeader)> = headers.iter().collect();
        sorted.sort_by_key(|(h, _)| h.0);
        let mut out = Vec::with_capacity(sorted.len() * HEADER_RECORD_SIZE);
        for (_, hdr) in &sorted {
            out.extend_from_slice(&hdr.to_bytes());
        }
        self.store.truncate_and_write(ring, &out)?;
        self.metadata.truncate_and_write(ring, &start.0.to_le_bytes())
    }
}

// ── Peer pool ───────────────────────────────────────────────────────────────

const MAX_PEERS: usize = TARGET_LIVE_PEERS;

struct PeerPool {
    slots: [Option<Peer>; MAX_PEERS],
}

#[allow(clippy::cast_possible_truncation)]
impl PeerPool {
    fn new() -> Self {
        Self { slots: std::array::from_fn(|_| None) }
    }

    fn get_mut(&mut self, slot: u16) -> Option<&mut Peer> {
        self.slots.get_mut(slot as usize)?.as_mut()
    }

    fn insert(&mut self, slot: u16, peer: Peer) {
        self.slots[slot as usize] = Some(peer);
    }

    fn remove(&mut self, slot: u16) -> Option<Peer> {
        self.slots.get_mut(slot as usize)?.take()
    }

    fn is_empty(&self) -> bool {
        self.slots.iter().all(Option::is_none)
    }

    fn live_count(&self) -> usize {
        self.slots.iter().filter(|s| {
            s.as_ref().is_some_and(|p| p.state != PeerState::Dead)
        }).count()
    }

    fn free_slot(&self) -> Option<u16> {
        self.slots.iter().position(Option::is_none).map(|i| i as u16)
    }

    fn iter_mut(&mut self) -> impl Iterator<Item = (u16, &mut Peer)> {
        self.slots.iter_mut().enumerate().filter_map(|(i, s)| {
            s.as_mut().map(|p| (i as u16, p))
        })
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut Peer> {
        self.slots.iter_mut().filter_map(|s| s.as_mut())
    }

    fn values(&self) -> impl Iterator<Item = &Peer> {
        self.slots.iter().filter_map(|s| s.as_ref())
    }

    fn dead_slots(&self) -> Vec<u16> {
        self.slots.iter().enumerate().filter_map(|(i, s)| {
            s.as_ref().and_then(|p| (p.state == PeerState::Dead).then_some(i as u16))
        }).collect()
    }
}

// ── Fetcher ─────────────────────────────────────────────────────────────────

pub struct Fetcher {
    network: Network,
    addr_pool: Vec<SocketAddrV4>,
    next_addr_idx: usize,
    data_dir: PathBuf,
    connect_timeout: Timespec,
    ring: IoUring,
    pbr: ququmatz::ProvidedBufferRing,
    send_arena: SendArena,
    peers: PeerPool,
    file_io: FileIoHandle,
}

impl Fetcher {
    /// # Errors
    /// Returns `io::Error` if the on-disk blocks dir cannot be created or
    /// the `io_uring` ring cannot be set up.
    pub fn new(
        network: Network,
        peer_addrs: Vec<SocketAddrV4>,
        data_dir: PathBuf,
    ) -> io::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let mut ring = make_ring().map_err(|e| io::Error::other(format!("{e}")))?;
        let pbr = ring
            .register_provided_buffers(PROVIDED_BUF_GROUP_ID, PROVIDED_BUF_COUNT, PROVIDED_BUF_SIZE)
            .map_err(|e| io::Error::other(format!("register_provided_buffers: {e:?}")))?;

        let file_io = FileIoHandle::spawn(&data_dir)?;

        Ok(Self {
            network,
            addr_pool: peer_addrs,
            next_addr_idx: 0,
            data_dir,
            connect_timeout: Timespec::from_millis(CONNECT_TIMEOUT_SECS * 1_000),
            ring,
            pbr,
            send_arena: SendArena::new(),
            peers: PeerPool::new(),
            file_io,
        })
    }

    fn ensure_peers(&mut self) -> Result<(), FetchError> {
        let live = self.peers.live_count();
        let target = TARGET_LIVE_PEERS.min(self.addr_pool.len());
        for _ in live..target {
            let Some(slot) = self.peers.free_slot() else { break };
            if let Some(peer) = self.dial_next_inner(slot)? {
                self.peers.insert(slot, peer);
            }
        }
        Ok(())
    }

    fn dial_next_inner(&mut self, slot: u16) -> Result<Option<Peer>, FetchError> {
        self.dial_next_avoiding_inner(slot, &[])
    }

    fn dial_next_avoiding_inner(
        &mut self,
        slot: u16,
        active: &[SocketAddrV4],
    ) -> Result<Option<Peer>, FetchError> {
        let Some(addr) = self.next_pool_addr(active) else {
            return Ok(None);
        };
        let sock = ququmatz::Socket::with_typed_flags(
            AddressFamily::Inet,
            SocketType::Stream,
            0,
            SocketFlags::NONBLOCK,
        )
        .map_err(|e| FetchError::UringErr(format!("socket: {e:?}")))?;
        let fd = sock.into_fd();
        let sockaddr = Box::new(SockAddrIn {
            sin_family: u16::try_from(AddressFamily::Inet.as_raw()).expect("AF_INET fits u16"),
            sin_port: addr.port().to_be(),
            sin_addr: u32::from_ne_bytes(addr.ip().octets()),
            sin_zero: [0; 8],
        });

        let peer = Peer {
            addr,
            fd: Some(fd),
            sockaddr,
            state: PeerState::Connecting,
            recv_buf: Vec::with_capacity(RECV_BUF_CAPACITY),
            recv_armed: false,
            in_flight: Vec::new(),
            connected_at: Instant::now(),
            generation: 0,
            assigned_count: 0,
            delivered_count: 0,
            last_delivery: Instant::now(),
            pending_send_seqs: Vec::with_capacity(PEER_SEND_OUTSTANDING_CAP as usize),
        };

        push_connect(
            &mut self.ring,
            slot,
            peer.generation,
            fd,
            &peer.sockaddr,
            &self.connect_timeout,
        )?;

        Ok(Some(peer))
    }

    pub fn read_block_bytes(&mut self, height: BlockHeight) -> io::Result<Vec<u8>> {
        self.file_io.read_block(height)
    }


    pub fn sync_headers(
        &mut self,
        start: BlockHeight,
        end: BlockHeight,
        locator: Hash256,
    ) -> Result<Vec<(BlockHeight, BlockHeader)>, FetchError> {
        if self.addr_pool.is_empty() {
            return Err(FetchError::NoPeers);
        }

        let mut collected = self.file_io.load_cached_headers(start)?;
        if !collected.is_empty() {
            let cached_end = collected.last().unwrap().0;
            eprintln!(
                "  headers: loaded {} cached ({}..={})",
                collected.len(), start.0, cached_end.0,
            );
            if cached_end >= end {
                return Ok(collected);
            }
        }

        let mut next_height = if collected.is_empty() {
            start.0
        } else {
            collected.last().unwrap().0 .0 + 1
        };
        let mut current_locator = if collected.is_empty() {
            locator
        } else {
            collected.last().unwrap().1.hash()
        };

        loop {
            let page = self.run_getheaders_session(current_locator)?;
            if page.is_empty() {
                eprintln!("  headers: peer returned empty page at height {next_height} — at tip");
                break;
            }
            let page_len = page.len();
            for hdr in page {
                let h = BlockHeight(next_height);
                if h > end {
                    break;
                }
                collected.push((h, hdr));
                next_height = next_height.wrapping_add(1);
            }
            eprintln!(
                "  headers: got {page_len} in page, {} total (height {}..={})",
                collected.len(),
                start.0,
                next_height.saturating_sub(1),
            );
            if collected.last().is_some_and(|(h, _)| *h >= end) {
                break;
            }
            let Some(last) = collected.last() else { break };
            current_locator = last.1.hash();
        }

        {
            let mut sorted: Vec<&(BlockHeight, BlockHeader)> = collected.iter().collect();
            sorted.sort_by_key(|(h, _)| h.0);
            let mut out = Vec::with_capacity(sorted.len() * 80);
            for (_, hdr) in &sorted {
                out.extend_from_slice(&hdr.to_bytes());
            }
            self.file_io.write_headers(start, out)?;
        }
        Ok(collected)
    }

    /// Download every block in `headers` that isn't already on disk.
    pub fn fetch_blocks(
        &mut self,
        headers: &[(BlockHeight, BlockHeader)],
    ) -> Result<(), FetchError> {
        if self.addr_pool.is_empty() {
            return Err(FetchError::NoPeers);
        }
        let needed: Vec<(BlockHeight, Hash256)> = headers
            .iter()
            .filter(|(h, _)| {
                !self.file_io.has_block(*h)
            })
            .map(|(h, hdr)| (*h, hdr.hash()))
            .collect();
        if needed.is_empty() {
            return Ok(());
        }
        self.run_block_download_session(&needed)?;
        self.file_io.flush_block_index()?;
        Ok(())
    }

    // ── headers session ──────────────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn run_getheaders_session(&mut self, locator: Hash256) -> Result<Vec<BlockHeader>, FetchError> {
        self.ensure_peers()?;
        if self.peers.is_empty() {
            return Err(FetchError::PoolExhausted);
        }
        eprintln!(
            "  headers: connecting to {} peers…",
            self.peers.live_count(),
        );
        for (slot, peer) in self.peers.iter_mut() {
            if !peer.recv_armed
                && peer.state != PeerState::Dead
                && peer.state != PeerState::Connecting
                && peer.fd.is_some()
            {
                push_recv_multishot(&mut self.ring, slot, peer.generation, peer.fd.ok_or(FetchError::NoPeers)?)?;
                peer.recv_armed = true;
            }
        }

        let phase_deadline = Instant::now() + Duration::from_secs(HEADERS_PHASE_DEADLINE_SECS);
        let payload = getheaders_msg(locator).encode_to_vec();
        let mut elected: Option<(u16, Instant)> = None;
        let dbg_arena = std::env::var_os("MANTLE_URING_DEBUG").is_some_and(|v| v == "1");
        let mut dbg_iter: u64 = 0;
        let mut last_status = Instant::now();

        loop {
            // Submit any pending SQEs and wait for at least one CQE, but
            // also poll a 1s tick so timeouts can fire even if no peer
            // produces a CQE on its own.
            push_tick(&mut self.ring, Duration::from_secs(1))?;
            let dbg_submitted = self.ring.submit_and_wait(1).map_err(uring_err)?;
            let dbg_alloc_pre = self.send_arena.dbg_alloc;
            let dbg_release_pre = self.send_arena.dbg_release;

            // Drain CQEs, capped so a recv torrent can't starve SEND CQEs.
            let mut drained = 0usize;
            let mut dbg_op_hist = [0u32; 6]; // CONNECT, LINK_TIMEOUT, SEND, RECV, CLOSE, OTHER
            let mut dbg_send_res_min: i32 = i32::MAX;
            let mut dbg_send_res_max: i32 = i32::MIN;
            while drained < CQE_DRAIN_CAP {
                let Some(cqe) = self.ring.complete() else { break };
                drained += 1;
                let (slot, op, g) = decode_user_data(cqe.user_data);
                let dbg_idx = match op {
                    OP_CONNECT => 0,
                    OP_LINK_TIMEOUT => 1,
                    OP_SEND => 2,
                    OP_RECV => 3,
                    OP_CLOSE => 4,
                    _ => 5,
                };
                dbg_op_hist[dbg_idx] += 1;
                if op == OP_TICK {
                    continue;
                }
                if op == OP_SEND {
                    if cqe.result < dbg_send_res_min {
                        dbg_send_res_min = cqe.result;
                    }
                    if cqe.result > dbg_send_res_max {
                        dbg_send_res_max = cqe.result;
                    }
                    let send_seq = decode_send_seq(cqe.user_data);
                    self.send_arena.release(send_seq);
                    if let Some(p) = self.peers.get_mut(slot) {
                        p.pending_send_seqs.retain(|s| *s != send_seq);
                    }
                }
                let res = cqe.result;
                let Some(peer) = self.peers.get_mut(slot) else {
                    continue;
                };
                if peer.generation != g {
                    continue; // stale CQE for a recycled slot
                }
                handle_getheaders_cqe(
                    &mut self.ring,
                    &mut self.pbr,
                    &mut self.send_arena,
                    self.network,
                    slot,
                    peer,
                    op,
                    res,
                    cqe.flags,
                )?;
            }

            // After draining CQEs, walk every peer's recv_buf once to
            // parse complete frames. Per-iteration parsing (not
            // per-CQE) bounds outbound sends — otherwise a multishot
            // recv torrent generates dozens of pong/verack sends per
            // CQE drain pass and exhausts the send arena before SEND
            // CQEs surface.
            for (slot, peer) in self.peers.iter_mut() {
                if peer.state == PeerState::Dead {
                    continue;
                }
                if let Some(headers) =
                    consume_handshake_frames(&mut self.ring, &mut self.send_arena, self.network, slot, peer)?
                {
                    return Ok(headers);
                }
            }

            if dbg_arena {
                let alloc_iter = self.send_arena.dbg_alloc - dbg_alloc_pre;
                let release_iter = self.send_arena.dbg_release - dbg_release_pre;
                let in_flight = self.send_arena.dbg_alloc - self.send_arena.dbg_release;
                let free = self.send_arena.free.len();
                // Only log meaningful iterations: any send activity, or
                // arena pressure (in_flight > 0). Skip the millions of
                // pure dial-redial iterations that produce no signal.
                let interesting =
                    alloc_iter > 0 || release_iter > 0 || in_flight > 0 || dbg_iter.is_multiple_of(1000);
                if interesting {
                    let overflow = self.ring.cq_overflow();
                    let send_res_str = if dbg_send_res_min == i32::MAX {
                        String::from("-")
                    } else {
                        format!("{dbg_send_res_min}..{dbg_send_res_max}")
                    };
                    let states: Vec<String> = self.peers
                        .iter_mut()
                        .map(|(s, p)| {
                            format!(
                                "{s}:{:?}/snd={}/buf={}/armed={}",
                                p.state,
                                p.pending_send_seqs.len(),
                                p.recv_buf.len(),
                                p.recv_armed,
                            )
                        })
                        .collect();
                    eprintln!(
                        "  arena[h iter={dbg_iter}]: submitted={dbg_submitted} drained={drained} \
                         ovfl={overflow} CQE[con={c} lto={l} snd={s} rcv={r} cls={cl} oth={o}] \
                         send_res={send_res_str} alloc+={alloc_iter} release+={release_iter} \
                         in_flight={in_flight} free={free} peers=[{states_joined}]",
                        c = dbg_op_hist[0],
                        l = dbg_op_hist[1],
                        s = dbg_op_hist[2],
                        r = dbg_op_hist[3],
                        cl = dbg_op_hist[4],
                        o = dbg_op_hist[5],
                        states_joined = states.join(","),
                    );
                }
                dbg_iter += 1;
            }

            // Timeouts and election.
            let now = Instant::now();
            if now > phase_deadline {
                return Err(FetchError::Timeout("headers"));
            }
            if now.duration_since(last_status) >= Duration::from_secs(5) {
                let ready = self.peers.slots.iter().filter(|s| s.as_ref().is_some_and(|p| p.state == PeerState::Ready)).count();
                let connecting = self.peers.slots.iter().filter(|s| s.as_ref().is_some_and(|p| p.state == PeerState::Connecting)).count();
                let handshaking = self.peers.slots.iter().filter(|s| s.as_ref().is_some_and(|p| matches!(p.state, PeerState::HandshakeSendVersion | PeerState::HandshakeRecving))).count();
                let dead = self.peers.slots.iter().filter(|s| s.as_ref().is_some_and(|p| p.state == PeerState::Dead)).count();
                let waiting = self.peers.slots.iter().filter(|s| s.as_ref().is_some_and(|p| p.state == PeerState::WaitingHeaders)).count();
                eprintln!(
                    "  headers: ready={ready} connecting={connecting} handshaking={handshaking} waiting={waiting} dead={dead} elected={:?}",
                    elected.map(|(s,_)| s),
                );
                last_status = now;
            }

            // Drop dead-on-arrival or response-timed-out peers.
            for (slot, peer) in self.peers.iter_mut() {
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
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                }
                // Handshake-stall: connect succeeded but peer never sent
                // their Version. Without this, silent peers occupy slots
                // (and arena entries for our queued Version) indefinitely.
                if matches!(
                    peer.state,
                    PeerState::HandshakeSendVersion | PeerState::HandshakeRecving
                ) && now.duration_since(peer.connected_at)
                    > Duration::from_secs(HANDSHAKE_TIMEOUT_SECS)
                {
                    eprintln!(
                        "  iouring-fetcher: peer {} ({}) handshake-timeout",
                        slot, peer.addr
                    );
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                }
            }
            if let Some((slot, sent)) = elected
                && now.duration_since(sent) > Duration::from_secs(HEADERS_RESPONSE_TIMEOUT_SECS)
            {
                eprintln!("  iouring-fetcher: elected peer {slot} headers-timeout, demoting");
                if let Some(peer) = self.peers.get_mut(slot) {
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
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
            for peer in self.peers.values_mut() {
                if peer.state == PeerState::Ready && peer.recv_buf.iter().any(|_| false) {
                    // Placeholder — actual parse happens in handle path.
                }
            }

            for slot in self.peers.dead_slots() {
                self.peers.remove(slot);
                if let Some(peer) = self.dial_next_inner(slot)? {
                    self.peers.insert(slot, peer);
                }
            }
            if self.peers.live_count() == 0 {
                eprintln!("  iouring-fetcher: all header peers dead, recycling pool in 5s…");
                std::thread::sleep(Duration::from_secs(5));
                self.next_addr_idx = 0;
                self.ensure_peers()?;
                if self.peers.is_empty() {
                    return Err(FetchError::PoolExhausted);
                }
            }

            // Election: any Ready peer with no current pick → send getheaders.
            if elected.is_none()
                && let Some((slot, peer)) =
                    self.peers.iter_mut().find(|(_, p)| p.state == PeerState::Ready)
            {
                let seq = push_send(
                    &mut self.ring,
                    &mut self.send_arena,
                    self.network,
                    Command::GetHeaders,
                    &payload,
                    slot,
                    peer.generation,
                    peer.fd.ok_or(FetchError::NoPeers)?,
                )?;
                peer.pending_send_seqs.push(seq);
                peer.state = PeerState::WaitingHeaders;
                elected = Some((slot, Instant::now()));
            }

            // Re-arm any peers whose multishot recv terminated this
            // pass. Doing this here rather than inline in the recv
            // handler keeps the SQE ring's pending-work mix balanced
            // across submit_and_wait cycles.
            for (slot, peer) in self.peers.iter_mut() {
                if !peer.recv_armed
                    && peer.state != PeerState::Dead
                    && peer.state != PeerState::Connecting
                    && peer.fd.is_some()
                {
                    push_recv_multishot(&mut self.ring, slot, peer.generation, peer.fd.ok_or(FetchError::NoPeers)?)?;
                    peer.recv_armed = true;
                }
            }

            // Publish recycled buffers + arena releases before the
            // next submit_and_wait. Both must be visible to the
            // kernel for the multishot recv to keep firing.
            self.pbr.commit();
        }
    }

    // ── block download session ──────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn run_block_download_session(
        &mut self,
        needed: &[(BlockHeight, Hash256)],
    ) -> Result<(), FetchError> {
        self.ensure_peers()?;
        if self.peers.is_empty() {
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
        let dbg_arena = std::env::var_os("MANTLE_URING_DEBUG").is_some_and(|v| v == "1");
        let mut dbg_iter: u64 = 0;

        loop {
            push_tick(&mut self.ring, Duration::from_secs(1))?;
            self.ring.submit_and_wait(1).map_err(uring_err)?;
            let dbg_alloc_pre = self.send_arena.dbg_alloc;
            let dbg_release_pre = self.send_arena.dbg_release;

            let mut got_block_hashes: Vec<Hash256> = Vec::new();
            let mut drained = 0usize;
            while drained < CQE_DRAIN_CAP {
                let Some(cqe) = self.ring.complete() else { break };
                drained += 1;
                let (slot, op, g) = decode_user_data(cqe.user_data);
                if op == OP_TICK {
                    continue;
                }
                if op == OP_SEND {
                    self.send_arena.release(decode_send_seq(cqe.user_data));
                }
                let Some(peer) = self.peers.get_mut(slot) else {
                    continue;
                };
                if peer.generation != g {
                    continue;
                }
                handle_block_cqe(
                    &mut self.ring,
                    &mut self.pbr,
                    &mut self.send_arena,
                    self.network,
                    slot,
                    peer,
                    op,
                    cqe.result,
                    cqe.flags,
                )?;
            }

            // After draining CQEs, walk every peer's recv_buf once to
            // parse complete frames. Per-iteration parsing (not
            // per-CQE) bounds outbound sends so the arena can't be
            // exhausted by a multishot recv torrent.
            for (slot, peer) in self.peers.iter_mut() {
                if peer.state == PeerState::Dead {
                    continue;
                }
                consume_block_frames(
                    &mut self.ring,
                    &mut self.send_arena,
                    self.network,
                    slot,
                    peer,
                    &mut pending,
                    &mut saved,
                    &mut got_block_hashes,
                    &mut self.file_io,
                )?;
            }

            if dbg_arena {
                let alloc_iter = self.send_arena.dbg_alloc - dbg_alloc_pre;
                let release_iter = self.send_arena.dbg_release - dbg_release_pre;
                let arena_in_flight = self.send_arena.dbg_alloc - self.send_arena.dbg_release;
                let free = self.send_arena.free.len();
                eprintln!(
                    "  arena[b iter={dbg_iter}]: drained={drained} alloc+={alloc_iter} \
                     release+={release_iter} in_flight={arena_in_flight} free={free} \
                     peers_live={live} blocks_in_flight={bif} blocks_todo={todo_n} saved={saved}",
                    live = self.peers
                        .values()
                        .filter(|p| p.state != PeerState::Dead)
                        .count(),
                    bif = in_flight.len(),
                    todo_n = todo.len(),
                );
                dbg_iter += 1;
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
                        for &s in &info.issued_to {
                            if let Some(peer) = self.peers.get_mut(s) {
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
                let ready = self.peers
                    .values()
                    .filter(|p| p.state == PeerState::Ready)
                    .count();
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
                    for &s in &info.issued_to {
                        if let Some(peer) = self.peers.get_mut(s) {
                            peer.in_flight.retain(|(h, _)| h != &hash);
                        }
                    }
                }
                todo.push_front(hash);
            }

            for (slot, peer) in self.peers.iter_mut() {
                if peer.state == PeerState::Connecting
                    && now.duration_since(peer.connected_at)
                        > Duration::from_secs(CONNECT_TIMEOUT_SECS + 2)
                {
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
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
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                }
            }

            for slot in self.peers.dead_slots() {
                if let Some(p) = self.peers.remove(slot) {
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
                let active: Vec<SocketAddrV4> = self.peers.values().map(|p| p.addr).collect();
                if let Some(peer) = self.dial_next_avoiding_inner(slot, &active)? {
                    self.peers.insert(slot, peer);
                }
            }

            for hash in to_hedge {
                let Some(info) = in_flight.get(&hash).cloned() else {
                    continue;
                };
                if info.issued_to.len() >= HEDGE_FANOUT {
                    continue;
                }
                let already: PrehashedSet<u16> = {
                    let mut s = HashSet::with_hasher(BuildPrehashedHasher);
                    for &sl in &info.issued_to {
                        s.insert(sl);
                    }
                    s
                };
                let pick = self.peers.iter_mut().find(|(slot, peer)| {
                    peer.state == PeerState::Ready
                        && peer.in_flight.len() < PEER_INFLIGHT_CAP
                        && !already.contains(slot)
                });
                if let Some((slot, peer)) = pick {
                    let pl = encode_getdata(&[hash]);
                    let seq = push_send(
                        &mut self.ring,
                        &mut self.send_arena,
                        self.network,
                        Command::GetData,
                        &pl,
                        slot,
                        peer.generation,
                        peer.fd.ok_or(FetchError::NoPeers)?,
                    )?;
                    peer.pending_send_seqs.push(seq);
                    peer.in_flight.push((hash, now));
                    peer.assigned_count = peer.assigned_count.saturating_add(1);
                    if let Some(info) = in_flight.get_mut(&hash) {
                        info.issued_to.push(slot);
                        info.last_issued = now;
                    }
                }
            }

            for (slot, peer) in self.peers.iter_mut() {
                if peer.state != PeerState::Ready {
                    continue;
                }
                while peer.in_flight.len() < PEER_INFLIGHT_CAP {
                    let Some(hash) = todo.pop_front() else { break };
                    let pl = encode_getdata(&[hash]);
                    let seq = push_send(
                        &mut self.ring,
                        &mut self.send_arena,
                        self.network,
                        Command::GetData,
                        &pl,
                        slot,
                        peer.generation,
                        peer.fd.ok_or(FetchError::NoPeers)?,
                    )?;
                    peer.pending_send_seqs.push(seq);
                    peer.in_flight.push((hash, now));
                    peer.assigned_count = peer.assigned_count.saturating_add(1);
                    in_flight.insert(
                        hash,
                        InFlight {
                            issued_to: vec![slot],
                            first_issued: now,
                            last_issued: now,
                        },
                    );
                }
            }

            for (slot, peer) in self.peers.iter_mut() {
                if !peer.recv_armed
                    && peer.state != PeerState::Dead
                    && peer.state != PeerState::Connecting
                    && peer.fd.is_some()
                {
                    push_recv_multishot(&mut self.ring, slot, peer.generation, peer.fd.ok_or(FetchError::NoPeers)?)?;
                    peer.recv_armed = true;
                }
            }

            self.pbr.commit();

            if self.peers.is_empty() && in_flight.is_empty() && todo.is_empty() {
                return Ok(());
            }
            if self.peers.live_count() == 0 {
                eprintln!("  iouring-fetcher: all peers dead, recycling pool in 5s…");
                std::thread::sleep(Duration::from_secs(5));
                self.next_addr_idx = 0;
                self.ensure_peers()?;
                if self.peers.is_empty() {
                    return Err(FetchError::PoolExhausted);
                }
            }
        }
    }

    // ── peer dial ───────────────────────────────────────────────────────

    /// Pull the next pool address, wrapping around the pool if needed
    /// and skipping anything in `avoid`. Returns `None` only when *every*
    /// pool entry is currently in use — i.e. we'd have to reuse an
    /// already-connected address. The cursor advances through the pool
    /// monotonically modulo `pool.len()` so order roughly matches the
    /// caller-supplied (latency-ranked) ordering.
    fn next_pool_addr(&mut self, avoid: &[SocketAddrV4]) -> Option<SocketAddrV4> {
        if self.addr_pool.is_empty() {
            return None;
        }
        let len = self.addr_pool.len();
        for _ in 0..len {
            let idx = self.next_addr_idx % len;
            self.next_addr_idx = idx.wrapping_add(1);
            let addr = self.addr_pool[idx];
            if !avoid.contains(&addr) {
                return Some(addr);
            }
        }
        None
    }
}

// ── Block index I/O ─────────────────────────────────────────────────────

fn load_blocks_index(path: &Path) -> io::Result<Vec<BlockEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let data = fs::read(path)?;
    let count = data.len() / BLOCK_INDEX_ENTRY_SIZE;
    let mut index = Vec::with_capacity(count);
    for chunk in data.chunks_exact(BLOCK_INDEX_ENTRY_SIZE) {
        let offset = u64::from_le_bytes(chunk[..8].try_into().unwrap());
        let length = u32::from_le_bytes(chunk[8..12].try_into().unwrap());
        index.push(BlockEntry { offset, length });
    }
    Ok(index)
}

fn save_blocks_index(path: &Path, index: &[BlockEntry]) -> io::Result<()> {
    let mut data = Vec::with_capacity(index.len() * BLOCK_INDEX_ENTRY_SIZE);
    for entry in index {
        data.extend_from_slice(&entry.offset.to_le_bytes());
        data.extend_from_slice(&entry.length.to_le_bytes());
    }
    fs::write(path, &data)
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

#[cfg(unix)]
#[allow(clippy::cast_sign_loss)]
fn file_to_rawfd(file: &fs::File) -> RawFd {
    use std::os::unix::io::AsRawFd;
    RawFd::from_raw(file.as_raw_fd() as usize)
}

/// `user_data` for SEND SQEs additionally carries the `SendArena` slot
/// index in bits 48..64 so the SEND CQE handler can release the slot.
#[inline]
fn make_send_user_data(slot: u16, g: u16, send_seq: u16) -> u64 {
    make_user_data(slot, g, OP_SEND) | (u64::from(send_seq) << 48)
}

#[inline]
fn decode_send_seq(raw: u64) -> u16 {
    #[allow(clippy::cast_possible_truncation)]
    let s = (raw >> 48) as u16;
    s
}

/// Decode the buffer id selected by the kernel from a CQE's flags.
/// Mirrors `Completion::buffer_id` — the kernel stores the id in the
/// upper 16 bits of `flags` when `CqeFlags::BUFFER` is set.
#[inline]
fn decode_cqe_buffer_id(flags: ququmatz::types::CqeFlags) -> Option<u16> {
    use ququmatz::types::CqeFlags;
    if flags.contains(CqeFlags::BUFFER) {
        #[allow(clippy::cast_possible_truncation)]
        Some((flags.bits() >> 16) as u16)
    } else {
        None
    }
}

fn make_ring() -> Result<IoUring, FetchError> {
    // `single_issuer` + `defer_taskrun` + `coop_taskrun` is the modern
    // "tight event loop" combo: the kernel knows only one thread will
    // submit, defers task-run work to the issuer thread (avoiding IPI
    // wakeups), and lets us cooperatively reap CQEs without the kernel
    // racing us. All three require Linux ≥ 6.0 in practice; older
    // kernels return EINVAL on builder.build(), so we fall back to a
    // bare `IoUring::new`.
    // A/B knob: `MANTLE_URING_BARE=1` skips the fast-loop flags so we can
    // measure their effect against a baseline `IoUring::new`.
    let bare = std::env::var_os("MANTLE_URING_BARE").is_some_and(|v| v == "1");
    if bare {
        eprintln!("  io_uring: bare (MANTLE_URING_BARE=1)");
    } else if let Ok(ring) = IoUring::builder(RING_ENTRIES)
        .single_issuer()
        .defer_taskrun()
        .coop_taskrun()
        .build()
    {
        return Ok(ring);
    }
    IoUring::new(RING_ENTRIES).map_err(|e| FetchError::UringErr(format!("IoUring::new: {e:?}")))
}

fn uring_err(e: ququmatz::Error) -> FetchError {
    FetchError::UringErr(format!("{e:?}"))
}

fn push_tick(ring: &mut IoUring, dur: Duration) -> Result<(), FetchError> {
    let ts = Timespec::from_millis(u64::try_from(dur.as_millis()).unwrap_or(u64::MAX));
    ring.push(
        Sqe::timeout(&ts, 1, TimeoutFlags::default()).user_data(make_user_data(0, 0, OP_TICK)),
    )
    .map_err(uring_err)
}

fn push_connect(
    ring: &mut IoUring,
    slot: u16,
    g: u16,
    fd: RawFd,
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
        Sqe::link_timeout(timeout, TimeoutFlags::default()).user_data(make_user_data(
            slot,
            g,
            OP_LINK_TIMEOUT,
        )),
    )
    .map_err(uring_err)?;
    Ok(())
}

/// Frame `(cmd, payload)` into the next free slot of `arena` and push
/// a SEND SQE that points into that slot. Returns the arena slot index
/// (`send_seq`) on success — callers track it on the peer so they can
/// proactively release on eviction (see [`Peer::pending_send_seqs`]).
/// The slot index rides in the upper 16 bits of `user_data` so the
/// SEND CQE handler can release the slot. No per-call allocation: the
/// slab and free-list are owned by the arena and live for the session.
#[allow(clippy::too_many_arguments)]
fn push_send(
    ring: &mut IoUring,
    arena: &mut SendArena,
    network: Network,
    cmd: Command,
    payload: &[u8],
    slot: u16,
    g: u16,
    fd: RawFd,
) -> Result<u16, FetchError> {
    let send_seq = arena
        .frame_into(network.magic(), cmd, payload)
        .ok_or_else(|| {
            FetchError::UringErr(format!(
                "send arena exhausted (peer slot={slot} g={g}, free={} cap={SEND_SLOT_COUNT})",
                arena.free.len()
            ))
        })?;
    // SAFETY: extending the borrow's lifetime is sound because the
    // slab outlives the SQE — the arena slot stays reserved
    // (used_len != 0) until the SEND CQE handler calls `release`.
    let bytes = arena.slice(send_seq);
    let bytes_static: &[u8] = unsafe { std::slice::from_raw_parts(bytes.as_ptr(), bytes.len()) };
    let r = ring
        .push(
            Sqe::send(fd, bytes_static, MsgFlags::default())
                .user_data(make_send_user_data(slot, g, send_seq)),
        )
        .map_err(uring_err);
    match r {
        Ok(()) => Ok(send_seq),
        Err(e) => {
            // SQE push failed — kernel never sees this send, so no CQE
            // will arrive to release the slot. Release here.
            arena.release(send_seq);
            Err(e)
        }
    }
}

/// Submit one multishot recv SQE. The kernel will deliver one CQE per
/// arrival (with `BUFFER` flag set and the buffer id in the upper 16
/// bits of `flags`) until either the peer closes, the kernel hits an
/// error, or the provided-buffer pool runs dry. On termination the
/// final CQE clears `MORE`; we re-arm in the per-iteration sweep.
fn push_recv_multishot(ring: &mut IoUring, slot: u16, g: u16, fd: RawFd) -> Result<(), FetchError> {
    ring.push(
        Sqe::recv_multishot(fd, MsgFlags::default())
            .buffer_select(PROVIDED_BUF_GROUP_ID)
            .user_data(make_user_data(slot, g, OP_RECV)),
    )
    .map_err(uring_err)
}

fn close_peer(ring: &mut IoUring, send_arena: &mut SendArena, peer: &mut Peer) {
    // Eagerly release any arena slots the kernel still owes us a CQE for.
    // With `SendRecvFlag::PollFirst` a SEND on a backpressured peer doesn't
    // get a CQE until the socket becomes writable, which on a peer we're
    // about to close may be never. The fd close below will eventually
    // produce `-ECANCELED` SEND CQEs that hit the same `arena.release`
    // path, so we tag the pending seqs here as already-released — late
    // CQEs become idempotent no-ops via `release`'s guard on `used_len`.
    for seq in peer.pending_send_seqs.drain(..) {
        send_arena.release(seq);
    }
    if let Some(fd) = peer.fd.take() {
        // Cancel the outstanding recv first so its CQE arrives with
        // `-ECANCELED` rather than racing the close: the kernel matches
        // `target_user_data` against pending SQEs and terminates the
        // matching op cleanly. Without this, the recv may produce a
        // surprising CQE *after* close completes, against an fd we
        // believe to be released. The generation counter on user_data
        // already protects against confusing late CQEs with new ones,
        // so this is belt-and-suspenders — but cheap.
        //
        // Cancel target = the user_data we'd have used for that recv
        // (slot is implicit in the peer's identity but we don't track
        // it here; encode 0 as slot to match the pre-close `slot=…,
        // op=OP_RECV` patterns that have already been submitted). The
        // generation field is the discriminator that makes this match.
        if let Err(e) = ring.push(
            Sqe::cancel(make_user_data(0, peer.generation, OP_RECV)).user_data(make_user_data(
                0,
                peer.generation,
                OP_CLOSE,
            )),
        ) {
            eprintln!("  iouring-fetcher: close_peer cancel-recv push failed for fd={fd:?}: {e:?}");
        }
        if let Err(e) =
            ring.push(Sqe::close(fd).user_data(make_user_data(0, peer.generation, OP_CLOSE)))
        {
            // SQE push failed: kernel won't close the fd via the ring.
            // Fall back to a synchronous close so we don't leak the
            // fd. This is the only safe path — a quiet leak compounds
            // into EMFILE on long runs.
            // Wrap the fd in a Socket and let Drop close it. Safe
            // because we just established (via the failed push) that
            // the kernel doesn't own this fd via the ring.
            drop(unsafe { ququmatz::Socket::from_fd(fd) });
            eprintln!("  iouring-fetcher: close_peer close push failed for fd={fd:?}: {e:?} (closed synchronously)");
        }
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


// ── CQE handlers ────────────────────────────────────────────────────────

/// Common multishot-recv CQE handling shared by both sessions. Copies
/// the kernel-selected provided buffer into `peer.recv_buf`, recycles
/// the buffer, and clears `recv_armed` if the kernel terminated the
/// multishot (CQE without `MORE`). Re-arming happens at the bottom
/// of the session loop, *not* here, so SEND CQEs get a chance to
/// surface between recv-CQE bursts.
fn ingest_recv_cqe(
    pbr: &mut ququmatz::ProvidedBufferRing,
    send_arena: &mut SendArena,
    peer: &mut Peer,
    cqe_flags: ququmatz::types::CqeFlags,
    res: i32,
    ring: &mut IoUring,
) {
    use ququmatz::types::CqeFlags;
    if res <= 0 {
        peer.state = PeerState::Dead;
        peer.recv_armed = false;
        close_peer(ring, send_arena, peer);
        return;
    }
    #[allow(clippy::cast_sign_loss)]
    let n = res as u32;
    if let Some(buf_id) = decode_cqe_buffer_id(cqe_flags) {
        match pbr.buffer(buf_id, n) {
            Some(bytes) => peer.recv_buf.extend_from_slice(bytes),
            None => {
                // buf_id out of range or n > buf_size — both indicate
                // the kernel returned something outside the contract
                // we registered. Surface; do NOT silently drop the
                // bytes (that masks framing loss).
                eprintln!(
                    "  iouring-fetcher: pbr.buffer({buf_id}, {n}) returned None \
                     (buf_size={}, entries={})",
                    pbr.buf_size(),
                    pbr.entries(),
                );
            }
        }
        pbr.recycle(buf_id);
    } else {
        // No BUFFER flag on a multishot recv CQE: should never happen
        // when we set buffer_select. If it does, we've lost track of
        // `n` bytes the kernel wrote somewhere we can't retrieve.
        eprintln!(
            "  iouring-fetcher: recv CQE with res={n} has no BUFFER flag (multishot setup is broken)"
        );
    }
    if !cqe_flags.contains(CqeFlags::MORE) {
        peer.recv_armed = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_getheaders_cqe(
    ring: &mut IoUring,
    pbr: &mut ququmatz::ProvidedBufferRing,
    send_arena: &mut SendArena,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    op: u8,
    res: i32,
    cqe_flags: ququmatz::types::CqeFlags,
) -> Result<(), FetchError> {
    match op {
        OP_CONNECT => {
            if std::env::var_os("MANTLE_URING_DEBUG").is_some_and(|v| v == "1") {
                eprintln!(
                    "  iouring-fetcher: peer slot={slot} ({}) connect-cqe res={res}",
                    peer.addr
                );
            }
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, send_arena, peer);
                return Ok(());
            }
            // Connect succeeded — arm multishot recv against the
            // provided-buffer pool, then send our version. Both wait
            // until success so a connect-failed peer doesn't leak
            // SQEs into the kernel without producing CQEs.
            push_recv_multishot(ring, slot, peer.generation, peer.fd.ok_or(FetchError::NoPeers)?)?;
            peer.recv_armed = true;
            let seq = push_send(
                ring,
                send_arena,
                network,
                Command::Version,
                &our_version().encode_to_vec(),
                slot,
                peer.generation,
                peer.fd.ok_or(FetchError::NoPeers)?,
            )?;
            peer.pending_send_seqs.push(seq);
            peer.state = PeerState::HandshakeSendVersion;
        }
        OP_SEND if res < 0 => {
            peer.state = PeerState::Dead;
            close_peer(ring, send_arena, peer);
        }
        OP_RECV => {
            ingest_recv_cqe(pbr, send_arena, peer, cqe_flags, res, ring);
            // Frame parsing happens once per loop iteration, *not* per
            // CQE — see the per-iter sweep that calls
            // `consume_handshake_frames`. With multishot recv the
            // kernel can deliver dozens of CQEs in one
            // submit_and_wait cycle; parsing each one inline would
            // generate dozens of pong/verack sends and exhaust the
            // send arena before the kernel surfaces their CQEs.
        }
        _ => {}
    }
    Ok(())
}

fn consume_handshake_frames(
    ring: &mut IoUring,
    send_arena: &mut SendArena,
    network: Network,
    slot: u16,
    peer: &mut Peer,
) -> Result<Option<Vec<BlockHeader>>, FetchError> {
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
            close_peer(ring, send_arena, peer);
            return Ok(None);
        }
        let cmd = header.command.as_str();
        // Per-peer SEND backpressure cap, enforced uniformly across all
        // send-emitting branches below (verack, pong). If a peer is at
        // cap, the kernel is sitting on too many outbound CQEs for it
        // — evict instead of issuing another send that would just
        // pile onto the arena. Without a uniform check, e.g. duplicate
        // version messages from a misbehaving peer in HandshakeRecving
        // state would leak Verack sends without bound.
        let needs_send = matches!(
            (peer.state, cmd),
            (
                PeerState::HandshakeSendVersion | PeerState::HandshakeRecving,
                "version"
            ) | (PeerState::Ready | PeerState::WaitingHeaders, "ping")
        );
        if needs_send && peer.pending_send_seqs.len() >= PEER_SEND_OUTSTANDING_CAP as usize {
            peer.state = PeerState::Dead;
            close_peer(ring, send_arena, peer);
            return Ok(None);
        }
        match (peer.state, cmd) {
            (PeerState::HandshakeSendVersion | PeerState::HandshakeRecving, "version") => {
                let seq = push_send(
                    ring,
                    send_arena,
                    network,
                    Command::Verack,
                    &[],
                    slot,
                    peer.generation,
                    peer.fd.ok_or(FetchError::NoPeers)?,
                )?;
                peer.pending_send_seqs.push(seq);
                peer.state = PeerState::HandshakeRecving;
            }
            (PeerState::HandshakeRecving | PeerState::HandshakeSendVersion, "verack") => {
                peer.state = PeerState::Ready;
            }
            (PeerState::Ready | PeerState::WaitingHeaders, "ping") => {
                // Echo nonce back. ping payload is 8 bytes; copy out
                // via a stack array so we don't keep a borrow into
                // recv_buf (which we drain below).
                let mut nonce = [0u8; 8];
                let copy_len = payload_slice.len().min(8);
                nonce[..copy_len].copy_from_slice(&payload_slice[..copy_len]);
                let seq = push_send(
                    ring,
                    send_arena,
                    network,
                    Command::Pong,
                    &nonce[..copy_len],
                    slot,
                    peer.generation,
                    peer.fd.ok_or(FetchError::NoPeers)?,
                )?;
                peer.pending_send_seqs.push(seq);
            }
            (PeerState::WaitingHeaders, "headers") => {
                let mut headers: Vec<BlockHeader> = Vec::new();
                if let Err(e) = HeadersMsg::decode_into_limited::<2_000>(
                    &mut std::io::Cursor::new(payload_slice),
                    &mut headers,
                ) {
                    // Don't swallow: parse failures usually mean the
                    // stream was misframed (corruption or wrong magic),
                    // not a recoverable hiccup. Surface so the caller
                    // re-elects rather than spin on a broken peer.
                    eprintln!(
                        "  iouring-fetcher: peer slot={slot} headers decode error: {e:?} \
                         (payload {} bytes, parsed {} headers before failing)",
                        payload_slice.len(),
                        headers.len(),
                    );
                    peer.recv_buf.drain(..total);
                    peer.state = PeerState::Dead;
                    close_peer(ring, send_arena, peer);
                    return Ok(None);
                }
                peer.recv_buf.drain(..total);
                peer.state = PeerState::Ready;
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
    pbr: &mut ququmatz::ProvidedBufferRing,
    send_arena: &mut SendArena,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    op: u8,
    res: i32,
    cqe_flags: ququmatz::types::CqeFlags,
) -> Result<(), FetchError> {
    match op {
        OP_CONNECT => {
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, send_arena, peer);
                return Ok(());
            }
            push_recv_multishot(ring, slot, peer.generation, peer.fd.ok_or(FetchError::NoPeers)?)?;
            peer.recv_armed = true;
            let seq = push_send(
                ring,
                send_arena,
                network,
                Command::Version,
                &our_version().encode_to_vec(),
                slot,
                peer.generation,
                peer.fd.ok_or(FetchError::NoPeers)?,
            )?;
            peer.pending_send_seqs.push(seq);
            peer.state = PeerState::HandshakeSendVersion;
            Ok(())
        }
        OP_SEND => {
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, send_arena, peer);
            }
            Ok(())
        }
        OP_RECV => {
            ingest_recv_cqe(pbr, send_arena, peer, cqe_flags, res, ring);
            // Frame parsing happens per-iteration, see the per-iter
            // sweep that calls `consume_block_frames`.
            Ok(())
        }
        _ => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
fn consume_block_frames(
    ring: &mut IoUring,
    send_arena: &mut SendArena,
    network: Network,
    slot: u16,
    peer: &mut Peer,
    pending: &mut PrehashedMap<Hash256, BlockHeight>,
    saved: &mut usize,
    arrived: &mut Vec<Hash256>,
    file_io: &mut FileIoHandle,
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
            close_peer(ring, send_arena, peer);
            return Ok(());
        }
        let cmd = header.command.as_str();
        match (peer.state, cmd) {
            (PeerState::HandshakeSendVersion | PeerState::HandshakeRecving, "version") => {
                let seq = push_send(
                    ring,
                    send_arena,
                    network,
                    Command::Verack,
                    &[],
                    slot,
                    peer.generation,
                    peer.fd.ok_or(FetchError::NoPeers)?,
                )?;
                peer.pending_send_seqs.push(seq);
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
                if let Some(height) = pending.remove(&hash) {
                    file_io.write_block_async(height, payload_slice.to_vec())?;
                    *saved += 1;
                    peer.delivered_count = peer.delivered_count.saturating_add(1);
                    peer.last_delivery = Instant::now();
                    arrived.push(hash);
                }
            }
            (_, "ping") => {
                if peer.pending_send_seqs.len() >= PEER_SEND_OUTSTANDING_CAP as usize {
                    peer.state = PeerState::Dead;
                    close_peer(ring, send_arena, peer);
                    return Ok(());
                }
                let mut nonce = [0u8; 8];
                let copy_len = payload_slice.len().min(8);
                nonce[..copy_len].copy_from_slice(&payload_slice[..copy_len]);
                let seq = push_send(
                    ring,
                    send_arena,
                    network,
                    Command::Pong,
                    &nonce[..copy_len],
                    slot,
                    peer.generation,
                    peer.fd.ok_or(FetchError::NoPeers)?,
                )?;
                peer.pending_send_seqs.push(seq);
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
        // `Vec<u8>` as `Write` cannot fail (its `write_all` returns
        // `Ok(())` for any input). If `encode` itself returns an error
        // we'd want to know — that signals the message is malformed
        // before it ever leaves us.
        self.encode(&mut out)
            .expect("Vec<u8> Write is infallible; encode failure indicates a bug");
        out
    }
}

fn getheaders_msg(locator: Hash256) -> GetHeadersMsg {
    GetHeadersMsg {
        version: 70016,
        locator_hashes: vec![locator],
        stop_hash: Hash256::ZERO,
    }
}

// `CommandName` is unused here today but imported for future use.
#[allow(dead_code)]
type _CommandName = CommandName;
