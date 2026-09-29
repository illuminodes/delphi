use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use mantle::network::Network;
use mantle_oracle::PeerSources;
use mantle_oracled::{
    ChainSyncSpec, ChainWindow, ChainWorker, Daemon, DaemonConfig, MempoolEstimator, MempoolWindow,
    MempoolWorker, NoteSink, PriceHub, PriceSigner, PublishPolicy, RelayPublisher, StdoutSink,
};
use nostro2_relay::{NostrRelay, ReconnectConfig};
use nostro2_signer::NostrKeypair;
use nostro2_signer::nostro2_traits::NostrKeypair as KeypairTrait;

struct OracleDaemonApp {
    config: DaemonConfig,
}

impl OracleDaemonApp {
    const KEY_VARIABLE: &'static str = "ORACLED_NSEC";
    const MEMPOOL_INTERVAL: Duration = Duration::from_secs(10);
    const MEMPOOL_MIN_CHANGE: f64 = 0.001;
    const HEARTBEAT: Duration = Duration::from_mins(1);
    const CHAIN_HEARTBEAT: Duration = Duration::from_mins(10);

    fn execute() -> ExitCode {
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        if arguments.iter().any(|argument| argument == "--help") {
            println!("{}", DaemonConfig::HELP);
            return ExitCode::SUCCESS;
        }
        let config = match DaemonConfig::parse(&arguments) {
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
        let signer = self.signer()?;
        eprintln!("publishing as {}", signer.npub().map_err(|error| error.to_string())?);
        if self.config.dry_run {
            self.serve(StdoutSink, signer)
        } else {
            let relay = NostrRelay::connect_blocking_with(
                &self.config.relay_url,
                ReconnectConfig::default(),
            )
            .map_err(|error| format!("connect {}: {error}", self.config.relay_url))?;
            eprintln!("connected to {}", relay.url());
            self.serve(relay, signer)
        }
    }

    fn signer(&self) -> Result<PriceSigner, String> {
        if self.config.dry_run {
            return Ok(PriceSigner::new(<NostrKeypair as KeypairTrait>::generate()));
        }
        let secret = std::env::var(Self::KEY_VARIABLE)
            .map_err(|_| format!("set {} to the signing key (nsec, hex or mnemonic)", Self::KEY_VARIABLE))?;
        PriceSigner::from_secret(&secret).map_err(|error| error.to_string())
    }

    fn policy() -> PublishPolicy {
        let mut policy = PublishPolicy::new(Self::MEMPOOL_MIN_CHANGE, Self::HEARTBEAT);
        for window in ChainWindow::ALL {
            policy = policy.with_rule(window.tag(), 0.0, Self::CHAIN_HEARTBEAT);
        }
        policy
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

    fn serve<S: NoteSink>(&self, sink: S, signer: PriceSigner) -> Result<(), String> {
        let peers = self.peers()?;
        eprintln!("{} candidate bitcoin peers", peers.len());
        let spec = ChainSyncSpec {
            network: Network::Mainnet,
            peers: peers.clone(),
            data_dir: self.config.data_dir.clone(),
            depth: ChainWindow::deepest().blocks(),
            cache_limit_bytes: self.config.cache_limit_bytes,
        };
        let publisher = RelayPublisher::new(sink, signer, Self::policy());
        let window = MempoolWindow::new(
            "mempool",
            mantle::difficulty::BlockHeight::new(0),
            MempoolEstimator::new(self.config.min_outputs, self.config.draws),
            0x6d65_6d70,
        );
        let hub = PriceHub::new(publisher, window, self.config.tolerance, Self::MEMPOOL_INTERVAL);

        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let chain = {
            let worker = ChainWorker::new(spec, Arc::clone(&stop), sender.clone());
            thread::Builder::new()
                .name("chain".into())
                .spawn(move || worker.run())
                .map_err(|error| format!("spawn chain worker: {error}"))?
        };
        let mempool = {
            let worker = MempoolWorker::new(peers, self.config.mempool_peers, Arc::clone(&stop), sender);
            thread::Builder::new()
                .name("mempool".into())
                .spawn(move || worker.run())
                .map_err(|error| format!("spawn mempool worker: {error}"))?
        };

        let deadline = self.config.run_for.map(|span| Instant::now() + span);
        let mut daemon = Daemon::new(hub);
        daemon.run(&receiver, deadline);

        stop.store(true, Ordering::Relaxed);
        drop(receiver);
        let _ = chain.join();
        let _ = mempool.join();
        let tally = daemon.hub().publisher().tally();
        eprintln!(
            "stopped: {} published, {} held, {} rejected, {} undelivered",
            tally.published, tally.held, tally.rejected, tally.undelivered
        );
        Ok(())
    }
}

fn main() -> ExitCode {
    OracleDaemonApp::execute()
}
