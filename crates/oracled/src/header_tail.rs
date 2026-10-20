use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

use mantle::network::Network;
use mantle::primitives::{BlockHeader, BlockHeight, ChainWork};
use mantle_consensus::header_chain::HeaderChain;

/// The verified headers the daemon keeps between runs: the newest few
/// thousand, from a base height whose total work from genesis is stored with
/// them, so a restart resumes from the tail instead of from genesis.
///
/// File layout: magic, base height (u32 LE), work through the base (u128
/// LE), then 80-byte headers from the base up. It is replaced whole through
/// a temporary file, so a crash leaves either the old tail or the new one.
#[derive(Debug, Clone)]
pub struct HeaderTail {
    path: PathBuf,
}

impl HeaderTail {
    pub const FILE: &'static str = "header_tail.bin";
    const MAGIC: [u8; 8] = *b"DTAIL\x01\0\0";
    const PREFIX: usize = 8 + 4 + 16;

    #[must_use]
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: data_dir.into().join(Self::FILE),
        }
    }

    /// The saved chain under `network`'s rules, or `None` when there is no
    /// file or it does not read as a linked tail.
    ///
    /// # Errors
    /// Returns an I/O error if the file exists but cannot be read.
    pub fn load(&self, network: Network) -> io::Result<Option<HeaderChain>> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        Ok(Self::decode(&bytes, network))
    }

    /// # Errors
    /// Returns an I/O error if the tail cannot be written.
    pub fn save(&self, chain: &HeaderChain) -> io::Result<()> {
        let temporary = self.path.with_extension("tmp");
        let mut file = File::create(&temporary)?;
        file.write_all(&Self::encode(chain))?;
        file.sync_data()?;
        fs::rename(&temporary, &self.path)
    }

    fn encode(chain: &HeaderChain) -> Vec<u8> {
        let base = chain.base();
        let headers = chain.headers_from(base);
        let mut bytes = Vec::with_capacity(Self::PREFIX + headers.len() * 80);
        bytes.extend_from_slice(&Self::MAGIC);
        bytes.extend_from_slice(&base.to_u32().to_le_bytes());
        bytes.extend_from_slice(&chain.work_at(base).unwrap_or(ChainWork::ZERO).to_u128().to_le_bytes());
        for header in headers {
            bytes.extend_from_slice(&header.to_bytes());
        }
        bytes
    }

    fn decode(bytes: &[u8], network: Network) -> Option<HeaderChain> {
        let body = bytes.strip_prefix(&Self::MAGIC)?;
        let (base, body) = body.split_first_chunk::<4>()?;
        let (work, body) = body.split_first_chunk::<16>()?;
        if body.is_empty() || !body.len().is_multiple_of(80) {
            return None;
        }
        let headers: Vec<BlockHeader> = body
            .chunks_exact(80)
            .map(|chunk| BlockHeader::from_slice(chunk.try_into().expect("80-byte chunk")))
            .collect();
        HeaderChain::tail_for_network(
            network,
            BlockHeight::new(u32::from_le_bytes(*base)),
            ChainWork::new(u128::from_le_bytes(*work)),
            &headers,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use mantle::network::Network;
    use mantle::primitives::{BlockHeight, ChainWork};
    use mantle_consensus::header_chain::HeaderChain;

    use super::HeaderTail;

    struct TailFixture {
        dir: PathBuf,
    }

    impl TailFixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("oracled-tail-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self { dir }
        }

        fn chain() -> HeaderChain {
            let genesis = Network::Mainnet.genesis_header().expect("genesis");
            HeaderChain::tail_for_network(Network::Mainnet, BlockHeight::new(500), ChainWork::new(77), &[genesis])
                .expect("tail")
        }
    }

    impl Drop for TailFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn a_saved_tail_loads_with_its_base_work_and_headers() {
        let fixture = TailFixture::new("round-trip");
        let tail = HeaderTail::new(&fixture.dir);
        let chain = TailFixture::chain();
        tail.save(&chain).expect("save");

        let loaded = tail.load(Network::Mainnet).expect("read").expect("tail");

        assert_eq!(loaded.base(), BlockHeight::new(500));
        assert_eq!(loaded.tip_work(), ChainWork::new(77));
        assert_eq!(loaded.header_at(BlockHeight::new(500)), chain.header_at(BlockHeight::new(500)));
    }

    #[test]
    fn a_missing_or_damaged_file_gives_no_tail() {
        let fixture = TailFixture::new("damaged");
        let tail = HeaderTail::new(&fixture.dir);
        assert!(tail.load(Network::Mainnet).expect("read").is_none());

        tail.save(&TailFixture::chain()).expect("save");
        let mut bytes = fs::read(fixture.dir.join(HeaderTail::FILE)).expect("read");
        bytes.truncate(bytes.len() - 1);
        fs::write(fixture.dir.join(HeaderTail::FILE), bytes).expect("write");

        assert!(tail.load(Network::Mainnet).expect("read").is_none());
    }
}
