use crate::{PanelId, TileSize};

/// One card of the spiral, the share of the remaining area it asks for, and the smallest size it accepts.
#[derive(Debug, Clone, PartialEq)]
pub struct SpiralCard {
    panel: PanelId,
    ratio: f64,
    minimum: TileSize,
}

impl SpiralCard {
    pub const MIN_RATIO: f64 = 0.05;
    pub const GOLDEN_RATIO: f64 = 0.618_033_988_749_895;

    #[must_use]
    pub fn new(panel: PanelId, ratio: f64, minimum: TileSize) -> Self {
        let mut card = Self {
            panel,
            ratio: Self::GOLDEN_RATIO,
            minimum,
        };
        card.set_ratio(ratio);
        card
    }

    #[must_use]
    pub const fn panel(&self) -> &PanelId {
        &self.panel
    }

    #[must_use]
    pub const fn ratio(&self) -> f64 {
        self.ratio
    }

    #[must_use]
    pub const fn minimum(&self) -> TileSize {
        self.minimum
    }

    pub fn set_ratio(&mut self, ratio: f64) {
        if ratio.is_finite() {
            self.ratio = ratio.clamp(Self::MIN_RATIO, 1.0 - Self::MIN_RATIO);
        }
    }

    /// Trades the cards shown in two places; each place keeps its share.
    pub fn swap_panels(&mut self, other: &mut Self) {
        std::mem::swap(&mut self.panel, &mut other.panel);
        std::mem::swap(&mut self.minimum, &mut other.minimum);
    }
}

#[cfg(test)]
mod tests {
    use super::SpiralCard;
    use crate::{PanelId, TileSize};

    #[test]
    fn the_ratio_is_kept_inside_the_area_and_nonsense_is_ignored() {
        let mut card = SpiralCard::new(PanelId::Blockchain, 1.5, TileSize::ZERO);
        assert!((card.ratio() - (1.0 - SpiralCard::MIN_RATIO)).abs() < 1e-9);

        card.set_ratio(f64::NAN);
        assert!((card.ratio() - (1.0 - SpiralCard::MIN_RATIO)).abs() < 1e-9);

        card.set_ratio(0.3);
        assert!((card.ratio() - 0.3).abs() < 1e-9);
    }

    #[test]
    fn swapping_moves_the_panel_and_its_minimum_but_not_the_share() {
        let mut a = SpiralCard::new(PanelId::Blockchain, 0.3, TileSize::new(10.0, 10.0));
        let mut b = SpiralCard::new(PanelId::Price, 0.7, TileSize::new(20.0, 20.0));
        a.swap_panels(&mut b);

        assert_eq!(a.panel(), &PanelId::Price);
        assert_eq!(a.minimum(), TileSize::new(20.0, 20.0));
        assert!((a.ratio() - 0.3).abs() < 1e-9);
    }
}
