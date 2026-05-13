use std::ops::Deref;
use std::path::Path;

use mantle::primitives::{BlockHeader, BlockHeight, Hash256, OutPoint};
use mantle_consensus::chain_view::TipInfo;

use crate::types::{
    CommitBlockReq, CommittedResp, HeaderKey, LookupResult, ReadHeaderReq, ReadHeaderResp,
    ReqToken, StorageError, StorageReq, StorageResp,
};

pub struct StorageHandle<
    'a,
    RReq = std::sync::Arc<quetzalcoatl::mpsc::RingBuffer<StorageReq<'a>>>,
    RResp = std::sync::Arc<quetzalcoatl::spsc::RingBuffer<StorageResp>>,
> where
    RReq: Deref<Target = quetzalcoatl::mpsc::RingBuffer<StorageReq<'a>>>,
    RResp: Deref<Target = quetzalcoatl::spsc::RingBuffer<StorageResp>>,
{
    req_tx: quetzalcoatl::mpsc::Producer<StorageReq<'a>, RReq>,
    resp_rx: quetzalcoatl::spsc::Consumer<StorageResp, RResp>,
    tip: TipInfo,
    next_token: u64,
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
    pub fn lookup_coins(
        &mut self,
        ops: &'a [OutPoint],
        out: &'a mut Vec<LookupResult>,
    ) -> Result<(), StorageError> {
        self.send_lookup(ops, out)?;
        self.recv_lookup()
    }

    /// Send a lookup request without waiting for the response.
    /// Must be followed by exactly one call to [`recv_lookup`] with the same `out`.
    ///
    /// # Errors
    /// Returns `StorageError` if the ring is full.
    pub fn send_lookup(
        &mut self,
        ops: &'a [OutPoint],
        out: &'a mut Vec<LookupResult>,
    ) -> Result<(), StorageError> {
        let token = self.next_token();
        self.send(StorageReq::LookupCoins(crate::types::LookupCoinsReq {
            token,
            outpoints: ops,
            out,
        }))
    }

    /// Batch UTXO lookup with locally-borrowed ops/out slices.
    ///
    /// Safe because `recv_lookup` blocks until the worker is done with both
    /// pointers before returning; the transmutes are erased before that point.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn lookup_coins_borrowed(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), StorageError> {
        self.send_lookup_borrowed(ops, out)?;
        self.recv_lookup()
    }

    /// Send a lookup request with locally-borrowed ops/out without waiting for
    /// the response. Must be followed by exactly one call to [`recv_lookup`].
    ///
    /// # Errors
    /// Returns `StorageError` if the ring is full.
    pub fn send_lookup_borrowed(
        &mut self,
        ops: &[OutPoint],
        out: &mut Vec<LookupResult>,
    ) -> Result<(), StorageError> {
        // SAFETY: Both `ops` and `out` outlive this call. `recv_lookup` blocks
        // until the worker signals completion, so the worker never touches
        // either borrow after recv_lookup returns. The send+recv pair is
        // synchronous from the caller's perspective — data is consumed before
        // the ring slot is reused.
        let req: crate::types::LookupCoinsReq<'_> = crate::types::LookupCoinsReq {
            token: self.next_token(),
            outpoints: ops,
            out,
        };
        let req_static: crate::types::LookupCoinsReq<'static> = unsafe {
            std::mem::transmute::<crate::types::LookupCoinsReq<'_>, crate::types::LookupCoinsReq<'static>>(req)
        };
        self.send_raw(StorageReq::LookupCoins(req_static))
    }

    /// Receive the response from a prior [`send_lookup`] call.
    /// The output buffer passed to `send_lookup` is populated on return.
    ///
    /// # Errors
    /// Returns `StorageError` on ring or I/O failure.
    pub fn recv_lookup(&mut self) -> Result<(), StorageError> {
        match self.recv()? {
            StorageResp::LookupCoins(r) => r.map(|_| ()),
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

    /// Commit a block whose `NewCoin` entries borrow from a non-`'a`
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
        let req_static: CommitBlockReq<'static> =
            unsafe { std::mem::transmute::<CommitBlockReq<'_>, CommitBlockReq<'static>>(req) };
        // We need to send the static version and receive a response.
        // Use a raw send so we don't fight the lifetime checker.
        self.send_raw(StorageReq::CommitBlock(req_static))?;
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

    /// Send a request whose lifetime has been erased to `'static` via transmute.
    /// Only call this from methods that guarantee synchronous send+recv.
    fn send_raw(&self, req: StorageReq<'static>) -> Result<(), StorageError> {
        // SAFETY: callers ensure the borrowed data outlives the synchronous
        // send+recv round-trip. The worker never touches the data after
        // sending its response.
        let req_a: StorageReq<'a> =
            unsafe { std::mem::transmute::<StorageReq<'static>, StorageReq<'a>>(req) };
        self.req_tx.push(req_a).map_err(|_| StorageError::RingFull)
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

// ── Convenience opener (kept for tests / one-off tools; prefer with_storage) ──

impl StorageHandle<'static> {
    /// Open the storage at `data_dir`, spawning the storage thread.
    ///
    /// Prefer [`crate::thread::with_storage`] for new code — it gives the
    /// compiler real lifetime information and avoids the `'static` transmute.
    ///
    /// # Errors
    /// Returns `StorageError` if the engine cannot be opened or the thread cannot be spawned.
    pub fn open(data_dir: &Path) -> Result<Self, StorageError> {
        use crate::engine::flat::FlatEngine;
        let engine = FlatEngine::open(data_dir).map_err(StorageError::from)?;
        open_arc(engine)
    }

    /// Open with the Robin Hood engine.
    ///
    /// # Errors
    /// Returns `StorageError` if the engine cannot be opened or the thread cannot be spawned.
    pub fn open_robin_hood(data_dir: &Path) -> Result<Self, StorageError> {
        use crate::engine::robin_hood::RobinHoodEngine;
        let engine = RobinHoodEngine::open(data_dir).map_err(StorageError::from)?;
        open_arc(engine)
    }
}

fn open_arc(engine: impl crate::engine::Engine + Send + 'static) -> Result<StorageHandle<'static>, StorageError> {
    use quetzalcoatl::capacity::Capacity;
    use quetzalcoatl::mpsc::RingBuffer as MpscRing;
    use quetzalcoatl::spsc::RingBuffer as SpscRing;
    use crate::thread::StorageWorker;

    const RING_CAP: usize = 1024;
    let initial_tip = engine.tip();

    // `.split()` wraps the ring in an Arc internally; the resulting
    // Producer/Consumer use Arc<RingBuffer<T>> as their ring accessor,
    // which matches the StorageHandle<'static> default type parameters.
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

    Ok(StorageHandle::from_parts(req_tx, resp_rx, initial_tip))
}
