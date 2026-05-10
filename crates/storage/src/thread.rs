use std::ops::Deref;

use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::RingBuffer as MpscRing;
use quetzalcoatl::spsc::RingBuffer as SpscRing;

use crate::engine::Engine;
use crate::engine::flat::FlatEngine;
use crate::types::{
    CommittedResp, HeaderKey, LookupCoinsResp, LookupResult, StorageError, StorageReq, StorageResp,
};

const RING_CAP: usize = 1024;

pub(crate) struct StorageWorker<'a, RReq: Deref<Target = MpscRing<StorageReq<'a>>>, RResp: Deref<Target = SpscRing<StorageResp>>> {
    req_rx: quetzalcoatl::mpsc::Consumer<StorageReq<'a>, RReq>,
    resp_tx: quetzalcoatl::spsc::Producer<StorageResp, RResp>,
    engine: Box<dyn Engine + Send>,
}

impl<'a, RReq: Deref<Target = MpscRing<StorageReq<'a>>>, RResp: Deref<Target = SpscRing<StorageResp>>> StorageWorker<'a, RReq, RResp> {
    pub(crate) fn run(mut self) {
        while let Some(req) = self.req_rx.pop_block() {
            let shutdown = matches!(req, StorageReq::Shutdown);
            let resp = self.dispatch(req);
            if self.resp_tx.push(resp).is_err() {
                eprintln!("[w] BUG: resp ring full!");
            }
            if shutdown {
                return;
            }
        }
    }

    fn dispatch(&mut self, req: StorageReq<'a>) -> StorageResp {
        match req {
            StorageReq::LookupCoins(r) => {
                let token = r.token;
                let mut results: Vec<LookupResult> = Vec::with_capacity(r.outpoints.len());
                let resp = self
                    .engine
                    .read_coins(&r.outpoints, &mut results)
                    .map(|()| LookupCoinsResp { token, results })
                    .map_err(|e| {
                        eprintln!("[w] read_coins error: {e}");
                        StorageError::from(e)
                    });
                StorageResp::LookupCoins(resp)
            }
            StorageReq::CommitBlock(r) => {
                let token = r.token;
                let durability = r.durability;
                let resp = self
                    .engine
                    .write_block(&r)
                    .and_then(|()| self.engine.flush(durability))
                    .map(|()| CommittedResp {
                        token,
                        tip: self.engine.tip(),
                    })
                    .map_err(StorageError::from);
                StorageResp::CommitBlock(resp)
            }
            StorageReq::ReadHeader(r) => {
                let token = r.token;
                let resp = match r.key {
                    HeaderKey::ByHash(h) => self.engine.read_header(&h),
                    HeaderKey::ByHeight(h) => self.engine.read_header_at(h),
                }
                .map(|header| crate::types::ReadHeaderResp { token, header })
                .map_err(StorageError::from);
                StorageResp::ReadHeader(resp)
            }
            StorageReq::Shutdown => StorageResp::Shutdown,
        }
    }
}

// ── Arc-based spawn (existing 'static path) ──────────────────────────────────

pub(crate) struct StorageThreadHandle {
    pub(crate) req_tx: quetzalcoatl::mpsc::Producer<StorageReq<'static>>,
    pub(crate) resp_rx: quetzalcoatl::spsc::Consumer<StorageResp>,
}

pub(crate) fn spawn(engine: impl Engine + Send + 'static) -> Result<StorageThreadHandle, StorageError> {
    let (req_tx, req_rx) = MpscRing::new(Capacity::exact(RING_CAP)).split();
    let (resp_tx, resp_rx) = SpscRing::new(Capacity::exact(RING_CAP)).split();

    let worker = StorageWorker {
        req_rx,
        resp_tx,
        engine: Box::new(engine),
    };

    std::thread::Builder::new()
        .name("storage".into())
        .spawn(move || worker.run())
        .map_err(|e| StorageError::Io(e.kind()))?;

    Ok(StorageThreadHandle { req_tx, resp_rx })
}

// ── Borrowed spawn (scoped-thread path for zero-copy replay) ─────────────────

pub(crate) fn make_worker_borrowed<'a>(
    engine: FlatEngine,
    req_ring: &'a MpscRing<StorageReq<'a>>,
    resp_ring: &'a SpscRing<StorageResp>,
) -> (
    StorageWorker<'a, &'a MpscRing<StorageReq<'a>>, &'a SpscRing<StorageResp>>,
    quetzalcoatl::mpsc::Producer<StorageReq<'a>, &'a MpscRing<StorageReq<'a>>>,
    quetzalcoatl::spsc::Consumer<StorageResp, &'a SpscRing<StorageResp>>,
) {
    let (req_tx, req_rx) = req_ring.split_borrowed();
    let (resp_tx, resp_rx) = resp_ring.split_borrowed();
    let worker = StorageWorker {
        req_rx,
        resp_tx,
        engine: Box::new(engine),
    };
    (worker, req_tx, resp_rx)
}
