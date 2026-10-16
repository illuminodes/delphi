use crate::{ChainCarving, Carving, FitCarving, HeaderCarving, HistoryCarving, InfoCarving, MarbleKind, MempoolCarving, PriceCarving, WireframeLayout};

#[derive(Debug, Clone, PartialEq)]
pub struct WireframePlan {
    layout: WireframeLayout,
    carvings: Vec<Carving>,
}

impl WireframePlan {
    #[must_use]
    pub fn new(width: f64, height: f64, kind: MarbleKind, seed: u64) -> Self {
        let layout = WireframeLayout::new(width, height);
        let unit = layout.unit;
        let mut carvings = HeaderCarving::new(unit, kind).carve(layout.header);
        carvings.extend(InfoCarving::new(unit).carve(layout.info));
        carvings.extend(PriceCarving::new(unit).carve(layout.price));
        carvings.extend(HistoryCarving::new(unit, seed).carve(layout.history));
        carvings.extend(FitCarving::new(unit).carve(layout.fit));
        carvings.extend(ChainCarving::new(unit).carve(layout.chain));
        carvings.extend(MempoolCarving::new(unit).carve(layout.mempool));
        Self { layout, carvings }
    }

    #[must_use]
    pub const fn layout(&self) -> &WireframeLayout {
        &self.layout
    }

    #[must_use]
    pub fn carvings(&self) -> &[Carving] {
        &self.carvings
    }
}

#[cfg(test)]
mod tests {
    use super::WireframePlan;
    use crate::{CarveFill, Carving, MarbleKind};

    #[test]
    fn every_carving_lies_on_the_slab() {
        for (width, height) in [(1440.0, 900.0), (1280.0, 760.0), (1920.0, 1080.0)] {
            let plan = WireframePlan::new(width, height, MarbleKind::Nero, 7);
            let slab = plan.layout().slab;
            assert!(plan.carvings().iter().all(|carving| slab.contains(carving.bounds())), "{width}x{height}");
        }
    }

    #[test]
    fn the_plan_cuts_both_lettering_and_grooves_and_gilds_only_a_few() {
        let plan = WireframePlan::new(1440.0, 900.0, MarbleKind::Parian, 7);
        let texts = plan.carvings().iter().filter(|carving| matches!(carving, Carving::Text(_))).count();
        let gilt = plan.carvings().iter().filter(|carving| carving.fill() == CarveFill::Gilt).count();

        assert!(texts > 40, "{texts}");
        assert!(plan.carvings().len() > texts);
        assert!(gilt > 0 && gilt * 4 < plan.carvings().len(), "{gilt}");
    }
}
