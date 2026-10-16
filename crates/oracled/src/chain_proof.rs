use mantle::network::Network;
use mantle::primitives::{Block, BlockHeader, BlockHeight};
use mantle_consensus::header_chain::HeaderChain;
use mantle_consensus::validation::{SystemClock, WitnessCommitment, validate_block_structure};
use mantle_fetcher::HeaderAdmission;

/// The header chain a sync pass has verified, grown one fetched page at a
/// time, and the check that a downloaded block belongs to it.
pub struct ChainProof {
    chain: HeaderChain,
}

impl ChainProof {
    /// # Errors
    /// Returns a message for a network without a genesis header.
    pub fn new(network: Network) -> Result<Self, String> {
        HeaderChain::for_network(network)
            .map(|chain| Self { chain })
            .ok_or_else(|| "network has no verifiable genesis header".to_string())
    }

    /// Confirm that `headers` is exactly the verified chain, passes the
    /// checkpoint and carries the network's minimum work.
    ///
    /// # Errors
    /// Returns a message if the chain stops short of the checkpoint or the
    /// minimum work, or the headers differ from what was admitted.
    pub fn finish(&self, headers: &[(BlockHeight, BlockHeader)]) -> Result<(), String> {
        if !self.chain.reached_checkpoint() {
            return Err(format!(
                "header chain ends at {} before checkpoint {:?}",
                self.chain.tip_height(),
                self.chain.checkpoint()
            ));
        }
        if !self.chain.has_minimum_work() {
            return Err(format!(
                "header chain work {} is below the network minimum {}",
                self.chain.tip_work(),
                self.chain.minimum_work()
            ));
        }
        let matches = headers.len() == usize::try_from(self.chain.tip_height().to_u32()).unwrap_or(0)
            && headers
                .iter()
                .all(|(height, header)| self.chain.header_at(*height) == Some(header));
        if matches {
            Ok(())
        } else {
            Err("synchronized headers differ from the verified chain".to_string())
        }
    }

    /// # Errors
    /// Returns a message if the block's header is not the verified one, or
    /// its transactions do not match the merkle root or witness commitment.
    pub fn check_block(&self, height: BlockHeight, block: &Block) -> Result<(), String> {
        let expected = self
            .chain
            .header_at(height)
            .ok_or_else(|| format!("unverified header at {height}"))?;
        if &block.header != expected {
            return Err(format!("block at {height} differs from verified header"));
        }
        validate_block_structure(block)
            .map_err(|error| format!("invalid block structure at {height}: {error}"))?;
        WitnessCommitment::check(block, height)
            .map_err(|error| format!("invalid witness commitment at {height}: {error}"))
    }
}

impl HeaderAdmission for ChainProof {
    fn admit(&mut self, first: BlockHeight, headers: &[BlockHeader]) -> Result<(), String> {
        self.chain
            .replace_from(first, headers, &SystemClock)
            .map_err(|(index, error)| match first.to_u32().checked_add(u32::try_from(index).unwrap_or(u32::MAX)) {
                Some(height) => format!("invalid header {height}: {error}"),
                None => format!("invalid header past {first}: {error}"),
            })
    }
}

#[cfg(test)]
mod tests {
    use mantle::network::Network;
    use mantle::primitives::{BlockHeader, BlockHeight};
    use mantle_fetcher::HeaderAdmission;

    use super::ChainProof;

    const BLOCK_1: &str = "010000006fe28c0ab6f1b372c1a6a246ae63f74f931e8365e15a089c68d6190000000000982051fd1e4ba744bbbe680e1fee14677ba1a3c3540bf7b1cdb606e857233e0e61bc6649ffff001d01e36299";

    struct ProofFixture;

    impl ProofFixture {
        fn block_1() -> BlockHeader {
            let bytes: Vec<u8> = (0..BLOCK_1.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&BLOCK_1[at..at + 2], 16).expect("hex"))
                .collect();
            BlockHeader::from_bytes(bytes.try_into().expect("80 bytes"))
        }

        fn forged() -> BlockHeader {
            let genesis = Network::Mainnet.genesis_header().expect("genesis");
            let mut forged = BlockHeader::from_bytes(genesis.to_bytes());
            forged.set_prev_block(genesis.hash());
            forged
        }
    }

    #[test]
    fn refuses_a_header_gap_before_pricing() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        assert!(proof.admit(BlockHeight::new(2), &[ProofFixture::block_1()]).is_err());
    }

    #[test]
    fn refuses_a_chain_below_its_checkpoint() {
        let proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        assert!(proof.finish(&[]).is_err());
    }

    #[test]
    fn refuses_an_unmined_header() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        assert!(proof.admit(BlockHeight::new(1), &[ProofFixture::forged()]).is_err());
    }

    #[test]
    fn a_refused_page_leaves_the_chain_where_it_was() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        let page = [ProofFixture::block_1(), ProofFixture::forged()];

        assert!(proof.admit(BlockHeight::new(1), &page).is_err());
        assert!(proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).is_ok());
        assert!(proof.admit(BlockHeight::new(3), &[ProofFixture::block_1()]).is_err());
    }

    #[test]
    fn a_page_from_below_the_tip_without_more_work_is_refused() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).expect("block 1");

        let same_work = proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]);

        assert!(same_work.is_err_and(|reason| reason.contains("InsufficientChainWork")));
        assert!(proof.admit(BlockHeight::new(1), &[ProofFixture::forged()]).is_err());
        assert!(proof.admit(BlockHeight::new(2), &[ProofFixture::block_1()]).is_err());
    }

    #[test]
    fn admitted_headers_below_the_checkpoint_do_not_finish() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).expect("block 1");

        assert!(proof.finish(&[(BlockHeight::new(1), ProofFixture::block_1())]).is_err());
    }
}
