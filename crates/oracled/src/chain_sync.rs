use std::io::Cursor;
use std::net::SocketAddrV4;
use std::path::{Path, PathBuf};

use mantle::codec::Decode;
use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::network::Network;
use mantle::primitives::{Block, BlockHeader};
use mantle_fetcher::{
    BlockFeed, BlockStaging, BlocksRequest, FedBlock, FeedItem, FeedSpec, HeaderRequest, RecvBuffers, SurveyRequest,
};
use delphi_oracle::DataDirLock;

use crate::chain_proof::ChainProof;
use crate::{
    BlockDigest, BlockPlan, BlockRecord, ChainQuorum, HeaderTail, LegacyBlockFiles, RecentBlocks, RecordLog,
};

/// What one synchronization pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub tip: BlockHeight,
    /// The tip header's timestamp.
    pub tip_time: BlockTime,
    pub appended: usize,
    pub caught_up: bool,
    /// The highest height enough peers agree on, at least
    /// [`ChainQuorum::DEPTH`] below the tip; `None` while no height is.
    pub settled: Option<BlockHeight>,
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
    pub lock: DataDirLock,
}

impl ChainSyncSpec {
    /// # Errors
    /// As [`ChainSync::open`].
    pub fn open(self) -> Result<ChainSync, String> {
        ChainSync::open(self.network, self.peers, self.data_dir, self.depth, self.lock)
    }
}

/// Keeps a [`RecordLog`] level with the chain tip.
///
/// The fetcher runs on a [`BlockFeed`] thread that lives as long as the
/// sync, so its peers are reused pass after pass, and it downloads the next
/// batch while this thread decodes, checks and records the current one. A
/// pass brings headers and the missing blocks in and turns each block into a
/// record. Blocks are held in memory only, one batch downloading and at most
/// [`Self::BUFFERED_BLOCKS`] waiting, since a record holds everything
/// pricing needs from its block; nothing but records and the header tail
/// reaches the disk. A feed whose thread has gone is started again on the
/// next pass.
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
    log: RecordLog,
    recent: RecentBlocks,
    feed: Option<BlockFeed<ChainProof>>,
    proof: Option<ChainProof>,
    tail: HeaderTail,
    depth: u32,
    quorum: ChainQuorum,
    _lock: DataDirLock,
}

impl ChainSync {
    pub const REORG_OVERLAP: u32 = 6;
    /// Blocks downloaded together and held in memory until recorded: about
    /// 200 MB of mainnet blocks, enough work to spread across the peers.
    pub const BATCH_BLOCKS: usize = 128;
    pub const BUFFERED_BLOCKS: usize = 16;
    pub const SURVEY_SHOWN: usize = 64;

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
    /// Returns a message if the record log or header tail cannot be read,
    /// or block files left by an earlier version cannot be removed.
    pub fn open(
        network: Network,
        peers: Vec<SocketAddrV4>,
        data_dir: impl Into<PathBuf>,
        depth: usize,
        lock: DataDirLock,
    ) -> Result<Self, String> {
        let data_dir = data_dir.into();
        let freed = LegacyBlockFiles::remove(&data_dir)
            .map_err(|error| format!("remove old block files: {error}"))?;
        if freed > 0 {
            eprintln!("removed {} MB of block files from an earlier version", freed / 1_000_000);
        }
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
            quorum: ChainQuorum::new(ChainQuorum::DEPTH, ChainQuorum::GROUPS),
            depth: u32::try_from(depth).unwrap_or(u32::MAX).saturating_add(Self::REORG_OVERLAP),
            network,
            peers,
            plan: BlockPlan::new(depth, Self::REORG_OVERLAP, Self::BATCH_BLOCKS),
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
                recv_buffers: RecvBuffers::LIGHT,
            })
            .map_err(|error| format!("start block feed: {error}")),
        }
    }

    fn sync_with(&mut self, feed: &mut BlockFeed<ChainProof>) -> Result<SyncReport, String> {
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
        let mut report = self.record_span(feed, &proof, &headers)?;
        if report.caught_up {
            report.settled = self.settle(feed, &headers)?;
        }
        proof.keep_newest(self.depth);
        self.tail
            .save(proof.chain())
            .map_err(|error| format!("save header tail: {error}"))?;
        self.proof = Some(proof);
        Ok(report)
    }

    /// Show the peers our chain up to [`ChainQuorum::lookback`] below the
    /// tip, and settle the heights above that most of them answer with.
    fn settle(
        &self,
        feed: &mut BlockFeed<ChainProof>,
        headers: &[(BlockHeight, BlockHeader)],
    ) -> Result<Option<BlockHeight>, String> {
        let Some(&(tip, _)) = headers.last() else {
            return Ok(None);
        };
        let shown_to = BlockHeight::new(tip.to_u32().saturating_sub(self.quorum.lookback()));
        let end = headers.partition_point(|(height, _)| *height <= shown_to);
        let shown = &headers[end.saturating_sub(Self::SURVEY_SHOWN)..end];
        let Some(&(start, first)) = shown.first() else {
            return Ok(None);
        };
        let pages = feed
            .survey(SurveyRequest {
                start,
                base: first.prev_block(),
                held: shown.to_vec(),
            })
            .map_err(|error| format!("survey peers: {error}"))?;
        Ok(self.quorum.settled(headers, &pages))
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
                caught_up: true,
                settled: None,
            });
        };
        feed.request_blocks(BlocksRequest {
            wanted: span.headers(headers).to_vec(),
            window: self.plan.batch(),
            discard_each_window: false,
            deliver: true,
            staging: BlockStaging::Memory,
        })?;
        let mut appended = 0;
        loop {
            match feed.next_item() {
                Some(FeedItem::Block(block)) => {
                    let header = Self::header_at(headers, block.height)?;
                    self.record(proof, header, &block)?;
                    appended += 1;
                }
                Some(FeedItem::WindowDone { .. }) => {}
                Some(FeedItem::Done) => break,
                Some(FeedItem::Failed(message)) => return Err(format!("fetch blocks: {message}")),
                Some(FeedItem::Headers(..) | FeedItem::Survey(_)) => {
                    return Err("block feed answered out of turn mid-download".to_string());
                }
                None => return Err("block feed thread has gone".to_string()),
            }
        }
        Ok(SyncReport {
            tip,
            tip_time,
            appended,
            caught_up: self.log.tip().map(BlockRecord::height) == Some(tip),
            settled: None,
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
