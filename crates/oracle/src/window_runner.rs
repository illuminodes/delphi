use std::net::SocketAddrV4;
use std::path::PathBuf;

use mantle::difficulty::BlockHeight;
use mantle::network::Network;
use mantle_fetcher::Fetcher;

use crate::data_dir_lock::DataDirLock;
use crate::{BlockRange, OraclePipeline, OracleReport, RangeSelection};

pub struct WindowRunner {
    network: Network,
    data_dir: PathBuf,
    peers: Vec<SocketAddrV4>,
}

impl WindowRunner {
    #[must_use]
    pub fn new(network: Network, data_dir: impl Into<PathBuf>, peers: Vec<SocketAddrV4>) -> Self {
        Self {
            network,
            data_dir: data_dir.into(),
            peers,
        }
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn run(&self, selection: RangeSelection) -> Result<OracleReport, String> {
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|error| format!("create {}: {error}", self.data_dir.display()))?;
        let _lock = DataDirLock::acquire(&self.data_dir)
            .map_err(|error| format!("lock data directory: {error}"))?;
        let mut fetcher = Fetcher::new(self.network, self.peers.clone(), &self.data_dir)
            .map_err(|error| format!("initialize fetcher: {error}"))?;
        self.analyze(&mut fetcher, selection)
    }

    fn analyze(
        &self,
        fetcher: &mut Fetcher,
        selection: RangeSelection,
    ) -> Result<OracleReport, String> {
        let headers = fetcher
            .sync_headers(
                BlockHeight::new(1),
                BlockHeight::new(u32::MAX),
                self.network.genesis_hash(),
            )
            .map_err(|error| format!("synchronize headers: {error}"))?;
        let range = BlockRange::select(&headers, selection)
            .map_err(|error| format!("select block range: {error}"))?;
        let selected_headers = range.headers(&headers);
        eprintln!(
            "window {}..={} ({} blocks)",
            range.start(),
            range.stop(),
            selected_headers.len()
        );
        fetcher
            .fetch_blocks(selected_headers)
            .map_err(|error| format!("fetch blocks: {error}"))?;
        let mut pipeline = OraclePipeline::new();
        for (height, header) in selected_headers {
            let block = fetcher
                .read_block_decoded(*height, header.hash())
                .map_err(|error| format!("decode block {height}: {error}"))?;
            for transaction in &block.transactions {
                pipeline.process(*height, header.time(), transaction);
            }
        }
        pipeline
            .finish()
            .map_err(|error| format!("compute oracle report: {error}"))
    }
}
