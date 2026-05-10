use std::ops::Deref;
use std::path::Path;

use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::TipInfo;

use crate::engine::Engine;
use crate::engine::flat::FlatEngine;
use crate::types::{
    CommitBlockReq, CommittedResp, HeaderKey, LookupCoinsResp, LookupResult, ReadHeaderReq,
    ReadHeaderResp, ReqToken, StorageError, StorageReq, StorageResp,
};

pub struct StorageHandle<'a, RReq = std::sync::Arc<quetzalcoatl::mpsc::RingBuffer<StorageReq<'a>>>, RResp = std::sync::Arc<quetzalcoatl::spsc::RingBuffer<StorageResp>>>
where
    RReq: Deref<Target = quetzalcoatl::mpsc::RingBuffer<StorageReq<'a>>>,
    RResp: Deref<Target = quetzalcoatl::spsc::RingBuffer<StorageResp>>,
{
    req_tx: quetzalcoatl::mpsc::Producer<StorageReq<'a>, RReq>,
    resp_rx: quetzalcoatl::spsc::Consumer<StorageResp, RResp>,
    tip: TipInfo,
    next_token: u64,
}

impl StorageHandle<'static> {
    /// Open the storage at `data_dir`, spawning the storage thread.
    ///
    /// # Errors
    /// Returns `StorageError` if the engine cannot be opened or the thread cannot be spawned.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        let engine = FlatEngine::open(data_dir).map_err(StorageError::from)?;
        let initial_tip = engine.tip();
        let handle = crate::thread::spawn(engine)?;
        Ok(Self {
            req_tx: handle.req_tx,
            resp_rx: handle.resp_rx,
            tip: initial_tip,
            next_token: 1,
        })
    }

    /// Commit a block whose `NewCoin` entries borrow from a non-`'static`
    /// source (e.g. a local `Block` or wire buffer).
    ///
    /// This is safe because `commit_block` is synchronous: the request is
    /// pushed onto the ring, the storage thread calls `write_block` (which
    /// copies all coin data including `script_pubkey` bytes to disk), and
    /// the response is received before this method returns. The borrowed
    /// data is fully consumed before the ring slot is reused.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn commit_block_borrowed(
        &mut self,
        req: CommitBlockReq<'_>,
    ) -> Result<CommittedResp, StorageError> {
        // SAFETY: The storage thread fully consumes the request
        // (writing all fields to disk) before sending the response.
        // `self.commit_block` blocks until that response arrives, so
        // the borrowed data is guaranteed to be live for the entire
        // duration the ring slot is occupied.
        let req_static: CommitBlockReq<'static> = unsafe {
            std::mem::transmute::<CommitBlockReq<'_>, CommitBlockReq<'static>>(req)
        };
        self.commit_block(req_static)
    }
}

impl<'a, RReq, RResp> StorageHandle<'a, RReq, RResp>
where
    RReq: Deref<Target = quetzalcoatl::mpsc::RingBuffer<StorageReq<'a>>>,
    RResp: Deref<Target = quetzalcoatl::spsc::RingBuffer<StorageResp>>,
{
    pub fn from_parts(
        req_tx: quetzalcoatl::mpsc::Producer<StorageReq<'a>, RReq>,
        resp_rx: quetzalcoatl::spsc::Consumer<StorageResp, RResp>,
        tip: TipInfo,
    ) -> Self {
        Self {
            req_tx,
            resp_rx,
            tip,
            next_token: 1,
        }
    }

    /// Batch UTXO lookup (send + recv in one call).
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn lookup_coins(&mut self, ops: &[OutPoint]) -> Result<Vec<LookupResult>, StorageError> {
        self.send_lookup(ops)?;
        self.recv_lookup()
    }

    /// Send a lookup request without waiting for the response.
    /// Must be followed by exactly one call to [`recv_lookup`].
    ///
    /// # Errors
    /// Returns `StorageError` if the ring is full.
    pub fn send_lookup(&mut self, ops: &[OutPoint]) -> Result<(), StorageError> {
        let token = self.next_token();
        self.send(StorageReq::LookupCoins(crate::types::LookupCoinsReq {
            token,
            outpoints: ops.to_vec(),
        }))
    }

    /// Receive the response from a prior [`send_lookup`] call.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn recv_lookup(&mut self) -> Result<Vec<LookupResult>, StorageError> {
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
    pub fn commit_block(&mut self, req: CommitBlockReq<'a>) -> Result<CommittedResp, StorageError> {
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

    fn next_token(&mut self) -> ReqToken {
        let t = ReqToken(self.next_token);
        self.next_token += 1;
        t
    }

    fn send(&self, req: StorageReq<'a>) -> Result<(), StorageError> {
        self.req_tx.push(req).map_err(|_| StorageError::RingFull)
    }

    fn recv(&mut self) -> Result<StorageResp, StorageError> {
        self.resp_rx.pop_block().ok_or(StorageError::ThreadGone)
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
