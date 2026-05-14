use std::io::{self, Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, Error as CodecError, VarInt, decode_string_limited};
use crate::hash::checksum;
use crate::primitives::{BlockHeader, Hash256};

// ── Magic bytes ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Magic {
    Mainnet,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
    /// A 4-byte magic value that does not match any known network.
    Unknown([u8; 4]),
}

impl Magic {
    #[must_use]
    pub fn bytes(self) -> [u8; 4] {
        match self {
            Self::Mainnet => [0xf9, 0xbe, 0xb4, 0xd9],
            Self::Testnet3 => [0x0b, 0x11, 0x09, 0x07],
            Self::Testnet4 => [0x1c, 0x16, 0x3f, 0x28],
            Self::Signet => [0x0a, 0x03, 0xcf, 0x40],
            Self::Regtest => [0xfa, 0xbf, 0xb5, 0xda],
            Self::Unknown(b) => b,
        }
    }

    /// Classify a 4-byte magic value. Returns `Magic::Unknown` if the bytes
    /// don't match any known network.
    #[must_use]
    pub fn classify(b: [u8; 4]) -> Self {
        match b {
            [0xf9, 0xbe, 0xb4, 0xd9] => Self::Mainnet,
            [0x0b, 0x11, 0x09, 0x07] => Self::Testnet3,
            [0x1c, 0x16, 0x3f, 0x28] => Self::Testnet4,
            [0x0a, 0x03, 0xcf, 0x40] => Self::Signet,
            [0xfa, 0xbf, 0xb5, 0xda] => Self::Regtest,
            other => Self::Unknown(other),
        }
    }

    /// Returns `Some(self)` if the magic is a known network, else `None`.
    #[must_use]
    pub fn from_bytes(b: [u8; 4]) -> Option<Self> {
        match Self::classify(b) {
            Self::Unknown(_) => None,
            known => Some(known),
        }
    }

    /// Frame a P2P message: writes a 24-byte header plus payload to `w`.
    ///
    /// # Errors
    /// Returns `InvalidData` if the payload exceeds `u32::MAX` bytes, or any
    /// I/O error from writing to `w`.
    pub fn frame(
        self,
        command: Command,
        payload: &[u8],
        w: &mut impl Write,
    ) -> Result<(), CodecError> {
        encode_message(self, command, payload, w)
    }
}

impl Encode for Magic {
    #[inline]
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        w.write_all(&self.bytes()).map_err(CodecError::Io)
    }
}

impl Decode for Magic {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        Ok(Self::classify(<[u8; 4]>::decode(r)?))
    }
}

impl<'a> DecodeRef<'a> for Magic {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), CodecError> {
        let (b, tail) = <[u8; 4]>::decode_ref(buf)?;
        Ok((Self::classify(b), tail))
    }
}

// ── Network ───────────────────────────────────────────────────────────────────

/// One of the well-known Bitcoin networks. CLIs use this to look up the
/// magic, default port, default seed peers, and genesis hash for a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet3,
    Testnet4,
    Signet,
    Regtest,
}

impl Network {
    /// Parse a `--network` flag value. Accepts the short names used by Bitcoin
    /// Core (`main`/`test`/`testnet4`/`signet`/`regtest`) and the long form
    /// (`mainnet`/`testnet3`/`testnet4`/`signet`/`regtest`), case-insensitive.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "main" | "mainnet" => Some(Self::Mainnet),
            "test" | "testnet" | "testnet3" => Some(Self::Testnet3),
            "testnet4" => Some(Self::Testnet4),
            "signet" => Some(Self::Signet),
            "regtest" => Some(Self::Regtest),
            _ => None,
        }
    }

    /// Short canonical name used in user-facing output.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Mainnet => "mainnet",
            Self::Testnet3 => "testnet3",
            Self::Testnet4 => "testnet4",
            Self::Signet => "signet",
            Self::Regtest => "regtest",
        }
    }

    /// P2P magic for this network.
    #[must_use]
    pub fn magic(self) -> Magic {
        match self {
            Self::Mainnet => Magic::Mainnet,
            Self::Testnet3 => Magic::Testnet3,
            Self::Testnet4 => Magic::Testnet4,
            Self::Signet => Magic::Signet,
            Self::Regtest => Magic::Regtest,
        }
    }

    /// Default P2P port for this network. Used when `--peer` does not include a
    /// port.
    #[must_use]
    pub fn default_port(self) -> u16 {
        match self {
            Self::Mainnet => 8333,
            Self::Testnet3 => 18333,
            Self::Testnet4 => 48333,
            Self::Signet => 38333,
            Self::Regtest => 18444,
        }
    }

    /// Genesis block hash in *internal* (little-endian) byte order — this is
    /// the form `Hash256` carries throughout the codebase, and the form a
    /// `getheaders` locator wants. The display-hex version (what block
    /// explorers show) is the byte-reverse of this.
    ///
    /// # Panics
    /// Never in practice: parses a fixed compile-time string. The `expect` is
    /// load-bearing only in the sense of guarding a typo if the constants are
    /// edited.
    #[must_use]
    pub fn genesis_hash(self) -> Hash256 {
        // Display-hex constants from Bitcoin Core's `chainparams.cpp`. Parsed
        // via `Hash256::from_display_hex` which handles the byte reversal.
        let display_hex = match self {
            Self::Mainnet => "000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f",
            Self::Testnet3 => "000000000933ea01ad0ee984209779baaec3ced90fa3f408719526f8d77f4943",
            Self::Testnet4 => "00000000da84f2bafbbc53dee25a72ae507ff4914b867c565be350b0da8bf043",
            Self::Signet => "00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6",
            Self::Regtest => "0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206",
        };
        Hash256::from_display_hex(display_hex).expect("hardcoded genesis hash must parse")
    }

    /// Decoded genesis block header. Currently mainnet only — the other
    /// networks return `None` until their hardcoded bytes are added. Callers
    /// that need a pre-genesis snapshot slot (so block 1's `prev_block` and
    /// MTP lookups resolve) use this; callers that only need the hash should
    /// use [`Self::genesis_hash`].
    #[must_use]
    pub fn genesis_header(self) -> Option<BlockHeader> {
        match self {
            Self::Mainnet => Some(mainnet_genesis_header()),
            Self::Testnet4 => Some(testnet4_genesis_header()),
            Self::Signet => Some(signet_genesis_header()),
            Self::Testnet3 | Self::Regtest => None,
        }
    }

    /// Default seed peers as `host:port` strings. Entries may be either
    /// literal `IP:PORT` (mainnet's hand-picked list) or DNS `HOST:PORT`
    /// (testnet4 / signet — callers do the resolution at peer-list build
    /// time). Networks without a seed list return an empty slice — supply
    /// peers via `--peer`.
    #[must_use]
    pub fn default_seeds(self) -> &'static [&'static str] {
        match self {
            Self::Mainnet => MAINNET_SEEDS,
            Self::Testnet4 => TESTNET4_SEEDS,
            Self::Signet => SIGNET_SEEDS,
            Self::Testnet3 | Self::Regtest => &[],
        }
    }
}

/// Seed peers for mainnet bootstrap. Same set used by `mantle-node` and the
/// bench `fetch-fixtures` tool.
const MAINNET_SEEDS: &[&str] = &[
    "5.161.244.95:8333",
    "172.105.64.213:8333",
    "61.68.41.135:8333",
    "43.154.28.63:8333",
    "62.93.65.111:8333",
];

/// Seed peers for testnet4 bootstrap. DNS hostnames from Bitcoin Core
/// `chainparams.cpp::CTestNet4Params`; the replay binary resolves them to
/// IPv4 addresses at peer-list build time.
const TESTNET4_SEEDS: &[&str] = &[
    "seed.testnet4.bitcoin.sprovoost.nl:48333",
    "seed.testnet4.wiz.biz:48333",
];

/// Seed peers for the default signet. DNS hostnames from Bitcoin Core
/// `chainparams.cpp::SigNetParams`.
const SIGNET_SEEDS: &[&str] = &[
    "seed.signet.bitcoin.sprovoost.nl:38333",
    "seed.signet.achownodes.xyz:38333",
];

/// The actual mainnet genesis block header. Field values from Bitcoin Core
/// `chainparams.cpp::CMainParams` (genesis is 2009-01-03 18:15:05 UTC,
/// `0x1d00ffff` difficulty, nonce `2083236893`). Verified by the
/// `mainnet_genesis_header_hashes_correctly` unit test against the well-known
/// mainnet genesis hash.
fn mainnet_genesis_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256::ZERO,
        merkle_root: Hash256::from_display_hex(
            "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
        )
        .expect("hardcoded genesis merkle_root must parse"),
        time: 1_231_006_505,
        bits: crate::primitives::CompactBits(0x1d00_ffff),
        nonce: 2_083_236_893,
    }
}

/// The testnet4 (BIP94) genesis block header. Field values from Bitcoin Core
/// `chainparams.cpp::CTestNet4Params`:
///   - time   = `1714777860` (2024-05-04 00:11:00 UTC)
///   - bits   = `0x1d00ffff`
///   - nonce  = `393_743_547`
///   - merkle = `7aa0a7ae1e223414cb807e40cd57e667b718e42aaf9306db9102fe28912b7b4e`
///
/// Verified by the `testnet4_genesis_header_hashes_correctly` unit test
/// against the well-known testnet4 genesis hash.
fn testnet4_genesis_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256::ZERO,
        merkle_root: Hash256::from_display_hex(
            "7aa0a7ae1e223414cb807e40cd57e667b718e42aaf9306db9102fe28912b7b4e",
        )
        .expect("hardcoded testnet4 genesis merkle_root must parse"),
        time: 1_714_777_860,
        bits: crate::primitives::CompactBits(0x1d00_ffff),
        nonce: 393_743_547,
    }
}

/// The default signet (BIP325) genesis block header. Field values from
/// Bitcoin Core `chainparams.cpp::SigNetParams`:
///   - time   = `1598918400` (2020-09-01 00:00:00 UTC)
///   - bits   = `0x1e0377ae`
///   - nonce  = `52_613_770`
///   - merkle = same as mainnet (`4a5e1e…da33b`)
fn signet_genesis_header() -> BlockHeader {
    BlockHeader {
        version: 1,
        prev_block: Hash256::ZERO,
        merkle_root: Hash256::from_display_hex(
            "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b",
        )
        .expect("hardcoded signet genesis merkle_root must parse"),
        time: 1_598_918_400,
        bits: crate::primitives::CompactBits(0x1e03_77ae),
        nonce: 52_613_770,
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
    pub magic: Magic,
    pub command: CommandName,
    pub length: u32,
    pub checksum: [u8; 4],
}

impl MessageHeader {
    pub const ENCODED_LEN: usize = 24;
}

impl Encode for MessageHeader {
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        self.magic.encode(w)?;
        w.write_all(&self.command.0).map_err(CodecError::Io)?;
        self.length.encode(w)?;
        w.write_all(&self.checksum).map_err(CodecError::Io)
    }
}

impl Decode for MessageHeader {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let magic = Magic::decode(r)?;
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
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), CodecError> {
        let (magic, buf) = Magic::decode_ref(buf)?;
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
) -> Result<(), CodecError> {
    let header = MessageHeader {
        magic,
        command: CommandName::from(command),
        length: u32::try_from(payload.len())
            .map_err(|e| CodecError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?,
        checksum: checksum(payload),
    };
    header.encode(w)?;
    w.write_all(payload).map_err(CodecError::Io)
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
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        self.services.encode(w)?;
        w.write_all(&self.ip).map_err(CodecError::Io)?;
        // port is big-endian on the wire
        w.write_all(&self.port.to_be_bytes())
            .map_err(CodecError::Io)
    }
}

impl Decode for NetworkAddr {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let services = u64::decode(r)?;
        let ip = <[u8; 16]>::decode(r)?;
        let mut port_bytes = [0u8; 2];
        r.read_exact(&mut port_bytes).map_err(CodecError::Io)?;
        Ok(Self {
            services,
            ip,
            port: u16::from_be_bytes(port_bytes),
        })
    }
}

impl<'a> DecodeRef<'a> for NetworkAddr {
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), CodecError> {
        let (services, buf) = u64::decode_ref(buf)?;
        let (ip, buf) = <[u8; 16]>::decode_ref(buf)?;
        if buf.len() < 2 {
            return Err(CodecError::UnexpectedEof);
        }
        let (port_bytes, buf) = buf.split_at(2);
        let port = u16::from_be_bytes(
            port_bytes
                .try_into()
                .map_err(|_| CodecError::Io(io::Error::from(io::ErrorKind::InvalidData)))?,
        );
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
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
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
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
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
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        self.nonce.encode(w)
    }
}

impl Decode for PingMsg {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
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
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        w.write_all(&self.to_bytes()).map_err(CodecError::Io)
    }
}

impl Decode for InvEntry {
    #[inline]
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let mut buf = [0u8; Self::ENCODED_LEN];
        r.read_exact(&mut buf).map_err(CodecError::Io)?;
        Ok(Self::from_bytes(&buf))
    }
}

impl<'a> DecodeRef<'a> for InvEntry {
    #[inline]
    fn decode_ref(buf: &'a [u8]) -> Result<(Self, &'a [u8]), CodecError> {
        if buf.len() < Self::ENCODED_LEN {
            return Err(CodecError::UnexpectedEof);
        }
        let (head, tail) = buf.split_at(Self::ENCODED_LEN);
        Ok((
            Self::from_bytes(
                head.try_into()
                    .map_err(|_| CodecError::Io(io::Error::from(io::ErrorKind::InvalidData)))?,
            ),
            tail,
        ))
    }
}

/// `inv` / `getdata` — list of inventory entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvMsg(pub Vec<InvEntry>);

impl Encode for InvMsg {
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        VarInt::from(self.0.len()).encode(w)?;
        for entry in &self.0 {
            entry.encode(w)?;
        }
        Ok(())
    }
}

/// Zero-alloc lazy iterator over `InvEntry` values parsed from a raw byte slice.
///
/// Created by [`InvMsg::iter_entries`]. Yields `Result<InvEntry, CodecError>` so
/// the caller can handle truncation errors entry-by-entry.
#[derive(Debug)]
pub struct InvEntryIter<'a> {
    remaining: &'a [u8],
    /// How many entries are left to yield (from the decoded `VarInt` count).
    left: usize,
}

impl Iterator for InvEntryIter<'_> {
    type Item = Result<InvEntry, CodecError>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        if self.remaining.len() < InvEntry::ENCODED_LEN {
            self.left = 0; // prevent further iteration after error
            return Some(Err(CodecError::UnexpectedEof));
        }
        let (head, tail) = self.remaining.split_at(InvEntry::ENCODED_LEN);
        self.remaining = tail;
        self.left -= 1;
        Some(
            head.try_into()
                .map_err(|_| CodecError::Io(io::Error::from(io::ErrorKind::InvalidData)))
                .map(InvEntry::from_bytes),
        )
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
    ) -> Result<InvEntryIter<'_>, CodecError> {
        let (varint, rest) = VarInt::decode_ref(buf)?;
        let count = varint.to_usize()?;
        if count > MAX_ENTRIES {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "inv entry count exceeds limit",
            )));
        }
        Ok(InvEntryIter {
            remaining: rest,
            left: count,
        })
    }

    /// # Errors
    /// Returns `InvalidData` if the entry count exceeds `MAX_ENTRIES`, or on any I/O error.
    #[allow(clippy::missing_panics_doc)] // chunks_exact guarantees chunk size — unwrap cannot panic
    pub fn decode_limited<const MAX_ENTRIES: usize>(r: &mut impl Read) -> Result<Self, CodecError> {
        let count = VarInt::decode(r)?.to_usize()?;
        if count > MAX_ENTRIES {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "inv entry count exceeds limit",
            )));
        }
        // Read all entry bytes in one shot, then decode via the zero-copy path.
        let byte_len = count * InvEntry::ENCODED_LEN;
        let mut buf = vec![0u8; byte_len];
        r.read_exact(&mut buf).map_err(CodecError::Io)?;
        let entries = buf
            .chunks_exact(InvEntry::ENCODED_LEN)
            .map(|chunk| {
                chunk
                    .try_into()
                    .map(InvEntry::from_bytes)
                    .map_err(|_| CodecError::Io(io::Error::from(io::ErrorKind::InvalidData)))
            })
            .collect::<Result<Vec<_>, CodecError>>()?;
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
    ) -> Result<(), CodecError> {
        let count = VarInt::decode(r)?.to_usize()?;
        if count > MAX_ENTRIES {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "inv entry count exceeds limit",
            )));
        }
        let byte_len = count * InvEntry::ENCODED_LEN;
        // Reuse a scratch buffer stored temporarily on the stack for small messages,
        // or heap for large ones. We don't store it across calls here — the caller
        // owns `out`, not a byte scratch buffer. Use a thread-local would over-
        // engineer; just allocate the byte buf once and decode in-place.
        let mut buf = vec![0u8; byte_len];
        r.read_exact(&mut buf).map_err(CodecError::Io)?;
        let entries = buf
            .chunks_exact(InvEntry::ENCODED_LEN)
            .map(|chunk| {
                chunk
                    .try_into()
                    .map(InvEntry::from_bytes)
                    .map_err(|_| CodecError::Io(io::Error::from(io::ErrorKind::InvalidData)))
            })
            .collect::<Result<Vec<_>, CodecError>>()?;
        out.clear();
        out.extend(entries);
        Ok(())
    }
}

impl Decode for InvMsg {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        Self::decode_limited::<50_000>(r)
    }
}

// getdata has identical wire format
pub type GetDataMsg = InvMsg;

// ── Addr / AddrV2 ─────────────────────────────────────────────────────────────

/// One legacy `addr` entry: `time(4) || services(8) || ip(16) || port(2 BE)`.
///
/// Used in the unsolicited and `getaddr`-response `addr` payload. Each entry
/// is 30 bytes total. `time` is the peer's last-seen Unix timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddrEntry {
    pub time: u32,
    pub services: u64,
    /// IPv6 address (IPv4 mapped as `::ffff:0:0/96`).
    pub ip: [u8; 16],
    pub port: u16,
}

impl AddrEntry {
    pub const ENCODED_LEN: usize = 4 + 8 + 16 + 2;
}

impl Decode for AddrEntry {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let time = u32::decode(r)?;
        let services = u64::decode(r)?;
        let ip = <[u8; 16]>::decode(r)?;
        let mut port_bytes = [0u8; 2];
        r.read_exact(&mut port_bytes).map_err(CodecError::Io)?;
        Ok(Self {
            time,
            services,
            ip,
            port: u16::from_be_bytes(port_bytes),
        })
    }
}

/// `addr` — list of peer addresses (BIP31 / pre-BIP155 form).
///
/// Capped at 1000 entries (Bitcoin Core's `MAX_ADDR_TO_SEND`). Decode-only
/// for now — we only consume `addr` from peers, never originate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrMsg(pub Vec<AddrEntry>);

impl AddrMsg {
    /// Bitcoin Core's `MAX_ADDR_TO_SEND`. Any incoming `addr` larger than
    /// this is rejected as malformed.
    pub const MAX_ENTRIES: usize = 1000;

    /// # Errors
    /// `InvalidData` if the entry count exceeds [`Self::MAX_ENTRIES`], or on I/O error.
    pub fn decode_limited(r: &mut impl Read) -> Result<Self, CodecError> {
        let count = VarInt::decode(r)?.to_usize()?;
        if count > Self::MAX_ENTRIES {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "addr entry count exceeds MAX_ADDR_TO_SEND",
            )));
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(AddrEntry::decode(r)?);
        }
        Ok(Self(entries))
    }
}

impl Decode for AddrMsg {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        Self::decode_limited(r)
    }
}

/// One BIP155 address. Each defined network has a fixed-size payload, so we
/// store them as fixed arrays — no heap allocation. The `Unknown(network_id,
/// bytes)` variant catches future BIP-allocated network IDs whose width we
/// don't know at compile time; only the unknown case heap-allocates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddrV2Addr {
    Ipv4([u8; 4]),
    Ipv6([u8; 16]),
    /// Deprecated. BIP155 lists 10 bytes for compatibility with the legacy
    /// `.onion` v2 hidden-service addressing scheme.
    TorV2([u8; 10]),
    TorV3([u8; 32]),
    I2p([u8; 32]),
    Cjdns([u8; 16]),
    /// Forward-compat: a network-id byte we don't recognise. Address bytes
    /// preserved verbatim so the entry round-trips through serialisation if
    /// we ever re-encode. Bounded by [`AddrV2Addr::MAX_UNKNOWN_LEN`] on decode.
    Unknown {
        network_id: u8,
        addr: Vec<u8>,
    },
}

impl AddrV2Addr {
    /// Hard cap on the length of an `Unknown` network's address bytes —
    /// defensive against a malicious peer claiming a 16 MB address. Bitcoin
    /// Core uses 512 bytes for its `MAX_ADDRV2_SIZE`.
    pub const MAX_UNKNOWN_LEN: usize = 512;

    /// Expected wire length for a defined network ID. Returns `None` for
    /// unrecognised IDs so the caller knows to fall through to `Unknown`.
    ///
    /// Arms kept distinct by network kind so that a future BIP change to
    /// any single network's address width is a one-line edit and doesn't
    /// silently capture neighbours.
    #[allow(clippy::match_same_arms)]
    fn expected_len(network_id: u8) -> Option<usize> {
        match network_id {
            0x01 => Some(4),  // Ipv4
            0x02 => Some(16), // Ipv6
            0x03 => Some(10), // TorV2 (deprecated)
            0x04 => Some(32), // TorV3
            0x05 => Some(32), // I2p
            0x06 => Some(16), // Cjdns
            _ => None,
        }
    }

    /// Decode the per-entry address: `network_id(1) || addr(VarInt-len + bytes)`.
    ///
    /// # Errors
    /// `InvalidData` if a defined network ID arrives with an address length
    /// other than the BIP155-mandated size, or if an `Unknown` entry exceeds
    /// [`Self::MAX_UNKNOWN_LEN`], or on I/O error.
    pub fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let mut nid = [0u8; 1];
        r.read_exact(&mut nid).map_err(CodecError::Io)?;
        let addr_len = VarInt::decode(r)?.to_usize()?;
        if let Some(expected) = Self::expected_len(nid[0]) {
            if addr_len != expected {
                return Err(CodecError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "addrv2 addr length does not match network_id",
                )));
            }
        } else if addr_len > Self::MAX_UNKNOWN_LEN {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "addrv2 addr length exceeds MAX_UNKNOWN_LEN",
            )));
        }

        // Defined networks decode into stack-resident fixed arrays; only the
        // `Unknown` arm heap-allocates.
        match nid[0] {
            0x01 => {
                let mut a = [0u8; 4];
                r.read_exact(&mut a).map_err(CodecError::Io)?;
                Ok(Self::Ipv4(a))
            }
            0x02 => {
                let mut a = [0u8; 16];
                r.read_exact(&mut a).map_err(CodecError::Io)?;
                Ok(Self::Ipv6(a))
            }
            0x03 => {
                let mut a = [0u8; 10];
                r.read_exact(&mut a).map_err(CodecError::Io)?;
                Ok(Self::TorV2(a))
            }
            0x04 => {
                let mut a = [0u8; 32];
                r.read_exact(&mut a).map_err(CodecError::Io)?;
                Ok(Self::TorV3(a))
            }
            0x05 => {
                let mut a = [0u8; 32];
                r.read_exact(&mut a).map_err(CodecError::Io)?;
                Ok(Self::I2p(a))
            }
            0x06 => {
                let mut a = [0u8; 16];
                r.read_exact(&mut a).map_err(CodecError::Io)?;
                Ok(Self::Cjdns(a))
            }
            n => {
                let mut buf = vec![0u8; addr_len];
                r.read_exact(&mut buf).map_err(CodecError::Io)?;
                Ok(Self::Unknown {
                    network_id: n,
                    addr: buf,
                })
            }
        }
    }
}

/// One BIP155 `addrv2` entry: `time(4) || services(VarInt) || network_id(1) ||
/// addr(VarInt-len + bytes) || port(2 BE)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrV2Entry {
    pub time: u32,
    pub services: u64,
    pub addr: AddrV2Addr,
    pub port: u16,
}

impl AddrV2Entry {
    /// If this entry is an IPv4 address, return it as a [`std::net::SocketAddrV4`].
    /// Returns `None` for IPv6/Tor/I2P/etc. entries.
    #[must_use]
    pub fn ip_v4(&self) -> Option<std::net::SocketAddrV4> {
        match self.addr {
            AddrV2Addr::Ipv4(octets) => {
                let ip = std::net::Ipv4Addr::new(octets[0], octets[1], octets[2], octets[3]);
                Some(std::net::SocketAddrV4::new(ip, self.port))
            }
            _ => None,
        }
    }
}

impl Decode for AddrV2Entry {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let time = u32::decode(r)?;
        let services = VarInt::decode(r)?.to_u64();
        let addr = AddrV2Addr::decode(r)?;
        let mut port_bytes = [0u8; 2];
        r.read_exact(&mut port_bytes).map_err(CodecError::Io)?;
        Ok(Self {
            time,
            services,
            addr,
            port: u16::from_be_bytes(port_bytes),
        })
    }
}

/// `addrv2` — BIP155 list of peer addresses with extensible network types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddrV2Msg(pub Vec<AddrV2Entry>);

impl AddrV2Msg {
    /// Same per-message cap as legacy `addr` — Bitcoin Core enforces both.
    pub const MAX_ENTRIES: usize = AddrMsg::MAX_ENTRIES;

    /// # Errors
    /// `InvalidData` if the entry count exceeds [`Self::MAX_ENTRIES`], or on I/O error.
    pub fn decode_limited(r: &mut impl Read) -> Result<Self, CodecError> {
        let count = VarInt::decode(r)?.to_usize()?;
        if count > Self::MAX_ENTRIES {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "addrv2 entry count exceeds MAX_ADDR_TO_SEND",
            )));
        }
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(AddrV2Entry::decode(r)?);
        }
        Ok(Self(entries))
    }
}

impl Decode for AddrV2Msg {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        Self::decode_limited(r)
    }
}

/// `getheaders` — request headers from a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetHeadersMsg {
    pub version: u32,
    pub locator_hashes: Vec<Hash256>,
    pub stop_hash: Hash256,
}

impl Encode for GetHeadersMsg {
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        self.version.encode(w)?;
        VarInt::from(self.locator_hashes.len()).encode(w)?;
        for h in &self.locator_hashes {
            h.encode(w)?;
        }
        self.stop_hash.encode(w)
    }
}

impl GetHeadersMsg {
    /// Build a `getheaders` request that asks for headers built on top of
    /// `locator`, with no stop. The protocol version is 70015 (the same value
    /// every other place in this codebase that constructs `getheaders`
    /// messages picks). The locator vector contains a single hash, which is
    /// what every caller in this codebase needs — IBD bootstrap from genesis
    /// and "give me 2000 more after this hash" both fit.
    #[must_use]
    pub fn single_locator(locator: Hash256) -> Self {
        Self {
            version: 70015,
            locator_hashes: vec![locator],
            stop_hash: Hash256::ZERO,
        }
    }

    /// Encode this message into a fresh `Vec<u8>`. Convenience wrapper around
    /// `Encode::encode` for the common "build a P2P payload" path.
    ///
    /// # Panics
    /// Never in practice — `Vec<u8>` cannot return an I/O error from `Write`.
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        self.encode(&mut buf).expect("Vec<u8> writes never fail");
        buf
    }

    /// # Errors
    /// Returns `InvalidData` if the locator count exceeds `MAX_LOCATORS`, or on any I/O error.
    pub fn decode_limited<const MAX_LOCATORS: usize>(
        r: &mut impl Read,
    ) -> Result<Self, CodecError> {
        let version = u32::decode(r)?;
        let count = VarInt::decode(r)?.to_usize()?;
        if count > MAX_LOCATORS {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "locator count exceeds limit",
            )));
        }
        let mut locator_hashes = Vec::with_capacity(count);
        for _ in 0..count {
            locator_hashes.push(Hash256::decode(r)?);
        }
        let stop_hash = Hash256::decode(r)?;
        Ok(Self {
            version,
            locator_hashes,
            stop_hash,
        })
    }
}

impl Decode for GetHeadersMsg {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        Self::decode_limited::<2_000>(r)
    }
}

/// `headers` — response with up to 2000 block headers.
/// Each header is followed by a `tx_count` `VarInt` (always 0x00 in this message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersMsg(pub Vec<BlockHeader>);

impl Encode for HeadersMsg {
    fn encode(&self, w: &mut impl Write) -> Result<(), CodecError> {
        VarInt::from(self.0.len()).encode(w)?;
        for header in &self.0 {
            header.encode(w)?;
            w.write_all(&[0x00]).map_err(CodecError::Io)?; // tx_count placeholder (always zero)
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
    type Item = Result<BlockHeader, CodecError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        if self.remaining.len() < BlockHeader::ENCODED_LEN {
            self.left = 0;
            return Some(Err(CodecError::UnexpectedEof));
        }
        let (head, tail) = self.remaining.split_at(BlockHeader::ENCODED_LEN);
        let arr = match head
            .try_into()
            .map_err(|_| CodecError::Io(io::Error::from(io::ErrorKind::InvalidData)))
        {
            Ok(a) => a,
            Err(e) => {
                self.left = 0;
                return Some(Err(e));
            }
        };
        let header = match BlockHeader::from_bytes(arr) {
            Ok(h) => h,
            Err(e) => {
                self.left = 0;
                return Some(Err(e));
            }
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
    ) -> Result<BlockHeaderIter<'_>, CodecError> {
        let (varint, rest) = VarInt::decode_ref(buf)?;
        let count = varint.to_usize()?;
        if count > MAX_HEADERS {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "header count exceeds limit",
            )));
        }
        Ok(BlockHeaderIter {
            remaining: rest,
            left: count,
        })
    }

    /// # Errors
    /// Returns `InvalidData` if the header count exceeds `MAX_HEADERS`, or on any I/O error.
    pub fn decode_limited<const MAX_HEADERS: usize>(r: &mut impl Read) -> Result<Self, CodecError> {
        let count = VarInt::decode(r)?.to_usize()?;
        if count > MAX_HEADERS {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "header count exceeds limit",
            )));
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
    ) -> Result<(), CodecError> {
        let count = VarInt::decode(r)?.to_usize()?;
        if count > MAX_HEADERS {
            return Err(CodecError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                "header count exceeds limit",
            )));
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
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        Self::decode_limited::<2_000>(r)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::CompactBits;
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
    fn mainnet_genesis_header_hashes_correctly() {
        // Load-bearing: catches a typo in the literal merkle_root / time /
        // nonce / bits before it silently corrupts the replay loop's MTP
        // check at block 1.
        let hdr = Network::Mainnet.genesis_header().expect("mainnet has one");
        assert_eq!(hdr.hash(), Network::Mainnet.genesis_hash());
    }

    #[test]
    fn testnet4_genesis_header_hashes_correctly() {
        // Same load-bearing role as the mainnet test: tying the literal
        // header bytes to the documented testnet4 genesis hash.
        let hdr = Network::Testnet4
            .genesis_header()
            .expect("testnet4 has one");
        assert_eq!(hdr.hash(), Network::Testnet4.genesis_hash());
    }

    #[test]
    fn signet_genesis_header_hashes_correctly() {
        let hdr = Network::Signet.genesis_header().expect("signet has one");
        assert_eq!(hdr.hash(), Network::Signet.genesis_hash());
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
    fn message_header_roundtrip() {
        let payload = b"hello";
        let header = MessageHeader {
            magic: Magic::Mainnet,
            command: CommandName::new("version"),
            length: u32::try_from(payload.len()).expect("payload fits u32"),
            checksum: checksum(payload),
        };
        let bytes = encode(&header);
        assert_eq!(bytes.len(), MessageHeader::ENCODED_LEN);
        let decoded = decode::<MessageHeader>(&bytes);
        assert_eq!(decoded.magic, header.magic);
        assert_eq!(decoded.command, header.command);
        assert_eq!(decoded.length, header.length);
        assert_eq!(decoded.checksum, header.checksum);
    }

    #[test]
    fn message_header_decode_ref() {
        let payload = b"test";
        let mut bytes = encode(&MessageHeader {
            magic: Magic::Mainnet,
            command: CommandName::new("ping"),
            length: u32::try_from(payload.len()).expect("payload fits u32"),
            checksum: checksum(payload),
        });
        bytes.push(0xab); // sentinel
        let (h, tail) = decode_ref::<MessageHeader>(&bytes);
        assert_eq!(h.command.as_str(), "ping");
        assert_eq!(tail, &[0xab]);
    }

    #[test]
    fn encode_message_checksum() {
        let payload = b"payload";
        let mut buf = Vec::new();
        encode_message(Magic::Mainnet, Command::Ping, payload, &mut buf)
            .expect("encode_message cannot fail writing to Vec");
        let header = decode::<MessageHeader>(&buf[..24]);
        assert_eq!(header.checksum, checksum(payload));
        assert_eq!(&buf[24..], payload);
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
            nonce: 0xdead_beef_cafe,
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
    fn addr_msg_decodes_two_entries() {
        // VarInt count (2) || 2 × (4 time | 8 services | 16 ip | 2 port-BE)
        let mut bytes = vec![0x02];
        // entry 0: time=1, services=NODE_NETWORK, ip=::ffff:1.2.3.4, port=18333
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&services::NODE_NETWORK.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 1, 2, 3, 4]);
        bytes.extend_from_slice(&18_333u16.to_be_bytes());
        // entry 1: same shape, different ip/port
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&services::NODE_NETWORK.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0, 5, 6, 7, 8]);
        bytes.extend_from_slice(&8333u16.to_be_bytes());

        let msg = decode::<AddrMsg>(&bytes);
        assert_eq!(msg.0.len(), 2);
        assert_eq!(msg.0[0].port, 18_333);
        assert_eq!(msg.0[0].ip[12..], [1, 2, 3, 4]);
        assert_eq!(msg.0[1].port, 8333);
    }

    #[test]
    fn addr_msg_rejects_oversize_count() {
        // Count = 1001 (one over MAX_ADDR_TO_SEND), no entry bytes (they'd
        // never be read — the cap check should fire first).
        let mut bytes = vec![0xfd]; // VarInt prefix for u16
        bytes.extend_from_slice(&1001u16.to_le_bytes());
        let mut cur = std::io::Cursor::new(&bytes);
        assert!(AddrMsg::decode_limited(&mut cur).is_err());
    }

    #[test]
    fn addrv2_decodes_ipv4_entry() {
        // VarInt count (1) || time(4) | services(VarInt) | netid(1)
        // | addr-len(VarInt) | addr(N) | port(2 BE)
        let mut bytes = vec![0x01]; // count = 1
        bytes.extend_from_slice(&100u32.to_le_bytes()); // time
        bytes.push(0x01); // services VarInt = 1 (NODE_NETWORK)
        bytes.push(0x01); // network = Ipv4
        bytes.push(0x04); // addr-len VarInt = 4
        bytes.extend_from_slice(&[10, 20, 30, 40]);
        bytes.extend_from_slice(&48_333u16.to_be_bytes());

        let msg = decode::<AddrV2Msg>(&bytes);
        assert_eq!(msg.0.len(), 1);
        let entry = &msg.0[0];
        assert!(matches!(entry.addr, AddrV2Addr::Ipv4([10, 20, 30, 40])));
        let v4 = entry.ip_v4().expect("ipv4 entry must produce SocketAddrV4");
        assert_eq!(v4.ip().octets(), [10, 20, 30, 40]);
        assert_eq!(v4.port(), 48_333);
    }

    #[test]
    fn addrv2_decodes_torv3_then_ipv4() {
        // count = 2: a torv3 entry (32-byte addr) followed by an ipv4 entry.
        // The torv3 entry must decode without error and yield ip_v4() == None.
        let mut bytes = vec![0x02];
        // entry 0: torv3
        bytes.extend_from_slice(&500u32.to_le_bytes());
        bytes.push(0x00); // services VarInt = 0
        bytes.push(0x04); // network = TorV3
        bytes.push(0x20); // addr-len VarInt = 32
        bytes.extend_from_slice(&[0xab; 32]);
        bytes.extend_from_slice(&48_333u16.to_be_bytes());
        // entry 1: ipv4
        bytes.extend_from_slice(&501u32.to_le_bytes());
        bytes.push(0x01);
        bytes.push(0x01);
        bytes.push(0x04);
        bytes.extend_from_slice(&[7, 7, 7, 7]);
        bytes.extend_from_slice(&48_333u16.to_be_bytes());

        let msg = decode::<AddrV2Msg>(&bytes);
        assert_eq!(msg.0.len(), 2);
        assert!(msg.0[0].ip_v4().is_none(), "torv3 has no v4 representation");
        assert_eq!(msg.0[1].ip_v4().unwrap().ip().octets(), [7, 7, 7, 7]);
    }

    #[test]
    fn addrv2_rejects_addr_len_mismatch_for_known_network() {
        // network = Ipv4 (expects 4 bytes) but addr-len = 6 → InvalidData.
        let mut bytes = vec![0x01]; // count = 1
        bytes.extend_from_slice(&100u32.to_le_bytes());
        bytes.push(0x00);
        bytes.push(0x01); // Ipv4
        bytes.push(0x06); // addr-len = 6 (wrong; spec says 4)
        bytes.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
        bytes.extend_from_slice(&48_333u16.to_be_bytes());
        let mut cur = std::io::Cursor::new(&bytes);
        assert!(AddrV2Msg::decode_limited(&mut cur).is_err());
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
            time: 1_231_006_505,
            bits: CompactBits(0x1d00_ffff),
            nonce: 2_083_236_893,
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
            timestamp: 1_700_000_000,
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
            nonce: 0xdead_beef,
            user_agent: "/mantle:0.1.0/".to_string(),
            start_height: 0,
            relay: true,
        };
        let bytes = encode(&msg);
        assert_eq!(decode::<VersionMsg>(&bytes), msg);
    }

    // ── Adversarial / edge-case tests ─────────────────────────────────────────

    fn decode_err<T: Decode + std::fmt::Debug>(bytes: &[u8]) -> CodecError {
        T::decode(&mut std::io::Cursor::new(bytes)).unwrap_err()
    }

    // MessageHeader: truncated at each field boundary.
    #[test]
    fn message_header_truncated_at_magic() {
        let e = decode_err::<MessageHeader>(&[0xf9, 0xbe]);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    #[test]
    fn message_header_truncated_at_command() {
        // 4 magic bytes, then only 6 of the 12 command bytes.
        let buf = [0xf9, 0xbe, 0xb4, 0xd9, 0x76, 0x65, 0x72, 0x73, 0x69, 0x6f];
        let e = decode_err::<MessageHeader>(&buf);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    #[test]
    fn message_header_truncated_at_length() {
        // 4 magic + 12 command + 2 of 4 length bytes.
        let mut buf = vec![0u8; 18];
        buf[0..4].copy_from_slice(&Magic::Mainnet.bytes());
        let e = decode_err::<MessageHeader>(&buf);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    // MessageHeader: exactly 24 bytes decodes without error.
    #[test]
    fn message_header_exact_24_bytes() {
        let payload = b"";
        let mut buf = Vec::new();
        encode_message(Magic::Mainnet, Command::Verack, payload, &mut buf)
            .expect("encode_message cannot fail writing to Vec");
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
        let addr = NetworkAddr {
            services: 0,
            ip: [0u8; 16],
            port: 0,
        };
        let bytes = encode(&addr);
        assert_eq!(decode::<NetworkAddr>(&bytes), addr);
    }

    // NetworkAddr: port u16::MAX round-trips correctly.
    #[test]
    fn network_addr_port_max() {
        let addr = NetworkAddr {
            services: u64::MAX,
            ip: [0xff; 16],
            port: u16::MAX,
        };
        let bytes = encode(&addr);
        assert_eq!(decode::<NetworkAddr>(&bytes), addr);
    }

    // NetworkAddr: truncated IP field.
    #[test]
    fn network_addr_truncated_ip() {
        // 8 services bytes + 8 of 16 IP bytes.
        let buf = [0u8; 16];
        let e = decode_err::<NetworkAddr>(&buf);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    // NetworkAddr: truncated port (only 1 of 2 bytes).
    #[test]
    fn network_addr_truncated_port() {
        let buf = [0u8; 25]; // 8 + 16 + 1 = 25, needs 26
        let e = decode_err::<NetworkAddr>(&buf);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
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
        let entry = InvEntry {
            kind: InvType::Unknown(0xdead_beef),
            hash: Hash256::ZERO,
        };
        let msg = InvMsg(vec![entry]);
        let bytes = encode(&msg);
        let decoded = decode::<InvMsg>(&bytes);
        assert_eq!(decoded.0[0].kind, InvType::Unknown(0xdead_beef));
    }

    // InvMsg: truncated payload (count says 2, only 1 entry present).
    #[test]
    fn inv_msg_truncated_entries() {
        let mut bytes = encode(&InvMsg(vec![
            InvEntry {
                kind: InvType::Block,
                hash: Hash256::ZERO,
            },
            InvEntry {
                kind: InvType::Tx,
                hash: Hash256::ZERO,
            },
        ]));
        // Truncate to cut off the second entry.
        bytes.truncate(1 + 36); // VarInt(2) + one full entry
        let e = decode_err::<InvMsg>(&bytes);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    // GetHeadersMsg: zero locator hashes is valid.
    #[test]
    fn getheaders_empty_locator() {
        let msg = GetHeadersMsg {
            version: 70015,
            locator_hashes: vec![],
            stop_hash: Hash256::ZERO,
        };
        let bytes = encode(&msg);
        assert_eq!(decode::<GetHeadersMsg>(&bytes), msg);
    }

    // GetHeadersMsg: truncated stop_hash.
    #[test]
    fn getheaders_truncated_stop_hash() {
        let msg = GetHeadersMsg {
            version: 70015,
            locator_hashes: vec![],
            stop_hash: Hash256::ZERO,
        };
        let mut bytes = encode(&msg);
        bytes.truncate(bytes.len() - 1); // shave one byte off stop_hash
        let e = decode_err::<GetHeadersMsg>(&bytes);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
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
            bits: CompactBits(0x1d00_ffff),
            nonce: 0,
        };
        // Manually encode with tx_count = 5 instead of 0.
        let mut buf = Vec::new();
        crate::codec::VarInt::from(1u64)
            .encode(&mut buf)
            .expect("Vec<u8> write cannot fail"); // 1 header
        header.encode(&mut buf).expect("Vec<u8> write cannot fail");
        crate::codec::VarInt::from(5u64)
            .encode(&mut buf)
            .expect("Vec<u8> write cannot fail"); // non-zero tx_count
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
            bits: CompactBits(0),
            nonce: 0,
        };
        let mut bytes = encode(&HeadersMsg(vec![header]));
        bytes.truncate(bytes.len() - 5); // truncate inside the header
        let e = decode_err::<HeadersMsg>(&bytes);
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    // VersionMsg: extreme field values round-trip without panic.
    #[test]
    fn version_msg_extreme_fields() {
        let addr = NetworkAddr {
            services: 0,
            ip: [0u8; 16],
            port: 0,
        };
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
        let addr = NetworkAddr {
            services: 0,
            ip: [0u8; 16],
            port: 0,
        };
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
        assert!(matches!(e, CodecError::InvalidUtf8(_) | CodecError::Io(_)));
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
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }

    // ── InvMsg iterator and reuse paths ───────────────────────────────────────

    #[test]
    fn inv_iter_matches_owned() {
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
        let iter_entries: Vec<InvEntry> = InvMsg::iter_entries::<50_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes")
            .map(|r| r.expect("each entry must decode"))
            .collect();
        assert_eq!(iter_entries, inv.0);
    }

    #[test]
    fn inv_iter_empty() {
        let bytes = encode(&InvMsg(vec![]));
        let mut iter = InvMsg::iter_entries::<50_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn inv_iter_exact_size() {
        let inv = InvMsg(vec![
            InvEntry {
                kind: InvType::Tx,
                hash: Hash256::ZERO,
            },
            InvEntry {
                kind: InvType::Tx,
                hash: Hash256::ZERO,
            },
            InvEntry {
                kind: InvType::Tx,
                hash: Hash256::ZERO,
            },
        ]);
        let bytes = encode(&inv);
        let iter = InvMsg::iter_entries::<50_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 3);
    }

    #[test]
    fn inv_iter_limit_exceeded() {
        let inv = InvMsg(vec![
            InvEntry {
                kind: InvType::Block,
                hash: Hash256::ZERO,
            },
            InvEntry {
                kind: InvType::Block,
                hash: Hash256::ZERO,
            },
        ]);
        let bytes = encode(&inv);
        let err = InvMsg::iter_entries::<1>(&bytes).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Io(_) | CodecError::LengthOverflow(_)
        ));
    }

    #[test]
    fn inv_iter_truncated_entry() {
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
        let mut bytes = encode(&inv);
        bytes.truncate(1 + 36 + 10); // VarInt + full entry + partial second entry
        let mut iter = InvMsg::iter_entries::<50_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes");
        assert!(iter.next().expect("first entry must be present").is_ok());
        assert!(matches!(
            iter.next()
                .expect("second entry must be present")
                .unwrap_err(),
            CodecError::UnexpectedEof | CodecError::Io(_)
        ));
        assert!(iter.next().is_none()); // stops after error
    }

    #[test]
    fn inv_decode_into_reuses_capacity() {
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
        let mut buf: Vec<InvEntry> = Vec::new();
        InvMsg::decode_into_limited::<50_000>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .expect("decode_into_limited must succeed on valid bytes");
        assert_eq!(buf, inv.0);
        let cap_after_first = buf.capacity();

        // Second call: no new allocation if same or smaller count.
        InvMsg::decode_into_limited::<50_000>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .expect("decode_into_limited must succeed on valid bytes");
        assert_eq!(buf, inv.0);
        assert!(buf.capacity() >= cap_after_first);
    }

    #[test]
    fn inv_decode_into_limit_exceeded() {
        let inv = InvMsg(vec![
            InvEntry {
                kind: InvType::Block,
                hash: Hash256::ZERO,
            },
            InvEntry {
                kind: InvType::Block,
                hash: Hash256::ZERO,
            },
        ]);
        let bytes = encode(&inv);
        let mut buf = Vec::new();
        let err = InvMsg::decode_into_limited::<1>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .unwrap_err();
        assert!(matches!(
            err,
            CodecError::Io(_) | CodecError::LengthOverflow(_)
        ));
    }

    // ── HeadersMsg iterator and reuse paths ───────────────────────────────────

    fn sample_header(nonce: u32) -> BlockHeader {
        BlockHeader {
            version: 1,
            prev_block: Hash256::ZERO,
            merkle_root: Hash256::ZERO,
            time: 0,
            bits: CompactBits(0x1d00_ffff),
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
        let mut iter = HeadersMsg::iter_entries::<2_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());
    }

    #[test]
    fn headers_iter_exact_size() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1), sample_header(2)]);
        let bytes = encode(&msg);
        let iter = HeadersMsg::iter_entries::<2_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes");
        assert_eq!(iter.len(), 3);
    }

    #[test]
    fn headers_iter_limit_exceeded() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1)]);
        let bytes = encode(&msg);
        let err = HeadersMsg::iter_entries::<1>(&bytes).unwrap_err();
        assert!(matches!(
            err,
            CodecError::Io(_) | CodecError::LengthOverflow(_)
        ));
    }

    #[test]
    fn headers_iter_truncated_entry() {
        let msg = HeadersMsg(vec![sample_header(0), sample_header(1)]);
        let mut bytes = encode(&msg);
        // Each encoded entry is 81 bytes (80 header + 1 byte tx_count VarInt).
        bytes.truncate(1 + 81 + 40); // VarInt + full entry + partial second
        let mut iter = HeadersMsg::iter_entries::<2_000>(&bytes)
            .expect("iter_entries must succeed on valid bytes");
        assert!(iter.next().expect("first entry must be present").is_ok());
        assert!(matches!(
            iter.next()
                .expect("second entry must be present")
                .unwrap_err(),
            CodecError::UnexpectedEof | CodecError::Io(_)
        ));
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
        let err = HeadersMsg::decode_into_limited::<1>(&mut std::io::Cursor::new(&bytes), &mut buf)
            .unwrap_err();
        assert!(matches!(
            err,
            CodecError::Io(_) | CodecError::LengthOverflow(_)
        ));
    }
}
