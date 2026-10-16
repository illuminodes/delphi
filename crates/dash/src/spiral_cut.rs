use crate::{SplitAxis, TileRect};

/// How one card is cut from the remaining area: along which axis, and from the leading or trailing side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpiralCut {
    axis: SplitAxis,
    leads: bool,
}

impl SpiralCut {
    #[must_use]
    pub const fn new(axis: SplitAxis, leads: bool) -> Self {
        Self { axis, leads }
    }

    #[must_use]
    pub const fn axis(self) -> SplitAxis {
        self.axis
    }

    /// The card's part (a `ratio` share of `area`) and the part left for the cards after it.
    #[must_use]
    pub fn split(self, area: TileRect, ratio: f64) -> (TileRect, TileRect) {
        if self.leads {
            area.split(self.axis, ratio)
        } else {
            let (rest, card) = area.split(self.axis, 1.0 - ratio);
            (card, rest)
        }
    }

    /// The ratio that puts the card's inner edge under the workspace point `(x, y)`.
    #[must_use]
    pub fn ratio_at(self, area: TileRect, x: f64, y: f64) -> f64 {
        let along = match self.axis {
            SplitAxis::Row => (x - area.x) / area.width,
            SplitAxis::Column => (y - area.y) / area.height,
        };
        if self.leads {
            along
        } else {
            1.0 - along
        }
    }

    /// Where the card's inner edge sits, in workspace fractions along the cut axis.
    #[must_use]
    pub fn boundary(self, area: TileRect, ratio: f64) -> f64 {
        let share = if self.leads { ratio } else { 1.0 - ratio };
        match self.axis {
            SplitAxis::Row => area.x + area.width * share,
            SplitAxis::Column => area.y + area.height * share,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SpiralCut;
    use crate::{SplitAxis, TileRect};

    #[test]
    fn a_leading_cut_takes_the_start_and_a_trailing_cut_the_end() {
        let (card, rest) = SpiralCut::new(SplitAxis::Row, true).split(TileRect::FULL, 0.25);
        assert!((card.width - 0.25).abs() < 1e-9 && card.x.abs() < 1e-9);
        assert!((rest.x - 0.25).abs() < 1e-9);

        let (card, rest) = SpiralCut::new(SplitAxis::Column, false).split(TileRect::FULL, 0.25);
        assert!((card.y - 0.75).abs() < 1e-9 && (card.height - 0.25).abs() < 1e-9);
        assert!((rest.height - 0.75).abs() < 1e-9 && rest.y.abs() < 1e-9);
    }

    #[test]
    fn the_ratio_under_the_pointer_puts_the_edge_back_under_it() {
        let area = TileRect { x: 0.5, y: 0.2, width: 0.5, height: 0.8 };
        for cut in [
            SpiralCut::new(SplitAxis::Row, true),
            SpiralCut::new(SplitAxis::Row, false),
            SpiralCut::new(SplitAxis::Column, true),
            SpiralCut::new(SplitAxis::Column, false),
        ] {
            let ratio = cut.ratio_at(area, 0.7, 0.4);
            let expected = match cut.axis() {
                SplitAxis::Row => 0.7,
                SplitAxis::Column => 0.4,
            };
            assert!((cut.boundary(area, ratio) - expected).abs() < 1e-9);
        }
    }
}
