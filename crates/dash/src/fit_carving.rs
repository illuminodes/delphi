use crate::{CarveAlign, CarveBox, CarveFont, CarveGroove, CarveHeading, CarveText, CarveWeight, Carving};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitCarving {
    unit: f64,
}

impl FitCarving {
    const BARS: [f64; 13] = [0.06, 0.12, 0.22, 0.4, 0.63, 0.86, 1.0, 0.9, 0.68, 0.44, 0.25, 0.13, 0.05];
    const PEAK: f64 = 6.0;
    const SPREAD: f64 = 2.3;

    #[must_use]
    pub const fn new(unit: f64) -> Self {
        Self { unit }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let (mut carvings, body) = CarveHeading::new("FIT", "daily · 412 tx", unit).carve(area);
        let plot = CarveBox::new(body.left, body.top + 8.0 * unit, body.right, body.bottom - 22.0 * unit);
        let slots = plot.columns(Self::BARS.len(), 4.0 * unit);
        for (slot, share) in slots.iter().zip(Self::BARS) {
            let top = plot.bottom - share * plot.height();
            carvings.push(CarveGroove::frame(slot.left, top, slot.right, plot.bottom, 0.0).weighted(CarveWeight::Hair, unit).into());
        }
        let width = Self::BARS.len() as f64;
        let curve: Vec<(f64, f64)> = (0..=80)
            .map(|step| {
                let slot = f64::from(step) / 80.0 * width - 0.5;
                let x = plot.left + (slot + 0.5) / width * plot.width();
                let density = (-((slot - Self::PEAK) / Self::SPREAD).powi(2) / 2.0).exp();
                (x, plot.bottom - density * plot.height())
            })
            .collect();
        carvings.push(CarveGroove::new(curve, 0.0).weighted(CarveWeight::Bold, unit).into());
        let centre = plot.left + (Self::PEAK + 0.5) / width * plot.width();
        for dash in CarveGroove::dashed((centre, plot.top - 4.0 * unit), (centre, plot.bottom), 0.0, 4.0 * unit, 4.0 * unit) {
            carvings.push(dash.weighted(CarveWeight::Rule, unit).gilt().into());
        }
        let label_y = body.bottom - 4.0 * unit;
        carvings.push(CarveText::new("103.2k", CarveFont::Figures, 10.5 * unit, plot.left, label_y).depth(0.7).into());
        carvings.push(CarveText::new("104.4k", CarveFont::Figures, 10.5 * unit, centre, label_y).align(CarveAlign::Centre).gilt().into());
        carvings.push(CarveText::new("105.6k", CarveFont::Figures, 10.5 * unit, plot.right, label_y).align(CarveAlign::End).depth(0.7).into());
        carvings
    }
}

#[cfg(test)]
mod tests {
    use super::FitCarving;
    use crate::CarveBox;

    #[test]
    fn the_fit_section_keeps_inside_its_area() {
        let area = CarveBox::new(1050.0, 140.0, 1400.0, 640.0);

        assert!(FitCarving::new(1.0).carve(area).iter().all(|carving| area.grow(2.0).contains(carving.bounds())));
    }
}
