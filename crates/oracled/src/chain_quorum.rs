use std::collections::HashSet;
use std::net::SocketAddrV4;

use mantle::primitives::{BlockHeader, BlockHeight, Hash256};
use mantle_fetcher::SurveyedPage;

/// Decides how far up our verified chain the peers agree with it.
///
/// Each surveyed page is one or more peers' view of the chain above a
/// shared header. A peer backs our header at a height when its page holds
/// the same hash there, and disputes it when its page holds another. Peers
/// are counted once per /16 group, so many addresses in one network count
/// as one witness. A height is settled when it is at least `depth` blocks
/// below our tip, at least `groups` groups back it, and more groups back it
/// than dispute it. The settled height is the highest such height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainQuorum {
    depth: u32,
    groups: usize,
}

impl ChainQuorum {
    pub const DEPTH: u32 = 6;
    pub const GROUPS: usize = 5;

    #[must_use]
    pub const fn new(depth: u32, groups: usize) -> Self {
        Self { depth, groups }
    }

    /// How far below our tip a survey should start. A peer on our chain
    /// answers with the headers above the newest hash it shares with us, so
    /// the survey shows it only the chain up to here, and its reply covers
    /// the heights a settlement is decided at.
    #[must_use]
    pub const fn lookback(&self) -> u32 {
        self.depth * 2
    }

    /// The highest height of `ours` (sorted by height) that `pages` settle.
    #[must_use]
    pub fn settled(&self, ours: &[(BlockHeight, BlockHeader)], pages: &[SurveyedPage]) -> Option<BlockHeight> {
        let tip = ours.last()?.0.to_u32();
        let deepest = tip.checked_sub(self.depth)?;
        ours.iter()
            .rev()
            .filter(|(height, _)| height.to_u32() <= deepest)
            .find(|(height, header)| self.is_settled(*height, header.hash(), pages))
            .map(|(height, _)| *height)
    }

    fn is_settled(&self, height: BlockHeight, ours: Hash256, pages: &[SurveyedPage]) -> bool {
        let mut backing = HashSet::new();
        let mut disputing = HashSet::new();
        for page in pages {
            let Some(theirs) = Self::hash_at(page, height) else {
                continue;
            };
            let side = if theirs == ours { &mut backing } else { &mut disputing };
            side.extend(page.senders.iter().copied().map(Self::group));
        }
        backing.len() >= self.groups && backing.len() > disputing.len()
    }

    fn hash_at(page: &SurveyedPage, height: BlockHeight) -> Option<Hash256> {
        let offset = usize::try_from(height.to_u32().checked_sub(page.first.to_u32())?).ok()?;
        page.headers.get(offset).map(BlockHeader::hash)
    }

    fn group(address: SocketAddrV4) -> [u8; 2] {
        let [a, b, _, _] = address.ip().octets();
        [a, b]
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4};

    use mantle::primitives::{BlockHeader, BlockHeight, BlockTime, BlockVersion, CompactBits, Hash256, Nonce};
    use mantle_fetcher::SurveyedPage;

    use super::ChainQuorum;

    struct QuorumFixture;

    impl QuorumFixture {
        fn branch(parent: Hash256, first: u32, length: u32, salt: u32) -> Vec<(BlockHeight, BlockHeader)> {
            let mut prev = parent;
            (first..first + length)
                .map(|height| {
                    let header = BlockHeader::new(
                        BlockVersion::new(4),
                        prev,
                        Hash256::ZERO,
                        BlockTime::new(height),
                        CompactBits::new(0x207f_ffff),
                        Nonce::new(salt),
                    );
                    prev = header.hash();
                    (BlockHeight::new(height), header)
                })
                .collect()
        }

        fn ours() -> Vec<(BlockHeight, BlockHeader)> {
            Self::branch(Hash256::ZERO, 100, 12, 0)
        }

        fn peer(group: u8, host: u8) -> SocketAddrV4 {
            SocketAddrV4::new(Ipv4Addr::new(group, group, 0, host), 8333)
        }

        fn page(chain: &[(BlockHeight, BlockHeader)], senders: Vec<SocketAddrV4>) -> SurveyedPage {
            SurveyedPage {
                first: chain[0].0,
                headers: chain.iter().map(|(_, header)| *header).collect(),
                senders,
            }
        }

        fn groups(count: u8) -> Vec<SocketAddrV4> {
            (1..=count).map(|group| Self::peer(group, 1)).collect()
        }
    }

    #[test]
    fn enough_agreeing_groups_settle_the_block_six_deep() {
        let ours = QuorumFixture::ours();
        let pages = [QuorumFixture::page(&ours, QuorumFixture::groups(5))];

        assert_eq!(ChainQuorum::new(6, 5).settled(&ours, &pages), Some(BlockHeight::new(105)));
    }

    #[test]
    fn too_few_groups_settle_nothing() {
        let ours = QuorumFixture::ours();
        let pages = [QuorumFixture::page(&ours, QuorumFixture::groups(4))];

        assert_eq!(ChainQuorum::new(6, 5).settled(&ours, &pages), None);
    }

    #[test]
    fn many_addresses_in_one_group_count_once() {
        let ours = QuorumFixture::ours();
        let crowd = (1..=20).map(|host| QuorumFixture::peer(7, host)).collect();
        let pages = [QuorumFixture::page(&ours, crowd)];

        assert_eq!(ChainQuorum::new(6, 2).settled(&ours, &pages), None);
    }

    #[test]
    fn a_disputing_majority_keeps_a_height_unsettled_and_the_agreed_one_below_it_settles() {
        let ours = QuorumFixture::ours();
        let rival = QuorumFixture::branch(ours[2].1.hash(), 103, 9, 7);
        let pages = [
            QuorumFixture::page(&ours, QuorumFixture::groups(3)),
            QuorumFixture::page(&rival, (10..=13).map(|group| QuorumFixture::peer(group, 1)).collect()),
            QuorumFixture::page(&ours[..3], (20..=21).map(|group| QuorumFixture::peer(group, 1)).collect()),
        ];

        assert_eq!(ChainQuorum::new(6, 3).settled(&ours, &pages), Some(BlockHeight::new(102)));
    }

    #[test]
    fn peers_behind_our_tip_still_back_the_heights_they_hold() {
        let ours = QuorumFixture::ours();
        let pages = [QuorumFixture::page(&ours[..4], QuorumFixture::groups(5))];

        assert_eq!(ChainQuorum::new(6, 5).settled(&ours, &pages), Some(BlockHeight::new(103)));
    }

    #[test]
    fn a_chain_shorter_than_the_depth_or_no_pages_settle_nothing() {
        let ours = QuorumFixture::ours();

        assert_eq!(ChainQuorum::new(6, 1).settled(&ours[..5], &[QuorumFixture::page(&ours, QuorumFixture::groups(1))]), None);
        assert_eq!(ChainQuorum::new(6, 1).settled(&ours, &[]), None);
    }
}
