use std::net::SocketAddrV4;
use std::path::PathBuf;

use mantle::difficulty::BlockHeight;
use mantle::network::Network;
use mantle_fetcher::Fetcher;
use mantle_oracle::DataDirLock;

use crate::{BlockCache, BlockPlan, BlockRecord, RecordLog};

/// What one synchronization pass did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncReport {
    pub tip: BlockHeight,
    pub appended: usize,
    pub purged_cache: bool,
    pub caught_up: bool,
}

/// Keeps a [`RecordLog`] level with the chain tip.
///
/// One fetcher serves many passes, because each new fetcher leaks a 16 MiB
/// slab inside the block-io crate. It is dropped and rebuilt only after a
/// failed pass or a cache purge. A pass brings headers and the missing blocks
/// in and turns each block into a record. A pass ends early once the block
/// files pass the cache limit, and the next pass deletes them first, since a
/// record holds everything pricing needs from its block. A report with
/// `caught_up` false asks for another pass.
pub struct ChainSync {
    network: Network,
    peers: Vec<SocketAddrV4>,
    data_dir: PathBuf,
    plan: BlockPlan,
    cache: BlockCache,
    log: RecordLog,
    fetcher: Option<Fetcher>,
    _lock: DataDirLock,
}

impl ChainSync {
    pub const REORG_OVERLAP: u32 = 6;
    pub const BATCH_BLOCKS: usize = 48;

    /// # Errors
    /// Returns a message if the data directory cannot be created or locked,
    /// or the record log cannot be opened.
    pub fn open(
        network: Network,
        peers: Vec<SocketAddrV4>,
        data_dir: impl Into<PathBuf>,
        depth: usize,
        cache_limit_bytes: u64,
    ) -> Result<Self, String> {
        let data_dir = data_dir.into();
        std::fs::create_dir_all(&data_dir)
            .map_err(|error| format!("create {}: {error}", data_dir.display()))?;
        let lock = DataDirLock::acquire(&data_dir)
            .map_err(|error| format!("lock data directory: {error}"))?;
        let log = RecordLog::open(data_dir.join("records.log"), depth)
            .map_err(|error| format!("open record log: {error}"))?;
        Ok(Self {
            network,
            peers,
            plan: BlockPlan::new(depth, Self::REORG_OVERLAP, Self::BATCH_BLOCKS),
            cache: BlockCache::new(data_dir.clone(), cache_limit_bytes),
            data_dir,
            log,
            fetcher: None,
            _lock: lock,
        })
    }

    #[must_use]
    pub const fn log(&self) -> &RecordLog {
        &self.log
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
        let purged_cache = self.purge_if_full()?;
        let mut fetcher = self.take_fetcher()?;
        let report = self.sync_with(&mut fetcher, purged_cache)?;
        self.fetcher = Some(fetcher);
        Ok(report)
    }

    fn take_fetcher(&mut self) -> Result<Fetcher, String> {
        match self.fetcher.take() {
            Some(fetcher) => Ok(fetcher),
            None => Fetcher::new(self.network, self.peers.clone(), &self.data_dir)
                .map_err(|error| format!("initialize fetcher: {error}")),
        }
    }

    fn sync_with(&mut self, fetcher: &mut Fetcher, purged_cache: bool) -> Result<SyncReport, String> {
        let headers = fetcher
            .sync_headers(
                BlockHeight::new(1),
                BlockHeight::new(u32::MAX),
                self.network.genesis_hash(),
            )
            .map_err(|error| format!("synchronize headers: {error}"))?;
        let tip = headers
            .last()
            .map(|(height, _)| *height)
            .ok_or_else(|| "peers returned no headers".to_string())?;
        let recorded_tip = self.log.tip().map(BlockRecord::height);
        let Some(span) = self.plan.missing(recorded_tip, tip) else {
            return Ok(SyncReport {
                tip,
                appended: 0,
                purged_cache,
                caught_up: true,
            });
        };
        let mut appended = 0;
        for batch in self.plan.batches(span) {
            if self.cache.exceeds_limit() {
                break;
            }
            let wanted = batch.headers(&headers);
            fetcher
                .fetch_blocks(wanted)
                .map_err(|error| format!("fetch blocks: {error}"))?;
            for (height, header) in wanted {
                let block = fetcher
                    .read_block_decoded(*height, header.hash())
                    .map_err(|error| format!("decode block {height}: {error}"))?;
                let record =
                    BlockRecord::from_transactions(*height, header.time(), &block.transactions);
                self.log
                    .append(record)
                    .map_err(|error| format!("store record {height}: {error}"))?;
                appended += 1;
            }
        }
        Ok(SyncReport {
            tip,
            appended,
            purged_cache,
            caught_up: self.log.tip().map(BlockRecord::height) == Some(tip),
        })
    }

    fn purge_if_full(&mut self) -> Result<bool, String> {
        if !self.cache.exceeds_limit() {
            return Ok(false);
        }
        self.fetcher = None;
        self.cache
            .purge()
            .map_err(|error| format!("purge block cache: {error}"))?;
        Ok(true)
    }
}
