use crate::{CarveAlign, CarveBox, CarveFont, CarveGroove, CarveText, CarveWeight, Carving, MarbleKind};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeaderCarving {
    unit: f64,
    kind: MarbleKind,
}

impl HeaderCarving {
    const WORDMARK: f64 = 46.0;
    const MOTTO: f64 = 15.0;
    const STATUS: f64 = 12.0;
    const SWITCH: f64 = 12.0;

    #[must_use]
    pub const fn new(unit: f64, kind: MarbleKind) -> Self {
        Self { unit, kind }
    }

    #[must_use]
    pub fn switch_label(&self) -> &'static str {
        match self.kind {
            MarbleKind::Parian => "NERO  ·  PARIAN ◆",
            _ => "◆ NERO  ·  PARIAN",
        }
    }

    #[must_use]
    pub fn switch(&self, area: CarveBox) -> CarveText {
        let size = Self::SWITCH * self.unit;
        CarveText::new(self.switch_label(), CarveFont::Display, size, area.right, area.top + size * 1.2)
            .align(CarveAlign::End)
            .tracking(0.12)
            .depth(0.8)
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let baseline = area.top + Self::WORDMARK * unit * 1.05;
        let wordmark = CarveText::new("ΔELPHI", CarveFont::Display, Self::WORDMARK * unit, area.left, baseline).tracking(0.18);
        let motto_x = wordmark.bounds().right + 28.0 * unit;
        let motto = CarveText::new("ΓΝΩΘΙ ΣΕΑΥΤΟΝ  ·  the on-chain oracle", CarveFont::Text, Self::MOTTO * unit, motto_x, baseline).depth(0.6);
        let status_size = Self::STATUS * unit;
        let status = CarveText::new("RELAY oracle.delphi   TIP 903,412   LIVE 4s", CarveFont::Figures, status_size, area.right, baseline)
            .align(CarveAlign::End)
            .depth(0.7);
        let rule_y = area.bottom - 6.0 * unit;
        vec![
            wordmark.into(),
            motto.into(),
            status.into(),
            self.switch(area).into(),
            CarveGroove::line((area.left + 1.3 * unit, rule_y), (area.right - 1.3 * unit, rule_y), 0.0).weighted(CarveWeight::Bold, unit).into(),
            CarveGroove::line((area.left + 0.8 * unit, rule_y + 4.5 * unit), (area.right - 0.8 * unit, rule_y + 4.5 * unit), 0.0).weighted(CarveWeight::Hair, unit).depth(0.6).into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::HeaderCarving;
    use crate::{CarveBox, MarbleKind};

    #[test]
    fn the_header_keeps_inside_its_band() {
        let area = CarveBox::new(40.0, 20.0, 1400.0, 116.0);
        let carvings = HeaderCarving::new(1.0, MarbleKind::Nero).carve(area);

        assert!(carvings.iter().all(|carving| area.grow(1.0).contains(carving.bounds())), "{:?}", carvings.iter().map(crate::Carving::bounds).collect::<Vec<_>>());
    }

    #[test]
    fn the_stone_switch_marks_the_stone_in_use() {
        assert!(HeaderCarving::new(1.0, MarbleKind::Nero).switch_label().starts_with('◆'));
        assert!(HeaderCarving::new(1.0, MarbleKind::Parian).switch_label().ends_with('◆'));
    }
}
