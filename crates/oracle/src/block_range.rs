use std::fmt;

use mantle::difficulty::{BlockHeight, BlockTime};
use mantle::primitives::BlockHeader;

const SECONDS_PER_DAY: u32 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSelection {
    PreviousUtcDay,
    UtcDay(BlockTime),
    RecentBlocks(usize),
    Heights {
        start: BlockHeight,
        stop: BlockHeight,
    },
    TimeSpan {
        start: BlockTime,
        stop: BlockTime,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockRange {
    start: BlockHeight,
    stop: BlockHeight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockRangeError {
    NoHeaders,
    EmptySelection,
    InvalidHeightOrder,
    InvalidDayStart,
}

impl fmt::Display for BlockRangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHeaders => formatter.write_str("no headers are available"),
            Self::EmptySelection => formatter.write_str("the selected range contains no blocks"),
            Self::InvalidHeightOrder => formatter.write_str("start height exceeds stop height"),
            Self::InvalidDayStart => formatter.write_str("UTC day start is not midnight"),
        }
    }
}

impl std::error::Error for BlockRangeError {}

impl BlockRange {
    #[allow(clippy::missing_errors_doc)]
    pub fn select(
        headers: &[(BlockHeight, BlockHeader)],
        selection: RangeSelection,
    ) -> Result<Self, BlockRangeError> {
        let (_, tip) = headers.last().ok_or(BlockRangeError::NoHeaders)?;
        match selection {
            RangeSelection::PreviousUtcDay => {
                let day_start = tip.time().to_u32() / SECONDS_PER_DAY * SECONDS_PER_DAY;
                let previous_start = day_start
                    .checked_sub(SECONDS_PER_DAY)
                    .ok_or(BlockRangeError::EmptySelection)?;
                Self::for_utc_day(headers, BlockTime::new(previous_start))
            }
            RangeSelection::UtcDay(day_start) => Self::for_utc_day(headers, day_start),
            RangeSelection::RecentBlocks(count) => Self::recent(headers, count),
            RangeSelection::Heights { start, stop } => Self::heights(headers, start, stop),
            RangeSelection::TimeSpan { start, stop } => Self::for_time_span(headers, start, stop),
        }
    }

    #[must_use]
    pub const fn start(self) -> BlockHeight {
        self.start
    }

    #[must_use]
    pub const fn stop(self) -> BlockHeight {
        self.stop
    }

    #[must_use]
    pub fn contains(self, height: BlockHeight) -> bool {
        height >= self.start && height <= self.stop
    }

    #[must_use]
    pub fn headers(self, headers: &[(BlockHeight, BlockHeader)]) -> &[(BlockHeight, BlockHeader)] {
        let start = headers.partition_point(|(height, _)| *height < self.start);
        let stop = headers.partition_point(|(height, _)| *height <= self.stop);
        &headers[start..stop]
    }

    fn for_utc_day(
        headers: &[(BlockHeight, BlockHeader)],
        day_start: BlockTime,
    ) -> Result<Self, BlockRangeError> {
        let start_time = day_start.to_u32();
        if !start_time.is_multiple_of(SECONDS_PER_DAY) {
            return Err(BlockRangeError::InvalidDayStart);
        }
        let stop_time = start_time
            .checked_add(SECONDS_PER_DAY)
            .ok_or(BlockRangeError::EmptySelection)?;
        let mut matching = headers.iter().filter(|(_, header)| {
            let time = header.time().to_u32();
            time >= start_time && time < stop_time
        });
        let start = matching
            .next()
            .map(|(height, _)| *height)
            .ok_or(BlockRangeError::EmptySelection)?;
        let stop = matching.next_back().map_or(start, |(height, _)| *height);
        Ok(Self { start, stop })
    }

    fn for_time_span(
        headers: &[(BlockHeight, BlockHeader)],
        start: BlockTime,
        stop: BlockTime,
    ) -> Result<Self, BlockRangeError> {
        let start_time = start.to_u32();
        let stop_time = stop.to_u32();
        if start_time >= stop_time {
            return Err(BlockRangeError::InvalidHeightOrder);
        }
        let mut matching = headers.iter().filter(|(_, header)| {
            let time = header.time().to_u32();
            time >= start_time && time < stop_time
        });
        let first = matching
            .next()
            .map(|(height, _)| *height)
            .ok_or(BlockRangeError::EmptySelection)?;
        let last = matching.next_back().map_or(first, |(height, _)| *height);
        Ok(Self {
            start: first,
            stop: last,
        })
    }

    fn recent(
        headers: &[(BlockHeight, BlockHeader)],
        count: usize,
    ) -> Result<Self, BlockRangeError> {
        if count == 0 {
            return Err(BlockRangeError::EmptySelection);
        }
        let start_index = headers.len().saturating_sub(count);
        let start = headers
            .get(start_index)
            .map(|(height, _)| *height)
            .ok_or(BlockRangeError::NoHeaders)?;
        let stop = headers
            .last()
            .map(|(height, _)| *height)
            .ok_or(BlockRangeError::NoHeaders)?;
        Ok(Self { start, stop })
    }

    fn heights(
        headers: &[(BlockHeight, BlockHeader)],
        start: BlockHeight,
        stop: BlockHeight,
    ) -> Result<Self, BlockRangeError> {
        if start > stop {
            return Err(BlockRangeError::InvalidHeightOrder);
        }
        let first = headers
            .first()
            .map(|(height, _)| *height)
            .ok_or(BlockRangeError::NoHeaders)?;
        let last = headers
            .last()
            .map(|(height, _)| *height)
            .ok_or(BlockRangeError::NoHeaders)?;
        if start < first || stop > last {
            return Err(BlockRangeError::EmptySelection);
        }
        Ok(Self { start, stop })
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::{BlockHeight, BlockTime, BlockVersion, CompactBits, Nonce};
    use mantle::hash::Hash256;
    use mantle::primitives::BlockHeader;

    use super::{BlockRange, BlockRangeError, RangeSelection};

    struct HeaderFixture;

    impl HeaderFixture {
        fn headers(times: &[u32]) -> Vec<(BlockHeight, BlockHeader)> {
            times
                .iter()
                .enumerate()
                .map(|(index, time)| {
                    let height = BlockHeight::new(
                        100 + u32::try_from(index).expect("fixture index fits u32"),
                    );
                    let header = BlockHeader::new(
                        BlockVersion::new(1),
                        Hash256::ZERO,
                        Hash256::from_bytes([u8::try_from(index).unwrap_or(0); 32]),
                        BlockTime::new(*time),
                        CompactBits::new(0x1d00_ffff),
                        Nonce::new(0),
                    );
                    (height, header)
                })
                .collect()
        }
    }

    #[test]
    fn selects_a_utc_day() {
        let headers = HeaderFixture::headers(&[86_399, 86_400, 90_000, 172_799, 172_800]);
        let range = BlockRange::select(&headers, RangeSelection::UtcDay(BlockTime::new(86_400)))
            .expect("day exists");

        assert_eq!(range.start(), BlockHeight::new(101));
        assert_eq!(range.stop(), BlockHeight::new(103));
        assert_eq!(range.headers(&headers).len(), 3);
    }

    #[test]
    fn selects_the_previous_day_from_tip_time() {
        let headers = HeaderFixture::headers(&[86_400, 172_799, 172_800, 180_000]);
        let range = BlockRange::select(&headers, RangeSelection::PreviousUtcDay)
            .expect("previous day exists");

        assert_eq!(range.start(), BlockHeight::new(100));
        assert_eq!(range.stop(), BlockHeight::new(101));
    }

    #[test]
    fn selects_recent_blocks_and_clamps_to_available_headers() {
        let headers = HeaderFixture::headers(&[1, 2, 3]);
        let range =
            BlockRange::select(&headers, RangeSelection::RecentBlocks(144)).expect("headers exist");

        assert_eq!(range.start(), BlockHeight::new(100));
        assert_eq!(range.stop(), BlockHeight::new(102));
    }

    #[test]
    fn rejects_reversed_explicit_heights() {
        let headers = HeaderFixture::headers(&[1, 2, 3]);
        assert_eq!(
            BlockRange::select(
                &headers,
                RangeSelection::Heights {
                    start: BlockHeight::new(102),
                    stop: BlockHeight::new(101),
                },
            ),
            Err(BlockRangeError::InvalidHeightOrder)
        );
    }

    #[test]
    fn selects_a_sub_day_time_span() {
        let headers = HeaderFixture::headers(&[86_400, 88_000, 90_000, 92_000]);
        let range = BlockRange::select(
            &headers,
            RangeSelection::TimeSpan {
                start: BlockTime::new(87_000),
                stop: BlockTime::new(91_000),
            },
        )
        .expect("span covers blocks");

        assert_eq!(range.start(), BlockHeight::new(101));
        assert_eq!(range.stop(), BlockHeight::new(102));
        assert_eq!(range.headers(&headers).len(), 2);
    }

    #[test]
    fn time_span_of_one_block_starts_and_stops_together() {
        let headers = HeaderFixture::headers(&[86_400, 88_000, 90_000]);
        let range = BlockRange::select(
            &headers,
            RangeSelection::TimeSpan {
                start: BlockTime::new(87_500),
                stop: BlockTime::new(89_000),
            },
        )
        .expect("span covers one block");

        assert_eq!(range.start(), BlockHeight::new(101));
        assert_eq!(range.stop(), BlockHeight::new(101));
    }

    #[test]
    fn rejects_a_time_span_with_no_blocks() {
        let headers = HeaderFixture::headers(&[86_400, 88_000]);
        assert_eq!(
            BlockRange::select(
                &headers,
                RangeSelection::TimeSpan {
                    start: BlockTime::new(100_000),
                    stop: BlockTime::new(110_000),
                },
            ),
            Err(BlockRangeError::EmptySelection)
        );
    }

    #[test]
    fn rejects_a_reversed_time_span() {
        let headers = HeaderFixture::headers(&[86_400, 88_000]);
        assert_eq!(
            BlockRange::select(
                &headers,
                RangeSelection::TimeSpan {
                    start: BlockTime::new(88_000),
                    stop: BlockTime::new(86_400),
                },
            ),
            Err(BlockRangeError::InvalidHeightOrder)
        );
    }

    #[test]
    fn rejects_non_midnight_day_start() {
        let headers = HeaderFixture::headers(&[86_400]);
        assert_eq!(
            BlockRange::select(&headers, RangeSelection::UtcDay(BlockTime::new(86_401)),),
            Err(BlockRangeError::InvalidDayStart)
        );
    }
}
