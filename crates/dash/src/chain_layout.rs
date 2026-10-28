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
    pub fn for_side(side: f64) -> Self {
        match side {
            side if side >= 104.0 => Self::All,
            side if side >= 72.0 => Self::Some,
            _ => Self::Least,
        }
    }
}

/// The chain of cubes flowed into a box without scrolling. Each cube's side grows with how full
/// its block was; the cubes run newest first, left to right, wrapping to a new row when the next
/// one does not fit, and the whole chain takes the largest scale at which every row fits the
/// box. When even the smallest scale cannot hold them all, the oldest blocks are left out.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainLayout {
    sides: Vec<f64>,
    rows: Vec<Vec<usize>>,
}

impl ChainLayout {
    /// The side of an empty block's cube, as a share of a full one's.
    pub const EMPTY_SHARE: f64 = 0.55;
    /// The smallest and largest sides of a full block's cube, in pixels.
    pub const MIN_SCALE: f64 = 56.0;
    pub const MAX_SCALE: f64 = 136.0;
    /// The 3D top and side faces, as a share of the cube's side.
    pub const DEPTH: f64 = 0.1;
    /// The gap between neighbours in a row, which holds the link between them.
    pub const LINK: f64 = 22.0;
    pub const ROW_GAP: f64 = 8.0;
    /// The height label above and the confirmations label below each cube, with their gaps,
    /// and room for the lift of the picked cube.
    pub const LABELS: f64 = 46.0;
    /// Room each side for the outline of the picked cube.
    pub const OUTLINE: f64 = 6.0;

    /// `fullness` holds each block's weight share, newest first.
    #[must_use]
    pub fn fit(fullness: &[f64], width: f64, height: f64) -> Self {
        let width = width - 2.0 * Self::OUTLINE;
        (1..=fullness.len())
            .rev()
            .find_map(|shown| Self::largest(&fullness[..shown], width, height))
            .unwrap_or(Self { sides: Vec::new(), rows: Vec::new() })
    }

    /// The layout at the largest scale that fits, if even the smallest does.
    fn largest(fullness: &[f64], width: f64, height: f64) -> Option<Self> {
        if let Some(layout) = Self::flow(fullness, Self::MAX_SCALE, width, height) {
            return Some(layout);
        }
        let mut fits = Self::flow(fullness, Self::MIN_SCALE, width, height)?;
        let (mut low, mut high) = (Self::MIN_SCALE, Self::MAX_SCALE);
        for _ in 0..24 {
            let scale = f64::midpoint(low, high);
            match Self::flow(fullness, scale, width, height) {
                Some(layout) => (fits, low) = (layout, scale),
                None => high = scale,
            }
        }
        Some(fits)
    }

    /// The cubes at `scale` wrapped into rows, if they fit the box.
    fn flow(fullness: &[f64], scale: f64, width: f64, height: f64) -> Option<Self> {
        let sides: Vec<f64> = fullness
            .iter()
            .map(|full| (scale * (Self::EMPTY_SHARE + (1.0 - Self::EMPTY_SHARE) * full.clamp(0.0, 1.0))).floor())
            .collect();
        let mut rows: Vec<Vec<usize>> = Vec::new();
        let mut used = 0.0;
        for (index, side) in sides.iter().enumerate() {
            let slot = Self::slot(*side);
            if slot > width {
                return None;
            }
            match rows.last_mut() {
                Some(row) if used + Self::LINK + slot <= width => {
                    row.push(index);
                    used += Self::LINK + slot;
                }
                _ => {
                    rows.push(vec![index]);
                    used = slot;
                }
            }
        }
        let layout = Self { sides, rows };
        (layout.height() <= height).then_some(layout)
    }

    fn slot(side: f64) -> f64 {
        side * (1.0 + Self::DEPTH)
    }

    fn height(&self) -> f64 {
        let rows: f64 = self
            .rows
            .iter()
            .map(|row| row.iter().map(|index| Self::slot(self.sides[*index])).fold(0.0, f64::max) + Self::LABELS)
            .sum();
        rows + self.rows.len().saturating_sub(1) as f64 * Self::ROW_GAP
    }

    /// The blocks shown in each row, by index into the newest-first list.
    #[must_use]
    pub fn rows(&self) -> &[Vec<usize>] {
        &self.rows
    }

    #[must_use]
    pub fn side(&self, index: usize) -> f64 {
        self.sides.get(index).copied().unwrap_or(0.0)
    }

    #[must_use]
    pub fn shown(&self) -> usize {
        self.sides.len()
    }
}

#[cfg(test)]
mod tests {
    use super::{ChainLayout, CubeDetail};

    struct LayoutFixture;

    impl LayoutFixture {
        const FULLNESS: [f64; 8] = [1.0, 0.4, 0.9, 1.0, 0.1, 0.7, 1.0, 0.95];

        fn used(layout: &ChainLayout) -> (f64, f64) {
            let width = layout
                .rows()
                .iter()
                .map(|row| {
                    row.iter().map(|index| ChainLayout::slot(layout.side(*index))).sum::<f64>()
                        + (row.len() - 1) as f64 * ChainLayout::LINK
                })
                .fold(0.0, f64::max);
            (width + 2.0 * ChainLayout::OUTLINE, layout.height())
        }
    }

    #[test]
    fn fuller_blocks_get_bigger_cubes() {
        let layout = ChainLayout::fit(&LayoutFixture::FULLNESS, 2_400.0, 400.0);

        assert!(layout.side(0) > layout.side(1));
        assert!((layout.side(1) / layout.side(0) - 0.73).abs() < 0.01);
        assert!((layout.side(0) - ChainLayout::MAX_SCALE).abs() < 1.0);
    }

    #[test]
    fn the_rows_follow_the_width() {
        let rows = |width: f64| ChainLayout::fit(&LayoutFixture::FULLNESS, width, 900.0).rows().len();

        assert_eq!(rows(2_400.0), 1);
        assert!(rows(700.0) > 1);
        assert!(rows(320.0) > rows(700.0));
    }

    #[test]
    fn rows_can_hold_different_numbers_of_blocks() {
        let layout = ChainLayout::fit(&LayoutFixture::FULLNESS[..5], 560.0, 600.0);
        let lengths: Vec<usize> = layout.rows().iter().map(Vec::len).collect();

        assert_eq!(lengths.iter().sum::<usize>(), 5);
        assert!(lengths.windows(2).any(|pair| pair[0] != pair[1]), "{lengths:?}");
    }

    #[test]
    fn nothing_overflows_the_box() {
        for (width, height) in [(320.0, 220.0), (800.0, 180.0), (500.0, 600.0), (1_400.0, 260.0), (260.0, 900.0)] {
            let layout = ChainLayout::fit(&LayoutFixture::FULLNESS, width, height);
            let (used_width, used_height) = LayoutFixture::used(&layout);

            assert!(layout.shown() > 0, "{width}x{height}");
            assert!(used_width <= width && used_height <= height, "{width}x{height}: {layout:?}");
        }
    }

    #[test]
    fn a_small_box_drops_the_oldest_blocks_instead_of_shrinking_past_the_minimum() {
        let layout = ChainLayout::fit(&LayoutFixture::FULLNESS, 240.0, 120.0);

        assert!(layout.shown() < 8 && layout.shown() > 0);
        assert!(layout.side(0) >= ChainLayout::MIN_SCALE);
    }

    #[test]
    fn an_unmeasured_box_shows_nothing() {
        assert_eq!(ChainLayout::fit(&LayoutFixture::FULLNESS, 0.0, 0.0).shown(), 0);
    }

    #[test]
    fn small_cubes_carry_less_writing() {
        assert_eq!(CubeDetail::for_side(120.0), CubeDetail::All);
        assert_eq!(CubeDetail::for_side(80.0), CubeDetail::Some);
        assert_eq!(CubeDetail::for_side(40.0), CubeDetail::Least);
    }
}
