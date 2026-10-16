use crate::{DividerSpot, DropZone, PanelId, SplitAxis, TileLayout, TileRect};

/// A pointer gesture on the workspace: dragging a boundary, or carrying a card to a new place.
#[derive(Debug, Clone, PartialEq)]
pub enum TileGesture {
    Resize(DividerSpot),
    Move { panel: PanelId, target: Option<(PanelId, DropZone)> },
}

impl TileGesture {
    #[must_use]
    pub const fn carrying(panel: PanelId) -> Self {
        Self::Move { panel, target: None }
    }

    /// Follows the pointer at workspace fractions `(x, y)`.
    pub fn follow(&mut self, layout: &mut TileLayout, x: f64, y: f64) {
        match self {
            Self::Resize(spot) => layout.resize(spot.index(), spot.ratio_at(x, y)),
            Self::Move { panel, target } => {
                *target = layout
                    .target_at(x, y)
                    .filter(|(over, _, _)| over != panel)
                    .map(|(over, _, zone)| (over, zone));
            }
        }
    }

    /// Ends the gesture, dropping a carried card where it was last aimed.
    pub fn finish(self, layout: &mut TileLayout) {
        if let Self::Move {
            panel,
            target: Some((over, zone)),
        } = self
        {
            layout.drop_onto(&panel, &over, zone);
        }
    }

    /// The area a carried card would take if dropped now.
    #[must_use]
    pub fn preview(&self, layout: &TileLayout) -> Option<TileRect> {
        let Self::Move {
            panel,
            target: Some((over, zone)),
        } = self
        else {
            return None;
        };
        layout.landing(panel, over, *zone)
    }

    #[must_use]
    pub fn carried(&self) -> Option<&PanelId> {
        match self {
            Self::Move { panel, .. } => Some(panel),
            Self::Resize(_) => None,
        }
    }

    #[must_use]
    pub fn css_class(&self) -> &'static str {
        match self {
            Self::Resize(spot) => match spot.axis() {
                SplitAxis::Row => "resizing-row",
                SplitAxis::Column => "resizing-column",
            },
            Self::Move { .. } => "moving",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TileGesture;
    use crate::{PanelId, SpiralCard, TileLayout, TileSize};

    struct GestureFixture;

    impl GestureFixture {
        fn price(letter: &str) -> PanelId {
            PanelId::lettered(letter)
        }

        fn pair() -> TileLayout {
            TileLayout::new(
                vec![
                    SpiralCard::new(Self::price("a"), 0.5, TileSize::ZERO),
                    SpiralCard::new(Self::price("b"), 0.5, TileSize::ZERO),
                ],
                TileSize::new(2000.0, 1000.0),
            )
        }
    }

    #[test]
    fn a_resize_puts_the_boundary_under_the_pointer() {
        let mut layout = GestureFixture::pair();
        let mut gesture = TileGesture::Resize(layout.dividers().remove(0));

        gesture.follow(&mut layout, 0.2, 0.7);
        gesture.follow(&mut layout, 0.35, 0.1);

        assert!((layout.tiles()[0].1.width - 0.35).abs() < 1e-9);
    }

    #[test]
    fn a_carried_card_aims_at_the_zone_under_the_pointer_and_lands_there() {
        let mut layout = GestureFixture::pair();
        let mut gesture = TileGesture::carrying(GestureFixture::price("a"));

        gesture.follow(&mut layout, 0.75, 0.95);
        let preview = gesture.preview(&layout).expect("aimed");
        assert!((preview.x - 0.5).abs() < 1e-9);

        gesture.finish(&mut layout);
        assert_eq!(layout.panels(), vec![GestureFixture::price("b"), GestureFixture::price("a")]);
    }

    #[test]
    fn hovering_its_own_card_or_nothing_aims_nowhere() {
        let mut layout = GestureFixture::pair();
        let before = layout.clone();
        let mut gesture = TileGesture::carrying(GestureFixture::price("a"));

        gesture.follow(&mut layout, 0.1, 0.5);
        assert!(gesture.preview(&layout).is_none());
        gesture.follow(&mut layout, 2.0, 0.5);
        assert!(gesture.preview(&layout).is_none());

        gesture.finish(&mut layout);
        assert_eq!(layout, before);
    }

    #[test]
    fn the_carried_card_is_known_only_while_moving() {
        let layout = GestureFixture::pair();
        assert_eq!(TileGesture::carrying(PanelId::Blockchain).carried(), Some(&PanelId::Blockchain));
        assert!(TileGesture::Resize(layout.dividers().remove(0)).carried().is_none());
        assert_eq!(TileGesture::carrying(PanelId::Blockchain).css_class(), "moving");
    }
}
