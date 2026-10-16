use crate::{FumeFate, FumePuff, FumeRoute, SplitMix, ValueNoise};

#[derive(Debug, Clone)]
pub struct FumeWisp {
    pub fate: FumeFate,
    route: FumeRoute,
    born: f64,
    travel: f64,
    break_at: f64,
    width: f64,
    noise: ValueNoise,
}

impl FumeWisp {
    const PARCELS: usize = 96;
    const LINGER: f64 = 1.7;
    const SPEED: f64 = 190.0;
    const BREAK_SECONDS: f64 = 2.0;
    const DENSITY: f64 = 0.2;
    const RIPEN: f64 = 0.2;
    const GROWTH: f64 = 2.2;
    const LIFT: f64 = 2.6;
    const CURL_CELL: f64 = 10.0;
    const CURL_DRIFT: f64 = 0.45;
    const CURL: f64 = 5.0;
    const FRAY: f64 = 2.2;
    const BURST: f64 = 9.0;

    #[must_use]
    pub fn new(fate: FumeFate, route: FumeRoute, born: f64, unit: f64, seed: u64) -> Self {
        let mut random = SplitMix::new(seed);
        let travel = route.length() / (Self::SPEED * unit * (0.85 + 0.3 * random.unit()));
        let break_at = if fate == FumeFate::Scattered { 0.4 + 0.2 * random.unit() } else { 1.0 };
        Self { fate, route, born, travel: travel.max(0.5), break_at, width: 9.0 * unit, noise: ValueNoise::new(seed ^ 0x5eed) }
    }

    #[must_use]
    pub fn head(&self, now: f64) -> f64 {
        ((now - self.born) / self.travel).max(0.0)
    }

    #[must_use]
    pub fn arrived(&self, now: f64) -> bool {
        self.fate.jar().is_some() && self.head(now) >= 1.0
    }

    #[must_use]
    pub fn broken_for(&self, now: f64) -> Option<f64> {
        (self.fate == FumeFate::Scattered && self.head(now) >= self.break_at).then(|| (self.head(now) - self.break_at) * self.travel)
    }

    #[must_use]
    pub fn finished(&self, now: f64) -> bool {
        match self.broken_for(now) {
            Some(since) => since >= Self::BREAK_SECONDS,
            None => now - self.born >= self.travel + Self::LINGER,
        }
    }

    #[must_use]
    pub fn puffs(&self, now: f64) -> Vec<FumePuff> {
        let last_emission = self.travel * self.break_at.min(1.0);
        let burst = self.broken_for(now).map(|since| since / Self::BREAK_SECONDS);
        (0..Self::PARCELS)
            .filter_map(|index| {
                let age = index as f64 / (Self::PARCELS - 1) as f64 * Self::LINGER;
                let emitted = now - self.born - age;
                (0.0..=last_emission).contains(&emitted).then(|| self.parcel(now, emitted, age, index, burst))
            })
            .filter(FumePuff::visible)
            .collect()
    }

    fn parcel(&self, now: f64, emitted: f64, age: f64, index: usize, burst: Option<f64>) -> FumePuff {
        let along = emitted / self.travel;
        let (start_x, start_y) = self.route.at(along);
        let (width, cell) = (self.width, self.width * Self::CURL_CELL);
        let swirl = self.noise.fractal(start_x / cell + 3.7, start_y / cell - now * Self::CURL_DRIFT, 2);
        let own = (self.noise.sample(emitted * 2.3, 3.1), self.noise.sample(emitted * 2.3, 9.4));
        let strand = if index.is_multiple_of(2) { 1.0 } else { -1.0 } * self.noise.sample(emitted * 1.1, 20.0).abs();
        let mut x = start_x + width * (swirl * Self::CURL * age.sqrt() + (own.0 + strand) * Self::FRAY * age);
        let mut y = start_y - width * (Self::LIFT * age + own.1 * Self::FRAY * 0.5 * age);
        let mut radius = width * (0.8 + along.min(1.0) * 0.5) * (1.0 + Self::GROWTH * age);
        let mut alpha = Self::DENSITY * (age / Self::RIPEN).min(1.0) * (1.0 - age / Self::LINGER).powf(1.5) * (along * 25.0).min(1.0);
        if let Some(spread) = burst {
            let heading = index as f64 * 2.399_963;
            x += heading.cos() * width * Self::BURST * spread.sqrt();
            y -= (0.4 + heading.sin().abs()) * width * Self::BURST * 0.6 * spread;
            radius *= 1.0 + spread * 2.5;
            alpha *= (1.0 - spread).max(0.0).powi(2);
        }
        if self.fate.jar().is_some() {
            alpha *= ((1.0 - along) * 12.0).clamp(0.0, 1.0);
        }
        FumePuff { x, y, radius, alpha, colour: self.fate.colour() }
    }
}

#[cfg(test)]
mod tests {
    use super::FumeWisp;
    use crate::{FumeFate, FumePuff, FumeRoute};

    fn wisp(fate: FumeFate) -> FumeWisp {
        FumeWisp::new(fate, FumeRoute::through(&[(0.0, 600.0), (20.0, 300.0), (0.0, 0.0)]), 0.0, 1.0, 11)
    }

    fn spread(puffs: &[FumePuff]) -> f64 {
        let mean = puffs.iter().map(|puff| puff.x).sum::<f64>() / puffs.len().max(1) as f64;
        (puffs.iter().map(|puff| (puff.x - mean).powi(2)).sum::<f64>() / puffs.len().max(1) as f64).sqrt()
    }

    #[test]
    fn an_accepted_wisp_arrives_then_finishes_and_a_scattered_one_never_arrives() {
        let accepted = wisp(FumeFate::Voting);
        let scattered = wisp(FumeFate::Scattered);
        let times: Vec<f64> = (0..2000).map(|step| f64::from(step) * 0.01).collect();
        let arrival = times.iter().position(|now| accepted.arrived(*now)).expect("arrives");
        let finish = times.iter().position(|now| accepted.finished(*now)).expect("finishes");

        assert!(finish > arrival);
        assert!(times.iter().all(|now| !scattered.arrived(*now)));
        assert!(times.iter().any(|now| scattered.finished(*now)));
    }

    #[test]
    fn a_scattered_wisp_breaks_up_before_it_is_half_way_to_the_top_of_its_route() {
        let scattered = wisp(FumeFate::Scattered);
        let highest = (0..2000)
            .flat_map(|step| scattered.puffs(f64::from(step) * 0.01))
            .filter(|puff| puff.alpha > 0.1)
            .map(|puff| puff.y)
            .fold(f64::INFINITY, f64::min);

        assert!(highest > 150.0, "{highest}");
    }

    #[test]
    fn a_scattered_wisp_spreads_out_and_fades_once_it_breaks() {
        let scattered = wisp(FumeFate::Scattered);
        let start = (0..2000).map(|step| f64::from(step) * 0.01).find(|now| scattered.broken_for(*now).is_some()).expect("breaks");
        let alpha = |now: f64| scattered.puffs(now).iter().map(|puff| puff.alpha).sum::<f64>();

        assert!(spread(&scattered.puffs(start + 1.0)) > spread(&scattered.puffs(start)) * 1.5);
        assert!(alpha(start + 1.5) < alpha(start) * 0.3);
    }

    #[test]
    fn a_wisp_starts_as_a_faint_thin_tip_and_billows_out_behind() {
        let accepted = wisp(FumeFate::Sampled);
        let puffs = accepted.puffs(accepted.travel * 0.7);
        let (tip, tail) = (puffs.first().expect("puffs"), puffs.last().expect("puffs"));
        let densest = puffs.iter().map(|puff| puff.alpha).fold(0.0, f64::max);
        let (young, old) = puffs.split_at(puffs.len() / 2);

        assert!(tip.alpha < densest * 0.4 && tail.alpha < densest * 0.4);
        assert!(tail.radius > tip.radius * 3.0);
        assert!(spread(old) > spread(young) * 1.5, "{} {}", spread(old), spread(young));
    }

    #[test]
    fn smoke_left_behind_drifts_upward_and_keeps_curling_once_emitted() {
        let accepted = wisp(FumeFate::Sampled);
        let now = accepted.travel * 0.5;
        let earlier = accepted.puffs(now);
        let later = accepted.puffs(now + 0.4);
        let tail = |puffs: &[FumePuff]| puffs.last().map(|puff| (puff.x, puff.y)).expect("puffs");

        assert!(tail(&later).1 < tail(&earlier).1);
        assert!(earlier.iter().zip(&later).any(|(one, two)| (one.x - two.x).abs() > 1.0));
    }

    #[test]
    fn nothing_is_drawn_before_birth_and_puffs_rise_as_the_wisp_travels() {
        let accepted = wisp(FumeFate::Sampled);
        let top = |now: f64| accepted.puffs(now).iter().map(|puff| puff.y).fold(f64::INFINITY, f64::min);

        assert!(accepted.puffs(-1.0).is_empty());
        assert!(top(2.0) < top(1.0));
    }
}
