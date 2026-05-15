use std::io::{self, Read};

use crate::codec::{Decode, Error as CodecError, VarInt};

// ── Addr / AddrV2 ─────────────────────────────────────────────────────────────

/// One legacy `addr` entry: `time(4) || services(8) || ip(16) || port(2 BE)`.
///
/// `#[repr(C)]` guarantees the field layout matches the wire format exactly —
/// no padding, no reordering. `size_of::<AddrEntry>() == 30`.
///
/// `services` is stored as `[u8; 8]` (LE) rather than `u64` to avoid 8-byte
/// alignment that would insert padding after `time`.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AddrEntry {
    time_le: [u8; 4],
    services_le: [u8; 8],
    /// IPv6 address (IPv4 mapped as `::ffff:0:0/96`).
    ip: [u8; 16],
    port_be: [u8; 2],
}

impl AddrEntry {
    pub const ENCODED_LEN: usize = 4 + 8 + 16 + 2;

    #[inline]
    #[must_use]
    pub fn time(&self) -> u32 {
        u32::from_le_bytes(self.time_le)
    }

    #[inline]
    #[must_use]
    pub fn services(&self) -> u64 {
        u64::from_le_bytes(self.services_le)
    }

    #[inline]
    #[must_use]
    pub fn ip(&self) -> [u8; 16] {
        self.ip
    }

    #[inline]
    #[must_use]
    pub fn ip_v4(&self) -> Option<[u8; 4]> {
        if self.ip[..12] == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff] {
            Some([self.ip[12], self.ip[13], self.ip[14], self.ip[15]])
        } else {
            None
        }
    }

    #[inline]
    #[must_use]
    pub fn port(&self) -> u16 {
        u16::from_be_bytes(self.port_be)
    }
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for AddrEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddrEntry")
            .field("time", &self.time())
            .field("services", &self.services())
            .field("ip", &self.ip)
            .field("port", &self.port())
            .finish()
    }
}

impl Decode for AddrEntry {
    fn decode(r: &mut impl Read) -> Result<Self, CodecError> {
        let time_le = <[u8; 4]>::decode(r)?;
        let services_le = <[u8; 8]>::decode(r)?;
        let ip = <[u8; 16]>::decode(r)?;
        let mut port_be = [0u8; 2];
        r.read_exact(&mut port_be).map_err(CodecError::Io)?;
        Ok(Self {
            time_le,
            services_le,
            ip,
            port_be,
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

#[cfg(test)]
mod tests {
    use super::super::envelope::services;
    use super::*;
    use std::io::Cursor;

    fn decode<T: Decode>(bytes: &[u8]) -> T {
        T::decode(&mut Cursor::new(bytes)).expect("test data must decode")
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
        assert_eq!(msg.0[0].port(), 18_333);
        assert_eq!(msg.0[0].ip()[12..], [1, 2, 3, 4]);
        assert_eq!(msg.0[1].port(), 8333);
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
    fn addr_entry_size_eq_encoded_len() {
        assert_eq!(
            std::mem::size_of::<AddrEntry>(),
            AddrEntry::ENCODED_LEN,
        );
    }
}
