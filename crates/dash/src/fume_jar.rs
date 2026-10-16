use crate::{FumeFate, FumePolygon, JarDrain, JarProfile};

#[derive(Debug, Clone)]
pub struct FumeJar {
    pub fate: FumeFate,
    pub centre: f64,
    pub hang: f64,
    pub height: f64,
    pub width: f64,
    collected: usize,
    held: usize,
    fullness: f64,
    clock: f64,
    drain: Option<JarDrain>,
}

impl FumeJar {
    pub const ASPECT: f64 = 0.649;
    const PROFILE: JarProfile = JarProfile::FLASK;
    const OUTLINE_STEPS: usize = 48;
    const HALF_FULL: f64 = 40.0;
    const MIX_SECONDS: f64 = 1.6;
    const WORTH_DRAINING: f64 = 0.02;

    #[must_use]
    pub const fn new(fate: FumeFate, centre: f64, hang: f64, height: f64, width: f64) -> Self {
        Self { fate, centre, hang, height, width, collected: 0, held: 0, fullness: 0.0, clock: 0.0, drain: None }
    }

    #[must_use]
    pub fn placed(&self, centre: f64, hang: f64, height: f64, width: f64) -> Self {
        Self { centre, hang, height, width, ..self.clone() }
    }

    #[must_use]
    pub const fn top(&self) -> f64 {
        self.hang
    }

    #[must_use]
    pub fn left(&self) -> f64 {
        self.centre - self.width * 0.5
    }

    #[must_use]
    pub fn mouth(&self) -> (f64, f64) {
        (self.centre, self.hang + self.height)
    }

    #[must_use]
    pub fn reach(&self, rise: f64) -> f64 {
        Self::PROFILE.reach(rise)
    }

    #[must_use]
    pub fn outline(&self) -> FumePolygon {
        let half = self.width * 0.5;
        let point = |sign: f64, step: usize| {
            let rise = step as f64 / Self::OUTLINE_STEPS as f64;
            (self.centre + sign * self.reach(rise) * half, self.hang + rise * self.height)
        };
        let right = (0..=Self::OUTLINE_STEPS).map(|step| point(1.0, step));
        let left = (0..=Self::OUTLINE_STEPS).rev().map(|step| point(-1.0, step));
        FumePolygon::new(right.chain(left).collect())
    }

    pub fn collect(&mut self) {
        self.collected += 1;
        self.held += 1;
    }

    pub fn advance(&mut self, seconds: f64) {
        let seconds = seconds.max(0.0);
        self.clock += seconds;
        let target = 1.0 - (-(self.held as f64) / Self::HALF_FULL).exp();
        self.fullness += (target - self.fullness) * (1.0 - (-seconds / Self::MIX_SECONDS).exp());
        if self.drain.is_some_and(|drain| !drain.running(self.clock)) {
            self.drain = None;
        }
    }

    pub fn harvest(&mut self, delay: f64) -> Option<JarDrain> {
        let amount = self.fullness();
        if self.drain.is_some() || amount < Self::WORTH_DRAINING {
            return None;
        }
        let drain = JarDrain::new(self.clock + delay.max(0.0), amount);
        self.drain = Some(drain);
        self.held = 0;
        self.fullness = 0.0;
        Some(drain)
    }

    #[must_use]
    pub fn pull(&self) -> f64 {
        self.drain.map_or(0.0, |drain| drain.pull(self.clock))
    }

    #[must_use]
    pub fn charge(&self) -> f64 {
        (self.fullness + self.drain.map_or(0.0, |drain| drain.amount)).min(1.0)
    }

    #[must_use]
    pub fn emptied(&self) -> f64 {
        self.drain.map_or(0.0, |drain| drain.drained(self.clock) / drain.amount)
    }

    #[must_use]
    pub const fn collected(&self) -> usize {
        self.collected
    }

    #[must_use]
    pub fn fullness(&self) -> f64 {
        (self.fullness + self.drain.map_or(0.0, |drain| drain.left(self.clock))).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::FumeJar;
    use crate::FumeFate;

    fn jar() -> FumeJar {
        FumeJar::new(FumeFate::Voting, 100.0, 100.0, 200.0, 112.0)
    }

    fn settle(jar: &mut FumeJar, seconds: f64) {
        for _ in 0..(seconds * 60.0) as usize {
            jar.advance(1.0 / 60.0);
        }
    }

    #[test]
    fn the_jar_hangs_with_its_foot_up_and_its_mouth_down() {
        let jar = jar();
        let outline = jar.outline();

        assert!(outline.contains(100.0, 200.0));
        assert!(outline.contains(150.0, 174.0));
        assert!(!outline.contains(115.0, 272.0));
        assert!(outline.contains(110.0, 292.0));
        assert!((outline.top() - 100.0).abs() < 1e-9);
        assert!((outline.bottom() - 300.0).abs() < 1e-9);
        assert_eq!(jar.mouth(), (100.0, 300.0));
    }

    #[test]
    fn a_fume_mixes_in_over_a_second_or_two_rather_than_at_once() {
        let mut jar = jar();
        jar.collect();
        jar.advance(1.0 / 60.0);
        let first = jar.fullness();
        settle(&mut jar, 0.5);
        let half_second = jar.fullness();
        settle(&mut jar, 8.0);

        assert!(first > 0.0 && first < half_second);
        assert!(half_second < jar.fullness() * 0.5);
    }

    #[test]
    fn more_fumes_make_denser_gas_that_never_passes_full() {
        let mut few = jar();
        let mut many = jar();
        for _ in 0..5 {
            few.collect();
        }
        for _ in 0..500 {
            many.collect();
        }
        settle(&mut few, 20.0);
        settle(&mut many, 20.0);

        assert!(few.fullness() > 0.05);
        assert!(many.fullness() > few.fullness() * 3.0);
        assert!(many.fullness() <= 1.0);
    }

    #[test]
    fn a_harvest_drains_the_gas_over_a_few_seconds_and_keeps_the_count() {
        let mut jar = jar();
        for _ in 0..60 {
            jar.collect();
        }
        settle(&mut jar, 10.0);
        let full = jar.fullness();
        let drain = jar.harvest(0.5).expect("drains");

        assert!((drain.amount - full).abs() < 1e-12);
        assert!((jar.fullness() - full).abs() < 1e-12);
        settle(&mut jar, 0.5 + drain.seconds * 0.5);
        assert!(jar.fullness() < full * 0.6 && jar.fullness() > full * 0.4);
        assert!(jar.pull() > 0.9 && jar.emptied() > 0.4);
        settle(&mut jar, drain.seconds);
        assert!(jar.fullness() < 1e-9);
        assert!(jar.pull().abs() < 1e-12);
        assert_eq!(jar.collected(), 60);
    }

    #[test]
    fn fumes_that_land_while_draining_stay_behind() {
        let mut jar = jar();
        for _ in 0..60 {
            jar.collect();
        }
        settle(&mut jar, 10.0);
        jar.harvest(0.0);
        for _ in 0..5 {
            jar.collect();
        }
        settle(&mut jar, 8.0);

        assert!(jar.fullness() > 0.05);
        assert!(jar.harvest(0.0).is_some());
    }

    #[test]
    fn an_empty_or_draining_jar_is_not_harvested_again() {
        let mut jar = jar();
        assert!(jar.harvest(0.0).is_none());
        for _ in 0..60 {
            jar.collect();
        }
        settle(&mut jar, 10.0);

        assert!(jar.harvest(0.0).is_some());
        assert!(jar.harvest(0.0).is_none());
    }

    #[test]
    fn moving_a_jar_keeps_what_it_collected() {
        let mut jar = jar();
        jar.collect();
        settle(&mut jar, 2.0);
        let moved = jar.placed(10.0, 20.0, 30.0, 40.0);

        assert_eq!(moved.collected(), 1);
        assert!((moved.fullness() - jar.fullness()).abs() < 1e-12);
    }
}
