use crate::{CarveAlign, CarveBox, CarveFont, CarveGroove, CarveText, CarveWeight, Carving};

#[derive(Debug, Clone, PartialEq)]
pub struct CarveHeading {
    title: &'static str,
    detail: String,
    unit: f64,
}

impl CarveHeading {
    const TITLE: f64 = 15.0;
    const DETAIL: f64 = 11.0;
    const TRACKING: f64 = 0.2;
    const GAP: f64 = 10.0;

    #[must_use]
    pub fn new(title: &'static str, detail: impl Into<String>, unit: f64) -> Self {
        Self { title, detail: detail.into(), unit }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> (Vec<Carving>, CarveBox) {
        let size = Self::TITLE * self.unit;
        let baseline = area.top + size;
        let title = CarveText::new(self.title, CarveFont::Display, size, area.left, baseline).tracking(Self::TRACKING);
        let detail = CarveText::new(self.detail.clone(), CarveFont::Figures, Self::DETAIL * self.unit, area.right, baseline)
            .align(CarveAlign::End)
            .depth(0.6);
        let gap = Self::GAP * self.unit;
        let rule_y = baseline - size * 0.3;
        let rule_from = title.bounds().right + gap;
        let rule_to = if self.detail.is_empty() { area.right } else { detail.bounds().left - gap };
        let mut carvings: Vec<Carving> = vec![title.into()];
        if rule_to > rule_from {
            carvings.push(CarveGroove::line((rule_from, rule_y), (rule_to, rule_y), 0.0).weighted(CarveWeight::Rule, self.unit).into());
        }
        if !self.detail.is_empty() {
            carvings.push(detail.into());
        }
        let body_top = baseline + size * 0.22 + gap;
        (carvings, CarveBox::new(area.left, body_top, area.right, area.bottom))
    }
}

#[cfg(test)]
mod tests {
    use super::CarveHeading;
    use crate::CarveBox;

    #[test]
    fn the_heading_stays_in_its_area_and_leaves_the_rest_as_the_body() {
        let area = CarveBox::new(10.0, 20.0, 400.0, 300.0);
        let (carvings, body) = CarveHeading::new("PRICE", "live", 1.0).carve(area);

        assert!(carvings.iter().all(|carving| area.contains(carving.bounds())));
        assert!(carvings.iter().all(|carving| carving.bounds().bottom <= body.top));
        assert!((body.bottom - 300.0).abs() < 1e-9);
    }

    #[test]
    fn a_heading_without_detail_runs_its_rule_to_the_edge() {
        let area = CarveBox::new(0.0, 0.0, 300.0, 100.0);
        let (carvings, _) = CarveHeading::new("FIT", "", 1.0).carve(area);

        assert_eq!(carvings.len(), 2);
        assert!((carvings[1].bounds().right - 300.9).abs() < 1e-9);
    }
}
