use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddrV4, TcpStream};
use std::time::{Duration, Instant};

use mantle::codec::{Decode, Encode};
use mantle::hash::Hash256;
use mantle::network::{
    Command, InvEntry, InvMsg, InvType, Magic, MessageHeader, Network, encode_message,
};
use mantle::tx::Transaction;

const RECV_CHUNK: usize = 64 * 1024;
const MAX_BUFFERED: usize = 8 * 1024 * 1024;
const MAX_GETDATA_BATCH: usize = 128;
const USER_AGENT: &str = "/mantle-oracle:0.1/";

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
    requested: Vec<Hash256>,
    announced: u64,
    delivered: u64,
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
            requested: Vec::new(),
            announced: 0,
            delivered: 0,
        };
        watcher.handshake()?;
        Ok(watcher)
    }

    /// Bitcoin Core sends no transactions to a peer that set `relay: false`,
    /// and disconnects one that asks for `mempool` without `NODE_BLOOM`. The
    /// shared block-fetcher version message wants relay off, so build our own.
    fn version_message() -> Result<Vec<u8>, WatchError> {
        let mut message = mantle_transport::our_version(USER_AGENT);
        message.relay = true;
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

    /// Pump the socket once, returning every transaction that arrived.
    ///
    /// # Errors
    /// Returns [`WatchError::Io`] if the peer disconnects.
    pub fn poll(&mut self) -> Result<Vec<SeenTransaction>, WatchError> {
        let mut seen = Vec::new();
        if !self.fill()? {
            return Ok(seen);
        }
        while let Some((command, payload)) = self.next_frame() {
            match command {
                Command::Inv => self.on_inv(&payload),
                Command::Tx => {
                    if let Ok(transaction) =
                        Transaction::decode(&mut io::Cursor::new(payload.as_slice()))
                    {
                        self.delivered += 1;
                        seen.push(SeenTransaction {
                            transaction,
                            at: Instant::now(),
                        });
                    }
                }
                Command::Ping => self.send(Command::Pong, &payload)?,
                _ => {}
            }
        }
        self.flush_getdata()?;
        Ok(seen)
    }

    fn on_inv(&mut self, payload: &[u8]) {
        let Ok(inv) = InvMsg::decode(&mut io::Cursor::new(payload)) else {
            return;
        };
        for entry in &inv.0 {
            if matches!(entry.kind(), InvType::Tx | InvType::WitnessTx) {
                self.announced += 1;
                self.requested.push(entry.hash());
            }
        }
    }

    fn flush_getdata(&mut self) -> Result<(), WatchError> {
        while !self.requested.is_empty() {
            let take = self.requested.len().min(MAX_GETDATA_BATCH);
            let batch: Vec<InvEntry> = self
                .requested
                .drain(..take)
                .map(|hash| InvEntry::new(InvType::WitnessTx, hash))
                .collect();
            let mut payload = Vec::new();
            InvMsg(batch)
                .encode(&mut payload)
                .map_err(|error| WatchError::Io(io::Error::other(error.to_string())))?;
            self.send(Command::GetData, &payload)?;
        }
        Ok(())
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
}
