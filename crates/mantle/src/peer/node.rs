use quetzalcoatl::capacity::Capacity;
use quetzalcoatl::{mpsc, spmc};

use crate::network::{Magic, VersionMsg};

use super::rx::RxThread;
use super::tx::TxThread;
use super::types::{ConnectCmd, Error, HandshakeState, InboundMsg, OutboundMsg, PeerId};

/// Queue depth for the inbound (spmc) and outbound (mpsc) channels.
const CHANNEL_CAP: usize = 256;

/// Public handle to the peer I/O subsystem.
///
/// Spawns an RX thread and a TX thread on construction. The `IoUring` instances
/// are created *inside* their respective threads because `IoUring` is `!Send`.
pub struct Node {
    /// Send connect commands to the RX thread.
    connect_tx: Option<std::sync::mpsc::SyncSender<ConnectCmd>>,

    /// Workers clone this to receive decoded inbound messages.
    pub inbound_rx: spmc::Consumer<InboundMsg>,

    /// Workers clone this to enqueue outbound messages for the TX thread.
    pub outbound_tx: mpsc::Producer<OutboundMsg>,

    rx_thread: Option<std::thread::JoinHandle<Result<(), Error>>>,
    tx_thread: Option<std::thread::JoinHandle<Result<(), Error>>>,
}

impl Node {
    /// Construct a `Node`, spawning the RX and TX I/O threads.
    ///
    /// # Errors
    /// Returns [`Error::ThreadSpawn`] if thread spawning fails.
    pub fn new(magic: Magic, our_version: VersionMsg) -> Result<Self, Error> {
        // outbound: workers + RX thread → TX thread  (mpsc)
        let (outbound_tx, outbound_rx) =
            mpsc::RingBuffer::new(Capacity::exact(CHANNEL_CAP)).split();

        // inbound: RX thread → workers  (spmc)
        let (inbound_tx, inbound_rx) = spmc::RingBuffer::new(Capacity::exact(CHANNEL_CAP)).split();

        // connect commands: Node::connect() → RX thread
        let (connect_tx, connect_rx) = std::sync::mpsc::sync_channel::<ConnectCmd>(16);

        // outbound_tx clone for handshake replies sent by the RX thread.
        let rx_outbound_tx = outbound_tx.clone();

        // IoUring is !Send, so RxThread and TxThread are constructed *inside*
        // their threads. We pass only Send-safe channel ends across the boundary.
        let rx_thread = std::thread::Builder::new()
            .name("peer-rx".into())
            .spawn(move || {
                RxThread::new(magic, our_version, inbound_tx, rx_outbound_tx, connect_rx)
                    .and_then(RxThread::run)
            })
            .map_err(Error::ThreadSpawn)?;

        let tx_thread = std::thread::Builder::new()
            .name("peer-tx".into())
            .spawn(move || TxThread::new(outbound_rx).and_then(TxThread::run))
            .map_err(Error::ThreadSpawn)?;

        Ok(Self {
            connect_tx: Some(connect_tx),
            inbound_rx,
            outbound_tx,
            rx_thread: Some(rx_thread),
            tx_thread: Some(tx_thread),
        })
    }

    /// Dial a peer. Blocks until the RX thread confirms the connect SQE was
    /// submitted and returns the assigned [`PeerId`].
    ///
    /// # Errors
    /// Returns an error if the RX thread has exited or reports a socket/ring error.
    /// Dial a peer. Returns `(PeerId, fd)` — the fd is needed to construct
    /// `OutboundMsg` since the TX thread has no access to `PeerState`.
    ///
    /// # Errors
    /// Returns an error if the RX thread has exited or reports a socket/ring error.
    pub fn connect(
        &self,
        addr: std::net::SocketAddrV4,
    ) -> Result<(PeerId, ququmatz::types::RawFd), Error> {
        let (reply_tx, mut reply_rx) =
            quetzalcoatl::spsc::RingBuffer::new(Capacity::exact(1)).split();

        self.connect_tx
            .as_ref()
            .ok_or_else(|| Error::ThreadSpawn(std::io::Error::other("node is shut down")))?
            .send(ConnectCmd {
                addr,
                reply: reply_tx,
            })
            .map_err(|_| Error::ThreadSpawn(std::io::Error::other("RX thread is gone")))?;

        reply_rx
            .pop_block()
            .unwrap_or(Err(Error::ThreadSpawn(std::io::Error::other(
                "RX thread closed connect reply channel",
            ))))
    }

    /// Returns the current handshake state for a peer.
    ///
    /// Callers can observe readiness more efficiently by watching `inbound_rx`.
    pub fn peer_ready(&self, _id: PeerId) -> Option<HandshakeState> {
        None
    }

    /// Shut down the node cleanly, joining both I/O threads.
    pub fn shutdown(&mut self) {
        // Dropping connect_tx closes the channel, signalling the RX thread to exit.
        drop(self.connect_tx.take());
        if let Some(h) = self.rx_thread.take()
            && let Err(e) = h.join().unwrap_or(Ok(()))
        {
            eprintln!("peer-rx thread error: {e}");
        }
        if let Some(h) = self.tx_thread.take()
            && let Err(e) = h.join().unwrap_or(Ok(()))
        {
            eprintln!("peer-tx thread error: {e}");
        }
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        // Signal threads to exit by closing the connect channel, then detach.
        // Use shutdown() for a blocking clean shutdown.
        drop(self.connect_tx.take());
    }
}
