use std::io::Cursor;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};

use mantle::codec::Decode;
use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::network::Network;
use mantle::primitives::{Block, BlockHeader};
use mantle_fetcher::{BlockFeed, BlocksRequest, FedBlock, FeedItem, FeedSpec, HeaderRequest};
use delphi_oracle::DataDirLock;

use crate::chain_proof::ChainProof;
use crate::{BlockCache, BlockDigest, BlockPlan, BlockRecord, HeaderTail, RecentBlocks, RecordLog};

/// What one synchronization pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub tip: BlockHeight,
    /// The tip header's timestamp.
    pub tip_time: BlockTime,
    pub appended: usize,
    pub purged_cache: bool,
    pub caught_up: bool,
}

/// Where and how a [`ChainSync`] runs, without the fetcher it will own, and
/// the data directory lock the caller took before touching anything in it.
///
/// The fetcher holds `io_uring` pointers and cannot cross threads, so a worker
/// thread receives this description and opens the sync itself.
pub struct ChainSyncSpec {
    pub network: Network,
    pub peers: Vec<SocketAddrV4>,
    pub data_dir: PathBuf,
    pub depth: usize,
    pub cache_limit_bytes: u64,
    pub lock: DataDirLock,
}

impl ChainSyncSpec {
    /// # Errors
    /// As [`ChainSync::open`].
    pub fn open(self) -> Result<ChainSync, String> {
        ChainSync::open(
            self.network,
            self.peers,
            self.data_dir,
            self.depth,
            self.cache_limit_bytes,
            self.lock,
        )
    }
}

/// Keeps a [`RecordLog`] level with the chain tip.
///
/// The fetcher runs on a [`BlockFeed`] thread that lives as long as the
/// sync, so its peers are reused pass after pass, and it downloads the next
/// batch while this thread decodes, checks and records the current one. A
/// pass brings headers and the missing blocks in and turns each block into a
/// record. A pass stops asking for batches once the block files pass the
/// cache limit, and the next pass discards them first, since a record holds
/// everything pricing needs from its block. A report with `caught_up` false
/// asks for another pass. A feed whose thread has gone is started again on
/// the next pass.
///
/// The verified header chain is kept between passes, so a pass checks only
/// the headers that are new since the last one, and only its newest headers
/// are held: enough to fetch the history window and to verify what comes
/// next. That tail is saved after every pass and resumed on the next start,
/// so only a first start syncs headers from genesis. A pass that fails
/// resumes the next one from the saved tail.
pub struct ChainSync {
    network: Network,
    peers: Vec<SocketAddrV4>,
    data_dir: PathBuf,
    plan: BlockPlan,
    cache: BlockCache,
    log: RecordLog,
    recent: RecentBlocks,
    feed: Option<BlockFeed<ChainProof>>,
    proof: Option<ChainProof>,
    tail: HeaderTail,
    depth: u32,
    _lock: DataDirLock,
}

impl ChainSync {
    pub const REORG_OVERLAP: u32 = 6;
    pub const BATCH_BLOCKS: usize = 200;
    pub const BUFFERED_BLOCKS: usize = 16;

    /// Create `data_dir` if needed and take its lock.
    ///
    /// # Errors
    /// Returns a message if the directory cannot be created or another
    /// process holds its lock.
    pub fn lock(data_dir: &Path) -> Result<DataDirLock, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|error| format!("create {}: {error}", data_dir.display()))?;
        DataDirLock::acquire(data_dir).map_err(|error| format!("lock data directory: {error}"))
    }

    /// # Errors
    /// Returns a message if the record log cannot be opened.
    pub fn open(
        network: Network,
        peers: Vec<SocketAddrV4>,
        data_dir: impl Into<PathBuf>,
        depth: usize,
        cache_limit_bytes: u64,
        lock: DataDirLock,
    ) -> Result<Self, String> {
        let data_dir = data_dir.into();
        let log = RecordLog::open(data_dir.join("records.log"), depth)
            .map_err(|error| format!("open record log: {error}"))?;
        let tail = HeaderTail::new(&data_dir);
        let proof = tail
            .load(network)
            .map_err(|error| format!("read header tail: {error}"))?
            .map(ChainProof::resumed);
        Ok(Self {
            tail,
            proof,
            depth: u32::try_from(depth).unwrap_or(u32::MAX).saturating_add(Self::REORG_OVERLAP),
            network,
            peers,
            plan: BlockPlan::new(depth, Self::REORG_OVERLAP, Self::BATCH_BLOCKS),
            cache: BlockCache::new(data_dir.clone(), cache_limit_bytes),
            data_dir,
            log,
            recent: RecentBlocks::default(),
            feed: None,
            _lock: lock,
        })
    }

    #[must_use]
    pub const fn log(&self) -> &RecordLog {
        &self.log
    }

    /// The newest blocks read since the sync opened.
    #[must_use]
    pub const fn recent(&self) -> &RecentBlocks {
        &self.recent
    }

    #[must_use]
    pub const fn cache(&self) -> &BlockCache {
        &self.cache
    }

    /// Bring the record log up to the current chain tip.
    ///
    /// # Errors
    /// Returns a message if the fetcher cannot start, headers or blocks
    /// cannot be fetched or decoded, or a record cannot be stored.
    pub fn sync(&mut self) -> Result<SyncReport, String> {
        let mut feed = self.take_feed()?;
        let report = self.sync_with(&mut feed);
        if report.is_ok() {
            self.feed = Some(feed);
        }
        report
    }

    fn take_feed(&mut self) -> Result<BlockFeed<ChainProof>, String> {
        match self.feed.take() {
            Some(feed) => Ok(feed),
            None => BlockFeed::spawn(FeedSpec {
                network: self.network,
                peers: self.peers.clone(),
                cache_dir: self.data_dir.clone(),
                buffered: Self::BUFFERED_BLOCKS,
            })
            .map_err(|error| format!("start block feed: {error}")),
        }
    }

    fn sync_with(&mut self, feed: &mut BlockFeed<ChainProof>) -> Result<SyncReport, String> {
        let purged_cache = self.cache.exceeds_limit();
        if purged_cache {
            feed.discard().map_err(|error| format!("purge block cache: {error}"))?;
        }
        let admission = self.resume()?;
        let (headers, mut proof) = feed
            .headers(HeaderRequest {
                start: admission.start(),
                stop: BlockHeight::new(u32::MAX),
                locator: admission.locator(),
                admission,
            })
            .map_err(|error| format!("synchronize headers: {error}"))?;
        proof.finish(&headers)?;
        let report = self.record_span(feed, &proof, &headers, purged_cache)?;
        proof.keep_newest(self.depth);
        self.tail
            .save(proof.chain())
            .map_err(|error| format!("save header tail: {error}"))?;
        self.proof = Some(proof);
        Ok(report)
    }

    /// The chain kept from the last pass, else the saved tail, else genesis.
    fn resume(&mut self) -> Result<ChainProof, String> {
        if let Some(proof) = self.proof.take() {
            return Ok(proof);
        }
        match self
            .tail
            .load(self.network)
            .map_err(|error| format!("read header tail: {error}"))?
        {
            Some(chain) => Ok(ChainProof::resumed(chain)),
            None => ChainProof::new(self.network),
        }
    }

    fn record_span(
        &mut self,
        feed: &mut BlockFeed<ChainProof>,
        proof: &ChainProof,
        headers: &[(BlockHeight, BlockHeader)],
        purged_cache: bool,
    ) -> Result<SyncReport, String> {
        let (tip, tip_time) = headers
            .last()
            .map(|(height, header)| (*height, header.time()))
            .ok_or_else(|| "peers returned no headers".to_string())?;
        let recorded_tip = self.log.tip().map(BlockRecord::height);
        let Some(span) = self.plan.missing(recorded_tip, tip) else {
            return Ok(SyncReport {
                tip,
                tip_time,
                appended: 0,
                purged_cache,
                caught_up: true,
            });
        };
        feed.request_blocks(BlocksRequest {
            wanted: span.headers(headers).to_vec(),
            window: self.plan.batch(),
            discard_each_window: false,
            deliver: true,
        })?;
        let mut appended = 0;
        let mut stopping = false;
        loop {
            match feed.next_item() {
                Some(FeedItem::Block(block)) => {
                    let header = Self::header_at(headers, block.height)?;
                    self.record(proof, header, &block)?;
                    appended += 1;
                }
                Some(FeedItem::WindowDone { .. }) => {
                    if !stopping && self.cache.exceeds_limit() {
                        feed.stop_after_window();
                        stopping = true;
                    }
                }
                Some(FeedItem::Done) => break,
                Some(FeedItem::Failed(message)) => return Err(format!("fetch blocks: {message}")),
                Some(FeedItem::Headers(..)) => return Err("block feed sent headers mid-download".to_string()),
                None => return Err("block feed thread has gone".to_string()),
            }
        }
        Ok(SyncReport {
            tip,
            tip_time,
            appended,
            purged_cache,
            caught_up: self.log.tip().map(BlockRecord::height) == Some(tip),
        })
    }

    fn header_at(headers: &[(BlockHeight, BlockHeader)], height: BlockHeight) -> Result<BlockHeader, String> {
        let index = headers.partition_point(|(held, _)| *held < height);
        headers
            .get(index)
            .filter(|(held, _)| *held == height)
            .map(|(_, header)| *header)
            .ok_or_else(|| format!("block feed sent height {height} outside the plan"))
    }

    /// Decode `fed`, check it belongs to the verified chain, and record it.
    fn record(&mut self, proof: &ChainProof, header: BlockHeader, fed: &FedBlock) -> Result<(), String> {
        let height = fed.height;
        let block = Block::decode(&mut Cursor::new(fed.bytes.as_slice()))
            .map_err(|error| format!("decode block {height}: {error}"))?;
        proof.check_block(height, &block)?;
        let (record, tally) = BlockRecord::screened(height, header.time(), header.hash(), &block.transactions);
        self.recent
            .record(BlockDigest::of(height, &block, tally, record.amounts().to_vec()));
        self.log
            .append(record)
            .map_err(|error| format!("store record {height}: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::ChainSync;

    #[test]
    fn a_second_lock_on_one_directory_is_refused_until_the_first_is_dropped() {
        let dir = std::env::temp_dir().join(format!("chain-sync-lock-{}", std::process::id()));
        let first = ChainSync::lock(&dir).expect("first lock");

        assert!(ChainSync::lock(&dir).is_err());
        drop(first);
        assert!(ChainSync::lock(&dir).is_ok());
        std::fs::remove_dir_all(&dir).expect("clean up");
    }
}
