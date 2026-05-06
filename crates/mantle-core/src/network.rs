use std::io::{self, Read, Write};

use crate::codec::{Decode, DecodeRef, Encode, VarInt};
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
        checksum: checksum(payload),
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
        let port = u16::from_be_bytes(port_bytes.try_into().unwrap());
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
            user_agent: String::decode(r)?,
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

impl Encode for InvEntry {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        self.kind.to_u32().encode(w)?;
        self.hash.encode(w)
    }
}

impl Decode for InvEntry {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        Ok(Self {
            kind: InvType::from_u32(u32::decode(r)?),
            hash: Hash256::decode(r)?,
        })
    }
}

impl<'a> DecodeRef<'a> for InvEntry {
    fn decode_ref(buf: &'a [u8]) -> io::Result<(Self, &'a [u8])> {
        let (kind_u32, buf) = u32::decode_ref(buf)?;
        let (hash, buf) = Hash256::decode_ref(buf)?;
        Ok((
            Self {
                kind: InvType::from_u32(kind_u32),
                hash,
            },
            buf,
        ))
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

impl Decode for InvMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(InvEntry::decode(r)?);
        }
        Ok(Self(entries))
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

impl Decode for GetHeadersMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let version = u32::decode(r)?;
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
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

/// `headers` — response with up to 2000 block headers.
/// Each header is followed by a `tx_count` `VarInt` (always 0x00 in this message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersMsg(pub Vec<BlockHeader>);

impl Encode for HeadersMsg {
    fn encode(&self, w: &mut impl Write) -> io::Result<()> {
        VarInt(self.0.len() as u64).encode(w)?;
        for header in &self.0 {
            header.encode(w)?;
            VarInt(0).encode(w)?; // tx_count placeholder
        }
        Ok(())
    }
}

impl Decode for HeadersMsg {
    fn decode(r: &mut impl Read) -> io::Result<Self> {
        let count = usize::try_from(VarInt::decode(r)?.0)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut headers = Vec::with_capacity(count);
        for _ in 0..count {
            headers.push(BlockHeader::decode(r)?);
            let _tx_count = VarInt::decode(r)?;
        }
        Ok(Self(headers))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn encode<T: Encode>(val: &T) -> Vec<u8> {
        let mut buf = Vec::new();
        val.encode(&mut buf).unwrap();
        buf
    }

    fn decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).unwrap()
    }

    fn decode_ref<'a, T: DecodeRef<'a>>(buf: &'a [u8]) -> (T, &'a [u8]) {
        T::decode_ref(buf).unwrap()
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
            magic: Magic::Mainnet.bytes(),
            command: CommandName::new("version"),
            length: payload.len() as u32,
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
            magic: Magic::Mainnet.bytes(),
            command: CommandName::new("ping"),
            length: payload.len() as u32,
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
        encode_message(Magic::Mainnet, Command::Ping, payload, &mut buf).unwrap();
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
}
