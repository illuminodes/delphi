use crate::{PanelFootprint, PanelId, SpiralCard};

/// The starting spiral: the six cards from largest on the outside to the price in the centre,
/// each card's share chosen so its area is its Fibonacci weight over the total.
pub struct WorkspacePlan;

impl WorkspacePlan {
    const WEIGHTS: [(PanelId, u32); 6] = [
        (PanelId::History, 5),
        (PanelId::Fit, 3),
        (PanelId::Blockchain, 3),
        (PanelId::Mempool, 3),
        (PanelId::Info, 2),
        (PanelId::Price, 1),
    ];

    #[must_use]
    pub fn cards() -> Vec<SpiralCard> {
        let mut remaining: u32 = Self::WEIGHTS.iter().map(|(_, weight)| weight).sum();
        Self::WEIGHTS
            .iter()
            .map(|&(panel, weight)| {
                let ratio = f64::from(weight) / f64::from(remaining.max(1));
                remaining -= weight;
                SpiralCard::new(panel, ratio, PanelFootprint::minimum(panel))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspacePlan;
    use crate::{PanelFootprint, PanelId, TileLayout, TileSize};

    #[test]
    fn there_is_one_card_of_each_kind_with_the_price_in_the_centre() {
        let panels: Vec<PanelId> = WorkspacePlan::cards().iter().map(|card| *card.panel()).collect();

        assert_eq!(panels, PanelId::ALL.to_vec());
        assert_eq!(WorkspacePlan::cards()[2].minimum(), PanelFootprint::minimum(PanelId::Blockchain));
    }

    #[test]
    fn on_a_roomy_screen_each_card_covers_its_weight_of_the_workspace() {
        let layout = TileLayout::new(WorkspacePlan::cards(), TileSize::new(4000.0, 2500.0));
        let areas: Vec<f64> = layout.tiles().iter().map(|(_, rect)| rect.area()).collect();
        let expected = [5.0, 3.0, 3.0, 3.0, 2.0, 1.0].map(|weight| weight / 17.0);

        for (area, want) in areas.iter().zip(expected) {
            assert!((area - want).abs() < 1e-9, "{area} vs {want}");
        }
    }

    #[test]
    fn on_a_laptop_screen_no_card_is_drawn_below_its_minimum() {
        let layout = TileLayout::new(WorkspacePlan::cards(), TileSize::new(1366.0, 700.0));
        let canvas = layout.canvas();

        for (panel, rect) in layout.tiles() {
            let minimum = PanelFootprint::minimum(panel);
            assert!(rect.width * canvas.width >= minimum.width - 1e-6, "{panel:?} too narrow");
            assert!(rect.height * canvas.height >= minimum.height - 1e-6, "{panel:?} too short");
        }
    }
}
