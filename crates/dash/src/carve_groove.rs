use crate::{CarveBox, CarveFill, CarveWeight};

#[derive(Debug, Clone, PartialEq)]
pub struct CarveGroove {
    pub points: Vec<(f64, f64)>,
    pub width: f64,
    pub depth: f64,
    pub fill: CarveFill,
    pub closed: bool,
}

impl CarveGroove {
    const RING_STEPS: usize = 48;

    #[must_use]
    pub fn new(points: Vec<(f64, f64)>, width: f64) -> Self {
        Self { points, width, depth: 1.0, fill: CarveFill::Plain, closed: false }
    }

    #[must_use]
    pub fn line(from: (f64, f64), to: (f64, f64), width: f64) -> Self {
        Self::new(vec![from, to], width)
    }

    #[must_use]
    pub fn dashed(from: (f64, f64), to: (f64, f64), width: f64, dash: f64, gap: f64) -> Vec<Self> {
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        let period = (dash + gap).max(f64::EPSILON);
        let count = ((length + gap) / period).floor().max(0.0) as usize;
        let at = |distance: f64| {
            let t = if length > 0.0 { distance / length } else { 0.0 };
            (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t)
        };
        (0..count)
            .map(|index| {
                let start = index as f64 * period;
                Self::line(at(start), at((start + dash).min(length)), width)
            })
            .collect()
    }

    #[must_use]
    pub fn frame(left: f64, top: f64, right: f64, bottom: f64, width: f64) -> Self {
        Self { closed: true, ..Self::new(vec![(left, top), (right, top), (right, bottom), (left, bottom)], width) }
    }

    #[must_use]
    pub fn ring(centre: (f64, f64), radius: f64, width: f64) -> Self {
        let points = (0..Self::RING_STEPS)
            .map(|step| {
                let angle = std::f64::consts::TAU * step as f64 / Self::RING_STEPS as f64;
                (centre.0 + radius * angle.cos(), centre.1 + radius * angle.sin())
            })
            .collect();
        Self { closed: true, ..Self::new(points, width) }
    }

    #[must_use]
    pub fn weighted(self, weight: CarveWeight, unit: f64) -> Self {
        Self { width: weight.width() * unit, depth: weight.depth(), ..self }
    }

    #[must_use]
    pub fn depth(self, depth: f64) -> Self {
        Self { depth, ..self }
    }

    #[must_use]
    pub fn gilt(self) -> Self {
        Self { fill: CarveFill::Gilt, ..self }
    }

    #[must_use]
    pub fn bounds(&self) -> CarveBox {
        let half = self.width / 2.0;
        let start = CarveBox::new(f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        self.points.iter().fold(start, |bounds, (x, y)| {
            CarveBox::new(bounds.left.min(x - half), bounds.top.min(y - half), bounds.right.max(x + half), bounds.bottom.max(y + half))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CarveGroove;

    #[test]
    fn a_ring_is_closed_and_keeps_to_its_radius() {
        let ring = CarveGroove::ring((50.0, 40.0), 10.0, 2.0);

        assert!(ring.closed);
        assert!(ring.points.iter().all(|(x, y)| ((x - 50.0).hypot(y - 40.0) - 10.0).abs() < 1e-9));
    }

    #[test]
    fn a_dashed_line_breaks_into_dashes_that_stay_on_the_line() {
        let dashes = CarveGroove::dashed((0.0, 5.0), (100.0, 5.0), 1.0, 4.0, 6.0);

        assert_eq!(dashes.len(), 10);
        assert!(dashes.iter().all(|dash| dash.points.iter().all(|(x, y)| (*y - 5.0).abs() < 1e-9 && *x <= 100.0)));
    }

    #[test]
    fn a_weight_sets_the_width_in_units_and_the_depth() {
        let groove = CarveGroove::line((0.0, 0.0), (10.0, 0.0), 9.0).weighted(crate::CarveWeight::Bold, 2.0);

        assert!((groove.width - 5.2).abs() < 1e-9);
        assert!((groove.depth - 0.9).abs() < 1e-9);
    }

    #[test]
    fn a_frame_is_a_closed_rectangle() {
        let frame = CarveGroove::frame(0.0, 0.0, 10.0, 4.0, 1.0);

        assert!(frame.closed);
        assert_eq!(frame.points.len(), 4);
    }

    #[test]
    fn the_bounds_include_half_the_groove_width() {
        let bounds = CarveGroove::line((10.0, 20.0), (30.0, 20.0), 4.0).bounds();

        assert!((bounds.left - 8.0).abs() < 1e-9);
        assert!((bounds.right - 32.0).abs() < 1e-9);
        assert!((bounds.height() - 4.0).abs() < 1e-9);
    }
}
