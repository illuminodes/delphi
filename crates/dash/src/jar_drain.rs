use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JarDrain {
    pub started: f64,
    pub amount: f64,
    pub seconds: f64,
}

impl JarDrain {
    pub const SECONDS: f64 = 3.0;

    #[must_use]
    pub const fn new(started: f64, amount: f64) -> Self {
        Self { started, amount, seconds: Self::SECONDS }
    }

    #[must_use]
    pub fn ends(&self) -> f64 {
        self.started + self.seconds
    }

    fn progress(&self, now: f64) -> f64 {
        ((now - self.started) / self.seconds).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn drained(&self, now: f64) -> f64 {
        let progress = self.progress(now);
        self.amount * progress * progress * (3.0 - 2.0 * progress)
    }

    #[must_use]
    pub fn left(&self, now: f64) -> f64 {
        self.amount - self.drained(now)
    }

    #[must_use]
    pub fn flow(&self, now: f64) -> f64 {
        let progress = self.progress(now);
        if progress <= 0.0 || progress >= 1.0 {
            return 0.0;
        }
        self.amount * 4.0 * progress * (1.0 - progress)
    }

    #[must_use]
    pub fn pull(&self, now: f64) -> f64 {
        let progress = self.progress(now);
        if progress <= 0.0 || progress >= 1.0 { 0.0 } else { (PI * progress).sin() }
    }

    #[must_use]
    pub fn running(&self, now: f64) -> bool {
        now < self.ends()
    }
}

#[cfg(test)]
mod tests {
    use super::JarDrain;

    const DRAIN: JarDrain = JarDrain::new(2.0, 0.8);

    #[test]
    fn nothing_leaves_before_the_drain_starts_and_everything_has_left_when_it_ends() {
        assert!(DRAIN.drained(1.0).abs() < 1e-12);
        assert!((DRAIN.left(1.9) - 0.8).abs() < 1e-12);
        assert!(DRAIN.left(DRAIN.ends()).abs() < 1e-12);
        assert!(DRAIN.left(DRAIN.ends() + 10.0).abs() < 1e-12);
    }

    #[test]
    fn the_gas_leaves_slowly_then_quickly_then_slowly() {
        let step = |at: f64| DRAIN.drained(at + 0.1) - DRAIN.drained(at);
        let middle = DRAIN.started + DRAIN.seconds * 0.5;

        assert!(step(middle - 0.05) > step(DRAIN.started) * 4.0);
        assert!(step(middle - 0.05) > step(DRAIN.ends() - 0.1) * 4.0);
        assert!((0..300).map(|at| DRAIN.started + f64::from(at) * 0.01).all(|at| DRAIN.left(at + 0.01) <= DRAIN.left(at) + 1e-12));
    }

    #[test]
    fn the_flow_peaks_at_the_amount_held_halfway_through() {
        let middle = DRAIN.started + DRAIN.seconds * 0.5;

        assert!((DRAIN.flow(middle) - 0.8).abs() < 1e-9);
        assert!(DRAIN.flow(DRAIN.started).abs() < 1e-12 && DRAIN.flow(DRAIN.ends()).abs() < 1e-12);
    }

    #[test]
    fn the_pull_on_the_gas_only_lasts_while_draining() {
        assert!(DRAIN.pull(1.0).abs() < 1e-12);
        assert!(DRAIN.pull(DRAIN.started + 1.5) > 0.99);
        assert!(DRAIN.pull(DRAIN.ends() + 0.1).abs() < 1e-12);
        assert!(DRAIN.running(DRAIN.ends() - 0.01) && !DRAIN.running(DRAIN.ends()));
    }
}
