use mantle::difficulty::BlockHeight;
use mantle::primitives::BlockHeader;
use mantle_oracle::{CachedWindows, OraclePipeline, RangeSelection};

/// The oracle's own confirmed-chain price from the blocks immediately before
/// a moment, using only what is cached on disk.
pub struct ChainAnchor;

impl ChainAnchor {
    /// The index range of the `blocks` newest headers whose time is before
    /// `before`, assuming `times` is in height order.
    #[must_use]
    pub fn span(times: &[u32], before: u32, blocks: usize) -> Option<(usize, usize)> {
        let end = times.partition_point(|time| *time < before);
        let start = end.checked_sub(blocks)?;
        (blocks > 0).then_some((start, end - 1))
    }

    #[must_use]
    pub fn price(
        windows: &mut CachedWindows,
        run: &[(BlockHeight, BlockHeader)],
        before: u32,
        blocks: usize,
    ) -> Option<f64> {
        let times: Vec<u32> = run.iter().map(|(_, header)| header.time().to_u32()).collect();
        let (first, last) = Self::span(&times, before, blocks)?;
        let selection = RangeSelection::Heights {
            start: run[first].0,
            stop: run[last].0,
        };
        let mut pipeline = OraclePipeline::new();
        windows.feed(selection, &mut pipeline).ok()?;
        pipeline
            .into_sample()
            .price()
            .ok()
            .map(|report| report.central_price)
    }
}

#[cfg(test)]
mod tests {
    use super::ChainAnchor;

    #[test]
    fn takes_the_newest_blocks_before_the_moment() {
        let times = [100, 200, 300, 400, 500];
        assert_eq!(ChainAnchor::span(&times, 450, 2), Some((2, 3)));
    }

    #[test]
    fn a_block_at_the_moment_is_not_before_it() {
        let times = [100, 200, 300];
        assert_eq!(ChainAnchor::span(&times, 300, 2), Some((0, 1)));
    }

    #[test]
    fn too_few_preceding_blocks_give_no_span() {
        let times = [100, 200, 300];
        assert_eq!(ChainAnchor::span(&times, 250, 3), None);
    }

    #[test]
    fn a_zero_block_window_gives_no_span() {
        assert_eq!(ChainAnchor::span(&[100, 200], 500, 0), None);
    }
}
