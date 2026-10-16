use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use mantle::network::Network;
use delphi_oracle::PeerSources;
use delphi_oracled::{
    AckLogger, AuditLedger, ChainSync, ChainSyncSpec, ChainWorker, Daemon, DaemonConfig, EstimatorWorker, EventBus,
    KeySource, LogFile,
    MempoolEstimator, MempoolWindow, MempoolWorker, NoteSink, PriceHub, PriceSigner, RelayConnector, RelayPublisher,
    StdoutSink, Ticker, TransactionStream,
};
use nostro2_relay::{NostrRelay, ReconnectConfig};

struct OracleDaemonApp {
    config: DaemonConfig,
}

impl OracleDaemonApp {
    const KEY_VARIABLE: &'static str = "ORACLED_NSEC";
    const MEMPOOL_INTERVAL: Duration = Duration::from_secs(10);
    const LEDGER_FILE: &'static str = "audit.ledger";
    const DRY_LEDGER_FILE: &'static str = "audit-dry.ledger";
    const RETIRED_MARKER: &'static str = "addressable.retired";
    const ESTIMATOR_SEED: u64 = 0x6d65_6d70;
    const CHAIN_JOIN_LIMIT: Duration = Duration::from_secs(10);

    /// Wait up to `limit` for `thread`. A chain sync stuck in a peer
    /// deadline is left to the process exit; the record log drops a torn
    /// tail on the next start.
    fn join_within(thread: thread::JoinHandle<()>, limit: Duration) {
        let started = Instant::now();
        while !thread.is_finished() {
            if started.elapsed() >= limit {
                eprintln!("chain worker still syncing after {}s; exiting without it", limit.as_secs());
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        let _ = thread.join();
    }

    fn execute() -> ExitCode {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        if arguments.iter().any(|argument| argument == "--help") {
            println!("{}", DaemonConfig::HELP);
            return ExitCode::SUCCESS;
        }
        let config = match DaemonConfig::parse(&arguments).and_then(DaemonConfig::with_absolute_data_dir) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("error: {error}\n\n{}", DaemonConfig::HELP);
                return ExitCode::from(2);
            }
        };
        match (Self { config }).run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fatal: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(&self) -> Result<(), String> {
        if let Some(path) = &self.config.log_file {
            LogFile::new(path)
                .redirect_stderr()
                .map_err(|error| format!("log file {}: {error}", path.display()))?;
        }
        let source = KeySource::new(&self.config, std::env::var(Self::KEY_VARIABLE).ok())
            .with_credentials_directory(std::env::var_os("CREDENTIALS_DIRECTORY").map(std::path::PathBuf::from));
        let signer = source.resolve().map_err(|error| error.to_string())?;
        let npub = signer.npub().map_err(|error| error.to_string())?;
        if self.config.print_npub {
            println!("{npub}");
            return Ok(());
        }
        eprintln!("publishing as {npub} (key from {})", source.describe());
        eprintln!("data directory {}", self.config.data_dir.display());
        if self.config.dry_run {
            self.serve(StdoutSink, signer, None)
        } else {
            let relay = RelayConnector::new(&self.config.relay_url, self.config.relay_insecure)
                .connect(ReconnectConfig::default())?;
            eprintln!("connected to {}", relay.url());
            self.serve(relay.clone(), signer, Some(&relay))
        }
    }

    fn ledger(&self) -> Result<AuditLedger, String> {
        let name = if self.config.dry_run {
            Self::DRY_LEDGER_FILE
        } else {
            Self::LEDGER_FILE
        };
        let path = self.config.data_dir.join(name);
        AuditLedger::open(&path).map_err(|error| format!("audit ledger {}: {error}", path.display()))
    }

    fn peers(&self) -> Result<Vec<std::net::SocketAddrV4>, String> {
        let mut sources = PeerSources::new();
        for peer in &self.config.peers {
            sources.add_explicit(*peer);
        }
        let peers = sources.resolve(Network::Mainnet);
        if peers.is_empty() {
            return Err("no bitcoin peers found; pass --peer ADDR".to_string());
        }
        Ok(peers)
    }

    fn serve<S: NoteSink>(&self, sink: S, signer: PriceSigner, acks: Option<&NostrRelay>) -> Result<(), String> {
        let lock = ChainSync::lock(&self.config.data_dir)?;
        let peers = self.peers()?;
        eprintln!("{} candidate bitcoin peers", peers.len());
        let spec = ChainSyncSpec {
            network: Network::Mainnet,
            peers: peers.clone(),
            data_dir: self.config.data_dir.clone(),
            depth: self.config.history_blocks,
            cache_limit_bytes: self.config.cache_limit_bytes,
            lock,
        };
        let ledger = self.ledger()?;
        eprintln!("{} audit notes already published", ledger.len());
        let already_audited = ledger.published();
        let publisher = RelayPublisher::new(sink, signer);
        let window = MempoolWindow::new(
            "mempool",
            mantle::difficulty::BlockHeight::new(0),
            MempoolEstimator::new(self.config.min_outputs, self.config.draws),
            Self::ESTIMATOR_SEED,
        );
        let hub = PriceHub::new(
            publisher,
            ledger,
            window,
            self.config.tolerance,
            Self::MEMPOOL_INTERVAL,
        )
        .with_transaction_stream(TransactionStream::new(self.config.tx_notes_per_second));

        let stop = Arc::new(AtomicBool::new(false));
        delphi_oracled::shutdown_signal::ShutdownSignal::arm(&stop)
            .map_err(|error| format!("install signal handlers: {error}"))?;
        let (sender, mut receiver) = EventBus::open(EventBus::CAPACITY);
        let deadline = self.config.run_for.map(|span| Instant::now() + span);
        let ticker = Ticker::new(Daemon::<S>::WAKE, deadline, Arc::clone(&stop), sender.clone()).spawn()?;
        let ack_reader = match acks {
            Some(relay) => Some(AckLogger::new(relay.clone(), sender.clone()).spawn()?),
            None => None,
        };
        let chain = {
            let worker = ChainWorker::new(spec, already_audited, Arc::clone(&stop), sender.clone());
            thread::Builder::new()
                .name("chain".into())
                .spawn(move || worker.run())
                .map_err(|error| format!("spawn chain worker: {error}"))?
        };
        let mempool = {
            let worker = MempoolWorker::new(peers, self.config.mempool_peers, Arc::clone(&stop), sender.clone());
            thread::Builder::new()
                .name("mempool".into())
                .spawn(move || worker.run())
                .map_err(|error| format!("spawn mempool worker: {error}"))?
        };

        let estimator = EstimatorWorker::spawn(
            MempoolEstimator::new(self.config.min_outputs, self.config.draws),
            Self::ESTIMATOR_SEED,
            sender,
        )?;
        let mut daemon = Daemon::new(hub).with_estimator(estimator);
        let retired_marker = self.config.data_dir.join(Self::RETIRED_MARKER);
        if !retired_marker.exists() {
            daemon.retire_addressable();
            if !self.config.dry_run
                && let Err(error) = std::fs::write(&retired_marker, b"")
            {
                eprintln!("{}: {error}", retired_marker.display());
            }
        }
        daemon.run(&mut receiver);

        stop.store(true, Ordering::Relaxed);
        drop(receiver);
        let _ = ticker.join();
        let _ = mempool.join();
        Self::join_within(chain, Self::CHAIN_JOIN_LIMIT);
        if let Some(relay) = acks {
            relay.close();
        }
        if let Some(reader) = ack_reader {
            let _ = reader.join();
        }
        eprintln!("{} audit notes still awaiting a relay ack", daemon.hub().pending_audits());
        eprintln!("{} mempool checks skipped while the estimator was busy", daemon.estimates_skipped());
        let tally = daemon.hub().publisher().tally();
        eprintln!(
            "stopped: {} audit, {} live, {} transaction, {} other, {} rejected, {} undelivered",
            tally.audit, tally.live, tally.transactions, tally.other, tally.rejected, tally.undelivered
        );
        Ok(())
    }
}

fn main() -> ExitCode {
    OracleDaemonApp::execute()
}
