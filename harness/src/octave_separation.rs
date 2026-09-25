/// Whether a draw's octave ratio tells correct estimates from octave locks.
///
/// Each draw contributes its [`mantle_oracle::SlideProfile::octave_ratio`]
/// and its error against the reference. A draw is `locked` when the error
/// exceeds [`Self::LOCK_ERROR`], which only an octave slip produces, and
/// `sound` when it is under [`Self::SOUND_ERROR`].
#[derive(Debug, Clone, Default)]
pub struct OctaveSeparation {
    sound: Vec<f64>,
    locked: Vec<f64>,
    between: usize,
    unscored: usize,
}

impl OctaveSeparation {
    pub const LOCK_ERROR: f64 = 0.25;
    pub const SOUND_ERROR: f64 = 0.05;

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, ratio: Option<f64>, error: f64) {
        let Some(ratio) = ratio.filter(|value| value.is_finite()) else {
            self.unscored += 1;
            return;
        };
        if error > Self::LOCK_ERROR {
            self.locked.push(ratio);
        } else if error < Self::SOUND_ERROR {
            self.sound.push(ratio);
        } else {
            self.between += 1;
        }
    }

    #[must_use]
    pub fn sound(&self) -> usize {
        self.sound.len()
    }

    #[must_use]
    pub fn locked(&self) -> usize {
        self.locked.len()
    }

    #[must_use]
    pub const fn between(&self) -> usize {
        self.between
    }

    #[must_use]
    pub const fn unscored(&self) -> usize {
        self.unscored
    }

    #[must_use]
    pub fn sound_quantile(&self, quantile: f64) -> Option<f64> {
        Self::quantile(&self.sound, quantile)
    }

    #[must_use]
    pub fn locked_quantile(&self, quantile: f64) -> Option<f64> {
        Self::quantile(&self.locked, quantile)
    }

    /// Share of sound and of locked draws whose ratio is at or above
    /// `threshold`, i.e. what a rule rejecting such draws would discard.
    #[must_use]
    pub fn flagged_at(&self, threshold: f64) -> (f64, f64) {
        (
            Self::share_at_least(&self.sound, threshold),
            Self::share_at_least(&self.locked, threshold),
        )
    }

    /// Probability that a random locked draw has a higher ratio than a random
    /// sound draw (ties count half). 0.5 means the ratio carries no signal.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn auc(&self) -> Option<f64> {
        if self.sound.is_empty() || self.locked.is_empty() {
            return None;
        }
        let mut wins = 0.0;
        for locked in &self.locked {
            for sound in &self.sound {
                if locked > sound {
                    wins += 1.0;
                } else if (locked - sound).abs() < f64::EPSILON {
                    wins += 0.5;
                }
            }
        }
        Some(wins / (self.locked.len() * self.sound.len()) as f64)
    }

    #[allow(clippy::cast_precision_loss)]
    fn share_at_least(values: &[f64], threshold: f64) -> f64 {
        if values.is_empty() {
            return f64::NAN;
        }
        values.iter().filter(|value| **value >= threshold).count() as f64 / values.len() as f64
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    fn quantile(values: &[f64], quantile: f64) -> Option<f64> {
        if values.is_empty() {
            return None;
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let index = ((sorted.len() - 1) as f64 * quantile).round() as usize;
        sorted.get(index).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::OctaveSeparation;

    struct Fixture;

    impl Fixture {
        fn split() -> OctaveSeparation {
            let mut separation = OctaveSeparation::new();
            for ratio in [0.1, 0.2, 0.3, 0.4] {
                separation.record(Some(ratio), 0.001);
            }
            for ratio in [0.8, 0.9] {
                separation.record(Some(ratio), 1.0);
            }
            separation.record(Some(0.5), 0.10);
            separation.record(None, 0.001);
            separation
        }
    }

    #[test]
    fn classifies_by_error() {
        let separation = Fixture::split();
        assert_eq!(separation.sound(), 4);
        assert_eq!(separation.locked(), 2);
        assert_eq!(separation.between(), 1);
        assert_eq!(separation.unscored(), 1);
    }

    #[test]
    fn perfect_split_has_unit_auc() {
        assert_eq!(Fixture::split().auc(), Some(1.0));
    }

    #[test]
    fn flagged_shares_follow_the_threshold() {
        let (sound, locked) = Fixture::split().flagged_at(0.35);
        assert!((sound - 0.25).abs() < 1e-12);
        assert!((locked - 1.0).abs() < 1e-12);
    }

    #[test]
    fn quantiles_come_from_the_right_class() {
        let separation = Fixture::split();
        assert_eq!(separation.sound_quantile(0.0), Some(0.1));
        assert_eq!(separation.locked_quantile(1.0), Some(0.9));
    }

    #[test]
    fn auc_is_none_without_both_classes() {
        assert_eq!(OctaveSeparation::new().auc(), None);
    }
}
