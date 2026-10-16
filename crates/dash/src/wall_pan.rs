#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WallPan {
    from: f64,
    to: f64,
    age: f64,
}

impl WallPan {
    const LENGTH: f64 = 2.6;

    #[must_use]
    pub const fn resting(view: f64) -> Self {
        Self { from: view, to: view, age: Self::LENGTH }
    }

    #[must_use]
    pub fn toward(&self, to: f64) -> Self {
        if (to - self.to).abs() < 1e-9 {
            return *self;
        }
        Self { from: self.view(), to, age: 0.0 }
    }

    pub fn advance(&mut self, step: f64) {
        self.age = (self.age + step.max(0.0)).min(Self::LENGTH);
    }

    #[must_use]
    pub fn done(&self) -> bool {
        self.age >= Self::LENGTH
    }

    #[must_use]
    pub fn view(&self) -> f64 {
        let t = (self.age / Self::LENGTH).clamp(0.0, 1.0);
        let eased = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
        self.from + (self.to - self.from) * eased
    }
}

#[cfg(test)]
mod tests {
    use super::WallPan;

    #[test]
    fn a_resting_pan_stays_where_it_is() {
        let pan = WallPan::resting(2.15);

        assert!(pan.done() && (pan.view() - 2.15).abs() < 1e-12);
        assert_eq!(pan.toward(2.15), pan);
    }

    #[test]
    fn the_pan_starts_and_ends_gently_and_only_moves_forward() {
        let mut pan = WallPan::resting(0.0).toward(1.0);
        let mut views = vec![pan.view()];
        while !pan.done() {
            pan.advance(0.05);
            views.push(pan.view());
        }

        assert!(views.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(views[1] - views[0] < 0.01);
        assert!(views[views.len() / 2 + 1] - views[views.len() / 2] > 0.03);
        assert!((views[views.len() - 1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_new_target_mid_pan_starts_from_where_the_view_is_now() {
        let mut pan = WallPan::resting(0.0).toward(1.0);
        pan.advance(1.3);
        let now = pan.view();
        let again = pan.toward(2.0);

        assert!((again.view() - now).abs() < 1e-12);
    }
}
