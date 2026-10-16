use mantle::difficulty::BlockHeight;
use mantle::primitives::BlockHeader;

/// An inclusive run of heights to fetch together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeightSpan {
    pub start: BlockHeight,
    pub stop: BlockHeight,
}

impl HeightSpan {
    #[must_use]
    pub const fn len(self) -> usize {
        (self.stop.to_u32() - self.start.to_u32()) as usize + 1
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        false
    }

    /// The headers of `headers` (sorted by height) that fall inside this span.
    #[must_use]
    pub fn headers(self, headers: &[(BlockHeight, BlockHeader)]) -> &[(BlockHeight, BlockHeader)] {
        let start = headers.partition_point(|(height, _)| *height < self.start);
        let stop = headers.partition_point(|(height, _)| *height <= self.stop);
        &headers[start..stop]
    }
}

/// Which blocks the record log still needs, and in what batches.
///
/// The newest `overlap` blocks already recorded are fetched again so a reorg
/// of that depth replaces them instead of leaving stale records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockPlan {
    depth: usize,
    overlap: u32,
    batch: usize,
}

impl BlockPlan {
    #[must_use]
    pub const fn new(depth: usize, overlap: u32, batch: usize) -> Self {
        Self {
            depth,
            overlap,
            batch,
        }
    }

    /// Blocks fetched together.
    #[must_use]
    pub const fn batch(&self) -> usize {
        self.batch
    }

    /// The heights to fetch given the record log's newest height and the
    /// chain tip, or `None` when nothing is missing.
    #[must_use]
    pub fn missing(&self, recorded_tip: Option<BlockHeight>, chain_tip: BlockHeight) -> Option<HeightSpan> {
        let tip = chain_tip.to_u32();
        let depth = u32::try_from(self.depth).unwrap_or(u32::MAX);
        let floor = tip.saturating_sub(depth.saturating_sub(1));
        let refetch_from = tip.saturating_sub(self.overlap.saturating_sub(1));
        let start = match recorded_tip {
            Some(recorded) if recorded.to_u32().saturating_add(1) >= floor => recorded
                .to_u32()
                .saturating_add(1)
                .saturating_sub(self.overlap)
                .min(refetch_from)
                .max(floor),
            _ => floor,
        };
        (start <= tip).then_some(HeightSpan {
            start: BlockHeight::new(start),
            stop: chain_tip,
        })
    }

    /// Split a span into consecutive batches, oldest first.
    #[must_use]
    pub fn batches(&self, span: HeightSpan) -> Vec<HeightSpan> {
        let size = u32::try_from(self.batch.max(1)).unwrap_or(u32::MAX);
        let mut batches = Vec::new();
        let mut start = span.start.to_u32();
        let stop = span.stop.to_u32();
        while start <= stop {
            let end = start.saturating_add(size - 1).min(stop);
            batches.push(HeightSpan {
                start: BlockHeight::new(start),
                stop: BlockHeight::new(end),
            });
            let Some(next) = end.checked_add(1) else { break };
            start = next;
        }
        batches
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockHeight;
    use mantle::primitives::BlockHeader;

    use super::{BlockPlan, HeightSpan};

    struct PlanFixture;

    impl PlanFixture {
        fn height(value: u32) -> BlockHeight {
            BlockHeight::new(value)
        }

        fn span(start: u32, stop: u32) -> HeightSpan {
            HeightSpan {
                start: Self::height(start),
                stop: Self::height(stop),
            }
        }
    }

    #[test]
    fn an_empty_log_covers_exactly_the_depth() {
        let plan = BlockPlan::new(1_008, 2, 64);
        let span = plan.missing(None, PlanFixture::height(965_000)).expect("span");

        assert_eq!(span.len(), 1_008);
        assert_eq!(span.start, PlanFixture::height(965_000 - 1_007));
    }

    #[test]
    fn a_caught_up_log_refetches_only_the_overlap() {
        let plan = BlockPlan::new(1_008, 2, 64);
        let span = plan
            .missing(Some(PlanFixture::height(965_000)), PlanFixture::height(965_000))
            .expect("span");

        assert_eq!(span, PlanFixture::span(964_999, 965_000));
    }

    #[test]
    fn new_blocks_are_fetched_after_the_overlap() {
        let plan = BlockPlan::new(1_008, 2, 64);
        let span = plan
            .missing(Some(PlanFixture::height(965_000)), PlanFixture::height(965_003))
            .expect("span");

        assert_eq!(span, PlanFixture::span(964_999, 965_003));
    }

    #[test]
    fn a_log_that_fell_out_of_the_window_starts_over_at_the_floor() {
        let plan = BlockPlan::new(1_008, 2, 64);
        let span = plan
            .missing(Some(PlanFixture::height(900_000)), PlanFixture::height(965_000))
            .expect("span");

        assert_eq!(span.start, PlanFixture::height(965_000 - 1_007));
    }

    #[test]
    fn a_log_ahead_of_the_chain_tip_refetches_the_overlap_at_the_tip() {
        let plan = BlockPlan::new(10, 2, 64);
        let span = plan
            .missing(Some(PlanFixture::height(120)), PlanFixture::height(100))
            .expect("span");

        assert_eq!(span, PlanFixture::span(99, 100));
    }

    #[test]
    fn a_young_chain_does_not_underflow() {
        let plan = BlockPlan::new(1_008, 2, 64);
        let span = plan.missing(None, PlanFixture::height(5)).expect("span");

        assert_eq!(span, PlanFixture::span(0, 5));
    }

    #[test]
    fn batches_tile_the_span_without_gaps_or_overlap() {
        let plan = BlockPlan::new(1_008, 2, 64);
        let span = PlanFixture::span(1_000, 1_200);
        let batches = plan.batches(span);

        assert_eq!(batches.first().map(|batch| batch.start), Some(span.start));
        assert_eq!(batches.last().map(|batch| batch.stop), Some(span.stop));
        assert!(batches.iter().all(|batch| batch.len() <= 64));
        for pair in batches.windows(2) {
            assert_eq!(pair[1].start.to_u32(), pair[0].stop.to_u32() + 1);
        }
        assert_eq!(batches.iter().map(|batch| batch.len()).sum::<usize>(), span.len());
    }

    #[test]
    fn a_span_selects_its_own_headers() {
        let headers: Vec<_> = (10..20)
            .map(|height| (PlanFixture::height(height), BlockHeader::from_slice(&[0; 80])))
            .collect();
        let selected = PlanFixture::span(12, 15).headers(&headers);

        assert_eq!(selected.len(), 4);
        assert_eq!(selected[0].0, PlanFixture::height(12));
        assert_eq!(selected[3].0, PlanFixture::height(15));
        assert!(PlanFixture::span(30, 40).headers(&headers).is_empty());
    }

    #[test]
    fn a_short_span_is_a_single_batch() {
        let plan = BlockPlan::new(1_008, 2, 64);

        assert_eq!(plan.batches(PlanFixture::span(7, 9)), vec![PlanFixture::span(7, 9)]);
    }

    #[test]
    fn a_span_at_the_top_of_the_height_range_terminates() {
        let plan = BlockPlan::new(10, 0, 4);
        let batches = plan.batches(PlanFixture::span(u32::MAX - 5, u32::MAX));

        assert_eq!(batches.len(), 2);
        assert_eq!(batches[1].stop, PlanFixture::height(u32::MAX));
    }
}
