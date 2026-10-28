use crate::{PanelId, SpiralCard, TileRect, TileSize};

/// The workspace on a narrow screen: the cards in spiral order, one under another at the full
/// width, each a little taller than its minimum height. The stack fills the viewport when it is
/// shorter and scrolls when it is taller.
pub struct TileStack<'a> {
    cards: &'a [SpiralCard],
    viewport: TileSize,
}

impl<'a> TileStack<'a> {
    /// Viewports narrower than this, in CSS pixels, stack. Matches the 40rem breakpoint in main.css.
    pub const BELOW: f64 = 640.0;
    const GROWTH: f64 = 1.3;

    #[must_use]
    pub const fn new(cards: &'a [SpiralCard], viewport: TileSize) -> Self {
        Self { cards, viewport }
    }

    #[must_use]
    pub fn applies(viewport: TileSize) -> bool {
        viewport.width > 0.0 && viewport.width < Self::BELOW
    }

    fn total(&self) -> f64 {
        self.cards.iter().map(|card| card.minimum().height * Self::GROWTH).sum()
    }

    #[must_use]
    pub fn canvas(&self) -> TileSize {
        TileSize::new(self.viewport.width, self.viewport.height.max(self.total()))
    }

    #[must_use]
    pub fn tiles(&self) -> Vec<(PanelId, TileRect)> {
        let total = self.total();
        let mut y = 0.0;
        self.cards
            .iter()
            .map(|card| {
                let height = if total > 0.0 {
                    card.minimum().height * Self::GROWTH / total
                } else {
                    1.0 / self.cards.len() as f64
                };
                let rect = TileRect { x: 0.0, y, width: 1.0, height };
                y += height;
                (*card.panel(), rect)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::TileStack;
    use crate::{PanelId, SpiralCard, TileSize};

    struct StackFixture;

    impl StackFixture {
        fn cards() -> Vec<SpiralCard> {
            vec![
                SpiralCard::new(PanelId::lettered("a"), 0.5, TileSize::new(400.0, 100.0)),
                SpiralCard::new(PanelId::lettered("b"), 0.5, TileSize::new(400.0, 300.0)),
            ]
        }
    }

    #[test]
    fn only_narrow_measured_viewports_stack() {
        assert!(TileStack::applies(TileSize::new(390.0, 800.0)));
        assert!(!TileStack::applies(TileSize::new(1024.0, 700.0)));
        assert!(!TileStack::applies(TileSize::ZERO));
    }

    #[test]
    fn a_tall_stack_scrolls_with_each_card_at_full_width_in_order() {
        let cards = StackFixture::cards();
        let stack = TileStack::new(&cards, TileSize::new(390.0, 300.0));
        let tiles = stack.tiles();

        assert_eq!(stack.canvas(), TileSize::new(390.0, 520.0));
        assert!((tiles[0].1.height - 0.25).abs() < 1e-9 && (tiles[1].1.y - 0.25).abs() < 1e-9);
        assert!(tiles.iter().all(|(_, rect)| (rect.width - 1.0).abs() < 1e-9));
    }

    #[test]
    fn a_short_stack_stretches_to_the_viewport() {
        let cards = StackFixture::cards();
        let stack = TileStack::new(&cards, TileSize::new(390.0, 900.0));

        assert_eq!(stack.canvas(), TileSize::new(390.0, 900.0));
        let covered: f64 = stack.tiles().iter().map(|(_, rect)| rect.height).sum();
        assert!((covered - 1.0).abs() < 1e-9);
    }
}
