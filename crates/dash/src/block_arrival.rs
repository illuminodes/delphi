#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockArrival {
    pub age: f64,
}

impl BlockArrival {
    const STRIKE: f64 = 1.2;
    const CROSS: f64 = 1.4;
    const SETTLE: f64 = 1.0;
    const CARVE: f64 = 4.0;
    const FADE: f64 = 2.0;

    #[must_use]
    pub const fn new() -> Self {
        Self { age: 0.0 }
    }

    #[must_use]
    pub const fn finished() -> Self {
        Self { age: Self::STRIKE + Self::CROSS + Self::SETTLE + Self::CARVE + Self::FADE }
    }

    pub fn advance(&mut self, step: f64) {
        self.age = (self.age + step.max(0.0)).min(Self::finished().age);
    }

    #[must_use]
    pub fn done(&self) -> bool {
        self.age >= Self::finished().age
    }

    fn phase(&self, start: f64, length: f64) -> f64 {
        ((self.age - start) / length).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn strike(&self) -> f64 {
        self.phase(0.0, Self::STRIKE)
    }

    #[must_use]
    pub fn cross(&self) -> f64 {
        let eased = self.phase(Self::STRIKE, Self::CROSS);
        eased * eased * (3.0 - 2.0 * eased)
    }

    #[must_use]
    pub fn settle(&self) -> f64 {
        let t = self.phase(Self::STRIKE + Self::CROSS * 0.6, Self::SETTLE + Self::CROSS * 0.4);
        1.0 - (1.0 - t).powi(3)
    }

    #[must_use]
    pub fn carve(&self) -> f64 {
        self.phase(Self::STRIKE + Self::CROSS + Self::SETTLE, Self::CARVE)
    }

    #[must_use]
    pub fn beam(&self) -> f64 {
        let rise = self.phase(0.0, Self::STRIKE * 0.5);
        let fall = self.phase(Self::STRIKE + Self::CROSS + Self::SETTLE + Self::CARVE, Self::FADE);
        rise * (1.0 - fall)
    }
}

impl Default for BlockArrival {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::BlockArrival;

    fn at(age: f64) -> BlockArrival {
        BlockArrival { age }
    }

    #[test]
    fn the_beam_strikes_the_tripod_before_it_crosses_to_the_wall() {
        assert!(at(0.6).strike() > 0.0 && at(0.6).cross().abs() < 1e-9);
        assert!(at(2.0).strike() >= 1.0 && at(2.0).cross() > 0.0 && at(2.0).cross() < 1.0);
    }

    #[test]
    fn the_stone_settles_before_the_letters_are_cut_and_the_beam_fades_last() {
        let settled = at(3.6);

        assert!(settled.settle() > 0.99 && settled.carve().abs() < 1e-9);
        assert!(at(5.6).carve() > 0.0 && at(5.6).beam() > 0.99);
        assert!(at(8.6).carve() >= 1.0 && at(8.6).beam() < 1.0 && at(8.6).beam() > 0.0);
        assert!(BlockArrival::finished().beam().abs() < 1e-9 && BlockArrival::finished().done());
    }

    #[test]
    fn every_phase_only_moves_forward() {
        let mut arrival = BlockArrival::new();
        let mut last = (0.0, 0.0, 0.0, 0.0);
        while !arrival.done() {
            arrival.advance(0.05);
            let now = (arrival.strike(), arrival.cross(), arrival.settle(), arrival.carve());
            assert!(now.0 >= last.0 && now.1 >= last.1 && now.2 >= last.2 && now.3 >= last.3);
            last = now;
        }
    }
}
