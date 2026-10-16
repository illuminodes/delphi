#[derive(Debug, Clone, PartialEq)]
pub struct FumeRoute {
    samples: Vec<(f64, f64)>,
    lengths: Vec<f64>,
}

impl FumeRoute {
    const STEPS: usize = 24;

    const BEZIER_STEPS: usize = 256;

    #[must_use]
    pub fn bezier(controls: [(f64, f64); 4]) -> Self {
        let point = |t: f64| {
            let u = 1.0 - t;
            let weights = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
            controls.iter().zip(weights).fold((0.0, 0.0), |sum, (control, weight)| (sum.0 + control.0 * weight, sum.1 + control.1 * weight))
        };
        Self::sampled((0..=Self::BEZIER_STEPS).map(|step| point(step as f64 / Self::BEZIER_STEPS as f64)).collect())
    }

    #[must_use]
    pub fn then_to(self, end: (f64, f64)) -> Self {
        let mut samples = self.samples;
        samples.push(end);
        Self::sampled(samples)
    }

    #[must_use]
    pub fn through(waypoints: &[(f64, f64)]) -> Self {
        let mut samples = Vec::new();
        for segment in 0..waypoints.len().saturating_sub(1) {
            let at = |index: isize| waypoints[index.clamp(0, waypoints.len() as isize - 1) as usize];
            let index = segment as isize;
            let (before, from, to, after) = (at(index - 1), at(index), at(index + 1), at(index + 2));
            for step in 0..Self::STEPS {
                samples.push(Self::catmull(before, from, to, after, step as f64 / Self::STEPS as f64));
            }
        }
        if let Some(last) = waypoints.last() {
            samples.push(*last);
        }
        Self::sampled(samples)
    }

    fn sampled(samples: Vec<(f64, f64)>) -> Self {
        let mut lengths = vec![0.0];
        for pair in samples.windows(2) {
            let run = (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1);
            lengths.push(lengths.last().copied().unwrap_or(0.0) + run);
        }
        Self { samples, lengths }
    }

    #[must_use]
    pub fn length(&self) -> f64 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    #[must_use]
    pub fn at(&self, fraction: f64) -> (f64, f64) {
        let Some(first) = self.samples.first().copied() else {
            return (0.0, 0.0);
        };
        let wanted = fraction.clamp(0.0, 1.0) * self.length();
        let after = self.lengths.partition_point(|length| *length < wanted);
        if after == 0 {
            return first;
        }
        if after >= self.samples.len() {
            return self.samples[self.samples.len() - 1];
        }
        let (from, to) = (self.samples[after - 1], self.samples[after]);
        let span = self.lengths[after] - self.lengths[after - 1];
        let amount = if span > 0.0 { (wanted - self.lengths[after - 1]) / span } else { 0.0 };
        (from.0 + (to.0 - from.0) * amount, from.1 + (to.1 - from.1) * amount)
    }

    fn catmull(before: (f64, f64), from: (f64, f64), to: (f64, f64), after: (f64, f64), t: f64) -> (f64, f64) {
        let blend = |a: f64, b: f64, c: f64, d: f64| {
            0.5 * ((2.0 * b) + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (3.0 * b - a - 3.0 * c + d) * t * t * t)
        };
        (blend(before.0, from.0, to.0, after.0), blend(before.1, from.1, to.1, after.1))
    }
}

#[cfg(test)]
mod tests {
    use super::FumeRoute;

    #[test]
    fn the_route_starts_and_ends_at_its_waypoints_and_passes_through_the_middle_ones() {
        let route = FumeRoute::through(&[(0.0, 100.0), (30.0, 50.0), (10.0, 0.0)]);
        let near = |point: (f64, f64), x: f64, y: f64| (point.0 - x).hypot(point.1 - y) < 1e-6;

        assert!(near(route.at(0.0), 0.0, 100.0));
        assert!(near(route.at(1.0), 10.0, 0.0));
        assert!((0..=1000).map(|step| route.at(f64::from(step) / 1000.0)).any(|point| (point.0 - 30.0).hypot(point.1 - 50.0) < 0.5));
    }

    #[test]
    fn equal_fractions_cover_equal_distances() {
        let route = FumeRoute::through(&[(0.0, 0.0), (100.0, 0.0), (100.0, 10.0)]);
        let steps: Vec<f64> = (0..10)
            .map(|step| {
                let (one, two) = (route.at(f64::from(step) / 10.0), route.at(f64::from(step + 1) / 10.0));
                (two.0 - one.0).hypot(two.1 - one.1)
            })
            .collect();

        assert!(steps.iter().all(|step| (step - route.length() / 10.0).abs() < route.length() * 0.02), "{steps:?}");
    }

    #[test]
    fn a_bezier_starts_and_ends_at_its_end_controls_and_leaves_along_its_first_handle() {
        let route = FumeRoute::bezier([(0.0, 100.0), (0.0, 50.0), (80.0, 50.0), (80.0, 0.0)]);
        let start = route.at(0.01);

        assert_eq!(route.at(0.0), (0.0, 100.0));
        assert!((route.at(1.0).0 - 80.0).abs() < 1e-9 && route.at(1.0).1.abs() < 1e-9);
        assert!(start.0.abs() < start.1 * 0.05 + 1e-9 || start.0.abs() < (100.0 - start.1) * 0.2);
    }

    #[test]
    fn a_route_can_carry_on_in_a_straight_line() {
        let route = FumeRoute::through(&[(0.0, 0.0), (0.0, 10.0)]).then_to((0.0, 30.0));

        assert!((route.length() - 30.0).abs() < 1e-9);
        assert!((route.at(0.5).1 - 15.0).abs() < 1e-9);
    }

    #[test]
    fn fractions_outside_the_route_clamp_to_its_ends() {
        let route = FumeRoute::through(&[(0.0, 0.0), (5.0, 5.0)]);

        assert_eq!(route.at(-1.0), route.at(0.0));
        assert_eq!(route.at(2.0), route.at(1.0));
    }
}
