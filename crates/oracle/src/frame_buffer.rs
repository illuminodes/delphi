use std::fmt;

use mantle::hash::Hash256;
use mantle::network::{Command, Magic, MessageHeader};

/// Bytes received from one peer, cut into whole P2P messages.
///
/// A frame whose header carries another network's magic or declares a
/// payload above [`Self::MAX_PAYLOAD`] makes the stream unusable, since no
/// later frame boundary can be trusted. Frames with a bad checksum or an
/// unknown command are skipped.
pub struct FrameBuffer {
    magic: Magic,
    bytes: Vec<u8>,
    start: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    WrongMagic,
    Oversized(u32),
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongMagic => formatter.write_str("frame carries another network's magic"),
            Self::Oversized(length) => write!(formatter, "frame declares a {length}-byte payload"),
        }
    }
}

impl std::error::Error for FrameError {}

impl FrameBuffer {
    /// Bitcoin Core's `MAX_PROTOCOL_MESSAGE_LENGTH`.
    pub const MAX_PAYLOAD: u32 = 4_000_000;

    #[must_use]
    pub fn new(magic: Magic, capacity: usize) -> Self {
        Self {
            magic,
            bytes: Vec::with_capacity(capacity),
            start: 0,
        }
    }

    pub fn extend(&mut self, received: &[u8]) {
        self.bytes.drain(..self.start);
        self.start = 0;
        self.bytes.extend_from_slice(received);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len() - self.start
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The next whole, valid frame, or `None` until more bytes arrive.
    ///
    /// # Errors
    /// [`FrameError`] when the stream can no longer be framed.
    pub fn next_frame(&mut self) -> Result<Option<(Command, Vec<u8>)>, FrameError> {
        loop {
            let pending = &self.bytes[self.start..];
            let Some(raw) = pending.first_chunk::<{ MessageHeader::ENCODED_LEN }>() else {
                return Ok(None);
            };
            let header = MessageHeader::from_bytes(raw);
            if header.magic() != self.magic {
                return Err(FrameError::WrongMagic);
            }
            if header.length() > Self::MAX_PAYLOAD {
                return Err(FrameError::Oversized(header.length()));
            }
            let total = MessageHeader::ENCODED_LEN + header.length() as usize;
            if pending.len() < total {
                return Ok(None);
            }
            let payload = pending[MessageHeader::ENCODED_LEN..total].to_vec();
            self.start += total;
            let command = Command::from_command_name(header.command());
            if let Some(command) = command
                && Hash256::checksum(&payload) == header.checksum()
            {
                return Ok(Some((command, payload)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mantle::network::{Command, MessageHeader, Network, encode_message};

    use super::{FrameBuffer, FrameError};

    struct FrameFixture;

    impl FrameFixture {
        fn buffer() -> FrameBuffer {
            FrameBuffer::new(Network::Mainnet.magic(), 64)
        }

        fn frame(network: Network, command: Command, payload: &[u8]) -> Vec<u8> {
            let mut bytes = Vec::new();
            encode_message(network.magic(), command, payload, &mut bytes).expect("encode");
            bytes
        }

        fn corrupted(command: Command, payload: &[u8]) -> Vec<u8> {
            let mut bytes = Self::frame(Network::Mainnet, command, payload);
            bytes[20] ^= 0xff;
            bytes
        }
    }

    #[test]
    fn a_frame_split_across_reads_comes_out_whole() {
        let mut buffer = FrameFixture::buffer();
        let frame = FrameFixture::frame(Network::Mainnet, Command::Ping, &[1; 8]);
        buffer.extend(&frame[..10]);
        assert_eq!(buffer.next_frame(), Ok(None));
        buffer.extend(&frame[10..]);
        assert_eq!(buffer.next_frame(), Ok(Some((Command::Ping, vec![1; 8]))));
        assert!(buffer.is_empty());
    }

    #[test]
    fn many_bad_checksums_are_skipped_without_recursing() {
        let mut buffer = FrameFixture::buffer();
        let bad = FrameFixture::corrupted(Command::Ping, &[2; 8]);
        for _ in 0..200_000 {
            buffer.extend(&bad);
        }
        buffer.extend(&FrameFixture::frame(Network::Mainnet, Command::Verack, &[]));
        assert_eq!(buffer.next_frame(), Ok(Some((Command::Verack, Vec::new()))));
    }

    #[test]
    fn an_oversized_declaration_ends_the_stream() {
        let mut buffer = FrameFixture::buffer();
        let mut frame = FrameFixture::frame(Network::Mainnet, Command::Tx, &[]);
        frame[16..20].copy_from_slice(&(FrameBuffer::MAX_PAYLOAD + 1).to_le_bytes());
        buffer.extend(&frame[..MessageHeader::ENCODED_LEN]);
        assert_eq!(
            buffer.next_frame(),
            Err(FrameError::Oversized(FrameBuffer::MAX_PAYLOAD + 1))
        );
    }

    #[test]
    fn another_networks_magic_ends_the_stream() {
        let mut buffer = FrameFixture::buffer();
        buffer.extend(&FrameFixture::frame(Network::Testnet4, Command::Ping, &[0; 8]));
        assert_eq!(buffer.next_frame(), Err(FrameError::WrongMagic));
    }
}
