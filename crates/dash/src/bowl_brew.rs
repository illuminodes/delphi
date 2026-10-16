use crate::{FumePuff, HexColor, ValueNoise};

#[derive(Debug, Clone)]
pub struct BowlBrew {
    charge: f64,
    received: f64,
    peak: f64,
    colour: HexColor,
    last_pour: f64,
    clock: f64,
    noise: ValueNoise,
}

impl BowlBrew {
    const HOLD: f64 = 1.2;
    const FADE: f64 = 7.0;
    const FLARE: f64 = 0.9;
    const EMPTY: f64 = 0.002;
    const SMOKE_EVERY: f64 = 0.06;
    const SMOKE_LIFE: f64 = 3.2;
    const SMOKE_DENSITY: f64 = 0.12;
    const SMOKE_LIFT: f64 = 2.6;
    const SMOKE_GROWTH: f64 = 2.0;
    const SMOKE_SWAY: f64 = 1.4;

    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { charge: 0.0, received: 0.0, peak: 0.0, colour: HexColor::rgb([0, 0, 0]), last_pour: f64::NEG_INFINITY, clock: 0.0, noise: ValueNoise::new(seed) }
    }

    pub fn pour(&mut self, colour: HexColor, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        let share = amount / (self.charge + amount);
        self.colour = self.colour.mix(colour, share);
        self.charge += amount;
        self.received += amount;
        self.peak = self.peak.max(self.charge);
        self.last_pour = self.clock;
    }

    pub fn advance(&mut self, seconds: f64) {
        let seconds = seconds.max(0.0);
        self.clock += seconds;
        if self.clock - self.last_pour > Self::HOLD {
            self.charge *= (-seconds / Self::FADE).exp();
        }
        if self.charge < Self::EMPTY {
            self.charge = 0.0;
            self.peak = 0.0;
        }
    }

    #[must_use]
    pub const fn charge(&self) -> f64 {
        self.charge
    }

    #[must_use]
    pub const fn received(&self) -> f64 {
        self.received
    }

    #[must_use]
    pub const fn colour(&self) -> HexColor {
        self.colour
    }

    #[must_use]
    pub fn flare(&self) -> f64 {
        let since = self.clock - self.last_pour;
        if since < 0.0 || self.charge <= 0.0 {
            return 0.0;
        }
        let rise = (since / 0.25).min(1.0);
        rise * (-(since / Self::FLARE).powi(2)).exp()
    }

    #[must_use]
    pub fn glow(&self) -> f64 {
        (self.charge.min(1.5) / 1.5) * (0.5 + 0.5 * self.flare()) + 0.5 * self.flare() * self.charge.min(1.0)
    }

    #[must_use]
    pub fn smoke(&self, (x, y): (f64, f64), spread: f64, unit: f64) -> Vec<FumePuff> {
        if self.charge <= 0.0 {
            return Vec::new();
        }
        let newest = (self.clock / Self::SMOKE_EVERY).floor() as i64;
        let count = (Self::SMOKE_LIFE / Self::SMOKE_EVERY).ceil() as i64;
        let width = 9.0 * unit;
        (newest - count..=newest)
            .filter_map(|index| {
                let emitted = index as f64 * Self::SMOKE_EVERY;
                let age = self.clock - emitted;
                if !(0.0..Self::SMOKE_LIFE).contains(&age) {
                    return None;
                }
                let then = (self.charge * (age / Self::FADE).exp()).min(self.peak);
                let life = age / Self::SMOKE_LIFE;
                let start = self.noise.sample(index as f64 * 0.77, 4.4) * spread * 0.7;
                let sway = self.noise.fractal(emitted * 0.6, age * 0.8 + 11.0, 2) * width * Self::SMOKE_SWAY * age;
                let alpha = Self::SMOKE_DENSITY * then.min(1.0) * (age / 0.3).min(1.0) * (1.0 - life).powf(1.5);
                Some(FumePuff {
                    x: x + start * (1.0 - life * 0.6) + sway,
                    y: y - width * Self::SMOKE_LIFT * age * (1.0 + 0.3 * life),
                    radius: width * (1.0 + Self::SMOKE_GROWTH * age),
                    alpha,
                    colour: self.colour,
                })
            })
            .filter(FumePuff::visible)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::BowlBrew;
    use crate::{FumePuff, HexColor};

    const TEAL: HexColor = HexColor::rgb([90, 171, 163]);
    const GOLD: HexColor = HexColor::rgb([201, 162, 74]);

    fn run(brew: &mut BowlBrew, seconds: f64) {
        for _ in 0..(seconds * 60.0) as usize {
            brew.advance(1.0 / 60.0);
        }
    }

    #[test]
    fn an_untouched_bowl_is_empty_and_smokeless() {
        let mut brew = BowlBrew::new(1);
        run(&mut brew, 2.0);

        assert!(brew.charge().abs() < 1e-12);
        assert!(brew.glow().abs() < 1e-12);
        assert!(brew.smoke((0.0, 0.0), 40.0, 1.0).is_empty());
    }

    #[test]
    fn pouring_mixes_the_colours_by_how_much_of_each_arrived() {
        let mut brew = BowlBrew::new(1);
        brew.pour(TEAL, 0.6);
        brew.pour(GOLD, 0.2);

        assert!((brew.charge() - 0.8).abs() < 1e-12);
        assert_eq!(brew.colour(), TEAL.mix(GOLD, 0.25));
    }

    #[test]
    fn the_bowl_flares_when_the_pouring_stops_then_burns_down() {
        let mut brew = BowlBrew::new(1);
        brew.pour(TEAL, 0.8);
        let poured = brew.glow();
        run(&mut brew, 0.4);
        let flared = brew.glow();
        run(&mut brew, 4.0);
        let later = brew.glow();
        run(&mut brew, 60.0);

        assert!(flared > poured * 1.3, "{poured} {flared}");
        assert!(later < flared * 0.6);
        assert!(brew.charge().abs() < 1e-12);
    }

    #[test]
    fn the_charge_holds_while_gas_keeps_arriving() {
        let mut brew = BowlBrew::new(1);
        for _ in 0..120 {
            brew.pour(TEAL, 0.005);
            brew.advance(1.0 / 60.0);
        }

        assert!((brew.charge() - 0.6).abs() < 1e-9);
    }

    #[test]
    fn smoke_rises_from_the_bowl_and_spreads_as_it_goes() {
        let mut brew = BowlBrew::new(1);
        brew.pour(GOLD, 1.0);
        run(&mut brew, 3.0);
        let smoke = brew.smoke((500.0, 400.0), 40.0, 1.0);
        let (young, old): (Vec<FumePuff>, Vec<FumePuff>) = smoke.iter().partition(|puff| puff.y > 360.0);

        assert!(smoke.len() > 20);
        assert!(smoke.iter().all(|puff| puff.y <= 400.0 + 1e-9));
        assert!(!young.is_empty() && !old.is_empty());
        assert!(old.iter().map(|puff| puff.radius).sum::<f64>() / old.len() as f64 > young.iter().map(|puff| puff.radius).sum::<f64>() / young.len() as f64);
        assert!(smoke.iter().all(|puff| puff.colour == GOLD));
    }
}
