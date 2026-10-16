use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

use mantle::codec::{Decode, Encode};
use mantle::hash::Hash256;
use mantle::network::{
    Command, InvEntry, InvMsg, InvType, Magic, MessageHeader, Network, Services, encode_message,
};
use mantle::tx::Transaction;

const RECV_CHUNK: usize = 64 * 1024;
const MAX_BUFFERED: usize = 8 * 1024 * 1024;
const MAX_GETDATA_BATCH: usize = 128;
const USER_AGENT: &str = "/delphi-oracle:0.1/";

#[derive(Debug)]
pub enum WatchError {
    Connect(io::Error),
    Io(io::Error),
    Handshake,
}

impl fmt::Display for WatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Connect(error) => write!(formatter, "connect failed: {error}"),
            Self::Io(error) => write!(formatter, "peer i/o failed: {error}"),
            Self::Handshake => formatter.write_str("peer never completed the handshake"),
        }
    }
}

impl std::error::Error for WatchError {}

/// A transaction announced by a peer, tagged with local arrival time.
pub struct SeenTransaction {
    pub transaction: Transaction,
    pub at: Instant,
}

/// What one peer produced during a single [`MempoolWatcher::poll`].
///
/// Announcements and bodies are reported separately so a caller watching
/// several peers can count every announcement while requesting each
/// transaction from only one of them.
#[derive(Default)]
pub struct PeerBatch {
    pub announced: Vec<Hash256>,
    pub transactions: Vec<SeenTransaction>,
}

impl PeerBatch {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.announced.is_empty() && self.transactions.is_empty()
    }
}

/// Single-peer mempool subscription over blocking TCP.
///
/// Announcements arrive as `inv`; this requests each unseen txid with
/// `getdata` and yields the `tx` messages that come back. Deliberately
/// independent of the `io_uring` [`Fetcher`], which is shaped for
/// run-to-completion block downloads rather than an open-ended stream.
pub struct MempoolWatcher {
    stream: TcpStream,
    magic: Magic,
    recv_buf: Vec<u8>,
    scratch: Vec<u8>,
    chunk: Box<[u8]>,
    announced: u64,
    delivered: u64,
    not_found: u64,
    decode_failures: u64,
}

impl MempoolWatcher {
    /// # Errors
    /// Returns [`WatchError`] if the peer is unreachable or drops during the handshake.
    pub fn connect(
        address: SocketAddrV4,
        network: Network,
        timeout: Duration,
    ) -> Result<Self, WatchError> {
        let stream =
            TcpStream::connect_timeout(&address.into(), timeout).map_err(WatchError::Connect)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(WatchError::Io)?;
        stream.set_nodelay(true).map_err(WatchError::Io)?;
        let mut watcher = Self {
            stream,
            magic: network.magic(),
            recv_buf: Vec::with_capacity(RECV_CHUNK),
            scratch: Vec::new(),
            chunk: vec![0u8; RECV_CHUNK].into_boxed_slice(),
            announced: 0,
            delivered: 0,
            not_found: 0,
            decode_failures: 0,
        };
        watcher.handshake()?;
        Ok(watcher)
    }

    /// Bound how long a single [`poll`](Self::poll) may block.
    ///
    /// A caller cycling through many peers wants this far below the connect
    /// timeout, so one idle peer cannot stall the rest of the round.
    ///
    /// # Errors
    /// Returns [`WatchError::Io`] if the socket rejects the timeout.
    pub fn set_poll_timeout(&mut self, timeout: Duration) -> Result<(), WatchError> {
        self.stream
            .set_read_timeout(Some(timeout))
            .map_err(WatchError::Io)
    }

    /// Bitcoin Core sends no transactions to a peer that set `relay: false`,
    /// and disconnects one that asks for `mempool` without `NODE_BLOOM`. It
    /// also refuses `MSG_WITNESS_TX` bodies to a peer that never advertised
    /// `NODE_WITNESS`, which announces transactions but silently serves none.
    /// The shared block-fetcher version message wants relay off, so build our
    /// own.
    fn version_message() -> Result<Vec<u8>, WatchError> {
        let mut message = mantle_transport::our_version(USER_AGENT);
        message.relay = true;
        message.services = message.services | Services::NODE_WITNESS;
        let mut encoded = Vec::new();
        message
            .encode(&mut encoded)
            .map_err(|error| WatchError::Io(io::Error::other(error.to_string())))?;
        Ok(encoded)
    }

    fn handshake(&mut self) -> Result<(), WatchError> {
        let version = Self::version_message()?;
        self.send(Command::Version, &version)?;
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut got_version = false;
        while Instant::now() < deadline {
            if !self.fill()? {
                return Err(WatchError::Handshake);
            }
            while let Some((command, payload)) = self.next_frame() {
                match command {
                    Command::Version => {
                        got_version = true;
                        self.send(Command::Verack, &[])?;
                    }
                    Command::Verack if got_version => return Ok(()),
                    Command::Ping => {
                        let nonce = payload;
                        self.send(Command::Pong, &nonce)?;
                    }
                    _ => {}
                }
            }
        }
        Err(WatchError::Handshake)
    }

    /// Pump the socket once, reporting announcements and delivered bodies.
    ///
    /// This does not request anything; the caller decides which announced
    /// txids to fetch and calls [`request`](Self::request).
    ///
    /// # Errors
    /// Returns [`WatchError::Io`] if the peer disconnects.
    pub fn poll(&mut self) -> Result<PeerBatch, WatchError> {
        let mut batch = PeerBatch::default();
        while self.fill()? && self.recv_buf.len() < MAX_BUFFERED {}
        while let Some((command, payload)) = self.next_frame() {
            match command {
                Command::Inv => Self::collect_inv(&payload, &mut batch.announced),
                Command::Tx => {
                    match Transaction::decode(&mut io::Cursor::new(payload.as_slice())) {
                        Ok(transaction) => {
                            self.delivered += 1;
                            batch.transactions.push(SeenTransaction {
                                transaction,
                                at: Instant::now(),
                            });
                        }
                        Err(_) => self.decode_failures += 1,
                    }
                }
                Command::NotFound => {
                    if let Ok(inv) = InvMsg::decode(&mut io::Cursor::new(payload.as_slice())) {
                        self.not_found += inv.0.len() as u64;
                    }
                }
                Command::Ping => self.send(Command::Pong, &payload)?,
                _ => {}
            }
        }
        self.announced += batch.announced.len() as u64;
        Ok(batch)
    }

    /// Ask this peer for the bodies of `hashes`, in `getdata` batches.
    ///
    /// # Errors
    /// Returns [`WatchError::Io`] if the peer disconnects.
    pub fn request(&mut self, hashes: &[Hash256]) -> Result<(), WatchError> {
        for chunk in hashes.chunks(MAX_GETDATA_BATCH) {
            let batch: Vec<InvEntry> = chunk
                .iter()
                .map(|hash| InvEntry::new(InvType::WitnessTx, *hash))
                .collect();
            let mut payload = Vec::new();
            InvMsg(batch)
                .encode(&mut payload)
                .map_err(|error| WatchError::Io(io::Error::other(error.to_string())))?;
            self.send(Command::GetData, &payload)?;
        }
        Ok(())
    }

    fn collect_inv(payload: &[u8], into: &mut Vec<Hash256>) {
        let Ok(inv) = InvMsg::decode(&mut io::Cursor::new(payload)) else {
            return;
        };
        for entry in &inv.0 {
            if matches!(entry.kind(), InvType::Tx | InvType::WitnessTx) {
                into.push(entry.hash());
            }
        }
    }

    fn send(&mut self, command: Command, payload: &[u8]) -> Result<(), WatchError> {
        self.scratch.clear();
        encode_message(self.magic, command, payload, &mut self.scratch)
            .map_err(|error| WatchError::Io(io::Error::other(error.to_string())))?;
        self.stream
            .write_all(&self.scratch)
            .map_err(WatchError::Io)?;
        Ok(())
    }

    fn fill(&mut self) -> Result<bool, WatchError> {
        if self.recv_buf.len() > MAX_BUFFERED {
            return Ok(true);
        }
        match self.stream.read(&mut self.chunk) {
            Ok(0) => Err(WatchError::Io(io::Error::from(
                io::ErrorKind::UnexpectedEof,
            ))),
            Ok(read) => {
                self.recv_buf.extend_from_slice(&self.chunk[..read]);
                Ok(true)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                Ok(false)
            }
            Err(error) => Err(WatchError::Io(error)),
        }
    }

    fn next_frame(&mut self) -> Option<(Command, Vec<u8>)> {
        if self.recv_buf.len() < MessageHeader::ENCODED_LEN {
            return None;
        }
        let mut raw = [0u8; MessageHeader::ENCODED_LEN];
        raw.copy_from_slice(&self.recv_buf[..MessageHeader::ENCODED_LEN]);
        let header = MessageHeader::from_bytes(&raw);
        let length = header.length() as usize;
        let total = MessageHeader::ENCODED_LEN + length;
        if self.recv_buf.len() < total {
            return None;
        }
        let payload = self.recv_buf[MessageHeader::ENCODED_LEN..total].to_vec();
        let command = Command::from_command_name(header.command());
        let valid = Hash256::checksum(&payload) == header.checksum();
        self.recv_buf.drain(..total);
        match command {
            Some(command) if valid => Some((command, payload)),
            _ => self.next_frame(),
        }
    }

    #[must_use]
    pub const fn announced(&self) -> u64 {
        self.announced
    }

    #[must_use]
    pub const fn delivered(&self) -> u64 {
        self.delivered
    }

    /// Bodies the peer explicitly refused to send.
    #[must_use]
    pub const fn not_found(&self) -> u64 {
        self.not_found
    }

    /// `tx` frames that arrived but would not parse.
    #[must_use]
    pub const fn decode_failures(&self) -> u64 {
        self.decode_failures
    }
}
