use std::io;
use std::path::Path;

use mantle::primitives::{BlockHeader, BlockHeight};
use ququmatz::IoUring;

use crate::store::{BlockStore, HeaderStore};

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
const BLOCK_INDEX_FLUSH_INTERVAL: u64 = 2_000;

pub struct FileIoHandle {
    tx: quetzalcoatl::mpsc::Producer<FileReq>,
    rx: quetzalcoatl::spsc::Consumer<FileResp>,
}

impl FileIoHandle {
    pub fn spawn(data_dir: &Path) -> io::Result<Self> {
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
                Err(e) => FileResp::Err(e),
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
