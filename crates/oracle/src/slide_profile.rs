/// Stencil scores for every slide the rough estimator tried, in slide order.
///
/// One octave (a factor of two in price) spans [`Self::OCTAVE_SLIDES`] bins,
/// so the rival to the best slide sits roughly that far on either side.
#[derive(Debug, Clone, PartialEq)]
pub struct SlideProfile {
    first_slide: i32,
    scores: Vec<f64>,
}

impl SlideProfile {
    pub const OCTAVE_SLIDES: i32 = 60;
    pub const OCTAVE_REACH: i32 = 3;

    #[must_use]
    pub const fn new(first_slide: i32, scores: Vec<f64>) -> Self {
        Self {
            first_slide,
            scores,
        }
    }

    #[must_use]
    pub const fn first_slide(&self) -> i32 {
        self.first_slide
    }

    #[must_use]
    pub fn scores(&self) -> &[f64] {
        &self.scores
    }

    #[must_use]
    pub fn score_at(&self, slide: i32) -> Option<f64> {
        let index = usize::try_from(slide - self.first_slide).ok()?;
        self.scores.get(index).copied()
    }

    /// The first slide holding the highest score.
    #[must_use]
    pub fn best(&self) -> Option<(i32, f64)> {
        self.best_where(|_| true)
    }

    /// The first highest-scoring slide among those `keep` accepts.
    #[must_use]
    pub fn best_where(&self, keep: impl Fn(i32) -> bool) -> Option<(i32, f64)> {
        let mut best: Option<(i32, f64)> = None;
        for (slide, score) in self.slides().filter(|(slide, _)| keep(*slide)) {
            if best.is_none_or(|(_, top)| score > top) {
                best = Some((slide, score));
            }
        }
        best
    }

    #[must_use]
    pub fn average(&self) -> f64 {
        if self.scores.is_empty() {
            return f64::NAN;
        }
        self.scores.iter().sum::<f64>() / f64::from(u32::try_from(self.scores.len()).unwrap_or(u32::MAX))
    }

    /// The highest score within [`Self::OCTAVE_REACH`] slides of one octave
    /// above or below the best slide.
    #[must_use]
    pub fn octave_rival(&self) -> Option<(i32, f64)> {
        let (best, _) = self.best()?;
        let mut rival: Option<(i32, f64)> = None;
        for center in [best - Self::OCTAVE_SLIDES, best + Self::OCTAVE_SLIDES] {
            for slide in center - Self::OCTAVE_REACH..=center + Self::OCTAVE_REACH {
                if let Some(score) = self.score_at(slide)
                    && rival.is_none_or(|(_, top)| score > top)
                {
                    rival = Some((slide, score));
                }
            }
        }
        rival
    }

    /// The rival's score above the average as a share of the best score
    /// above the average: near 1 when an octave neighbour almost won, near 0
    /// or negative when the best slide stands alone.
    #[must_use]
    pub fn octave_ratio(&self) -> Option<f64> {
        let (_, best) = self.best()?;
        let (_, rival) = self.octave_rival()?;
        let average = self.average();
        let lead = best - average;
        (lead > f64::EPSILON).then(|| (rival - average) / lead)
    }

    fn slides(&self) -> impl Iterator<Item = (i32, f64)> + '_ {
        (self.first_slide..).zip(self.scores.iter().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::SlideProfile;

    struct Fixture;

    impl Fixture {
        fn peaks(first: i32, len: usize, peaks: &[(i32, f64)]) -> SlideProfile {
            let mut scores = vec![1.0; len];
            for (slide, score) in peaks {
                scores[usize::try_from(slide - first).expect("peak in range")] = *score;
            }
            SlideProfile::new(first, scores)
        }
    }

    #[test]
    fn best_is_the_highest_slide() {
        let profile = Fixture::peaks(-10, 200, &[(40, 9.0), (100, 5.0)]);
        assert_eq!(profile.best(), Some((40, 9.0)));
    }

    #[test]
    fn rival_is_found_one_octave_up() {
        let profile = Fixture::peaks(-10, 200, &[(40, 9.0), (101, 5.0)]);
        assert_eq!(profile.octave_rival(), Some((101, 5.0)));
    }

    #[test]
    fn rival_is_found_one_octave_down() {
        let profile = Fixture::peaks(-100, 300, &[(40, 9.0), (-22, 7.0)]);
        assert_eq!(profile.octave_rival(), Some((-22, 7.0)));
    }

    #[test]
    fn ratio_is_near_one_for_a_close_rival() {
        let profile = Fixture::peaks(0, 200, &[(20, 10.0), (80, 9.9)]);
        let ratio = profile.octave_ratio().expect("has a lead");
        assert!(ratio > 0.95, "{ratio}");
    }

    #[test]
    fn ratio_is_near_zero_without_a_rival() {
        let profile = Fixture::peaks(0, 200, &[(20, 10.0)]);
        let ratio = profile.octave_ratio().expect("has a lead");
        assert!(ratio.abs() < 0.05, "{ratio}");
    }

    #[test]
    fn a_flat_profile_has_no_ratio() {
        assert_eq!(SlideProfile::new(0, vec![1.0; 100]).octave_ratio(), None);
    }
}
