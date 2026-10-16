use std::f64::consts::PI;

use crate::{FumePuff, FumeRoute, HexColor, JarDrain, ValueNoise};

#[derive(Debug, Clone)]
pub struct FumeStream {
    pub colour: HexColor,
    route: FumeRoute,
    drain: JarDrain,
    travel: f64,
    width: f64,
    noise: ValueNoise,
}

impl FumeStream {
    const PARCELS: usize = 150;
    const SPEED: f64 = 230.0;
    const SETTLE: f64 = 0.35;
    const DENSITY: f64 = 0.2;
    const SWELL: f64 = 1.3;
    const BODY: f64 = 13.0;
    const SWAY: f64 = 2.4;
    const SWAY_CELL: f64 = 0.012;
    const SINK: f64 = 0.5;

    #[must_use]
    pub fn new(colour: HexColor, route: FumeRoute, drain: JarDrain, unit: f64, seed: u64) -> Self {
        let travel = (route.length() / (Self::SPEED * unit)).max(0.4);
        Self { colour, route, drain, travel, width: Self::BODY * unit, noise: ValueNoise::new(seed) }
    }

    #[must_use]
    pub fn rerouted(&self, route: FumeRoute, unit: f64) -> Self {
        let travel = (route.length() / (Self::SPEED * unit)).max(0.4);
        Self { route, travel, width: Self::BODY * unit, ..self.clone() }
    }

    #[must_use]
    pub const fn amount(&self) -> f64 {
        self.drain.amount
    }

    #[must_use]
    pub fn landed(&self, now: f64) -> f64 {
        self.drain.drained(now - self.travel)
    }

    #[must_use]
    pub fn last_landing(&self) -> f64 {
        self.drain.ends() + self.travel
    }

    #[must_use]
    pub fn end(&self) -> (f64, f64) {
        self.route.at(1.0)
    }

    #[must_use]
    pub fn puffs(&self, now: f64) -> Vec<FumePuff> {
        let peak = self.drain.amount.max(1e-9);
        (0..Self::PARCELS)
            .filter_map(|index| {
                let emitted = self.drain.started + self.drain.seconds * index as f64 / (Self::PARCELS - 1) as f64;
                let age = now - emitted;
                (age >= 0.0 && age <= self.travel + Self::SETTLE).then(|| self.parcel(age, emitted, self.drain.flow(emitted) / peak, index))
            })
            .filter(FumePuff::visible)
            .collect()
    }

    fn parcel(&self, age: f64, emitted: f64, flow: f64, index: usize) -> FumePuff {
        let fall = (age / self.travel).min(1.0);
        let along = fall * (0.55 + 0.45 * fall);
        let (x, y) = self.route.at(along);
        let settled = ((age - self.travel) / Self::SETTLE).max(0.0);
        let sway = self.noise.fractal(x * Self::SWAY_CELL + emitted * 0.9, y * Self::SWAY_CELL + index as f64 * 0.01, 2);
        let bulge = (PI * along).sin();
        let radius = self.width * (0.6 + Self::SWELL * along) * (0.6 + 0.4 * flow);
        let alpha = Self::DENSITY * (0.45 + 0.55 * self.drain.amount.min(1.0)) * flow * (age / 0.12).min(1.0) * (1.0 - settled).max(0.0).powi(2);
        FumePuff { x: x + sway * self.width * Self::SWAY * bulge, y: y + settled * self.width * Self::SINK, radius, alpha, colour: self.colour }
    }
}

#[cfg(test)]
mod tests {
    use super::FumeStream;
    use crate::{FumeRoute, HexColor, JarDrain};

    fn stream() -> FumeStream {
        let route = FumeRoute::bezier([(0.0, 0.0), (0.0, 120.0), (300.0, 180.0), (300.0, 300.0)]);
        FumeStream::new(HexColor::rgb([90, 171, 163]), route, JarDrain::new(1.0, 0.8), 1.0, 3)
    }

    #[test]
    fn nothing_pours_before_the_drain_starts_and_everything_lands_in_the_end() {
        let stream = stream();

        assert!(stream.puffs(0.9).is_empty());
        assert!(stream.landed(1.5).abs() < 1e-12);
        assert!((stream.landed(stream.last_landing()) - 0.8).abs() < 1e-9);
        assert!(stream.puffs(stream.last_landing() + 1.0).is_empty());
    }

    #[test]
    fn the_stream_leaves_the_mouth_and_reaches_the_bowl_as_one_ribbon() {
        let stream = stream();
        let puffs = stream.puffs(1.0 + 2.2);
        let (highest, lowest) = puffs.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(high, low), puff| (high.min(puff.y), low.max(puff.y)));
        let mut heights: Vec<f64> = puffs.iter().map(|puff| puff.y).collect();
        heights.sort_by(f64::total_cmp);
        let widest_gap = heights.windows(2).map(|pair| pair[1] - pair[0]).fold(0.0, f64::max);

        assert!(highest < 30.0, "{highest}");
        assert!(lowest > 270.0, "{lowest}");
        assert!(widest_gap < 25.0, "{widest_gap}");
    }

    #[test]
    fn the_stream_is_thickest_in_the_middle_of_the_drain() {
        let stream = stream();
        let mouth_density = |now: f64| stream.puffs(now).iter().filter(|puff| puff.y < 40.0).map(|puff| puff.alpha).sum::<f64>();

        assert!(mouth_density(2.5) > mouth_density(1.15) * 2.0);
        assert!(mouth_density(2.5) > mouth_density(3.9) * 2.0);
    }

    #[test]
    fn the_gas_falls_faster_as_it_goes() {
        let stream = stream();
        let first = stream.puffs(1.0 + 0.3).iter().map(|puff| puff.y).fold(f64::NEG_INFINITY, f64::max);
        let later = stream.puffs(1.0 + 0.6).iter().map(|puff| puff.y).fold(f64::NEG_INFINITY, f64::max);

        assert!(later - first > first);
    }
}
