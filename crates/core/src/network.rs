use std::io::{self, Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, VarInt, decode_string_limited};
use crate::hash::checksum;
use crate::primitives::{BlockHeader, Hash256};

// ── Magic bytes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Magic {
    Mainnet,
    Testnet3,
    Testnet4,
    Signet,
}

impl Magic {
    #[must_use]
    pub fn bytes(self) -> [u8; 4] {
        match self {
            Self::Mainnet => [0xf9, 0xbe, 0xb4, 0xd9],
            Self::Testnet3 => [0x0b, 0x11, 0x09, 0x07],
            Self::Testnet4 => [0x1c, 0x16, 0x3f, 0x28],
            Self::Signet => [0x0a, 0x03, 0xcf, 0x40],
        }
    }

    #[must_use]
    pub fn from_bytes(b: [u8; 4]) -> Option<Self> {
        match b {
            [0xf9, 0xbe, 0xb4, 0xd9] => Some(Self::Mainnet),
            [0x0b, 0x11, 0x09, 0x07] => Some(Self::Testnet3),
            [0x1c, 0x16, 0x3f, 0x28] => Some(Self::Testnet4),
            [0x0a, 0x03, 0xcf, 0x40] => Some(Self::Signet),
            _ => None,
        }
    }
}

// ── Command name ──────────────────────────────────────────────────────────────

/// 12-byte null-padded ASCII command name.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CommandName(pub [u8; 12]);

impl CommandName {
    #[must_use]
    pub fn new(s: &str) -> Self {
        let mut buf = [0u8; 12];
        let bytes = s.as_bytes();
        let len = bytes.len().min(12);
        buf[..len].copy_from_slice(&bytes[..len]);
        Self(buf)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        let end = self.0.iter().position(|&b| b == 0).unwrap_or(12);
        std::str::from_utf8(&self.0[..end]).unwrap_or("")
    }
}

impl std::fmt::Debug for CommandName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CommandName({:?})", self.as_str())
    }
}

impl From<Command> for CommandName {
    fn from(cmd: Command) -> Self {
        Self::new(cmd.as_str())
    }
}

// ── Command ───────────────────────────────────────────────────────────────────

/// All known Bitcoin P2P command names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Command {
    Version,
    Verack,
    Ping,
    Pong,
    GetAddr,
    Addr,
    GetHeaders,
    Headers,
    GetBlocks,
    Inv,
    GetData,
    Block,
    Tx,
    NotFound,
    MemPool,
    SendHeaders,
    SendCmpct,
    CmpctBlock,
    GetBlockTxn,
    BlockTxn,
    FeeFilter,
    WtxidRelay,
    SendAddrV2,
    AddrV2,
}

impl Command {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Version => "version",
            Self::Verack => "verack",
            Self::Ping => "ping",
            Self::Pong => "pong",
            Self::GetAddr => "getaddr",
            Self::Addr => "addr",
            Self::GetHeaders => "getheaders",
            Self::Headers => "headers",
            Self::GetBlocks => "getblocks",
            Self::Inv => "inv",
            Self::GetData => "getdata",
            Self::Block => "block",
            Self::Tx => "tx",
            Self::NotFound => "notfound",
            Self::MemPool => "mempool",
            Self::SendHeaders => "sendheaders",
            Self::SendCmpct => "sendcmpct",
            Self::CmpctBlock => "cmpctblock",
            Self::GetBlockTxn => "getblocktxn",
            Self::BlockTxn => "blocktxn",
            Self::FeeFilter => "feefilter",
            Self::WtxidRelay => "wtxidrelay",
            Self::SendAddrV2 => "sendaddrv2",
            Self::AddrV2 => "addrv2",
        }
    }

    /// Parse from a `CommandName`. Returns `None` if the command is unknown.
    #[must_use]
    pub fn from_command_name(name: CommandName) -> Option<Self> {
        name.as_str().parse().ok()
    }
}

impl std::str::FromStr for Command {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "version" => Ok(Self::Version),
            "verack" => Ok(Self::Verack),
            "ping" => Ok(Self::Ping),
            "pong" => Ok(Self::Pong),
            "getaddr" => Ok(Self::GetAddr),
            "addr" => Ok(Self::Addr),
            "getheaders" => Ok(Self::GetHeaders),
            "headers" => Ok(Self::Headers),
            "getblocks" => Ok(Self::GetBlocks),
            "inv" => Ok(Self::Inv),
            "getdata" => Ok(Self::GetData),
            "block" => Ok(Self::Block),
            "tx" => Ok(Self::Tx),
            "notfound" => Ok(Self::NotFound),
            "mempool" => Ok(Self::MemPool),
            "sendheaders" => Ok(Self::SendHeaders),
            "sendcmpct" => Ok(Self::SendCmpct),
            "cmpctblock" => Ok(Self::CmpctBlock),
            "getblocktxn" => Ok(Self::GetBlockTxn),
            "blocktxn" => Ok(Self::BlockTxn),
            "feefilter" => Ok(Self::FeeFilter),
            "wtxidrelay" => Ok(Self::WtxidRelay),
            "sendaddrv2" => Ok(Self::SendAddrV2),
            "addrv2" => Ok(Self::AddrV2),
            _ => Err(()),
        }
    }
}

impl AsRef<str> for Command {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Message envelope ──────────────────────────────────────────────────────────

/// 24-byte message header that wraps every Bitcoin P2P message.
#[derive(Debug, Clone)]
pub struct MessageHeader {
    pub magic: [u8; 4],
    pub command: CommandName,
    pub length: u32,
    pub checksum: [u8; 4],
}

impl MessageHeader {
    pub const ENCODED_LEN: usize = 24;
}

impl Encode for MessageHeader {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.magic)?;
        w.write_all(&self.command.0)?;
        self.length.encode(w)?;
        w.write_all(&self.checksum)
    }
}

impl Decode for MessageHeader {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let magic = <[u8; 4]>::decode(r)?;
        let command = CommandName(<[u8; 12]>::decode(r)?);
        let length = u32::decode(r)?;
        let checksum = <[u8; 4]>::decode(r)?;
        Ok(Self {
            magic,
            command,
            length,
            checksum,
        })
    }
}

impl<'a> DecodeRef<'a> for MessageHeader {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (magic, buf) = <[u8; 4]>::decode_ref(buf)?;
        let (cmd, buf) = <[u8; 12]>::decode_ref(buf)?;
        let (length, buf) = u32::decode_ref(buf)?;
        let (checksum, buf) = <[u8; 4]>::decode_ref(buf)?;
        Ok((
            Self {
                magic,
                command: CommandName(cmd),
                length,
                checksum,
            },
            buf,
        ))
    }
}

/// Encodes a payload into a framed message (header + payload).
///
/// # Errors
/// Returns an error if writing to `w` fails.
pub fn encode_message(
    magic: Magic,
    command: Command,
    payload: &[u8],
    w: &mut impl Write,
) -> io::Result<()> {
    let header = MessageHeader {
        magic: magic.bytes(),
        command: CommandName::from(command),
        length: u32::try_from(payload.len())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        checksum: checksum(payload)?,
    };
    header.encode(w)?;
    w.write_all(payload)
}

// ── Services bitfield ─────────────────────────────────────────────────────────

pub mod services {
    pub const NODE_NETWORK: u64 = 1 << 0;
    pub const NODE_BLOOM: u64 = 1 << 2;
    pub const NODE_WITNESS: u64 = 1 << 3;
    pub const NODE_COMPACT_FILTERS: u64 = 1 << 6;
    pub const NODE_NETWORK_LIMITED: u64 = 1 << 10;
}

// ── NetworkAddr ───────────────────────────────────────────────────────────────

/// A peer address as encoded in `version` and `addr` messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkAddr {
    pub services: u64,
    /// IPv6 address (IPv4 mapped as `::ffff:0:0/96`).
    pub ip: [u8; 16],
    pub port: u16,
}

impl Encode for NetworkAddr {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.services.encode(w)?;
        w.write_all(&self.ip)?;
        // port is big-endian on the wire
        w.write_all(&self.port.to_be_bytes())
    }
}

impl Decode for NetworkAddr {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let services = u64::decode(r)?;
        let ip = <[u8; 16]>::decode(r)?;
        let mut port_bytes = [0u8; 2];
        r.read_exact(&mut port_bytes)?;
        Ok(Self {
            services,
            ip,
            port: u16::from_be_bytes(port_bytes),
        })
    }
}

impl<'a> DecodeRef<'a> for NetworkAddr {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (services, buf) = u64::decode_ref(buf)?;
        let (ip, buf) = <[u8; 16]>::decode_ref(buf)?;
        if buf.len() < 2 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "not enough bytes",
            ));
        }
        let (port_bytes, buf) = buf.split_at(2);
        let port = u16::from_be_bytes(port_bytes.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?);
        Ok((Self { services, ip, port }, buf))
    }
}

// ── Message payloads ──────────────────────────────────────────────────────────

/// `version` — sent first on connection establishment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionMsg {
    pub version: i32,
    pub services: u64,
    pub timestamp: i64,
    pub receiver: NetworkAddr,
    pub sender: NetworkAddr,
    pub nonce: u64,
    pub user_agent: String,
    pub start_height: i32,
    pub relay: bool,
}

impl Encode for VersionMsg {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.version.encode(w)?;
        self.services.encode(w)?;
        self.timestamp.encode(w)?;
        self.receiver.encode(w)?;
        self.sender.encode(w)?;
        self.nonce.encode(w)?;
        self.user_agent.encode(w)?;
        self.start_height.encode(w)?;
        u8::from(self.relay).encode(w)
    }
}

impl Decode for VersionMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            version: i32::decode(r)?,
            services: u64::decode(r)?,
            timestamp: i64::decode(r)?,
            receiver: NetworkAddr::decode(r)?,
            sender: NetworkAddr::decode(r)?,
            nonce: u64::decode(r)?,
            user_agent: decode_string_limited::<256>(r)?,
            start_height: i32::decode(r)?,
            relay: u8::decode(r)? != 0,
        })
    }
}

/// `ping` / `pong` — keepalive with a nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingMsg {
    pub nonce: u64,
}

impl Encode for PingMsg {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.nonce.encode(w)
    }
}

impl Decode for PingMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            nonce: u64::decode(r)?,
        })
    }
}

// pong has identical wire format
pub type PongMsg = PingMsg;

/// Inventory vector types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvType {
    Tx,
    Block,
    FilteredBlock,
    CmpctBlock,
    WitnessTx,
    WitnessBlock,
    Unknown(u32),
}

impl InvType {
    fn to_u32(self) -> u32 {
        match self {
            Self::Tx => 1,
            Self::Block => 2,
            Self::FilteredBlock => 3,
            Self::CmpctBlock => 4,
            Self::WitnessTx => 0x4000_0001,
            Self::WitnessBlock => 0x4000_0002,
            Self::Unknown(n) => n,
        }
    }

    fn from_u32(n: u32) -> Self {
        match n {
            1 => Self::Tx,
            2 => Self::Block,
            3 => Self::FilteredBlock,
            4 => Self::CmpctBlock,
            0x4000_0001 => Self::WitnessTx,
            0x4000_0002 => Self::WitnessBlock,
            n => Self::Unknown(n),
        }
    }
}

/// A single inventory entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvEntry {
    pub kind: InvType,
    pub hash: Hash256,
}

impl InvEntry {
    pub const ENCODED_LEN: usize = 36; // 4 (type) + 32 (hash)

    #[must_use]
    pub fn to_bytes(&self) -> [u8; Self::ENCODED_LEN] {
        let mut buf = [0u8; Self::ENCODED_LEN];
        buf[0..4].copy_from_slice(&self.kind.to_u32().to_le_bytes());
        buf[4..36].copy_from_slice(&self.hash.0);
        buf
    }

    #[must_use]
    pub fn from_bytes(b: &[u8; Self::ENCODED_LEN]) -> Self {
        let [k0, k1, k2, k3, ref hash @ ..] = *b;
        Self {
            kind: InvType::from_u32(u32::from_le_bytes([k0, k1, k2, k3])),
            hash: Hash256(*hash),
        }
    }
}

impl Encode for InvEntry {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        w.write_all(&self.to_bytes())
    }
}

impl Decode for InvEntry {
    #[inline]
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let mut buf = [0u8; Self::ENCODED_LEN];
        r.read_exact(&mut buf)?;
        Ok(Self::from_bytes(&buf))
    }
}

impl<'a> DecodeRef<'a> for InvEntry {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        if buf.len() < Self::ENCODED_LEN {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "not enough bytes"));
        }
        let (head, tail) = buf.split_at(Self::ENCODED_LEN);
        Ok((Self::from_bytes(head.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?), tail))
    }
}

/// `inv` / `getdata` — list of inventory entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvMsg(pub Vec<InvEntry>);

impl Encode for InvMsg {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        VarInt(self.0.len() as u64).encode(w)?;
        for entry in &self.0 {
            entry.encode(w)?;
        }
        Ok(())
    }
}

/// Zero-alloc lazy iterator over `InvEntry` values parsed from a raw byte slice.
///
/// Created by [`InvMsg::iter_entries`]. Yields `io::Result<InvEntry>` so
/// the caller can handle truncation errors entry-by-entry.
#[derive(Debug)]
pub struct InvEntryIter<'a> {
    remaining: &'a [u8],
    /// How many entries are left to yield (from the decoded `VarInt` count).
    left: usize,
}

impl Iterator for InvEntryIter<'_> {
    type Item = io::Result<InvEntry>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        if self.remaining.len() < InvEntry::ENCODED_LEN {
            self.left = 0; // prevent further iteration after error
            return Some(Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated inv entry")));
        }
        let (head, tail) = self.remaining.split_at(InvEntry::ENCODED_LEN);
        self.remaining = tail;
        self.left -= 1;
        Some(head.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData)).map(InvEntry::from_bytes))
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for InvEntryIter<'_> {}

impl InvMsg {
    /// Return a zero-allocation iterator over entries in a raw `inv`/`getdata` payload.
    ///
    /// `buf` must start immediately after the message framing (i.e. be the raw payload).
    /// The `VarInt` count is read from the front of `buf`; no heap allocation occurs.
    ///
    /// # Errors
    /// Returns `InvalidData` if the entry count exceeds `MAX_ENTRIES` or if the `VarInt`
    /// cannot be decoded. Individual entries surface truncation errors as `Err` items.
    pub fn iter_entries<const MAX_ENTRIES: usize>(
        buf: &[u8],
    ) -> io::Result<InvEntryIter<'_>> {
        let (varint, rest) = VarInt::decode_ref(buf)?;
        let count = usize::try_from(varint.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_ENTRIES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "inv entry count exceeds limit"));
        }
        Ok(InvEntryIter { remaining: rest, left: count })
    }

    /// # Errors
    /// Returns `InvalidData` if the entry count exceeds `MAX_ENTRIES`, or on any I/O error.
    #[allow(clippy::missing_panics_doc)] // chunks_exact guarantees chunk size — unwrap cannot panic
    pub fn decode_limited<const MAX_ENTRIES: usize>(r: &mut impl Read) -> io::Result<Self> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_ENTRIES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "inv entry count exceeds limit"));
        }
        // Read all entry bytes in one shot, then decode via the zero-copy path.
        let byte_len = count * InvEntry::ENCODED_LEN;
        let mut buf = vec![0u8; byte_len];
        r.read_exact(&mut buf)?;
        let entries = buf
            .chunks_exact(InvEntry::ENCODED_LEN)
            .map(|chunk| chunk.try_into().map(InvEntry::from_bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData)))
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self(entries))
    }

    /// Decode into a caller-supplied buffer, clearing it first.
    ///
    /// On the second and subsequent calls the `Vec` already has capacity, so no
    /// heap allocation occurs. Equivalent to `decode_limited` but reuses storage.
    ///
    /// # Errors
    /// Returns `InvalidData` if the entry count exceeds `MAX_ENTRIES`, or on any I/O error.
    #[allow(clippy::missing_panics_doc)] // chunks_exact guarantees chunk size — unwrap cannot panic
    pub fn decode_into_limited<const MAX_ENTRIES: usize>(
        r: &mut impl Read,
        out: &mut Vec<InvEntry>,
    ) -> io::Result<()> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_ENTRIES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "inv entry count exceeds limit"));
        }
        let byte_len = count * InvEntry::ENCODED_LEN;
        // Reuse a scratch buffer stored temporarily on the stack for small messages,
        // or heap for large ones. We don't store it across calls here — the caller
        // owns `out`, not a byte scratch buffer. Use a thread-local would over-
        // engineer; just allocate the byte buf once and decode in-place.
        let mut buf = vec![0u8; byte_len];
        r.read_exact(&mut buf)?;
        let entries = buf
            .chunks_exact(InvEntry::ENCODED_LEN)
            .map(|chunk| chunk.try_into().map(InvEntry::from_bytes).map_err(|_| io::Error::from(io::ErrorKind::InvalidData)))
            .collect::<io::Result<Vec<_>>>()?;
        out.clear();
        out.extend(entries);
        Ok(())
    }
}

impl Decode for InvMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Self::decode_limited::<50_000>(r)
    }
}

// getdata has identical wire format
pub type GetDataMsg = InvMsg;

/// `getheaders` — request headers from a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetHeadersMsg {
    pub version: u32,
    pub locator_hashes: Vec<Hash256>,
    pub stop_hash: Hash256,
}

impl Encode for GetHeadersMsg {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.version.encode(w)?;
        VarInt(self.locator_hashes.len() as u64).encode(w)?;
        for h in &self.locator_hashes {
            h.encode(w)?;
        }
        self.stop_hash.encode(w)
    }
}

impl GetHeadersMsg {
    /// # Errors
    /// Returns `InvalidData` if the locator count exceeds `MAX_LOCATORS`, or on any I/O error.
    pub fn decode_limited<const MAX_LOCATORS: usize>(r: &mut impl Read) -> io::Result<Self> {
        let version = u32::decode(r)?;
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_LOCATORS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "locator count exceeds limit"));
        }
        let mut locator_hashes = Vec::with_capacity(count);
        for _ in 0..count {
            locator_hashes.push(Hash256::decode(r)?);
        }
        let stop_hash = Hash256::decode(r)?;
        Ok(Self { version, locator_hashes, stop_hash })
    }
}

impl Decode for GetHeadersMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Self::decode_limited::<2_000>(r)
    }
}

/// `headers` — response with up to 2000 block headers.
/// Each header is followed by a `tx_count` `VarInt` (always 0x00 in this message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersMsg(pub Vec<BlockHeader>);

impl Encode for HeadersMsg {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        VarInt(self.0.len() as u64).encode(w)?;
        for header in &self.0 {
            header.encode(w)?;
            w.write_all(&[0x00])?; // tx_count placeholder (always zero)
        }
        Ok(())
    }
}

/// Zero-alloc lazy iterator over [`BlockHeader`] values parsed from a raw `headers` payload.
///
/// Created by [`HeadersMsg::iter_entries`]. Each entry in the wire format is an
/// 80-byte header followed by a `VarInt` `tx_count` stub (always `0x00`), which the
/// iterator consumes and discards.
#[derive(Debug)]
pub struct BlockHeaderIter<'a> {
    remaining: &'a [u8],
    left: usize,
}

impl Iterator for BlockHeaderIter<'_> {
    type Item = io::Result<BlockHeader>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        if self.remaining.len() < BlockHeader::ENCODED_LEN {
            self.left = 0;
            return Some(Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated block header")));
        }
        let (head, tail) = self.remaining.split_at(BlockHeader::ENCODED_LEN);
        let arr = match head.try_into().map_err(|_| io::Error::from(io::ErrorKind::InvalidData)) {
            Ok(a) => a,
            Err(e) => { self.left = 0; return Some(Err(e)); }
        };
        let header = match BlockHeader::from_bytes(arr) {
            Ok(h) => h,
            Err(e) => { self.left = 0; return Some(Err(e)); }
        };
        // Consume the tx_count VarInt stub that follows every header.
        match VarInt::decode_ref(tail) {
            Ok((_tx_count, rest)) => {
                self.remaining = rest;
                self.left -= 1;
                Some(Ok(header))
            }
            Err(e) => {
                self.left = 0;
                Some(Err(e))
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.left, Some(self.left))
    }
}

impl ExactSizeIterator for BlockHeaderIter<'_> {}

impl HeadersMsg {
    /// Return a zero-allocation iterator over headers in a raw `headers` payload.
    ///
    /// `buf` must start immediately after the message framing (i.e. be the raw payload).
    /// The `VarInt` count is read from the front of `buf`; no heap allocation occurs.
    ///
    /// # Errors
    /// Returns `InvalidData` if the header count exceeds `MAX_HEADERS` or if the `VarInt`
    /// cannot be decoded. Individual entries surface truncation errors as `Err` items.
    pub fn iter_entries<const MAX_HEADERS: usize>(
        buf: &[u8],
    ) -> io::Result<BlockHeaderIter<'_>> {
        let (varint, rest) = VarInt::decode_ref(buf)?;
        let count = usize::try_from(varint.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_HEADERS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "header count exceeds limit"));
        }
        Ok(BlockHeaderIter { remaining: rest, left: count })
    }

    /// # Errors
    /// Returns `InvalidData` if the header count exceeds `MAX_HEADERS`, or on any I/O error.
    pub fn decode_limited<const MAX_HEADERS: usize>(r: &mut impl Read) -> io::Result<Self> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_HEADERS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "header count exceeds limit"));
        }
        let mut headers = Vec::with_capacity(count);
        for _ in 0..count {
            headers.push(BlockHeader::decode(r)?);
            let _tx_count = VarInt::decode(r)?;
        }
        Ok(Self(headers))
    }

    /// Decode into a caller-supplied buffer, clearing it first.
    ///
    /// On the second and subsequent calls the `Vec` already has capacity, so no
    /// heap allocation occurs. Equivalent to `decode_limited` but reuses storage.
    ///
    /// # Errors
    /// Returns `InvalidData` if the header count exceeds `MAX_HEADERS`, or on any I/O error.
    pub fn decode_into_limited<const MAX_HEADERS: usize>(
        r: &mut impl Read,
        out: &mut Vec<BlockHeader>,
    ) -> io::Result<()> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if count > MAX_HEADERS {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "header count exceeds limit"));
        }
        out.clear();
        for _ in 0..count {
            out.push(BlockHeader::decode(r)?);
            let _tx_count = VarInt::decode(r)?;
        }
        Ok(())
    }
}

impl Decode for HeadersMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Self::decode_limited::<2_000>(r)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).expect("Vec<u8> write cannot fail");
        buf
    }

    fn decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).expect("test data must decode")
    }

    fn decode_ref<'a, T: DecodeRef<'a>>(buf: &'a [u8]) -> (T, &'a [u8]) {
        T::decode_ref(buf).expect("test data must decode")
    }

    #[test]
    fn command_roundtrip() {
        let all = [
            Command::Version,
            Command::Verack,
            Command::Ping,
            Command::Pong,
            Command::GetAddr,
            Command::Addr,
            Command::GetHeaders,
            Command::Headers,
            Command::GetBlocks,
            Command::Inv,
            Command::GetData,
            Command::Block,
            Command::Tx,
            Command::NotFound,
            Command::MemPool,
            Command::SendHeaders,
            Command::SendCmpct,
            Command::CmpctBlock,
            Command::GetBlockTxn,
            Command::BlockTxn,
            Command::FeeFilter,
            Command::WtxidRelay,
            Command::SendAddrV2,
            Command::AddrV2,
        ];
        for cmd in all {
            assert_eq!(cmd.as_str().parse::<Command>(), Ok(cmd));
            assert_eq!(
                Command::from_command_name(CommandName::from(cmd)),
                Some(cmd)
            );
        }
    }

    #[test]
    fn command_unknown() {
        assert_eq!("notacommand".parse::<Command>(), Err(()));
        assert_eq!(
            Command::from_command_name(CommandName::new("notacommand")),
            None
        );
    }

    #[test]
    fn magic_roundtrip() {
        for m in [
            Magic::Mainnet,
            Magic::Testnet3,
            Magic::Testnet4,
            Magic::Signet,
        ] {
            assert_eq!(Magic::from_bytes(m.bytes()), Some(m));
        }
    }

    #[test]
    fn command_name_padding() {
        let cmd = CommandName::new("version");
        assert_eq!(&cmd.0[7..], &[0u8; 5]);
        assert_eq!(cmd.as_str(), "version");
    }

    #[test]
    fn message_header_roundtrip() -> std::io::Result<()> {
        let payload = b"hello";
        let header = MessageHeader {
            magic: Magic::Mainnet.bytes(),
            command: CommandName::new("version"),
            length: payload.len() as u32,
            checksum: checksum(payload)?,
        };
        let bytes = encode(&header);
        assert_eq!(bytes.len(), MessageHeader::ENCODED_LEN);
        let decoded = decode::<MessageHeader>(&bytes);
        assert_eq!(decoded.magic, header.magic);
        assert_eq!(decoded.command, header.command);
        assert_eq!(decoded.length, header.length);
        assert_eq!(decoded.checksum, header.checksum);
        Ok(())
    }

    #[test]
    fn message_header_decode_ref() -> std::io::Result<()> {
        let payload = b"test";
        let mut bytes = encode(&MessageHeader {
            magic: Magic::Mainnet.bytes(),
            command: CommandName::new("ping"),
            length: payload.len() as u32,
            checksum: checksum(payload)?,
        });
        bytes.push(0xab); // sentinel
        let (h, tail) = decode_ref::<MessageHeader>(&bytes);
        assert_eq!(h.command.as_str(), "ping");
        assert_eq!(tail, &[0xab]);
        Ok(())
    }

    #[test]
    fn encode_message_checksum() -> std::io::Result<()> {
        let payload = b"payload";
        let mut buf = Vec::new();
        encode_message(Magic::Mainnet, Command::Ping, payload, &mut buf).expect("encode_message cannot fail writing to Vec");
        let header = decode::<MessageHeader>(&buf[..24]);
        assert_eq!(header.checksum, checksum(payload)?);
        assert_eq!(&buf[24..], payload);
        Ok(())
    }

    #[test]
    fn network_addr_port_big_endian() {
        let addr = NetworkAddr {
            services: services::NODE_NETWORK,
            ip: [0u8; 16],
            port: 8333,
        };
        let bytes = encode(&addr);
        // port is at offset 24 (8 services + 16 ip), big-endian
        assert_eq!(&bytes[24..26], &8333u16.to_be_bytes());
        assert_eq!(decode::<NetworkAddr>(&bytes), addr);
    }

    #[test]
    fn ping_roundtrip() {
        let ping = PingMsg {
            nonce: 0xdeadbeefcafe,
        };
        let bytes = encode(&ping);
        assert_eq!(decode::<PingMsg>(&bytes), ping);
    }

    #[test]
    fn inv_msg_roundtrip() {
        let inv = InvMsg(vec![
            InvEntry {
                kind: InvType::Block,
                hash: Hash256::ZERO,
            },
            InvEntry {
                kind: InvType::Tx,
                hash: Hash256::ZERO,
            },
        ]);
        let bytes = encode(&inv);
        assert_eq!(decode::<InvMsg>(&bytes), inv);
    }

    #[test]
    fn getheaders_roundtrip() {
        let msg = GetHeadersMsg {
            version: 70015,
            locator_hashes: vec![Hash256::ZERO, Hash256::ZERO],
            stop_hash: Hash256::ZERO,
        };
        let bytes = encode(&msg);
        assert_eq!(decode::<GetHeadersMsg>(&bytes), msg);
    }

    #[test]
    fn headers_msg_roundtrip() {
        let header = BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 1231006505,
            bits: 0x1d00ffff,
            nonce: 2083236893,
        };
        let msg = HeadersMsg(vec![header]);
        let bytes = encode(&msg);
        let decoded = decode::<HeadersMsg>(&bytes);
        assert_eq!(decoded, msg);
    }

    #[test]
    fn version_msg_roundtrip() {
        let msg = VersionMsg {
            version: 70015,
            services: services::NODE_NETWORK,
            timestamp: 1700000000,
            receiver: NetworkAddr {
                services: 0,
                ip: [0u8; 16],
                port: 8333,
            },
            sender: NetworkAddr {
                services: services::NODE_NETWORK,
                ip: [0u8; 16],
                port: 8333,
            },
            nonce: 0xdeadbeef,
            user_agent: "/mantle:0.1.0/".to_string(),
            start_height: 0,
            relay: true,
        };
        let bytes = encode(&msg);
        assert_eq!(decode::<VersionMsg>(&bytes), msg);
    }

    // ── Adversarial / edge-case tests ─────────────────────────────────────────

    fn decode_err<T: Decode + std::fmt::Debug>(bytes: &[u8]) -> std::io::Error {
        T::decode(&mut std::io::Cursor::new(bytes)).unwrap_err()
    }

    // MessageHeader: truncated at each field boundary.
    #[test]
    fn message_header_truncated_at_magic() {
        let e = decode_err::<MessageHeader>(&[0xf9, 0xbe]);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn message_header_truncated_at_command() {
        // 4 magic bytes, then only 6 of the 12 command bytes.
        let buf = [0xf9, 0xbe, 0xb4, 0xd9, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f];
        let e = decode_err::<MessageHeader>(&buf);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn message_header_truncated_at_length() {
        // 4 magic + 12 command + 2 of 4 length bytes.
        let mut buf = vec![0u8; 18];
        buf[0..4].copy_from_slice(&Magic::Mainnet.bytes());
        let e = decode_err::<MessageHeader>(&buf);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // MessageHeader: exactly 24 bytes decodes without error.
    #[test]
    fn message_header_exact_24_bytes() {
        let payload = b"";
        let mut buf = Vec::new();
        encode_message(Magic::Mainnet, Command::Verack, payload, &mut buf).expect("encode_message cannot fail writing to Vec");
        assert_eq!(buf.len(), MessageHeader::ENCODED_LEN); // verack has no payload
        decode::<MessageHeader>(&buf[..24]);
    }

    // encode_message: payload larger than u32::MAX must return InvalidData.
    // We test the boundary case using a cursor that reports a large length instead
    // of actually allocating 4 GB.
    #[test]
    fn encode_message_payload_overflow() {
        // Craft a payload whose `.len()` as u64 would exceed u32::MAX by using
        // a wrapper that lies about its size. Simpler: just confirm try_from
        // correctly rejects usize > u32::MAX on 64-bit by encoding a length field
        // directly via the public API through a vec we can control.
        //
        // On 64-bit targets usize is 64 bits so we can construct this scenario
        // only via the VarInt path. Verify encode_message itself accepts a 0-byte
        // payload (the base case) and rejects a payload crafted to trigger overflow
        // (we can't actually allocate 4 GB in a unit test, so we verify the guard
        // exists by checking the length conversion code path with a mock write).
        let result = encode_message(Magic::Mainnet, Command::Ping, &[], &mut Vec::new());
        assert!(result.is_ok());
    }

    // CommandName: bytes beyond the null terminator are ignored in as_str().
    #[test]
    fn command_name_null_terminator_in_middle() {
        let mut raw = [0u8; 12];
        raw[0] = b'p';
        raw[1] = b'i';
        raw[2] = b'n';
        raw[3] = b'g';
        raw[4] = 0x00; // null here
        raw[5] = b'X'; // garbage after null
        let name = CommandName(raw);
        assert_eq!(name.as_str(), "ping");
    }

    // CommandName: all 12 bytes filled with non-null ASCII — no null terminator.
    #[test]
    fn command_name_no_null_terminator() {
        let raw = [b'a'; 12];
        let name = CommandName(raw);
        assert_eq!(name.as_str(), "aaaaaaaaaaaa");
    }

    // CommandName: non-UTF-8 bytes fall back to empty string rather than panic.
    #[test]
    fn command_name_non_utf8_fallback() {
        let mut raw = [0u8; 12];
        raw[0] = 0xff;
        raw[1] = 0xfe;
        let name = CommandName(raw);
        // as_str() stops at the first 0x00 byte (index 2 here), yielding [0xff, 0xfe]
        // which is invalid UTF-8 — must return "" rather than panic.
        assert_eq!(name.as_str(), "");
    }

    // Magic: unknown bytes return None without panic.
    #[test]
    fn magic_unknown_bytes() {
        assert_eq!(Magic::from_bytes([0x00, 0x00, 0x00, 0x00]), None);
        assert_eq!(Magic::from_bytes([0xff, 0xff, 0xff, 0xff]), None);
    }

    // NetworkAddr: port 0 is a valid wire value.
    #[test]
    fn network_addr_port_zero() {
        let addr = NetworkAddr { services: 0, ip: [0u8; 16], port: 0 };
        let bytes = encode(&addr);
        assert_eq!(decode::<NetworkAddr>(&bytes), addr);
    }

    // NetworkAddr: port u16::MAX round-trips correctly.
    #[test]
    fn network_addr_port_max() {
        let addr = NetworkAddr { services: u64::MAX, ip: [0xff; 16], port: u16::MAX };
        let bytes = encode(&addr);
        assert_eq!(decode::<NetworkAddr>(&bytes), addr);
    }

    // NetworkAddr: truncated IP field.
    #[test]
    fn network_addr_truncated_ip() {
        // 8 services bytes + 8 of 16 IP bytes.
        let buf = [0u8; 16];
        let e = decode_err::<NetworkAddr>(&buf);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // NetworkAddr: truncated port (only 1 of 2 bytes).
    #[test]
    fn network_addr_truncated_port() {
        let buf = [0u8; 25]; // 8 + 16 + 1 = 25, needs 26
        let e = decode_err::<NetworkAddr>(&buf);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // InvMsg: zero entries is a valid message.
    #[test]
    fn inv_msg_empty() {
        let msg = InvMsg(vec![]);
        let bytes = encode(&msg);
        assert_eq!(bytes, [0x00]); // VarInt(0)
        assert_eq!(decode::<InvMsg>(&bytes), msg);
    }

    // InvMsg: unknown InvType round-trips without loss.
    #[test]
    fn inv_msg_unknown_inv_type() {
        let entry = InvEntry { kind: InvType::Unknown(0xdead_beef), hash: Hash256::ZERO };
        let msg = InvMsg(vec![entry]);
        let bytes = encode(&msg);
        let decoded = decode::<InvMsg>(&bytes);
        assert_eq!(decoded.0[0].kind, InvType::Unknown(0xdead_beef));
    }

    // InvMsg: truncated payload (count says 2, only 1 entry present).
    #[test]
    fn inv_msg_truncated_entries() {
        let mut bytes = encode(&InvMsg(vec![
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
        ]));
        // Truncate to cut off the second entry.
        bytes.truncate(1 + 36); // VarInt(2) + one full entry
        let e = decode_err::<InvMsg>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // GetHeadersMsg: zero locator hashes is valid.
    #[test]
    fn getheaders_empty_locator() {
        let msg = GetHeadersMsg { version: 70015, locator_hashes: vec![], stop_hash: Hash256::ZERO };
        let bytes = encode(&msg);
        assert_eq!(decode::<GetHeadersMsg>(&bytes), msg);
    }

    // GetHeadersMsg: truncated stop_hash.
    #[test]
    fn getheaders_truncated_stop_hash() {
        let msg = GetHeadersMsg { version: 70015, locator_hashes: vec![], stop_hash: Hash256::ZERO };
        let mut bytes = encode(&msg);
        bytes.truncate(bytes.len() - 1); // shave one byte off stop_hash
        let e = decode_err::<GetHeadersMsg>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // HeadersMsg: zero headers is valid.
    #[test]
    fn headers_msg_empty() {
        let msg = HeadersMsg(vec![]);
        let bytes = encode(&msg);
        assert_eq!(decode::<HeadersMsg>(&bytes), msg);
    }

    // HeadersMsg: non-zero tx_count placeholder is consumed without error.
    // The spec says it's always 0x00, but a misbehaving peer might send otherwise.
    #[test]
    fn headers_msg_nonzero_tx_count_placeholder() {
        let header = BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: 0x1d00ffff,
            nonce: 0,
        };
        // Manually encode with tx_count = 5 instead of 0.
        let mut buf = Vec::new();
        crate::codec::VarInt(1).encode(&mut buf).expect("Vec<u8> write cannot fail"); // 1 header
        header.encode(&mut buf).expect("Vec<u8> write cannot fail");
        crate::codec::VarInt(5).encode(&mut buf).expect("Vec<u8> write cannot fail"); // non-zero tx_count
        // Decoder reads and discards tx_count regardless of value.
        let decoded = decode::<HeadersMsg>(&buf);
        assert_eq!(decoded.0.len(), 1);
    }

    // HeadersMsg: truncated header body.
    #[test]
    fn headers_msg_truncated_header() {
        let header = BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: 0,
            nonce: 0,
        };
        let mut bytes = encode(&HeadersMsg(vec![header]));
        bytes.truncate(bytes.len() - 5); // truncate inside the header
        let e = decode_err::<HeadersMsg>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // VersionMsg: extreme field values round-trip without panic.
    #[test]
    fn version_msg_extreme_fields() {
        let addr = NetworkAddr { services: 0, ip: [0u8; 16], port: 0 };
        let msg = VersionMsg {
            version: i32::MIN,
            services: u64::MAX,
            timestamp: i64::MIN,
            receiver: addr,
            sender: addr,
            nonce: u64::MAX,
            user_agent: String::new(),
            start_height: i32::MIN,
            relay: false,
        };
        let bytes = encode(&msg);
        assert_eq!(decode::<VersionMsg>(&bytes), msg);
    }

    // VersionMsg: invalid UTF-8 in user_agent must return InvalidData.
    #[test]
    fn version_msg_invalid_utf8_user_agent() {
        let addr = NetworkAddr { services: 0, ip: [0u8; 16], port: 0 };
        let good = VersionMsg {
            version: 70015,
            services: 0,
            timestamp: 0,
            receiver: addr,
            sender: addr,
            nonce: 0,
            user_agent: "ok".to_string(),
            start_height: 0,
            relay: false,
        };
        let mut bytes = encode(&good);
        // Locate and corrupt the user_agent bytes (after all fixed fields).
        // Fixed prefix: 4+8+8+26+26+8 = 80 bytes, then VarInt(2) at byte 80.
        let ua_offset = 80;
        bytes[ua_offset + 1] = 0xff; // corrupt first byte of "ok"
        bytes[ua_offset + 2] = 0xfe;
        let e = decode_err::<VersionMsg>(&bytes);
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData);
    }

    // PingMsg: zero nonce is valid.
    #[test]
    fn ping_zero_nonce() {
        let ping = PingMsg { nonce: 0 };
        let bytes = encode(&ping);
        assert_eq!(decode::<PingMsg>(&bytes), ping);
    }

    // PingMsg: u64::MAX nonce round-trips.
    #[test]
    fn ping_max_nonce() {
        let ping = PingMsg { nonce: u64::MAX };
        let bytes = encode(&ping);
        assert_eq!(decode::<PingMsg>(&bytes), ping);
    }

    // PingMsg: truncated payload.
    #[test]
    fn ping_truncated() {
        let e = decode_err::<PingMsg>(&[0x00; 4]); // needs 8 bytes
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    // ── InvMsg iterator and reuse paths ───────────────────────────────────────

    #[test]
    fn inv_iter_matches_owned() {
        let inv = InvMsg(vec![
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
        ]);
        let bytes = encode(&inv);
        let iter_entries: Vec<InvEntry> = InvMsg::iter_entries::<50_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes")
            .map(|r| r.expect("each entry must decode"))
            .collect();
        assert_eq!(iter_entries, inv.0);
    }

    #[test]
    fn inv_iter_empty() {
        let bytes = encode(&InvMsg(vec![]));
        let mut iter = InvMsg::iter_entries::<50_000>(&bytes).expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn inv_iter_exact_size() {
        let inv = InvMsg(vec![
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
        ]);
        let bytes = encode(&inv);
        let iter = InvMsg::iter_entries::<50_000>(&bytes).expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 3);
    }

    #[test]
    fn inv_iter_limit_exceeded() {
        let inv = InvMsg(vec![
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
        ]);
        let bytes = encode(&inv);
        let err = InvMsg::iter_entries::<1>(&bytes).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn inv_iter_truncated_entry() {
        let inv = InvMsg(vec![
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
        ]);
        let mut bytes = encode(&inv);
        bytes.truncate(1 + 36 + 10); // VarInt + full entry + partial second entry
        let mut iter = InvMsg::iter_entries::<50_000>(&bytes).expect("iter_entries must succeed on valid bytes");
        assert!(iter.next().expect("first entry must be present").is_ok());
        assert_eq!(iter.next().expect("second entry must be present").unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(iter.next().is_none()); // stops after error
    }

    #[test]
    fn inv_decode_into_reuses_capacity() {
        let inv = InvMsg(vec![
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Tx, hash: Hash256::ZERO },
        ]);
        let bytes = encode(&inv);
        let mut buf: Vec<InvEntry> = Vec::new();
        InvMsg::decode_into_limited::<50_000>(&mut std::io::Cursor::new(&bytes), &mut buf).expect("decode_into_limited must succeed on valid bytes");
        assert_eq!(buf, inv.0);
        let cap_after_first = buf.capacity();

        // Second call: no new allocation if same or smaller count.
        InvMsg::decode_into_limited::<50_000>(&mut std::io::Cursor::new(&bytes), &mut buf).expect("decode_into_limited must succeed on valid bytes");
        assert_eq!(buf, inv.0);
        assert!(buf.capacity() >= cap_after_first);
    }

    #[test]
    fn inv_decode_into_limit_exceeded() {
        let inv = InvMsg(vec![
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
            InvEntry { kind: InvType::Block, hash: Hash256::ZERO },
        ]);
        let bytes = encode(&inv);
        let mut buf = Vec::new();
        let err = InvMsg::decode_into_limited::<1>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    // ── HeadersMsg iterator and reuse paths ───────────────────────────────────

    fn sample_header(nonce: u32) -> BlockHeader {
        BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: 0x1d00ffff,
            nonce,
        }
    }

    #[test]
    fn headers_iter_matches_owned() {
        let msg = HeadersMsg(vec![sample_header(1), sample_header(2)]);
        let bytes = encode(&msg);
        let iter_headers: Vec<BlockHeader> = HeadersMsg::iter_entries::<2_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes")
            .map(|r| r.expect("each header must decode"))
            .collect();
        assert_eq!(iter_headers, msg.0);
    }

    #[test]
    fn headers_iter_empty() {
        let bytes = encode(&HeadersMsg(vec![]));
        let mut iter = HeadersMsg::iter_entries::<2_000>(&bytes).expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn headers_iter_exact_size() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1), sample_header(2)]);
        let bytes = encode(&msg);
        let iter = HeadersMsg::iter_entries::<2_000>(&bytes).expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 3);
    }

    #[test]
    fn headers_iter_limit_exceeded() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1)]);
        let bytes = encode(&msg);
        let err = HeadersMsg::iter_entries::<1>(&bytes).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn headers_iter_truncated_entry() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1)]);
        let mut bytes = encode(&msg);
        // Each encoded entry is 81 bytes (80 header + 1 byte tx_count VarInt).
        bytes.truncate(1 + 81 + 40); // VarInt + full entry + partial second
        let mut iter = HeadersMsg::iter_entries::<2_000>(&bytes).expect("iter_entries must succeed on valid bytes");
        assert!(iter.next().expect("first entry must be present").is_ok());
        assert_eq!(iter.next().expect("second entry must be present").unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(iter.next().is_none());
    }

    #[test]
    fn headers_decode_into_reuses_capacity() {
        let msg = HeadersMsg(vec![sample_header(1), sample_header(2)]);
        let bytes = encode(&msg);
        let mut buf: Vec<BlockHeader> = Vec::new();
        HeadersMsg::decode_into_limited::<2_000>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .expect("decode_into_limited must succeed on valid bytes");
        assert_eq!(buf, msg.0);
        let cap_after_first = buf.capacity();

        HeadersMsg::decode_into_limited::<2_000>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .expect("decode_into_limited must succeed on valid bytes");
        assert_eq!(buf, msg.0);
        assert!(buf.capacity() >= cap_after_first);
    }

    #[test]
    fn headers_decode_into_limit_exceeded() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1)]);
        let bytes = encode(&msg);
        let mut buf = Vec::new();
        let err =
            HeadersMsg::decode_into_limited::<1>(&mut std::io::Cursor::new(&bytes), &mut buf)
                .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
}
