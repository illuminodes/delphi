use crate::{CarveAlign, CarveBox, CarveFont, CarveHeading, CarveText, Carving};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriceCarving {
    unit: f64,
}

impl PriceCarving {
    const WINDOWS: [(&'static str, &'static str); 4] = [("LIVE", "104,387"), ("HOURLY", "104,212"), ("DAILY", "103,960"), ("AUDIT", "103,881")];
    const FIGURE: f64 = 68.0;
    const WINDOW_FIGURE: f64 = 30.0;
    const WINDOW_LABEL: f64 = 10.5;
    const FIGURE_DESCENT: f64 = 0.25;

    #[must_use]
    pub const fn new(unit: f64) -> Self {
        Self { unit }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let (mut carvings, body) = CarveHeading::new("PRICE", "live window · mempool", unit).carve(area);
        let figure_size = (Self::FIGURE * unit).min(body.height() * 0.5);
        let baseline = body.top + figure_size * 0.8;
        let dollar = CarveText::new("$", CarveFont::Figures, figure_size * 0.5, body.left, baseline).depth(1.0);
        let figure_x = dollar.bounds().right + 4.0 * unit;
        carvings.push(dollar.into());
        carvings.push(CarveText::new("104,387", CarveFont::Figures, figure_size, figure_x, baseline).depth(1.0).gilt().into());
        carvings.push(
            CarveText::new("+1.84% 24h", CarveFont::Figures, 14.0 * unit, body.right, baseline - figure_size * 0.42)
                .align(CarveAlign::End)
                .depth(0.8)
                .into(),
        );
        carvings.push(
            CarveText::new("spread vs daily −0.12%", CarveFont::Text, 13.0 * unit, body.right, baseline)
                .align(CarveAlign::End)
                .depth(0.6)
                .into(),
        );
        let value_y = body.bottom - Self::WINDOW_FIGURE * Self::FIGURE_DESCENT * unit;
        let label_y = value_y - (Self::WINDOW_FIGURE * 0.8 + 8.0) * unit;
        for (column, (label, value)) in body.columns(Self::WINDOWS.len(), 18.0 * unit).iter().zip(Self::WINDOWS) {
            carvings.push(CarveText::new(label, CarveFont::Display, Self::WINDOW_LABEL * unit, column.left, label_y).tracking(0.16).depth(0.7).into());
            carvings.push(CarveText::new(value, CarveFont::Figures, Self::WINDOW_FIGURE * unit, column.left, value_y).incised().into());
        }
        carvings
    }
}

#[cfg(test)]
mod tests {
    use super::PriceCarving;
    use crate::{CarveBox, CarveFill};

    #[test]
    fn the_live_price_is_the_only_gilt_cut() {
        let carvings = PriceCarving::new(1.0).carve(CarveBox::new(0.0, 0.0, 720.0, 230.0));

        assert_eq!(carvings.iter().filter(|carving| carving.fill() == CarveFill::Gilt).count(), 1);
    }

    #[test]
    fn the_window_figures_stay_incised_at_their_larger_size() {
        let carvings = PriceCarving::new(1.0).carve(CarveBox::new(0.0, 0.0, 720.0, 230.0));
        let figures: Vec<_> = carvings.iter().filter_map(|carving| match carving {
            crate::Carving::Text(text) if text.text == "104,212" => Some(text),
            _ => None,
        }).collect();

        assert_eq!(figures.len(), 1);
        assert!(figures[0].size > 17.0);
        assert_eq!(figures[0].cut, crate::CarveCut::Incised);
        assert!(figures[0].depth < 1.0);
    }

    #[test]
    fn the_gilt_live_price_is_cut_to_full_depth() {
        let carvings = PriceCarving::new(1.0).carve(CarveBox::new(0.0, 0.0, 720.0, 230.0));
        let gilt = carvings.iter().find_map(|carving| match carving {
            crate::Carving::Text(text) if text.fill == CarveFill::Gilt => Some(text),
            _ => None,
        });

        assert!(gilt.is_some_and(|text| (text.depth - 1.0).abs() < 1e-9));
    }

    #[test]
    fn the_price_section_keeps_inside_its_area() {
        let area = CarveBox::new(300.0, 140.0, 1020.0, 370.0);

        for carving in PriceCarving::new(1.0).carve(area) {
            assert!(area.grow(1.0).contains(carving.bounds()), "{carving:?} escapes {area:?}");
        }
    }
}
