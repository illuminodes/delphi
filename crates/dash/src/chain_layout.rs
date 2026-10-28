/// How much writing a cube has room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CubeDetail {
    /// Fee rate only.
    Least,
    /// Fee rate, fees and transactions.
    Some,
    /// Everything: also votes and age.
    All,
}

impl CubeDetail {
    #[must_use]
    pub const fn css_class(self) -> &'static str {
        match self {
            Self::Least => "detail-least",
            Self::Some => "detail-some",
            Self::All => "detail-all",
        }
    }
}

/// The chain of cubes fitted into a box without scrolling: how many blocks fit, in how many rows,
/// and the cube side in pixels. Cubes grow as large as the box allows, wrapping into more rows
/// when that makes them bigger; when even the smallest cubes cannot all fit, the oldest blocks
/// are left out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChainLayout {
    pub cube: f64,
    pub columns: usize,
    pub rows: usize,
    pub shown: usize,
}

impl ChainLayout {
    pub const MIN_CUBE: f64 = 44.0;
    pub const MAX_CUBE: f64 = 128.0;
    /// The 3D top and side faces, as a share of the cube's side.
    pub const DEPTH: f64 = 0.1;
    /// The gap between columns, which holds the link to the next block.
    pub const LINK: f64 = 22.0;
    pub const ROW_GAP: f64 = 6.0;
    /// The height label above and the confirmations label below each cube, with their gaps,
    /// and room for the lift and outline of the picked cube.
    pub const LABELS: f64 = 46.0;
    /// Room each side for the outline of the picked cube.
    pub const OUTLINE: f64 = 6.0;

    #[must_use]
    pub fn fit(blocks: usize, width: f64, height: f64) -> Self {
        let width = width - 2.0 * Self::OUTLINE;
        (1..=blocks)
            .rev()
            .find_map(|shown| Self::best(shown, width, height).filter(|layout| layout.cube >= Self::MIN_CUBE))
            .unwrap_or(Self { cube: Self::MIN_CUBE, columns: 1, rows: 1, shown: 0 })
    }

    /// The row count giving `shown` blocks their largest cubes.
    fn best(shown: usize, width: f64, height: f64) -> Option<Self> {
        (1..=shown)
            .map(|rows| {
                let columns = shown.div_ceil(rows);
                let rows = shown.div_ceil(columns);
                let across = (width - (columns - 1) as f64 * Self::LINK) / (columns as f64 * (1.0 + Self::DEPTH));
                let down = ((height - (rows - 1) as f64 * Self::ROW_GAP) / rows as f64 - Self::LABELS) / (1.0 + Self::DEPTH);
                Self { cube: across.min(down).min(Self::MAX_CUBE).floor(), columns, rows, shown }
            })
            .max_by(|left, right| left.cube.total_cmp(&right.cube))
    }

    #[must_use]
    pub fn detail(&self) -> CubeDetail {
        match self.cube {
            cube if cube >= 104.0 => CubeDetail::All,
            cube if cube >= 72.0 => CubeDetail::Some,
            _ => CubeDetail::Least,
        }
    }

    /// Whether the block at `index` (newest first) has a link to its left: every block but the
    /// first in each row.
    #[must_use]
    pub fn linked(&self, index: usize) -> bool {
        !index.is_multiple_of(self.columns)
    }
}

#[cfg(test)]
mod tests {
    use super::{ChainLayout, CubeDetail};

    #[test]
    fn a_wide_box_holds_every_block_in_one_row_at_the_largest_size() {
        let layout = ChainLayout::fit(6, 2_000.0, 300.0);

        assert_eq!((layout.rows, layout.columns, layout.shown), (1, 6, 6));
        assert!((layout.cube - ChainLayout::MAX_CUBE).abs() < f64::EPSILON);
        assert_eq!(layout.detail(), CubeDetail::All);
    }

    #[test]
    fn a_tall_narrow_box_wraps_into_rows_for_bigger_cubes() {
        let layout = ChainLayout::fit(6, 300.0, 900.0);

        assert!(layout.rows > 1);
        assert_eq!(layout.shown, 6);
        assert!(layout.cube > ChainLayout::fit(6, 300.0, 150.0).cube);
    }

    #[test]
    fn nothing_overflows_the_box() {
        for (width, height) in [(320.0, 200.0), (800.0, 180.0), (500.0, 600.0), (1_400.0, 260.0)] {
            let layout = ChainLayout::fit(8, width, height);
            let columns = layout.columns as f64;
            let rows = layout.rows as f64;
            let used_width = columns * layout.cube * (1.0 + ChainLayout::DEPTH) + (columns - 1.0) * ChainLayout::LINK;
            let used_height = rows * (layout.cube * (1.0 + ChainLayout::DEPTH) + ChainLayout::LABELS)
                + (rows - 1.0) * ChainLayout::ROW_GAP;

            assert!(used_width <= width && used_height <= height, "{width}x{height}: {layout:?}");
        }
    }

    #[test]
    fn a_small_box_drops_the_oldest_blocks_instead_of_shrinking_past_the_minimum() {
        let layout = ChainLayout::fit(8, 220.0, 110.0);

        assert!(layout.shown < 8 && layout.shown > 0);
        assert!(layout.cube >= ChainLayout::MIN_CUBE);
        assert_eq!(layout.detail(), CubeDetail::Least);
    }

    #[test]
    fn an_unmeasured_box_shows_nothing() {
        assert_eq!(ChainLayout::fit(6, 0.0, 0.0).shown, 0);
    }

    #[test]
    fn only_the_first_block_of_each_row_has_no_link() {
        let layout = ChainLayout { cube: 80.0, columns: 3, rows: 2, shown: 6 };
        let linked: Vec<bool> = (0..6).map(|index| layout.linked(index)).collect();

        assert_eq!(linked, vec![false, true, true, false, true, true]);
    }
}
