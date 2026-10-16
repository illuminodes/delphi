use crate::{SpiralCut, SplitAxis, TileRect};

/// Chooses each successive cut of the spiral. The first cut goes across the workspace's longer
/// side; after that the axis alternates and each axis alternates its side, so on a wide screen
/// the cards wind left, top, right, bottom, independently of how large each card is.
#[derive(Debug, Clone, Copy)]
pub struct SpiralCompass {
    axis: SplitAxis,
    row_leads: bool,
    column_leads: bool,
}

impl SpiralCompass {
    #[must_use]
    pub fn new(aspect: f64) -> Self {
        Self {
            axis: TileRect::FULL.longer_axis(aspect),
            row_leads: true,
            column_leads: true,
        }
    }

    pub fn turn(&mut self) -> SpiralCut {
        let axis = self.axis;
        let side = match axis {
            SplitAxis::Row => &mut self.row_leads,
            SplitAxis::Column => &mut self.column_leads,
        };
        let leads = *side;
        *side = !leads;
        self.axis = match axis {
            SplitAxis::Row => SplitAxis::Column,
            SplitAxis::Column => SplitAxis::Row,
        };
        SpiralCut::new(axis, leads)
    }
}

#[cfg(test)]
mod tests {
    use super::SpiralCompass;
    use crate::{SpiralCut, SplitAxis};

    #[test]
    fn a_wide_workspace_winds_left_top_right_bottom() {
        let mut compass = SpiralCompass::new(1.8);
        let cuts: Vec<SpiralCut> = (0..5).map(|_| compass.turn()).collect();

        assert_eq!(
            cuts,
            vec![
                SpiralCut::new(SplitAxis::Row, true),
                SpiralCut::new(SplitAxis::Column, true),
                SpiralCut::new(SplitAxis::Row, false),
                SpiralCut::new(SplitAxis::Column, false),
                SpiralCut::new(SplitAxis::Row, true),
            ]
        );
    }

    #[test]
    fn a_tall_workspace_starts_with_a_horizontal_cut() {
        assert_eq!(SpiralCompass::new(0.5).turn().axis(), SplitAxis::Column);
    }
}
