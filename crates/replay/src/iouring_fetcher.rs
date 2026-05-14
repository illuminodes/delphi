//! Single-ring `io_uring` fetcher for `mantle-replay`.
//!
//! Uses `mantle-transport` for the `io_uring` peer lifecycle (connect, handshake,
//! send arena, provided-buffer recv, peer pool) and adds fetcher-specific logic:
//! header sync, pipelined block download with hedging/reclaim, and file I/O.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mantle::codec::{Decode, Encode};
use mantle::collections::{BuildPrehashedHasher, PrehashedMap, PrehashedSet};
use mantle::hash::checksum;
use mantle::network::{
    Command, GetHeadersMsg, HeadersMsg, InvEntry, InvMsg, InvType, MessageHeader, Network,
};
use mantle::primitives::{BlockHeader, BlockHeight, Hash256};
use mantle_transport::{
    OP_CLOSE, OP_CONNECT, OP_LINK_TIMEOUT, OP_RECV, OP_SEND, OP_TICK, Peer, PeerPool, PeerState,
    SendArena, close_peer, consume_handshake_frames, decode_send_seq, decode_user_data,
    encode_version, ingest_recv_cqe, make_ring, push_connect, push_recv_multishot, push_send,
    push_tick,
};
use ququmatz::types::{AddressFamily, RawFd, SockAddrIn, SocketFlags, SocketType, Timespec};
use ququmatz::IoUring;

// ── Constants ──────────────────────────────────────────────────────────────

const PROVIDED_BUF_SIZE: u32 = 256 * 1024;
const PROVIDED_BUF_COUNT: u32 = 4_096;
const PROVIDED_BUF_GROUP_ID: u16 = 1;
const CQE_DRAIN_CAP: usize = 1024;
const PEER_INFLIGHT_CAP: usize = 2;
const CONNECT_TIMEOUT_SECS: u64 = 5;
const HEADERS_RESPONSE_TIMEOUT_SECS: u64 = 10;
const HANDSHAKE_TIMEOUT_SECS: u64 = 10;
const HEDGE_FLOOR_MS: u64 = 5_000;
const HEDGE_FACTOR: u32 = 2;
const HEDGE_FANOUT: usize = 2;
const RECLAIM_FLOOR_MS: u64 = 10_000;
const RECLAIM_FACTOR: u32 = 4;
const BLACK_HOLE_ASSIGNED_THRESHOLD: u32 = 4;
const BLACK_HOLE_GRACE_SECS: u64 = 60;
const BLOCK_NO_PROGRESS_TIMEOUT_SECS: u64 = 300;
const HEADERS_PHASE_DEADLINE_SECS: u64 = 120;
const TARGET_LIVE_PEERS: usize = 64;
const RECV_BUF_CAPACITY: usize = 4 * 1024 * 1024;
const INITIAL_EWMA_MS: u64 = 1_000;
const EWMA_N: u32 = 8;

// ── Error ──────────────────────────────────────────────────────────────────

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

fn uring_err(e: ququmatz::Error) -> FetchError {
    FetchError::UringErr(format!("{e:?}"))
}

// ── Fetcher-specific peer extension ────────────────────────────────────────

#[derive(Debug)]
struct FetcherExt {
    in_flight: Vec<(Hash256, Instant)>,
    assigned_count: u32,
    delivered_count: u32,
    last_delivery: Instant,
    waiting_headers: bool,
}

impl FetcherExt {
    fn new() -> Self {
        Self {
            in_flight: Vec::new(),
            assigned_count: 0,
            delivered_count: 0,
            last_delivery: Instant::now(),
            waiting_headers: false,
        }
    }
}

type FPeer = Peer<FetcherExt>;
type FPool = PeerPool<FetcherExt, { TARGET_LIVE_PEERS }>;

// ── Hedging bookkeeping ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct InFlight {
    issued_to: Vec<u16>,
    first_issued: Instant,
    #[allow(dead_code)]
    last_issued: Instant,
}

#[derive(Debug, Clone, Copy)]
struct EwmaLatency {
    ms: u64,
}

impl EwmaLatency {
    const fn new() -> Self {
        Self {
            ms: INITIAL_EWMA_MS,
        }
    }

    fn observe(&mut self, sample: Duration) {
        let s = u64::try_from(sample.as_millis()).unwrap_or(u64::MAX);
        let n = u64::from(EWMA_N);
        if s >= self.ms {
            self.ms = self.ms.saturating_add((s - self.ms) / n);
        } else {
            self.ms = self.ms.saturating_sub((self.ms - s) / n);
        }
        self.ms = self.ms.min(10_000);
    }

    fn threshold(self, floor_ms: u64, factor: u32) -> Duration {
        let scaled = self.ms.saturating_mul(u64::from(factor));
        Duration::from_millis(scaled.max(floor_ms))
    }

    const fn as_millis(self) -> u64 {
        self.ms
    }
}

// ── File I/O thread ────────────────────────────────────────────────────────

enum FileReq {
    #[allow(dead_code)]
    WriteBlock {
        height: BlockHeight,
        data: Vec<u8>,
    },
    WriteBlockAsync {
        height: BlockHeight,
        data: Vec<u8>,
    },
    ReadBlock {
        height: BlockHeight,
    },
    HasBlock {
        height: BlockHeight,
    },
    WriteHeaders {
        start: BlockHeight,
        data: Vec<u8>,
    },
    LoadCachedHeaders {
        start: BlockHeight,
    },
    FlushBlockIndex,
    #[allow(dead_code)]
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

        Ok(Self {
            tx: req_tx,
            rx: resp_rx,
        })
    }

    #[allow(dead_code)]
    fn write_block(&mut self, height: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx
            .push(FileReq::WriteBlock { height, data })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn write_block_async(&self, height: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx
            .push(FileReq::WriteBlockAsync { height, data })
            .map_err(|_| io::Error::other("file-io ring full"))
    }

    fn read_block(&mut self, height: BlockHeight) -> io::Result<Vec<u8>> {
        self.tx
            .push(FileReq::ReadBlock { height })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::BlockData(data)) => Ok(data),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn write_headers(&mut self, start: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx
            .push(FileReq::WriteHeaders { start, data })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    fn flush_block_index(&mut self) -> io::Result<()> {
        self.tx
            .push(FileReq::FlushBlockIndex)
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
        self.tx
            .push(FileReq::LoadCachedHeaders { start })
            .map_err(|_| FetchError::UringErr("file-io ring full".into()))?;
        match self.rx.pop_block() {
            Some(FileResp::Headers(h)) => Ok(h),
            Some(FileResp::Err(e)) => Err(FetchError::Io(e)),
            _ => Ok(Vec::new()),
        }
    }

    #[allow(dead_code)]
    fn shutdown(&mut self) {
        let _ = self.tx.push(FileReq::Shutdown);
        let _ = self.rx.pop_block();
    }
}

const BLOCK_INDEX_FLUSH_INTERVAL: u64 = 2_000;

#[allow(clippy::needless_pass_by_value)]
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
            FileReq::WriteBlock { height, data } => match blocks.write(&mut ring, height, &data) {
                Ok(()) => {
                    writes_since_flush += 1;
                    if writes_since_flush >= BLOCK_INDEX_FLUSH_INTERVAL {
                        let _ = blocks.flush_index(data_dir);
                        writes_since_flush = 0;
                    }
                    FileResp::Ok
                }
                Err(e) => FileResp::Err(e),
            },
            FileReq::WriteBlockAsync { height, data } => {
                let _ = blocks.write(&mut ring, height, &data);
                writes_since_flush += 1;
                if writes_since_flush >= BLOCK_INDEX_FLUSH_INTERVAL {
                    let _ = blocks.flush_index(data_dir);
                    writes_since_flush = 0;
                }
                continue;
            }
            FileReq::ReadBlock { height } => match blocks.read(&mut ring, height) {
                Ok(data) => FileResp::BlockData(data),
                Err(e) => FileResp::Err(e),
            },
            FileReq::HasBlock { height } => FileResp::Bool(blocks.has_block(height)),
            FileReq::WriteHeaders { start, data } => {
                match headers.write_raw(&mut ring, start, &data) {
                    Ok(()) => FileResp::Ok,
                    Err(e) => FileResp::Err(e),
                }
            }
            FileReq::LoadCachedHeaders { start } => match headers.load_cached(&mut ring, start) {
                Ok(h) => FileResp::Headers(h),
                Err(e) => FileResp::Err(io::Error::other(format!("{e}"))),
            },
            FileReq::FlushBlockIndex => match blocks.flush_index(data_dir) {
                Ok(()) => FileResp::Ok,
                Err(e) => FileResp::Err(e),
            },
            FileReq::Shutdown => {
                let _ = tx.push(FileResp::Ok);
                return;
            }
        };
        let _ = tx.push(resp);
    }
}

// ── File-backed store ──────────────────────────────────────────────────────

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
        Ok(Self {
            file,
            fd,
            write_offset,
        })
    }

    #[allow(clippy::cast_possible_truncation)]
    fn uring_read(&self, ring: &mut IoUring, offset: u64, len: u32) -> io::Result<Vec<u8>> {
        let mut buf = vec![0u8; len as usize];
        let sqe = unsafe { ququmatz::Sqe::read_ptr(self.fd, buf.as_mut_ptr(), len, offset) }
            .user_data(u64::MAX);
        ring.push(sqe)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        loop {
            ring.submit_and_wait(1)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
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
            ququmatz::Sqe::write_ptr(self.fd, data.as_ptr(), data.len() as u32, offset)
        }
        .user_data(u64::MAX);
        ring.push(sqe)
            .map_err(|e| io::Error::other(format!("{e:?}")))?;
        loop {
            ring.submit_and_wait(1)
                .map_err(|e| io::Error::other(format!("{e:?}")))?;
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

// ── Block store ────────────────────────────────────────────────────────────

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
        let entry = self
            .index
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

// ── Header store ───────────────────────────────────────────────────────────

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
        let data = self
            .store
            .uring_read(ring, 0, self.store.write_offset as u32)?;
        let mut headers = Vec::with_capacity(count);
        for (i, chunk) in data.chunks_exact(HEADER_RECORD_SIZE).enumerate() {
            let arr: &[u8; 80] = chunk.try_into().unwrap();
            let hdr = BlockHeader::from_bytes(arr)
                .map_err(|e| FetchError::UringErr(format!("cached header {i} decode: {e}")))?;
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
        self.metadata
            .truncate_and_write(ring, &start.0.to_le_bytes())
    }
}

// ── Fetcher ────────────────────────────────────────────────────────────────

pub struct Fetcher {
    network: Network,
    addr_pool: Vec<SocketAddrV4>,
    next_addr_idx: usize,
    connect_timeout: Timespec,
    ring: IoUring,
    pbr: ququmatz::ProvidedBufferRing,
    send_arena: SendArena,
    peers: FPool,
    file_io: FileIoHandle,
}

impl Fetcher {
    pub fn new(
        network: Network,
        peer_addrs: Vec<SocketAddrV4>,
        data_dir: PathBuf,
    ) -> io::Result<Self> {
        std::fs::create_dir_all(&data_dir)?;
        let mut ring = make_ring()?;
        let pbr = ring
            .register_provided_buffers(PROVIDED_BUF_GROUP_ID, PROVIDED_BUF_COUNT, PROVIDED_BUF_SIZE)
            .map_err(|e| io::Error::other(format!("register_provided_buffers: {e:?}")))?;

        let file_io = FileIoHandle::spawn(&data_dir)?;

        Ok(Self {
            network,
            addr_pool: peer_addrs,
            next_addr_idx: 0,
            connect_timeout: Timespec::from_millis(CONNECT_TIMEOUT_SECS * 1_000),
            ring,
            pbr,
            send_arena: SendArena::new(),
            peers: FPool::new(),
            file_io,
        })
    }

    fn ensure_peers(&mut self) -> Result<(), FetchError> {
        let live = self.peers.live_count();
        let target = TARGET_LIVE_PEERS.min(self.addr_pool.len());
        for _ in live..target {
            let Some(slot) = self.peers.free_slot() else {
                break;
            };
            if let Some(peer) = self.dial_next_inner(slot)? {
                self.peers.insert(slot, peer);
            }
        }
        Ok(())
    }

    fn dial_next_inner(&mut self, slot: u16) -> Result<Option<FPeer>, FetchError> {
        self.dial_next_avoiding_inner(slot, &[])
    }

    fn dial_next_avoiding_inner(
        &mut self,
        slot: u16,
        active: &[SocketAddrV4],
    ) -> Result<Option<FPeer>, FetchError> {
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

        let peer = FPeer {
            addr,
            fd: Some(fd),
            sockaddr,
            state: PeerState::Connecting,
            recv_buf: Vec::with_capacity(RECV_BUF_CAPACITY),
            recv_armed: false,
            connected_at: Instant::now(),
            generation: 0,
            pending_send_seqs: Vec::with_capacity(8),
            app: FetcherExt::new(),
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
                collected.len(),
                start.0,
                cached_end.0,
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

    pub fn fetch_blocks(
        &mut self,
        headers: &[(BlockHeight, BlockHeader)],
    ) -> Result<(), FetchError> {
        if self.addr_pool.is_empty() {
            return Err(FetchError::NoPeers);
        }
        let needed: Vec<(BlockHeight, Hash256)> = headers
            .iter()
            .filter(|(h, _)| !self.file_io.has_block(*h))
            .map(|(h, hdr)| (*h, hdr.hash()))
            .collect();
        if needed.is_empty() {
            return Ok(());
        }
        self.run_block_download_session(&needed)?;
        self.file_io.flush_block_index()?;
        Ok(())
    }

    // ── headers session ────────────────────────────────────────────────

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
                push_recv_multishot(
                    &mut self.ring,
                    slot,
                    peer.generation,
                    peer.fd.ok_or(FetchError::NoPeers)?,
                    PROVIDED_BUF_GROUP_ID,
                )?;
                peer.recv_armed = true;
            }
        }

        let phase_deadline = Instant::now() + Duration::from_secs(HEADERS_PHASE_DEADLINE_SECS);
        let payload = getheaders_payload(locator);
        let mut elected: Option<(u16, Instant)> = None;
        let version_payload = encode_version("/mantle-replay-iouring:0.1.0/");
        let mut last_status = Instant::now();

        loop {
            push_tick(&mut self.ring, Duration::from_secs(1))?;
            self.ring.submit_and_wait(1).map_err(uring_err)?;

            let mut drained = 0usize;
            while drained < CQE_DRAIN_CAP {
                let Some(cqe) = self.ring.complete() else {
                    break;
                };
                drained += 1;
                let (slot, op, g) = decode_user_data(cqe.user_data);
                if op == OP_TICK {
                    continue;
                }
                if op == OP_SEND {
                    let send_seq = decode_send_seq(cqe.user_data);
                    self.send_arena.release(send_seq);
                    if let Some(p) = self.peers.get_mut(slot) {
                        p.pending_send_seqs.retain(|s| *s != send_seq);
                    }
                }
                if op == OP_LINK_TIMEOUT || op == OP_CLOSE {
                    continue;
                }
                let res = cqe.result;
                let Some(peer) = self.peers.get_mut(slot) else {
                    continue;
                };
                if peer.generation != g {
                    continue;
                }
                handle_connect_or_recv(
                    &mut self.ring,
                    &mut self.pbr,
                    &mut self.send_arena,
                    self.network,
                    slot,
                    peer,
                    op,
                    res,
                    cqe.flags,
                    &version_payload,
                )?;
            }

            // Parse frames: handshake + headers
            let mut got_headers: Option<Vec<BlockHeader>> = None;
            for (slot, peer) in self.peers.iter_mut() {
                if peer.state == PeerState::Dead {
                    continue;
                }
                let waiting = peer.app.waiting_headers;
                consume_handshake_frames(
                    &mut self.ring,
                    &mut self.send_arena,
                    self.network,
                    slot,
                    peer,
                    |cmd, payload| {
                        if waiting && cmd.as_str() == "headers" {
                            let mut headers: Vec<BlockHeader> = Vec::new();
                            if HeadersMsg::decode_into_limited::<2_000>(
                                &mut std::io::Cursor::new(payload),
                                &mut headers,
                            )
                            .is_ok()
                            {
                                got_headers = Some(headers);
                            }
                        }
                    },
                )?;
                if got_headers.is_some() {
                    peer.app.waiting_headers = false;
                }
            }

            if let Some(headers) = got_headers {
                return Ok(headers);
            }

            let now = Instant::now();
            if now > phase_deadline {
                return Err(FetchError::Timeout("headers"));
            }
            if now.duration_since(last_status) >= Duration::from_secs(5) {
                let ready = self
                    .peers
                    .values()
                    .filter(|p| p.state == PeerState::Ready)
                    .count();
                let connecting = self
                    .peers
                    .values()
                    .filter(|p| p.state == PeerState::Connecting)
                    .count();
                let handshaking = self
                    .peers
                    .values()
                    .filter(|p| {
                        matches!(
                            p.state,
                            PeerState::HandshakeSendVersion | PeerState::HandshakeRecving
                        )
                    })
                    .count();
                let dead = self
                    .peers
                    .values()
                    .filter(|p| p.state == PeerState::Dead)
                    .count();
                let waiting = self
                    .peers
                    .values()
                    .filter(|p| p.app.waiting_headers)
                    .count();
                eprintln!(
                    "  headers: ready={ready} connecting={connecting} handshaking={handshaking} \
                     waiting={waiting} dead={dead} elected={:?}",
                    elected.map(|(s, _)| s),
                );
                last_status = now;
            }

            // Timeout stale peers.
            for (_slot, peer) in self.peers.iter_mut() {
                if peer.state == PeerState::Dead {
                    continue;
                }
                if peer.state == PeerState::Connecting
                    && now.duration_since(peer.connected_at)
                        > Duration::from_secs(CONNECT_TIMEOUT_SECS + 2)
                {
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                }
                if matches!(
                    peer.state,
                    PeerState::HandshakeSendVersion | PeerState::HandshakeRecving
                ) && now.duration_since(peer.connected_at)
                    > Duration::from_secs(HANDSHAKE_TIMEOUT_SECS)
                {
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                }
            }

            // Headers response timeout for elected peer.
            if let Some((slot, sent)) = elected
                && now.duration_since(sent) > Duration::from_secs(HEADERS_RESPONSE_TIMEOUT_SECS)
            {
                eprintln!("  iouring-fetcher: elected peer {slot} headers-timeout, demoting");
                if let Some(peer) = self.peers.get_mut(slot) {
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                    peer.app.waiting_headers = false;
                }
                elected = None;
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

            // Election: pick a Ready peer and send getheaders.
            if elected.is_none()
                && let Some((slot, peer)) = self
                    .peers
                    .iter_mut()
                    .find(|(_, p)| p.state == PeerState::Ready && !p.app.waiting_headers)
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
                peer.app.waiting_headers = true;
                elected = Some((slot, Instant::now()));
            }

            // Re-arm recv.
            for (slot, peer) in self.peers.iter_mut() {
                if !peer.recv_armed
                    && peer.state != PeerState::Dead
                    && peer.state != PeerState::Connecting
                    && peer.fd.is_some()
                {
                    push_recv_multishot(
                        &mut self.ring,
                        slot,
                        peer.generation,
                        peer.fd.ok_or(FetchError::NoPeers)?,
                        PROVIDED_BUF_GROUP_ID,
                    )?;
                    peer.recv_armed = true;
                }
            }

            self.pbr.commit();
        }
    }

    // ── block download session ─────────────────────────────────────────

    #[allow(clippy::too_many_lines)]
    fn run_block_download_session(
        &mut self,
        needed: &[(BlockHeight, Hash256)],
    ) -> Result<(), FetchError> {
        self.ensure_peers()?;
        if self.peers.is_empty() {
            return Err(FetchError::PoolExhausted);
        }

        let mut pending: PrehashedMap<Hash256, BlockHeight> =
            HashMap::with_capacity_and_hasher(needed.len(), BuildPrehashedHasher);
        for (h, hash) in needed {
            pending.insert(*hash, *h);
        }

        let mut todo: VecDeque<Hash256> = needed.iter().map(|(_, h)| *h).collect();
        let mut in_flight: PrehashedMap<Hash256, InFlight> =
            HashMap::with_capacity_and_hasher(needed.len() / 4 + 16, BuildPrehashedHasher);
        let mut ewma = EwmaLatency::new();

        let mut saved = 0usize;
        let total = needed.len();
        let mut last_progress = Instant::now();
        let mut last_log = Instant::now();
        let mut saved_at_last_log = 0usize;
        let log_interval = Duration::from_secs(2);
        let version_payload = encode_version("/mantle-replay-iouring:0.1.0/");

        loop {
            push_tick(&mut self.ring, Duration::from_secs(1))?;
            self.ring.submit_and_wait(1).map_err(uring_err)?;

            let mut got_block_hashes: Vec<Hash256> = Vec::new();
            let mut drained = 0usize;
            while drained < CQE_DRAIN_CAP {
                let Some(cqe) = self.ring.complete() else {
                    break;
                };
                drained += 1;
                let (slot, op, g) = decode_user_data(cqe.user_data);
                if op == OP_TICK {
                    continue;
                }
                if op == OP_SEND {
                    self.send_arena.release(decode_send_seq(cqe.user_data));
                }
                if op == OP_LINK_TIMEOUT || op == OP_CLOSE {
                    continue;
                }
                let Some(peer) = self.peers.get_mut(slot) else {
                    continue;
                };
                if peer.generation != g {
                    continue;
                }
                handle_connect_or_recv(
                    &mut self.ring,
                    &mut self.pbr,
                    &mut self.send_arena,
                    self.network,
                    slot,
                    peer,
                    op,
                    cqe.result,
                    cqe.flags,
                    &version_payload,
                )?;
            }

            self.pbr.commit();

            // Parse frames: handshake + blocks.
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

            if !got_block_hashes.is_empty() {
                last_progress = Instant::now();
                for hash in &got_block_hashes {
                    if let Some(info) = in_flight.remove(hash) {
                        let arrival_latency = info.first_issued.elapsed();
                        ewma.observe(arrival_latency);
                        for &s in &info.issued_to {
                            if let Some(peer) = self.peers.get_mut(s) {
                                peer.app.in_flight.retain(|(h, _)| h != hash);
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

            if now.duration_since(last_log) >= log_interval {
                let window = now.duration_since(last_log).as_secs_f64().max(1e-9);
                #[allow(clippy::cast_precision_loss)]
                let bps = (saved - saved_at_last_log) as f64 / window;
                let ready = self
                    .peers
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

            // Hedge / reclaim.
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
                            peer.app.in_flight.retain(|(h, _)| h != &hash);
                        }
                    }
                }
                todo.push_front(hash);
            }

            // Timeout / black-hole detection.
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
                    && peer.app.assigned_count >= BLACK_HOLE_ASSIGNED_THRESHOLD
                    && peer.app.delivered_count == 0
                    && now.duration_since(peer.app.last_delivery)
                        >= Duration::from_secs(BLACK_HOLE_GRACE_SECS)
                {
                    eprintln!(
                        "  iouring-fetcher: peer slot={slot} ({}) is a black hole \
                         (assigned={} delivered=0 grace={}s) — evicting",
                        peer.addr, peer.app.assigned_count, BLACK_HOLE_GRACE_SECS,
                    );
                    close_peer(&mut self.ring, &mut self.send_arena, peer);
                    peer.state = PeerState::Dead;
                }
            }

            for slot in self.peers.dead_slots() {
                if let Some(p) = self.peers.remove(slot) {
                    for (h, _) in p.app.in_flight {
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
                        && peer.app.in_flight.len() < PEER_INFLIGHT_CAP
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
                    peer.app.in_flight.push((hash, now));
                    peer.app.assigned_count = peer.app.assigned_count.saturating_add(1);
                    if let Some(info) = in_flight.get_mut(&hash) {
                        info.issued_to.push(slot);
                        info.last_issued = now;
                    }
                }
            }

            // Issue new work to idle peers.
            for (slot, peer) in self.peers.iter_mut() {
                if peer.state != PeerState::Ready {
                    continue;
                }
                while peer.app.in_flight.len() < PEER_INFLIGHT_CAP {
                    let Some(hash) = todo.pop_front() else {
                        break;
                    };
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
                    peer.app.in_flight.push((hash, now));
                    peer.app.assigned_count = peer.app.assigned_count.saturating_add(1);
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

            // Re-arm recv.
            for (slot, peer) in self.peers.iter_mut() {
                if !peer.recv_armed
                    && peer.state != PeerState::Dead
                    && peer.state != PeerState::Connecting
                    && peer.fd.is_some()
                {
                    push_recv_multishot(
                        &mut self.ring,
                        slot,
                        peer.generation,
                        peer.fd.ok_or(FetchError::NoPeers)?,
                        PROVIDED_BUF_GROUP_ID,
                    )?;
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

// ── Block index I/O ────────────────────────────────────────────────────────

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

// ── Helpers ────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[allow(clippy::cast_sign_loss)]
fn file_to_rawfd(file: &fs::File) -> RawFd {
    use std::os::unix::io::AsRawFd;
    RawFd::from_raw(file.as_raw_fd() as usize)
}

#[allow(clippy::too_many_arguments)]
fn handle_connect_or_recv(
    ring: &mut IoUring,
    pbr: &mut ququmatz::ProvidedBufferRing,
    send_arena: &mut SendArena,
    network: Network,
    slot: u16,
    peer: &mut FPeer,
    op: u8,
    res: i32,
    cqe_flags: ququmatz::types::CqeFlags,
    version_payload: &[u8],
) -> Result<(), FetchError> {
    match op {
        OP_CONNECT => {
            if res < 0 {
                peer.state = PeerState::Dead;
                close_peer(ring, send_arena, peer);
                return Ok(());
            }
            push_recv_multishot(
                ring,
                slot,
                peer.generation,
                peer.fd.ok_or(FetchError::NoPeers)?,
                PROVIDED_BUF_GROUP_ID,
            )?;
            peer.recv_armed = true;
            let seq = push_send(
                ring,
                send_arena,
                network,
                Command::Version,
                version_payload,
                slot,
                peer.generation,
                peer.fd.ok_or(FetchError::NoPeers)?,
            )?;
            peer.pending_send_seqs.push(seq);
            peer.state = PeerState::HandshakeSendVersion;
            Ok(())
        }
        OP_SEND if res < 0 => {
            peer.state = PeerState::Dead;
            close_peer(ring, send_arena, peer);
            Ok(())
        }
        OP_RECV => {
            ingest_recv_cqe(pbr, send_arena, peer, cqe_flags, res, ring);
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
    peer: &mut FPeer,
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
                    peer.app.delivered_count = peer.app.delivered_count.saturating_add(1);
                    peer.app.last_delivery = Instant::now();
                    arrived.push(hash);
                }
            }
            (_, "ping") => {
                if peer.pending_send_seqs.len()
                    >= mantle_transport::PEER_SEND_OUTSTANDING_CAP as usize
                {
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

fn getheaders_payload(locator: Hash256) -> Vec<u8> {
    let msg = GetHeadersMsg {
        version: 70016,
        locator_hashes: vec![locator],
        stop_hash: Hash256::ZERO,
    };
    let mut buf = Vec::new();
    msg.encode(&mut buf)
        .expect("Vec<u8> Write is infallible");
    buf
}
