#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HarvestTimer {
    every: f64,
    until_next: f64,
}

impl HarvestTimer {
    pub const SIMULATED: Self = Self::new(40.0);

    #[must_use]
    pub const fn new(every: f64) -> Self {
        Self { every, until_next: every }
    }

    pub fn advance(&mut self, seconds: f64) -> bool {
        self.until_next -= seconds.max(0.0);
        if self.until_next > 0.0 {
            return false;
        }
        self.until_next = self.every.max(self.until_next + self.every);
        true
    }

    pub fn restart(&mut self) {
        self.until_next = self.every;
    }
}

#[cfg(test)]
mod tests {
    use super::HarvestTimer;

    #[test]
    fn the_timer_fires_once_per_period() {
        let mut timer = HarvestTimer::new(10.0);
        let fired = (0..(35 * 60)).filter(|_| timer.advance(1.0 / 60.0)).count();

        assert_eq!(fired, 3);
    }

    #[test]
    fn a_long_pause_fires_once_rather_than_catching_up() {
        let mut timer = HarvestTimer::new(10.0);

        assert!(timer.advance(100.0));
        assert!(!timer.advance(1.0));
    }

    #[test]
    fn a_manual_harvest_restarts_the_wait() {
        let mut timer = HarvestTimer::new(10.0);
        timer.advance(9.0);
        timer.restart();

        assert!(!timer.advance(5.0));
        assert!(timer.advance(5.0));
    }
}
