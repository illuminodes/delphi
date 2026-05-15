use std::io::{Read, Write};

use crate::codec::{Decode, Encode, Error as CodecError, VarInt};

use super::envelope::NetworkAddr;

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
            user_agent: VarInt::decode_string_limited::<256>(r)?,
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

#[cfg(test)]
mod tests {
    use super::super::envelope::services;
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

    fn decode_err<T: Decode + std::fmt::Debug>(bytes: &[u8]) -> CodecError {
        T::decode(&mut std::io::Cursor::new(bytes)).unwrap_err()
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

    #[test]
    fn ping_roundtrip() {
        let ping = PingMsg {
            nonce: 0xdead_beef_cafe,
        };
        let bytes = encode(&ping);
        assert_eq!(decode::<PingMsg>(&bytes), ping);
    }

    #[test]
    fn ping_zero_nonce() {
        let ping = PingMsg { nonce: 0 };
        let bytes = encode(&ping);
        assert_eq!(decode::<PingMsg>(&bytes), ping);
    }

    #[test]
    fn ping_max_nonce() {
        let ping = PingMsg { nonce: u64::MAX };
        let bytes = encode(&ping);
        assert_eq!(decode::<PingMsg>(&bytes), ping);
    }

    #[test]
    fn ping_truncated() {
        let e = decode_err::<PingMsg>(&[0x00; 4]); // needs 8 bytes
        assert!(matches!(e, CodecError::UnexpectedEof | CodecError::Io(_)));
    }
}
