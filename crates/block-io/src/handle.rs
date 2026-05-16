use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use mantle::codec::Decode;
use mantle::primitives::{Block, BlockHeader, BlockHeight, Hash256};
use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::RingBuffer as MpscRing;
use quetzalcoatl::spsc::RingBuffer as SpscRing;
use ququmatz::IoUring;

use crate::slab::{Slab, SlabWriter};
use crate::store::{BlockStore, HeaderStore};

pub enum FileReq {
    WriteBlock { height: BlockHeight, data: Vec<u8> },
    WriteBlockAsync { height: BlockHeight, data: Vec<u8> },
    ReadBlock { height: BlockHeight },
    ReadBlockDecoded { height: BlockHeight, expected_hash: Hash256 },
    ReadBlockSlab { height: BlockHeight },
    HasBlock { height: BlockHeight },
    WriteHeaders { start: BlockHeight, data: Vec<u8> },
    LoadCachedHeaders { start: BlockHeight },
    FlushBlockIndex,
    ReleaseSlab { slot: u16 },
    Shutdown,
}

pub enum FileResp {
    Ok,
    Bool(bool),
    BlockData(Vec<u8>),
    DecodedBlock(Block),
    SlabReady(crate::slab::SlabToken),
    Headers(Vec<(BlockHeight, BlockHeader)>),
    Err(io::Error),
}

const FILE_RING_CAP: usize = 4096;
const BLOCK_INDEX_FLUSH_INTERVAL: u64 = 2_000;

// ── Generic handle ───────────────────────────────────────────────────────────

pub struct FileIoHandle<
    RReq = Arc<MpscRing<FileReq>>,
    RResp = Arc<SpscRing<FileResp>>,
> where
    RReq: Deref<Target = MpscRing<FileReq>>,
    RResp: Deref<Target = SpscRing<FileResp>>,
{
    tx: quetzalcoatl::mpsc::Producer<FileReq, RReq>,
    rx: quetzalcoatl::spsc::Consumer<FileResp, RResp>,
}

impl<RReq, RResp> FileIoHandle<RReq, RResp>
where
    RReq: Deref<Target = MpscRing<FileReq>>,
    RResp: Deref<Target = SpscRing<FileResp>>,
{
    pub(crate) fn from_parts(
        tx: quetzalcoatl::mpsc::Producer<FileReq, RReq>,
        rx: quetzalcoatl::spsc::Consumer<FileResp, RResp>,
    ) -> Self {
        Self { tx, rx }
    }

    pub fn write_block(&mut self, height: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx
            .push(FileReq::WriteBlock { height, data })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    pub fn write_block_async(&self, height: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx
            .push(FileReq::WriteBlockAsync { height, data })
            .map_err(|_| io::Error::other("file-io ring full"))
    }

    pub fn read_block(&mut self, height: BlockHeight) -> io::Result<Vec<u8>> {
        self.tx
            .push(FileReq::ReadBlock { height })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::BlockData(data)) => Ok(data),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    pub fn read_block_decoded(
        &mut self,
        height: BlockHeight,
        expected_hash: Hash256,
    ) -> io::Result<Block> {
        self.tx
            .push(FileReq::ReadBlockDecoded {
                height,
                expected_hash,
            })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::DecodedBlock(block)) => Ok(block),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    /// Read block bytes into a slab slot and return a token.
    ///
    /// The caller uses [`SlabReader::handle`] with the token to get a
    /// zero-copy `BufHandle<'a>` borrowing the block bytes directly from
    /// the pre-allocated slab.  After processing, send the slot index
    /// back via [`release_slab`](Self::release_slab) so the worker can
    /// reuse it.
    pub fn read_block_slab(
        &mut self,
        height: BlockHeight,
    ) -> io::Result<crate::slab::SlabToken> {
        self.tx
            .push(FileReq::ReadBlockSlab { height })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::SlabReady(token)) => Ok(token),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    pub fn release_slab(&self, slot: u16) {
        let _ = self.tx.push(FileReq::ReleaseSlab { slot });
    }

    pub fn write_headers(&mut self, start: BlockHeight, data: Vec<u8>) -> io::Result<()> {
        self.tx
            .push(FileReq::WriteHeaders { start, data })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    pub fn flush_block_index(&mut self) -> io::Result<()> {
        self.tx
            .push(FileReq::FlushBlockIndex)
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Ok) => Ok(()),
            Some(FileResp::Err(e)) => Err(e),
            _ => Err(io::Error::other("file-io unexpected response")),
        }
    }

    pub fn has_block(&mut self, height: BlockHeight) -> bool {
        if self.tx.push(FileReq::HasBlock { height }).is_err() {
            return false;
        }
        matches!(self.rx.pop_block(), Some(FileResp::Bool(true)))
    }

    pub fn load_cached_headers(
        &mut self,
        start: BlockHeight,
    ) -> io::Result<Vec<(BlockHeight, BlockHeader)>> {
        self.tx
            .push(FileReq::LoadCachedHeaders { start })
            .map_err(|_| io::Error::other("file-io ring full"))?;
        match self.rx.pop_block() {
            Some(FileResp::Headers(h)) => Ok(h),
            Some(FileResp::Err(e)) => Err(e),
            _ => Ok(Vec::new()),
        }
    }

    pub fn shutdown(&mut self) {
        let _ = self.tx.push(FileReq::Shutdown);
        let _ = self.rx.pop_block();
    }
}

// ── Arc-based spawn (backward compat) ────────────────────────────────────────

impl FileIoHandle {
    pub fn spawn(data_dir: &Path) -> io::Result<Self> {
        let blocks = BlockStore::open(data_dir)?;
        let headers = HeaderStore::open(data_dir)?;
        let data_dir = data_dir.to_owned();

        let req_ring = MpscRing::new(Capacity::exact(FILE_RING_CAP));
        let resp_ring = SpscRing::new(Capacity::exact(FILE_RING_CAP));
        let (req_tx, req_rx) = req_ring.split();
        let (resp_tx, resp_rx) = resp_ring.split();

        // Leak a Slab for the 'static detached-thread path.
        let slab: &'static Slab = Box::leak(Box::new(Slab::new()));
        let (slab_writer, _slab_reader) = slab.split();

        std::thread::Builder::new()
            .name("file-io".into())
            .spawn(move || {
                FileIoWorker {
                    req_rx,
                    resp_tx,
                    blocks,
                    headers,
                    data_dir,
                    ring: IoUring::new(256).expect("io_uring"),
                    slab_writer,
                }
                .run();
            })
            .map_err(|e| io::Error::other(format!("spawn file-io: {e}")))?;

        Ok(Self::from_parts(req_tx, resp_rx))
    }
}

// ── Scoped-thread entry point ────────────────────────────────────────────────

pub fn with_file_io<F, R>(data_dir: &Path, f: F) -> io::Result<R>
where
    F: FnOnce(
        &mut FileIoHandle<&MpscRing<FileReq>, &SpscRing<FileResp>>,
        &crate::slab::SlabReader<'_>,
    ) -> R,
{
    let blocks = BlockStore::open(data_dir)?;
    let headers = HeaderStore::open(data_dir)?;
    let data_dir_owned = data_dir.to_owned();
    let slab = Slab::new();

    let req_ring: MpscRing<FileReq> = MpscRing::new(Capacity::exact(FILE_RING_CAP));
    let resp_ring: SpscRing<FileResp> = SpscRing::new(Capacity::exact(FILE_RING_CAP));

    let result = std::thread::scope(|s| {
        let (slab_writer, slab_reader) = slab.split();
        let (req_tx, req_rx) = req_ring.split_borrowed();
        let (resp_tx, resp_rx) = resp_ring.split_borrowed();

        s.spawn(|| {
            FileIoWorker {
                req_rx,
                resp_tx,
                blocks,
                headers,
                data_dir: data_dir_owned,
                ring: IoUring::new(256).expect("io_uring"),
                slab_writer,
            }
            .run();
        });

        let mut handle = FileIoHandle::from_parts(req_tx, resp_rx);
        f(&mut handle, &slab_reader)
    });

    Ok(result)
}

// ── Worker ───────────────────────────────────────────────────────────────────

struct FileIoWorker<
    'slab,
    RReq: Deref<Target = MpscRing<FileReq>>,
    RResp: Deref<Target = SpscRing<FileResp>>,
> {
    req_rx: quetzalcoatl::mpsc::Consumer<FileReq, RReq>,
    resp_tx: quetzalcoatl::spsc::Producer<FileResp, RResp>,
    blocks: BlockStore,
    headers: HeaderStore,
    data_dir: PathBuf,
    ring: IoUring,
    slab_writer: SlabWriter<'slab>,
}

#[allow(clippy::too_many_lines)]
impl<RReq, RResp> FileIoWorker<'_, RReq, RResp>
where
    RReq: Deref<Target = MpscRing<FileReq>>,
    RResp: Deref<Target = SpscRing<FileResp>>,
{
    fn run(mut self) {
        let mut writes_since_flush: u64 = 0;

        while let Some(req) = self.req_rx.pop_block() {
            let resp = match req {
                FileReq::WriteBlock { height, data } => {
                    match self.blocks.write(&mut self.ring, height, &data) {
                        Ok(()) => {
                            writes_since_flush += 1;
                            if writes_since_flush >= BLOCK_INDEX_FLUSH_INTERVAL {
                                let _ = self.blocks.flush_index(&self.data_dir);
                                writes_since_flush = 0;
                            }
                            FileResp::Ok
                        }
                        Err(e) => FileResp::Err(e),
                    }
                }
                FileReq::WriteBlockAsync { height, data } => {
                    let _ = self.blocks.write(&mut self.ring, height, &data);
                    writes_since_flush += 1;
                    if writes_since_flush >= BLOCK_INDEX_FLUSH_INTERVAL {
                        let _ = self.blocks.flush_index(&self.data_dir);
                        writes_since_flush = 0;
                    }
                    continue;
                }
                FileReq::ReadBlock { height } => match self.blocks.block_location(height) {
                    Ok((offset, length)) => {
                        match self.slab_writer.alloc() {
                            Some(slot) => {
                                match self.slab_writer.fill_from_disk(
                                    slot,
                                    self.blocks.file_store(),
                                    &mut self.ring,
                                    offset,
                                    length,
                                ) {
                                    Ok(_token) => {
                                        let data = self.slab_writer.slot_buf_mut(slot)
                                            [..length as usize]
                                            .to_vec();
                                        self.slab_writer.release(slot);
                                        FileResp::BlockData(data)
                                    }
                                    Err(e) => {
                                        self.slab_writer.release(slot);
                                        FileResp::Err(e)
                                    }
                                }
                            }
                            None => FileResp::Err(io::Error::other("slab full")),
                        }
                    }
                    Err(e) => FileResp::Err(e),
                },
                FileReq::ReadBlockDecoded {
                    height,
                    expected_hash,
                } => match self.blocks.block_location(height) {
                    Ok((offset, length)) => {
                        match self.slab_writer.alloc() {
                            Some(slot) => {
                                match self.slab_writer.fill_from_disk(
                                    slot,
                                    self.blocks.file_store(),
                                    &mut self.ring,
                                    offset,
                                    length,
                                ) {
                                    Ok(_token) => {
                                        let data = &self.slab_writer.slot_buf_mut(slot)
                                            [..length as usize];
                                        let result = Block::decode(
                                            &mut std::io::Cursor::new(data),
                                        );
                                        self.slab_writer.release(slot);
                                        match result {
                                            Ok(block) => {
                                                if block.header.hash() == expected_hash {
                                                    FileResp::DecodedBlock(block)
                                                } else {
                                                    FileResp::Err(io::Error::other("hash mismatch"))
                                                }
                                            }
                                            Err(e) => FileResp::Err(io::Error::other(
                                                format!("decode: {e}"),
                                            )),
                                        }
                                    }
                                    Err(e) => {
                                        self.slab_writer.release(slot);
                                        FileResp::Err(e)
                                    }
                                }
                            }
                            None => FileResp::Err(io::Error::other("slab full")),
                        }
                    }
                    Err(e) => FileResp::Err(e),
                },
                FileReq::ReadBlockSlab { height } => match self.blocks.block_location(height) {
                    Ok((offset, length)) => {
                        match self.slab_writer.alloc() {
                            Some(slot) => {
                                match self.slab_writer.fill_from_disk(
                                    slot,
                                    self.blocks.file_store(),
                                    &mut self.ring,
                                    offset,
                                    length,
                                ) {
                                    Ok(token) => FileResp::SlabReady(token),
                                    Err(e) => {
                                        self.slab_writer.release(slot);
                                        FileResp::Err(e)
                                    }
                                }
                            }
                            None => FileResp::Err(io::Error::other("slab full")),
                        }
                    }
                    Err(e) => FileResp::Err(e),
                },
                FileReq::HasBlock { height } => {
                    FileResp::Bool(self.blocks.has_block(height))
                }
                FileReq::WriteHeaders { start, data } => {
                    match self.headers.write_raw(&mut self.ring, start, &data) {
                        Ok(()) => FileResp::Ok,
                        Err(e) => FileResp::Err(e),
                    }
                }
                FileReq::LoadCachedHeaders { start } => {
                    match self.headers.load_cached(&mut self.ring, start) {
                        Ok(h) => FileResp::Headers(h),
                        Err(e) => FileResp::Err(e),
                    }
                }
                FileReq::FlushBlockIndex => match self.blocks.flush_index(&self.data_dir) {
                    Ok(()) => FileResp::Ok,
                    Err(e) => FileResp::Err(e),
                },
                FileReq::ReleaseSlab { slot } => {
                    self.slab_writer.release(slot);
                    continue;
                }
                FileReq::Shutdown => {
                    let _ = self.resp_tx.push(FileResp::Ok);
                    return;
                }
            };
            if self.resp_tx.push(resp).is_err() {
                eprintln!("[file-io] FAILED to push response!");
            }
        }
    }
}
