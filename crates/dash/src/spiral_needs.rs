use crate::{SpiralCard, SpiralCut, TileSize};

/// For each place in the spiral, the smallest area that holds its card and every card inside it.
pub struct SpiralNeeds {
    needs: Vec<TileSize>,
}

impl SpiralNeeds {
    /// `cuts[i]` is the cut that separates card `i` from the cards after it.
    #[must_use]
    pub fn new(cards: &[SpiralCard], cuts: &[SpiralCut]) -> Self {
        let mut needs = vec![TileSize::ZERO; cards.len()];
        let mut inner: Option<TileSize> = None;
        for (index, card) in cards.iter().enumerate().rev() {
            let need = match (inner, cuts.get(index)) {
                (Some(rest), Some(cut)) => card.minimum().beside(rest, cut.axis()),
                _ => card.minimum(),
            };
            needs[index] = need;
            inner = Some(need);
        }
        Self { needs }
    }

    /// The smallest workspace that shows every card at its minimum.
    #[must_use]
    pub fn whole(&self) -> TileSize {
        self.needs.first().copied().unwrap_or(TileSize::ZERO)
    }

    /// The share card `index` takes of an area `length` pixels long along `cut`, as close to `wanted`
    /// as leaves the card and the cards inside it their minimums. When both cannot fit, the
    /// area is shared in proportion to what each side needs.
    #[must_use]
    pub fn bounded(&self, cards: &[SpiralCard], index: usize, cut: SpiralCut, length: f64, wanted: f64) -> f64 {
        let own = cards.get(index).map_or(0.0, |card| card.minimum().along(cut.axis()));
        let rest = self.needs.get(index + 1).map_or(0.0, |need| need.along(cut.axis()));
        if length <= 0.0 {
            return wanted;
        }
        let (low, high) = (own / length, 1.0 - rest / length);
        if low < high {
            wanted.clamp(low, high)
        } else if own + rest > 0.0 {
            own / (own + rest)
        } else {
            wanted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SpiralNeeds;
    use crate::{PanelId, SpiralCard, SpiralCut, SplitAxis, TileSize};

    struct NeedsFixture;

    impl NeedsFixture {
        fn cards() -> Vec<SpiralCard> {
            vec![
                SpiralCard::new(PanelId::History, 0.5, TileSize::new(100.0, 50.0)),
                SpiralCard::new(PanelId::Fit, 0.5, TileSize::new(80.0, 60.0)),
                SpiralCard::new(PanelId::Price, 0.5, TileSize::new(40.0, 30.0)),
            ]
        }

        fn cuts() -> Vec<SpiralCut> {
            vec![SpiralCut::new(SplitAxis::Row, true), SpiralCut::new(SplitAxis::Column, true)]
        }
    }

    #[test]
    fn each_place_needs_room_for_itself_and_everything_inside() {
        let needs = SpiralNeeds::new(&NeedsFixture::cards(), &NeedsFixture::cuts());

        assert_eq!(needs.whole(), TileSize::new(180.0, 90.0));
    }

    #[test]
    fn a_share_is_kept_between_the_two_minimums() {
        let (cards, cuts) = (NeedsFixture::cards(), NeedsFixture::cuts());
        let needs = SpiralNeeds::new(&cards, &cuts);

        assert!((needs.bounded(&cards, 0, cuts[0], 400.0, 0.1) - 0.25).abs() < 1e-9);
        assert!((needs.bounded(&cards, 0, cuts[0], 400.0, 0.9) - 0.8).abs() < 1e-9);
        assert!((needs.bounded(&cards, 0, cuts[0], 400.0, 0.5) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn an_area_exactly_filled_by_the_minimums_does_not_panic() {
        let (cards, cuts) = (NeedsFixture::cards(), NeedsFixture::cuts());
        let needs = SpiralNeeds::new(&cards, &cuts);

        let share = needs.bounded(&cards, 0, cuts[0], 180.0, 0.9);
        assert!((share * 180.0 - 100.0).abs() < 1e-9);
    }

    #[test]
    fn a_cramped_area_is_shared_by_need() {
        let (cards, cuts) = (NeedsFixture::cards(), NeedsFixture::cuts());
        let needs = SpiralNeeds::new(&cards, &cuts);

        assert!((needs.bounded(&cards, 0, cuts[0], 90.0, 0.9) - 100.0 / 180.0).abs() < 1e-9);
    }
}
