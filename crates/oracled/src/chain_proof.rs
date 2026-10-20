use mantle::network::Network;
use mantle::primitives::{Block, BlockHeader, BlockHeight, Hash256};
use mantle_consensus::header_chain::HeaderChain;
use mantle_consensus::validation::{SystemClock, WitnessCommitment, validate_block_structure};
use mantle_fetcher::HeaderAdmission;

/// The header chain a sync pass has verified, grown one fetched page at a
/// time, and the check that a downloaded block belongs to it.
///
/// The chain may be a tail resumed from disk: headers are then requested
/// from just above its base, whose hash is the locator.
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

    #[must_use]
    pub const fn resumed(chain: HeaderChain) -> Self {
        Self { chain }
    }

    #[must_use]
    pub const fn chain(&self) -> &HeaderChain {
        &self.chain
    }

    /// The first height to request headers from.
    #[must_use]
    pub fn start(&self) -> BlockHeight {
        BlockHeight::new(self.chain.base().to_u32() + 1)
    }

    /// The hash of the header just below [`Self::start`].
    #[must_use]
    pub fn locator(&self) -> Hash256 {
        self.chain.header_at(self.chain.base()).map_or(Hash256::ZERO, BlockHeader::hash)
    }

    /// Drop the headers no longer needed to verify the next ones or to
    /// fetch the newest `depth` blocks.
    pub fn keep_newest(&mut self, depth: u32) {
        let tip = self.chain.tip_height().to_u32();
        self.chain.forget_below(BlockHeight::new(tip.saturating_sub(depth)));
    }

    /// Confirm that `headers` is exactly the verified chain, passes the
    /// checkpoint and carries the network's minimum work. A proof kept from
    /// an earlier pass that runs past `headers` is cut back to their tip.
    ///
    /// # Errors
    /// Returns a message if the chain stops short of the checkpoint or the
    /// minimum work, or the headers differ from what was admitted.
    pub fn finish(&mut self, headers: &[(BlockHeight, BlockHeader)]) -> Result<(), String> {
        if let Some((tip, _)) = headers.last()
            && *tip < self.chain.tip_height()
        {
            self.chain.truncate(*tip);
        }
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
        let held = self.chain.tip_height().to_u32() - self.chain.base().to_u32();
        let matches = headers.len() == usize::try_from(held).unwrap_or(0)
            && headers.first().is_none_or(|(height, _)| *height == self.start())
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

impl ChainProof {
    /// How many of `headers`, placed from `first`, the chain already holds.
    fn held_prefix(&self, first: BlockHeight, headers: &[BlockHeader]) -> usize {
        headers
            .iter()
            .zip(first.to_u32()..)
            .take_while(|(header, height)| self.chain.header_at(BlockHeight::new(*height)) == Some(*header))
            .count()
    }
}

/// Headers the chain already holds at their heights are taken as they are,
/// so a proof kept across passes accepts the cached file again without
/// re-checking it; the rest is admitted from the first difference.
impl HeaderAdmission for ChainProof {
    fn admit(&mut self, first: BlockHeight, headers: &[BlockHeader]) -> Result<(), String> {
        let held = self.held_prefix(first, headers);
        if held == headers.len() {
            return Ok(());
        }
        let first = BlockHeight::new(first.to_u32() + u32::try_from(held).unwrap_or(u32::MAX));
        let headers = &headers[held..];
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
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
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

        assert!(proof.admit(BlockHeight::new(1), &[ProofFixture::forged()]).is_err());
        assert!(proof.admit(BlockHeight::new(2), &[ProofFixture::block_1()]).is_err());
    }

    #[test]
    fn admitted_headers_below_the_checkpoint_do_not_finish() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).expect("block 1");

        assert!(proof.finish(&[(BlockHeight::new(1), ProofFixture::block_1())]).is_err());
    }

    #[test]
    fn a_resumed_tail_asks_from_above_its_base_and_finishes_on_its_own_headers() {
        let mut whole = ChainProof::new(Network::Mainnet).expect("mainnet");
        whole.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).expect("block 1");
        let tail = mantle_consensus::header_chain::HeaderChain::tail_for_network(
            Network::Mainnet,
            BlockHeight::new(1),
            whole.chain().tip_work(),
            &[ProofFixture::block_1()],
        )
        .expect("tail");
        let mut resumed = ChainProof::resumed(tail);

        assert_eq!(resumed.start(), BlockHeight::new(2));
        assert_eq!(resumed.locator(), ProofFixture::block_1().hash());
        assert!(resumed.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).is_ok());
        let error = resumed.finish(&[]).expect_err("below the checkpoint");
        assert!(error.contains("checkpoint"), "{error}");
    }

    #[test]
    fn headers_already_held_are_taken_again_without_a_recheck() {
        let mut proof = ChainProof::new(Network::Mainnet).expect("mainnet");
        proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).expect("block 1");

        assert!(proof.admit(BlockHeight::new(1), &[ProofFixture::block_1()]).is_ok());
        assert_eq!(proof.held_prefix(BlockHeight::new(1), &[ProofFixture::block_1()]), 1);
        assert_eq!(proof.held_prefix(BlockHeight::new(1), &[ProofFixture::forged()]), 0);
    }
}
