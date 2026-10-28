use std::ops::Range;

/// The chain of blocks tiled over a box with no space left over: the row and column count whose
/// slots come closest to square, so a wide box gets one long row and a tall one a column. A
/// short last row stretches its slots to the full width. When the slots would be smaller than
/// [`Self::MIN_SLOT`], the oldest blocks are left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainLayout {
    pub columns: usize,
    pub shown: usize,
}

impl ChainLayout {
    pub const MIN_SLOT: f64 = 48.0;

    #[must_use]
    pub fn fit(blocks: usize, width: f64, height: f64) -> Self {
        (1..=blocks)
            .rev()
            .find_map(|shown| Self::squarest(shown, width, height))
            .unwrap_or(Self { columns: 1, shown: 0 })
    }

    fn squarest(shown: usize, width: f64, height: f64) -> Option<Self> {
        (1..=shown)
            .map(|rows| shown.div_ceil(rows))
            .filter(|columns| width / *columns as f64 >= Self::MIN_SLOT && height / shown.div_ceil(*columns) as f64 >= Self::MIN_SLOT)
            .min_by(|left, right| Self::skew(*left, shown, width, height).total_cmp(&Self::skew(*right, shown, width, height)))
            .map(|columns| Self { columns, shown })
    }

    /// How far a slot is from square, on a log scale so too wide and too tall weigh the same.
    fn skew(columns: usize, shown: usize, width: f64, height: f64) -> f64 {
        let rows = shown.div_ceil(columns) as f64;
        ((width / columns as f64) / (height / rows)).ln().abs()
    }

    /// The blocks in each row, as index ranges into the newest-first list.
    #[must_use]
    pub fn rows(&self) -> Vec<Range<usize>> {
        (0..self.shown)
            .step_by(self.columns)
            .map(|start| start..(start + self.columns).min(self.shown))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::ChainLayout;

    #[test]
    fn the_grid_follows_the_shape_of_the_box() {
        assert_eq!(ChainLayout::fit(6, 1_200.0, 200.0).columns, 6);
        assert_eq!(ChainLayout::fit(6, 440.0, 300.0).columns, 3);
        assert_eq!(ChainLayout::fit(6, 300.0, 440.0).columns, 2);
        assert_eq!(ChainLayout::fit(6, 120.0, 900.0).columns, 1);
    }

    #[test]
    fn a_short_last_row_holds_the_remaining_blocks() {
        let layout = ChainLayout { columns: 3, shown: 5 };

        assert_eq!(layout.rows(), vec![0..3, 3..5]);
    }

    #[test]
    fn a_small_box_drops_the_oldest_blocks_instead_of_shrinking_past_the_minimum() {
        let layout = ChainLayout::fit(6, 200.0, 60.0);

        assert_eq!(layout.shown, 4);
        assert_eq!(layout.columns, 4);
    }

    #[test]
    fn an_unmeasured_box_shows_nothing() {
        assert_eq!(ChainLayout::fit(6, 0.0, 0.0).shown, 0);
        assert!(ChainLayout::fit(6, 0.0, 0.0).rows().is_empty());
    }
}
