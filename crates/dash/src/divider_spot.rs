use crate::{SpiralCut, SplitAxis, TileRect};

/// The inner edge of one card: which card, how it was cut, the area it was cut from, and its share.
#[derive(Debug, Clone, PartialEq)]
pub struct DividerSpot {
    index: usize,
    cut: SpiralCut,
    area: TileRect,
    ratio: f64,
}

impl DividerSpot {
    #[must_use]
    pub const fn new(index: usize, cut: SpiralCut, area: TileRect, ratio: f64) -> Self {
        Self { index, cut, area, ratio }
    }

    #[must_use]
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn ratio(&self) -> f64 {
        self.ratio
    }

    #[must_use]
    pub const fn axis(&self) -> SplitAxis {
        self.cut.axis()
    }

    #[must_use]
    pub fn key(&self) -> String {
        format!("divider:{}", self.index)
    }

    /// The card's ratio that puts this edge under the workspace point `(x, y)`.
    #[must_use]
    pub fn ratio_at(&self, x: f64, y: f64) -> f64 {
        self.cut.ratio_at(self.area, x, y)
    }

    #[must_use]
    pub fn style(&self) -> String {
        let at = self.cut.boundary(self.area, self.ratio) * 100.0;
        match self.cut.axis() {
            SplitAxis::Row => format!(
                "left:{at:.4}%;top:{:.4}%;height:{:.4}%",
                self.area.y * 100.0,
                self.area.height * 100.0
            ),
            SplitAxis::Column => format!(
                "top:{at:.4}%;left:{:.4}%;width:{:.4}%",
                self.area.x * 100.0,
                self.area.width * 100.0
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DividerSpot;
    use crate::{SpiralCut, SplitAxis, TileRect};

    #[test]
    fn a_trailing_edge_reads_the_pointer_from_the_far_side() {
        let area = TileRect { x: 0.5, y: 0.0, width: 0.5, height: 1.0 };
        let spot = DividerSpot::new(2, SpiralCut::new(SplitAxis::Row, false), area, 0.5);

        assert!((spot.ratio_at(0.6, 0.9) - 0.8).abs() < 1e-9);
        assert_eq!(spot.key(), "divider:2");
    }

    #[test]
    fn the_style_places_the_edge_across_its_area() {
        let spot = DividerSpot::new(0, SpiralCut::new(SplitAxis::Column, true), TileRect::FULL, 0.25);
        assert_eq!(spot.style(), "top:25.0000%;left:0.0000%;width:100.0000%");
    }
}
