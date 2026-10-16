use std::io;

use mantle::codec::{Decode, Encode};
use mantle::hash::Hash256;
use mantle::network::{Command, InvEntry, InvMsg, InvType, Magic, Services};
use mantle::tx::Transaction;

use crate::frame_buffer::{FrameBuffer, FrameError};

/// What one peer's bytes produced: messages to send back, transactions it
/// announced, and transaction bodies it delivered.
#[derive(Default)]
pub struct SessionOutput {
    pub replies: Vec<(Command, Vec<u8>)>,
    pub announced: Vec<Hash256>,
    pub transactions: Vec<Transaction>,
}

/// The mempool side of one peer's protocol, without a socket: framing, the
/// version handshake, pings, `inv` announcements and `tx` bodies.
pub struct MempoolSession {
    frames: FrameBuffer,
    got_version: bool,
    ready: bool,
    delivered: u64,
    not_found: u64,
    decode_failures: u64,
}

impl MempoolSession {
    /// Largest `getdata` batch that fits one 4 KiB transport send slot.
    pub const MAX_GETDATA_BATCH: usize = 100;
    const USER_AGENT: &'static str = "/delphi-oracle:0.1/";

    #[must_use]
    pub fn new(magic: Magic) -> Self {
        Self {
            frames: FrameBuffer::new(magic, 64 * 1024),
            got_version: false,
            ready: false,
            delivered: 0,
            not_found: 0,
            decode_failures: 0,
        }
    }

    /// The `version` payload asking for transaction relay, with
    /// `NODE_WITNESS` so the peer serves witness bodies.
    ///
    /// # Errors
    /// Returns an I/O error if the message cannot be encoded.
    pub fn version_payload() -> io::Result<Vec<u8>> {
        let mut message = mantle_transport::our_version(Self::USER_AGENT);
        message.relay = true;
        message.services = message.services | Services::NODE_WITNESS;
        let mut encoded = Vec::new();
        message
            .encode(&mut encoded)
            .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(encoded)
    }

    /// `getdata` payloads for `hashes`, each within one send slot.
    ///
    /// # Errors
    /// Returns an I/O error if a payload cannot be encoded.
    pub fn getdata_payloads(hashes: &[Hash256]) -> io::Result<Vec<Vec<u8>>> {
        hashes
            .chunks(Self::MAX_GETDATA_BATCH)
            .map(|chunk| {
                let entries = chunk
                    .iter()
                    .map(|hash| InvEntry::new(InvType::WitnessTx, *hash))
                    .collect();
                let mut payload = Vec::new();
                InvMsg(entries)
                    .encode(&mut payload)
                    .map_err(|error| io::Error::other(error.to_string()))?;
                Ok(payload)
            })
            .collect()
    }

    /// Feed received bytes and act on every whole frame.
    ///
    /// # Errors
    /// [`FrameError`] when the peer's stream can no longer be framed.
    pub fn receive(&mut self, bytes: &[u8]) -> Result<SessionOutput, FrameError> {
        self.frames.extend(bytes);
        let mut output = SessionOutput::default();
        while let Some((command, payload)) = self.frames.next_frame()? {
            self.dispatch(command, payload, &mut output);
        }
        Ok(output)
    }

    fn dispatch(&mut self, command: Command, payload: Vec<u8>, output: &mut SessionOutput) {
        match command {
            Command::Version if !self.got_version => {
                self.got_version = true;
                output.replies.push((Command::Verack, Vec::new()));
            }
            Command::Verack if self.got_version => self.ready = true,
            Command::Ping => output.replies.push((Command::Pong, payload)),
            Command::Inv if self.ready => Self::collect_inv(&payload, &mut output.announced),
            Command::Tx if self.ready => match Transaction::decode(&mut io::Cursor::new(payload.as_slice())) {
                Ok(transaction) => {
                    self.delivered += 1;
                    output.transactions.push(transaction);
                }
                Err(_) => self.decode_failures += 1,
            },
            Command::NotFound => {
                if let Ok(inv) = InvMsg::decode(&mut io::Cursor::new(payload.as_slice())) {
                    self.not_found += inv.0.len() as u64;
                }
            }
            _ => {}
        }
    }

    fn collect_inv(payload: &[u8], into: &mut Vec<Hash256>) {
        let Ok(inv) = InvMsg::decode(&mut io::Cursor::new(payload)) else {
            return;
        };
        into.extend(
            inv.0
                .iter()
                .filter(|entry| matches!(entry.kind(), InvType::Tx | InvType::WitnessTx))
                .map(InvEntry::hash),
        );
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.ready
    }

    #[must_use]
    pub const fn delivered(&self) -> u64 {
        self.delivered
    }

    #[must_use]
    pub const fn not_found(&self) -> u64 {
        self.not_found
    }

    #[must_use]
    pub const fn decode_failures(&self) -> u64 {
        self.decode_failures
    }
}

#[cfg(test)]
mod tests {
    use mantle::codec::Encode;
    use mantle::hash::Hash256;
    use mantle::network::{Command, InvEntry, InvMsg, InvType, Network, encode_message};

    use super::MempoolSession;

    struct SessionFixture;

    impl SessionFixture {
        fn frame(command: Command, payload: &[u8]) -> Vec<u8> {
            let mut bytes = Vec::new();
            encode_message(Network::Mainnet.magic(), command, payload, &mut bytes).expect("encode");
            bytes
        }

        fn ready() -> MempoolSession {
            let mut session = MempoolSession::new(Network::Mainnet.magic());
            let version = MempoolSession::version_payload().expect("version");
            let mut bytes = Self::frame(Command::Version, &version);
            bytes.extend(Self::frame(Command::Verack, &[]));
            let output = session.receive(&bytes).expect("frames");
            assert_eq!(output.replies.len(), 1);
            assert!(session.is_ready());
            session
        }

        fn inv(hashes: &[Hash256]) -> Vec<u8> {
            let mut payload = Vec::new();
            InvMsg(hashes.iter().map(|hash| InvEntry::new(InvType::Tx, *hash)).collect())
                .encode(&mut payload)
                .expect("inv");
            payload
        }
    }

    #[test]
    fn a_version_is_answered_with_verack_and_verack_makes_the_session_ready() {
        let mut session = MempoolSession::new(Network::Mainnet.magic());
        let version = MempoolSession::version_payload().expect("version");

        let output = session.receive(&SessionFixture::frame(Command::Version, &version)).expect("frame");

        assert_eq!(output.replies[0].0, Command::Verack);
        assert!(!session.is_ready());
        session.receive(&SessionFixture::frame(Command::Verack, &[])).expect("frame");
        assert!(session.is_ready());
    }

    #[test]
    fn a_ping_is_answered_with_its_nonce() {
        let mut session = SessionFixture::ready();
        let output = session.receive(&SessionFixture::frame(Command::Ping, &[7; 8])).expect("frame");

        assert_eq!(output.replies, vec![(Command::Pong, vec![7; 8])]);
    }

    #[test]
    fn announcements_are_ignored_before_the_handshake_and_read_after() {
        let hashes = [Hash256::from_bytes([1; 32]), Hash256::from_bytes([2; 32])];
        let inv = SessionFixture::frame(Command::Inv, &SessionFixture::inv(&hashes));
        let mut early = MempoolSession::new(Network::Mainnet.magic());

        assert!(early.receive(&inv).expect("frame").announced.is_empty());
        assert_eq!(SessionFixture::ready().receive(&inv).expect("frame").announced, hashes);
    }

    #[test]
    fn getdata_is_split_to_fit_a_send_slot() {
        let hashes: Vec<Hash256> = (0..250u32)
            .map(|index| Hash256::hash(&index.to_le_bytes()))
            .collect();
        let payloads = MempoolSession::getdata_payloads(&hashes).expect("payloads");

        assert_eq!(payloads.len(), 3);
        assert!(payloads.iter().all(|payload| payload.len() + 24 <= 4 * 1024));
    }

    #[test]
    fn an_unreadable_tx_is_counted_not_delivered() {
        let mut session = SessionFixture::ready();
        let output = session.receive(&SessionFixture::frame(Command::Tx, &[1, 2, 3])).expect("frame");

        assert!(output.transactions.is_empty());
        assert_eq!(session.decode_failures(), 1);
    }
}
