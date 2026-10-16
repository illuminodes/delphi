use crate::{CarveAlign, CarveBox, CarveFont, CarveGroove, CarveHeading, CarveText, Carving};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InfoCarving {
    unit: f64,
}

impl InfoCarving {
    const FACTS: [(&'static str, &'static str); 9] = [
        ("RELAY", "oracle.delphi"),
        ("PUBLISHER", "npub1…k8qz"),
        ("DAEMON", "oracled 0.9.2"),
        ("UPTIME", "12d 4h"),
        ("CHAIN TIP", "903,412"),
        ("MEMPOOL", "48,211 tx"),
        ("FEE RATE", "6.2 sat/vB"),
        ("LAST AUDIT", "2m ago"),
        ("RANGE", "7 days"),
    ];

    #[must_use]
    pub const fn new(unit: f64) -> Self {
        Self { unit }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let (mut carvings, body) = CarveHeading::new("INFORMATION", "", unit).carve(area);
        let pitch = (body.height() / Self::FACTS.len() as f64).min(34.0 * unit);
        for (index, (label, value)) in Self::FACTS.iter().enumerate() {
            let baseline = body.top + pitch * (index as f64 + 0.7);
            let label = CarveText::new(*label, CarveFont::Display, 10.5 * unit, body.left, baseline).tracking(0.14).depth(0.7);
            let value = CarveText::new(*value, CarveFont::Figures, 12.5 * unit, body.right, baseline).align(CarveAlign::End);
            let (from, to) = (label.bounds().right + 8.0 * unit, value.bounds().left - 8.0 * unit);
            if to > from {
                for dot in CarveGroove::dashed((from, baseline - 1.0 * unit), (to, baseline - 1.0 * unit), 1.0 * unit, 1.0 * unit, 5.0 * unit) {
                    carvings.push(dot.depth(0.35).into());
                }
            }
            carvings.push(label.into());
            carvings.push(value.into());
        }
        carvings
    }
}

#[cfg(test)]
mod tests {
    use super::InfoCarving;
    use crate::CarveBox;

    #[test]
    fn the_information_section_keeps_inside_its_area() {
        let area = CarveBox::new(44.0, 140.0, 300.0, 640.0);

        assert!(InfoCarving::new(1.0).carve(area).iter().all(|carving| area.grow(2.0).contains(carving.bounds())));
    }
}
