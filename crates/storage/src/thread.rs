use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::RingBuffer as MpscRing;
use quetzalcoatl::spsc::RingBuffer as SpscRing;
use ququmatz::types::RawFd;
use ququmatz::{IoUring, Sqe};

use crate::engine::flat::FlatEngine;
use crate::engine::Engine;
use crate::types::{
    CommittedResp, HeaderKey, LookupCoinsResp, LookupResult, ReadHeaderResp, StorageError,
    StorageReq, StorageResp,
};

const RING_CAP: usize = 1024;

/// All send-safe pieces that the storage thread needs. `IoUring` is not `Send`
/// so it is created inside the spawned closure, on the thread itself.
struct StorageWorker {
    req_rx: quetzalcoatl::mpsc::Consumer<StorageReq>,
    resp_tx: quetzalcoatl::spsc::Producer<StorageResp>,
    engine: FlatEngine,
    /// Raw eventfd fd owned by `StorageHandle`. Thread reads from it via
    /// `io_uring` to receive wakeup signals; the fd is closed by the handle on
    /// drop (no double-close here — we only use the integer to build SQEs).
    wake_fd: RawFd,
}

impl StorageWorker {
    fn run(mut self) {
        let Ok(mut ring) = IoUring::new(16) else { return };

        let mut wake_buf = [0u8; 8];
        let sqe = Sqe::read(self.wake_fd, &mut wake_buf, 0).user_data(0);
        if ring.push(sqe).is_err() {
            return;
        }

        loop {
            while let Some(req) = self.req_rx.pop() {
                let shutdown = matches!(req, StorageReq::Shutdown);
                let resp = self.dispatch(req);
                let _ = self.resp_tx.push(resp);
                if shutdown {
                    return;
                }
            }

            if ring.submit_and_wait(1).is_err() {
                return;
            }
            while ring.complete().is_some() {
                let sqe = Sqe::read(self.wake_fd, &mut wake_buf, 0).user_data(0);
                let _ = ring.push(sqe);
            }
        }
    }

    fn dispatch(&mut self, req: StorageReq) -> StorageResp {
        match req {
            StorageReq::LookupCoins(r) => {
                let token = r.token;
                let mut results: Vec<LookupResult> = Vec::with_capacity(r.outpoints.len());
                match self.engine.read_coins(&r.outpoints, &mut results) {
                    Ok(()) => StorageResp::LookupCoins(Ok(LookupCoinsResp { token, results })),
                    Err(e) => StorageResp::LookupCoins(Err(StorageError::new(e.to_string()))),
                }
            }
            StorageReq::CommitBlock(r) => {
                let token = r.token;
                let durability = r.durability;
                match self.engine.write_block(&r) {
                    Ok(()) => {
                        if let Err(e) = self.engine.flush(durability) {
                            return StorageResp::CommitBlock(Err(StorageError::new(e.to_string())));
                        }
                        let tip = self.engine.tip();
                        StorageResp::CommitBlock(Ok(CommittedResp { token, tip }))
                    }
                    Err(e) => StorageResp::CommitBlock(Err(StorageError::new(e.to_string()))),
                }
            }
            StorageReq::ReadHeader(r) => {
                let token = r.token;
                let result = match r.key {
                    HeaderKey::ByHash(h) => self.engine.read_header(&h),
                    HeaderKey::ByHeight(h) => self.engine.read_header_at(h),
                };
                match result {
                    Ok(header) => StorageResp::ReadHeader(Ok(ReadHeaderResp { token, header })),
                    Err(e) => StorageResp::ReadHeader(Err(StorageError::new(e.to_string()))),
                }
            }
            StorageReq::Shutdown => StorageResp::Shutdown,
        }
    }
}

/// Producer-side handle returned from `spawn`.
pub(crate) struct StorageThreadHandle {
    pub(crate) req_tx: quetzalcoatl::mpsc::Producer<StorageReq>,
    pub(crate) resp_rx: quetzalcoatl::spsc::Consumer<StorageResp>,
}

/// Spawn the storage thread. `wake_fd` is the raw fd of the `EventFd` owned
/// by the caller; the thread reads from it to receive wakeups.
pub(crate) fn spawn(
    engine: FlatEngine,
    wake_fd: RawFd,
) -> Result<StorageThreadHandle, StorageError> {
    let (req_tx, req_rx) = MpscRing::new(Capacity::exact(RING_CAP)).split();
    let (resp_tx, resp_rx) = SpscRing::new(Capacity::exact(RING_CAP)).split();

    let worker = StorageWorker {
        req_rx,
        resp_tx,
        engine,
        wake_fd,
    };

    std::thread::Builder::new()
        .name("storage".into())
        .spawn(move || worker.run())
        .map_err(|e| StorageError::new(format!("spawn: {e}")))?;

    Ok(StorageThreadHandle { req_tx, resp_rx })
}
