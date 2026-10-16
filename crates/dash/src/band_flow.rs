use crate::VeinCourse;

#[derive(Debug, Clone, PartialEq)]
pub struct BandFlow {
    ladder: Vec<(VeinCourse, f64)>,
}

impl BandFlow {
    const RUNGS: [(usize, f64); 5] = [(1, 0.0), (5, 14.0), (16, 26.0), (45, 48.0), (140, 100.0)];
    const SLACK: f64 = 1.4;

    #[must_use]
    pub fn new(points: &[(f64, f64)], width: f64) -> Option<Self> {
        let ladder = Self::RUNGS
            .iter()
            .map(|(radius, reach)| VeinCourse::new(points, width, *radius).map(|course| (course, *reach)))
            .collect::<Option<Vec<_>>>()?;
        Some(Self { ladder })
    }

    #[must_use]
    pub fn offset(&self, x: f64, y: f64) -> f64 {
        let line = self.ladder[0].0.trend(x);
        let distance = y - line;
        let centre = self.ladder[1..].iter().fold(line, |centre, (course, reach)| {
            let gap = course.trend(x) - centre;
            let spread = reach + Self::SLACK * gap.abs();
            let across = distance / spread;
            centre + gap * (1.0 - (-across * across).exp())
        });
        y - centre
    }
}

#[cfg(test)]
mod tests {
    use super::BandFlow;

    struct FlowFixture;

    impl FlowFixture {
        fn spike() -> BandFlow {
            BandFlow::new(&[(0.0, 150.0), (170.0, 150.0), (200.0, 60.0), (230.0, 150.0), (400.0, 150.0)], 400.0).unwrap()
        }
    }

    #[test]
    fn the_band_through_the_price_line_is_the_centre_band() {
        let flow = FlowFixture::spike();

        assert!(flow.offset(60.0, 150.0).abs() < 0.5);
        assert!(flow.offset(200.0, 60.0).abs() < 4.0, "{}", flow.offset(200.0, 60.0));
    }

    #[test]
    fn bands_beside_the_line_ride_the_spike_and_far_bands_barely_move() {
        let flow = FlowFixture::spike();
        let beside_spike = flow.offset(200.0, 54.0);
        let beside_flat = flow.offset(60.0, 144.0);
        let far_shift = (flow.offset(200.0, 450.0) - flow.offset(60.0, 450.0)).abs();

        assert!((beside_spike - beside_flat).abs() < 8.0, "{beside_spike} {beside_flat}");
        assert!(far_shift < 90.0 / 4.0, "{far_shift}");
    }

    #[test]
    fn the_bending_fades_with_distance_from_the_line() {
        let flow = FlowFixture::spike();
        let shift = |below: f64| (flow.offset(200.0, 150.0 + below) - flow.offset(60.0, 150.0 + below)).abs();

        assert!(shift(20.0) > shift(120.0) && shift(120.0) > shift(300.0), "{} {} {}", shift(20.0), shift(120.0), shift(300.0));
    }

    #[test]
    fn the_bands_never_fold_over_each_other() {
        let flow = FlowFixture::spike();
        for x in (0..400).step_by(5).map(f64::from) {
            let offsets: Vec<f64> = (-100..500).map(|y| flow.offset(x, f64::from(y))).collect();
            assert!(offsets.windows(2).all(|pair| pair[1] > pair[0]), "fold at x {x}");
        }
    }

    #[test]
    fn the_flow_changes_smoothly_from_pixel_to_pixel() {
        let flow = FlowFixture::spike();
        let steepest = (0..400)
            .flat_map(|x| (0..300).step_by(3).map(move |y| (f64::from(x), f64::from(y))))
            .map(|(x, y)| (flow.offset(x + 1.0, y) - flow.offset(x, y)).abs())
            .fold(0.0, f64::max);

        assert!(steepest < 6.0, "{steepest}");
    }
}
