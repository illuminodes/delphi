use std::path::Path;

use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::TipInfo;

use crate::engine::flat::FlatEngine;
use crate::engine::Engine;
use crate::thread;
use crate::types::{
    CommitBlockReq, CommittedResp, HeaderKey, LookupCoinsResp, LookupResult, ReadHeaderReq,
    ReadHeaderResp, ReqToken, StorageError, StorageReq, StorageResp,
};

pub struct StorageHandle {
    req_tx: quetzalcoatl::mpsc::Producer<StorageReq>,
    resp_rx: quetzalcoatl::spsc::Consumer<StorageResp>,
    tip: TipInfo,
    next_token: u64,
}

impl StorageHandle {
    /// Open the storage at `data_dir`, spawning the storage thread.
    ///
    /// # Errors
    /// Returns `StorageError` if the engine cannot be opened or the thread cannot be spawned.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let engine = FlatEngine::open(data_dir).map_err(StorageError::from)?;
        let initial_tip = engine.tip();
        let handle = thread::spawn(engine)?;
        Ok(Self {
            req_tx: handle.req_tx,
            resp_rx: handle.resp_rx,
            tip: initial_tip,
            next_token: 1,
        })
    }

    /// Batch UTXO lookup.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn lookup_coins(&mut self, ops: &[OutPoint]) -> Result<Vec<LookupResult>, StorageError> {
        let token = self.next_token();
        self.send(StorageReq::LookupCoins(crate::types::LookupCoinsReq {
            token,
            outpoints: ops.to_vec(),
        }))?;
        match self.recv()? {
            StorageResp::LookupCoins(r) => r.map(|ok: LookupCoinsResp| ok.results),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    /// Read a header by hash.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn read_header(&mut self, hash: Hash256) -> Result<Option<BlockHeader>, StorageError> {
        self.read_header_inner(HeaderKey::ByHash(hash))
    }

    /// Read a header by height.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn read_header_at(&mut self, h: BlockHeight) -> Result<Option<BlockHeader>, StorageError> {
        self.read_header_inner(HeaderKey::ByHeight(h))
    }

    /// Commit a block to storage. Updates the locally-cached tip.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn commit_block(&mut self, req: CommitBlockReq) -> Result<CommittedResp, StorageError> {
        self.send(StorageReq::CommitBlock(req))?;
        match self.recv()? {
            StorageResp::CommitBlock(r) => {
                let resp = r?;
                self.tip = resp.tip.clone();
                Ok(resp)
            }
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    /// Last tip seen via a `commit_block` response, or the recovered tip on startup.
    pub fn tip(&self) -> TipInfo {
        self.tip.clone()
    }

    /// Gracefully shut down the storage thread.
    ///
    /// # Errors
    /// Returns `StorageError` if the shutdown request cannot be sent.
    pub fn shutdown(mut self) -> Result<(), StorageError> {
        self.send(StorageReq::Shutdown)?;
        match self.recv()? {
            StorageResp::Shutdown => Ok(()),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn next_token(&mut self) -> ReqToken {
        let t = ReqToken(self.next_token);
        self.next_token += 1;
        t
    }

    fn send(&self, req: StorageReq) -> Result<(), StorageError> {
        #[cfg(feature = "storage-dbg")]
        eprintln!("[h] send {}", match &req {
            StorageReq::LookupCoins(r) => format!("Lookup({} ops, tok={})", r.outpoints.len(), r.token.0),
            StorageReq::CommitBlock(r) => format!("Commit(h={}, tok={})", r.height.0, r.token.0),
            StorageReq::ReadHeader(r) => format!("ReadHdr(tok={})", r.token.0),
            StorageReq::Shutdown => "Shutdown".to_string(),
        });
        let r = self.req_tx.push(req).map_err(|_| StorageError::RingFull);
        #[cfg(feature = "storage-dbg")]
        eprintln!("[h] send done: {:?}", r.is_ok());
        r
    }

    fn recv(&mut self) -> Result<StorageResp, StorageError> {
        #[cfg(feature = "storage-dbg")]
        eprintln!("[h] recv waiting");
        let r = self.resp_rx.pop_block().ok_or(StorageError::ThreadGone);
        #[cfg(feature = "storage-dbg")]
        eprintln!("[h] recv got: {:?}", r.is_ok());
        r
    }

    fn read_header_inner(&mut self, key: HeaderKey) -> Result<Option<BlockHeader>, StorageError> {
        let token = self.next_token();
        self.send(StorageReq::ReadHeader(ReadHeaderReq { token, key }))?;
        match self.recv()? {
            StorageResp::ReadHeader(r) => r.map(|ok: ReadHeaderResp| ok.header),
            _ => Err(StorageError::UnexpectedResponse),
        }
    }
}
