use crate::{
    DividerSpot, DropZone, PanelId, SpiralCard, SpiralCompass, SpiralCut, SpiralNeeds, TileRect, TileSize, WorkspaceArrangement,
};

/// The workspace as a spiral: each card in turn cuts its own share from the area the cards
/// before it left, and the last card fills what remains. Resizing a card moves only its inner
/// edge, and every card after it reflows inside the smaller or larger remainder. No card is
/// drawn below its minimum: shares are bounded so each card and everything inside it fits,
/// and the canvas grows past the viewport when even that is not possible.
#[derive(Debug, Clone, PartialEq)]
pub struct TileLayout {
    cards: Vec<SpiralCard>,
    viewport: TileSize,
}

impl TileLayout {
    const DEFAULT_ASPECT: f64 = 2.0;

    #[must_use]
    pub const fn new(cards: Vec<SpiralCard>, viewport: TileSize) -> Self {
        Self { cards, viewport }
    }

    #[must_use]
    pub fn panels(&self) -> Vec<PanelId> {
        self.cards.iter().map(|card| *card.panel()).collect()
    }

    /// The card order and shares, to save for the next visit.
    #[must_use]
    pub fn arrangement(&self) -> WorkspaceArrangement {
        WorkspaceArrangement::of(&self.cards)
    }

    #[must_use]
    pub const fn viewport(&self) -> TileSize {
        self.viewport
    }

    /// Records the visible workspace size in pixels.
    pub fn set_viewport(&mut self, viewport: TileSize) {
        if viewport.width.is_finite() && viewport.height.is_finite() {
            self.viewport = viewport;
        }
    }

    /// The pixel size the tiles are laid out on: the viewport, grown to fit every card's minimum.
    #[must_use]
    pub fn canvas(&self) -> TileSize {
        self.canvas_for(&SpiralNeeds::new(&self.cards, &self.cuts()))
    }

    fn canvas_for(&self, needs: &SpiralNeeds) -> TileSize {
        let whole = needs.whole();
        TileSize::new(self.viewport.width.max(whole.width), self.viewport.height.max(whole.height))
    }

    fn cuts(&self) -> Vec<SpiralCut> {
        let mut compass = SpiralCompass::new(self.viewport.aspect().unwrap_or(Self::DEFAULT_ASPECT));
        (1..self.cards.len()).map(|_| compass.turn()).collect()
    }

    #[must_use]
    pub fn tiles(&self) -> Vec<(PanelId, TileRect)> {
        self.arranged().0
    }

    #[must_use]
    pub fn dividers(&self) -> Vec<DividerSpot> {
        self.arranged().1
    }

    fn arranged(&self) -> (Vec<(PanelId, TileRect)>, Vec<DividerSpot>) {
        let cuts = self.cuts();
        let needs = SpiralNeeds::new(&self.cards, &cuts);
        let canvas = self.canvas_for(&needs);
        let mut rest = TileRect::FULL;
        let (mut tiles, mut dividers) = (Vec::new(), Vec::new());
        for (index, card) in self.cards.iter().enumerate() {
            let Some(cut) = cuts.get(index).copied() else {
                tiles.push((*card.panel(), rest));
                break;
            };
            let length = rest.along(cut.axis()) * canvas.along(cut.axis());
            let ratio = needs.bounded(&self.cards, index, cut, length, card.ratio());
            dividers.push(DividerSpot::new(index, cut, rest, ratio));
            let (own, remaining) = cut.split(rest, ratio);
            tiles.push((*card.panel(), own));
            rest = remaining;
        }
        (tiles, dividers)
    }

    /// Asks card `index` for a `ratio` share; it keeps the nearest share that respects every minimum.
    pub fn resize(&mut self, index: usize, ratio: f64) {
        let Some(card) = self.cards.get_mut(index) else {
            return;
        };
        card.set_ratio(ratio);
        if let Some(granted) = self.dividers().get(index).map(DividerSpot::ratio) {
            self.cards[index].set_ratio(granted);
        }
    }

    /// The card under the workspace point `(x, y)` and the zone of it the point is in.
    #[must_use]
    pub fn target_at(&self, x: f64, y: f64) -> Option<(PanelId, TileRect, DropZone)> {
        self.tiles().into_iter().find_map(|(panel, rect)| {
            let (fx, fy) = rect.fraction(x, y).filter(|_| rect.contains(x, y))?;
            Some((panel, rect, DropZone::at(fx, fy)))
        })
    }

    /// Drops `moving` onto `target`. The centre swaps their places in the spiral, keeping each
    /// place's size; a leading edge puts `moving` just outside `target`, a trailing edge just inside.
    pub fn drop_onto(&mut self, moving: &PanelId, target: &PanelId, zone: DropZone) {
        let (Some(from), Some(to)) = (self.position(moving), self.position(target)) else {
            return;
        };
        if from == to {
            return;
        }
        if zone == DropZone::Center {
            let (low, high) = (from.min(to), from.max(to));
            let (head, tail) = self.cards.split_at_mut(high);
            head[low].swap_panels(&mut tail[0]);
            return;
        }
        let card = self.cards.remove(from);
        let to = self.position(target).unwrap_or(to);
        let at = if zone.leads() { to } else { to + 1 };
        self.cards.insert(at, card);
    }

    /// The area `moving` would take if dropped onto `target` in `zone`.
    #[must_use]
    pub fn landing(&self, moving: &PanelId, target: &PanelId, zone: DropZone) -> Option<TileRect> {
        let mut after = self.clone();
        after.drop_onto(moving, target, zone);
        after.tiles().into_iter().find(|(panel, _)| panel == moving).map(|(_, rect)| rect)
    }

    fn position(&self, panel: &PanelId) -> Option<usize> {
        self.cards.iter().position(|card| card.panel() == panel)
    }
}

#[cfg(test)]
mod tests {
    use super::TileLayout;
    use crate::{DropZone, PanelId, SpiralCard, TileRect, TileSize};

    struct LayoutFixture;

    impl LayoutFixture {
        const VIEWPORT: TileSize = TileSize::new(2000.0, 1000.0);

        fn price(letter: &str) -> PanelId {
            PanelId::lettered(letter)
        }

        fn card(window: &str) -> SpiralCard {
            SpiralCard::new(Self::price(window), 0.5, TileSize::ZERO)
        }

        fn sized(window: &str, width: f64, height: f64) -> SpiralCard {
            SpiralCard::new(Self::price(window), 0.5, TileSize::new(width, height))
        }

        fn four() -> TileLayout {
            TileLayout::new(
                vec![Self::card("a"), Self::card("b"), Self::card("c"), Self::card("d")],
                Self::VIEWPORT,
            )
        }

        fn rect(layout: &TileLayout, window: &str) -> TileRect {
            let panel = Self::price(window);
            layout
                .tiles()
                .into_iter()
                .find(|(own, _)| *own == panel)
                .map(|(_, rect)| rect)
                .expect("placed")
        }

        fn pixels(layout: &TileLayout, window: &str) -> TileSize {
            let (rect, canvas) = (Self::rect(layout, window), layout.canvas());
            TileSize::new(rect.width * canvas.width, rect.height * canvas.height)
        }

        fn close(a: TileRect, b: TileRect) -> bool {
            [(a.x, b.x), (a.y, b.y), (a.width, b.width), (a.height, b.height)]
                .iter()
                .all(|(p, q)| (p - q).abs() < 1e-9)
        }
    }

    #[test]
    fn the_cards_wind_inwards_and_cover_the_workspace() {
        let layout = LayoutFixture::four();

        assert!(LayoutFixture::close(LayoutFixture::rect(&layout, "a"), TileRect { x: 0.0, y: 0.0, width: 0.5, height: 1.0 }));
        assert!(LayoutFixture::close(LayoutFixture::rect(&layout, "b"), TileRect { x: 0.5, y: 0.0, width: 0.5, height: 0.5 }));
        assert!(LayoutFixture::close(LayoutFixture::rect(&layout, "c"), TileRect { x: 0.75, y: 0.5, width: 0.25, height: 0.5 }));
        assert!(LayoutFixture::close(LayoutFixture::rect(&layout, "d"), TileRect { x: 0.5, y: 0.5, width: 0.25, height: 0.5 }));
        let covered: f64 = layout.tiles().iter().map(|(_, rect)| rect.area()).sum();
        assert!((covered - 1.0).abs() < 1e-9);
        assert_eq!(layout.dividers().len(), 3);
    }

    #[test]
    fn resizing_one_card_keeps_the_cards_outside_it_and_reflows_those_inside() {
        let mut layout = LayoutFixture::four();
        layout.resize(1, 0.2);

        assert!(LayoutFixture::close(LayoutFixture::rect(&layout, "a"), TileRect { x: 0.0, y: 0.0, width: 0.5, height: 1.0 }));
        assert!((LayoutFixture::rect(&layout, "b").height - 0.2).abs() < 1e-9);
        assert!((LayoutFixture::rect(&layout, "c").height - 0.8).abs() < 1e-9);
        assert!((LayoutFixture::rect(&layout, "c").width - 0.25).abs() < 1e-9);
    }

    #[test]
    fn cards_in_the_same_column_or_row_size_independently() {
        let mut layout = LayoutFixture::four();
        layout.resize(2, 0.8);

        let c = LayoutFixture::rect(&layout, "c");
        let d = LayoutFixture::rect(&layout, "d");
        assert!((c.width - 0.4).abs() < 1e-9 && (d.width - 0.1).abs() < 1e-9);
        assert!((LayoutFixture::rect(&layout, "b").width - 0.5).abs() < 1e-9);
    }

    #[test]
    fn shrinking_a_card_stops_at_its_minimum() {
        let mut layout = TileLayout::new(
            vec![LayoutFixture::sized("a", 400.0, 0.0), LayoutFixture::card("b")],
            LayoutFixture::VIEWPORT,
        );
        layout.resize(0, 0.01);

        assert!((LayoutFixture::pixels(&layout, "a").width - 400.0).abs() < 1e-6);
    }

    #[test]
    fn growing_a_card_stops_where_the_cards_inside_reach_their_minimums() {
        let mut layout = TileLayout::new(
            vec![
                LayoutFixture::card("a"),
                LayoutFixture::sized("b", 300.0, 200.0),
                LayoutFixture::sized("c", 500.0, 100.0),
            ],
            LayoutFixture::VIEWPORT,
        );
        layout.resize(0, 0.99);

        assert!((LayoutFixture::pixels(&layout, "a").width - 1500.0).abs() < 1e-6);
        let c = LayoutFixture::pixels(&layout, "c");
        assert!(c.width >= 500.0 - 1e-6 && c.height >= 100.0 - 1e-6);
    }

    #[test]
    fn a_viewport_too_small_for_every_minimum_grows_the_canvas() {
        let layout = TileLayout::new(
            vec![LayoutFixture::sized("a", 600.0, 300.0), LayoutFixture::sized("b", 600.0, 300.0)],
            TileSize::new(800.0, 200.0),
        );

        assert_eq!(layout.canvas(), TileSize::new(1200.0, 300.0));
        for window in ["a", "b"] {
            let size = LayoutFixture::pixels(&layout, window);
            assert!(size.width >= 600.0 - 1e-6 && size.height >= 300.0 - 1e-6);
        }
    }

    #[test]
    fn the_point_under_a_card_edge_names_that_edge() {
        let layout = LayoutFixture::four();

        let (panel, _, zone) = layout.target_at(0.75, 0.02).expect("a card");
        assert_eq!(panel, LayoutFixture::price("b"));
        assert_eq!(zone, DropZone::Top);
        assert!(layout.target_at(1.5, 0.5).is_none());
    }

    #[test]
    fn a_leading_edge_moves_the_card_outside_its_target_and_a_trailing_one_inside() {
        let mut layout = LayoutFixture::four();
        layout.drop_onto(&LayoutFixture::price("d"), &LayoutFixture::price("b"), DropZone::Left);
        assert_eq!(layout.panels(), ["a", "d", "b", "c"].map(LayoutFixture::price).to_vec());

        layout.drop_onto(&LayoutFixture::price("a"), &LayoutFixture::price("c"), DropZone::Bottom);
        assert_eq!(layout.panels(), ["d", "b", "c", "a"].map(LayoutFixture::price).to_vec());
    }

    #[test]
    fn the_centre_swaps_places_and_each_place_keeps_its_size() {
        let mut layout = LayoutFixture::four();
        layout.resize(0, 0.3);
        let before = LayoutFixture::rect(&layout, "a");
        layout.drop_onto(&LayoutFixture::price("c"), &LayoutFixture::price("a"), DropZone::Center);

        assert!(LayoutFixture::close(LayoutFixture::rect(&layout, "c"), before));
        let unchanged = layout.clone();
        layout.drop_onto(&LayoutFixture::price("c"), &LayoutFixture::price("c"), DropZone::Left);
        assert_eq!(layout, unchanged);
    }

    #[test]
    fn the_landing_is_where_the_card_ends_up() {
        let layout = LayoutFixture::four();
        let landing = layout
            .landing(&LayoutFixture::price("d"), &LayoutFixture::price("a"), DropZone::Left)
            .expect("lands");

        let mut after = layout;
        after.drop_onto(&LayoutFixture::price("d"), &LayoutFixture::price("a"), DropZone::Left);
        assert!(LayoutFixture::close(landing, LayoutFixture::rect(&after, "d")));
    }
}
