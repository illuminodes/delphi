use std::path::Path;

use ququmatz::EventFd;

use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::TipInfo;

use crate::engine::flat::FlatEngine;
use crate::engine::Engine;
use crate::thread;
use crate::types::{
    CommitBlockReq, CommittedResp, HeaderKey, LookupCoinsResp, LookupResult, ReadHeaderReq,
    ReadHeaderResp, ReqToken, StorageError, StorageReq, StorageResp,
};

/// Public API for submitting requests to the storage thread.
///
/// Each call takes the next monotonic token, pushes a request, signals the
/// thread via the eventfd, then spins on the response ring until the reply
/// arrives. v1 issues one in-flight request at a time — no shared state,
/// no locks.
pub struct StorageHandle {
    req_tx: quetzalcoatl::mpsc::Producer<StorageReq>,
    resp_rx: quetzalcoatl::spsc::Consumer<StorageResp>,
    /// Owned eventfd; handle writes 1 to wake the storage thread.
    wake_efd: EventFd,
    /// Last known tip, updated from every `CommittedResp`.
    tip: TipInfo,
    next_token: u64,
}

impl StorageHandle {
    /// Open the storage at `data_dir`, spawning the storage thread.
    ///
    /// # Errors
    /// Returns `StorageError` if the engine cannot be opened or the thread cannot be spawned.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let engine =
            FlatEngine::open(data_dir).map_err(|e| StorageError::new(e.to_string()))?;

        // Snapshot the tip that recover() left in the engine before handing it off.
        let initial_tip = engine.tip();

        let wake_efd = EventFd::new(0)
            .map_err(|e| StorageError::new(format!("eventfd: {e:?}")))?;
        let raw_wake_fd = wake_efd.fd();

        let handle = thread::spawn(engine, raw_wake_fd)?;

        Ok(Self {
            req_tx: handle.req_tx,
            resp_rx: handle.resp_rx,
            wake_efd,
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
            other => Err(StorageError::new(format!("unexpected response: {other:?}"))),
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
    pub fn read_header_at(
        &mut self,
        h: BlockHeight,
    ) -> Result<Option<BlockHeader>, StorageError> {
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
            other => Err(StorageError::new(format!("unexpected response: {other:?}"))),
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
        loop {
            if let Some(resp) = self.resp_rx.pop()
                && matches!(resp, StorageResp::Shutdown)
            {
                return Ok(());
            }
            std::hint::spin_loop();
        }
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn next_token(&mut self) -> ReqToken {
        let t = ReqToken(self.next_token);
        self.next_token += 1;
        t
    }

    fn send(&self, req: StorageReq) -> Result<(), StorageError> {
        self.req_tx
            .push(req)
            .map_err(|_| StorageError::new("request ring full"))?;
        self.wake_efd
            .write(1)
            .map_err(|e| StorageError::new(format!("eventfd write: {e:?}")))?;
        Ok(())
    }

    fn recv(&mut self) -> Result<StorageResp, StorageError> {
        loop {
            if let Some(resp) = self.resp_rx.pop() {
                return Ok(resp);
            }
            std::hint::spin_loop();
        }
    }

    fn read_header_inner(
        &mut self,
        key: HeaderKey,
    ) -> Result<Option<BlockHeader>, StorageError> {
        let token = self.next_token();
        self.send(StorageReq::ReadHeader(ReadHeaderReq { token, key }))?;
        match self.recv()? {
            StorageResp::ReadHeader(r) => r.map(|ok: ReadHeaderResp| ok.header),
            other => Err(StorageError::new(format!("unexpected response: {other:?}"))),
        }
    }
}
