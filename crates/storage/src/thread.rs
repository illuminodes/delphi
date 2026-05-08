use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::mpsc::RingBuffer as MpscRing;
use quetzalcoatl::spsc::RingBuffer as SpscRing;

use crate::engine::Engine;
use crate::engine::flat::FlatEngine;
use crate::types::{
    CommittedResp, HeaderKey, LookupCoinsResp, LookupResult, StorageError, StorageReq, StorageResp,
};

const RING_CAP: usize = 1024;

struct StorageWorker {
    req_rx: quetzalcoatl::mpsc::Consumer<StorageReq>,
    resp_tx: quetzalcoatl::spsc::Producer<StorageResp>,
    engine: FlatEngine,
}

impl StorageWorker {
    fn run(mut self) {
        let mut n: u64 = 0;
        while let Some(req) = self.req_rx.pop_block() {
            n += 1;
            let shutdown = matches!(req, StorageReq::Shutdown);
            #[cfg(feature = "storage-dbg")]
            eprintln!(
                "[w] req#{n} {}",
                match &req {
                    StorageReq::LookupCoins(r) =>
                        format!("Lookup({} ops, tok={})", r.outpoints.len(), r.token.0),
                    StorageReq::CommitBlock(r) =>
                        format!("Commit(h={}, tok={})", r.height.0, r.token.0),
                    StorageReq::ReadHeader(r) => format!("ReadHdr(tok={})", r.token.0),
                    StorageReq::Shutdown => "Shutdown".to_string(),
                }
            );
            let resp = self.dispatch(req);
            #[cfg(feature = "storage-dbg")]
            eprintln!("[w] resp#{n} pushing");
            if self.resp_tx.push(resp).is_err() {
                eprintln!("[w] BUG: resp ring full at req#{n}!");
            }
            #[cfg(feature = "storage-dbg")]
            eprintln!("[w] resp#{n} pushed");
            if shutdown {
                #[cfg(feature = "storage-dbg")]
                eprintln!("[w] shutdown received, exit");
                return;
            }
        }
        #[cfg(feature = "storage-dbg")]
        eprintln!("[w] req_rx returned None (producer dropped), exit n={n}");
    }

    fn dispatch(&mut self, req: StorageReq) -> StorageResp {
        match req {
            StorageReq::LookupCoins(r) => {
                let token = r.token;
                let mut results: Vec<LookupResult> = Vec::with_capacity(r.outpoints.len());
                let resp = self
                    .engine
                    .read_coins(&r.outpoints, &mut results)
                    .map(|()| LookupCoinsResp { token, results })
                    .map_err(StorageError::from);
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

pub(crate) struct StorageThreadHandle {
    pub(crate) req_tx: quetzalcoatl::mpsc::Producer<StorageReq>,
    pub(crate) resp_rx: quetzalcoatl::spsc::Consumer<StorageResp>,
}

pub(crate) fn spawn(engine: FlatEngine) -> Result<StorageThreadHandle, StorageError> {
    let (req_tx, req_rx) = MpscRing::new(Capacity::exact(RING_CAP)).split();
    let (resp_tx, resp_rx) = SpscRing::new(Capacity::exact(RING_CAP)).split();

    let worker = StorageWorker {
        req_rx,
        resp_tx,
        engine,
    };

    std::thread::Builder::new()
        .name("storage".into())
        .spawn(move || worker.run())
        .map_err(|e| StorageError::Io(e.kind()))?;

    Ok(StorageThreadHandle { req_tx, resp_rx })
}
