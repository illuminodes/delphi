use crate::{CarveAlign, CarveBox, CarveFont, CarveGroove, CarveHeading, CarveText, CarveWeight, Carving};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MempoolCarving {
    unit: f64,
}

impl MempoolCarving {
    const ROWS: [(&'static str, &'static str, &'static str); 5] = [
        ("3f1a…09c2", "vote", "104,402"),
        ("a7e0…118b", "dust", "—"),
        ("51cd…ee74", "vote", "104,371"),
        ("0b92…6d30", "round", "—"),
        ("c4f8…a2e1", "vote", "104,388"),
    ];

    #[must_use]
    pub const fn new(unit: f64) -> Self {
        Self { unit }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let (mut carvings, body) = CarveHeading::new("MEMPOOL", "streaming", unit).carve(area);
        let counts_y = body.top + 10.0 * unit;
        carvings.push(CarveText::new("sampled 214   voting 61   rejected 153", CarveFont::Figures, 11.0 * unit, body.left, counts_y).depth(0.7).into());
        let rows_top = counts_y + 10.0 * unit;
        let pitch = (body.bottom - rows_top) / Self::ROWS.len() as f64;
        for (index, (id, verdict, price)) in Self::ROWS.iter().enumerate() {
            let top = rows_top + pitch * index as f64;
            let baseline = top + pitch * 0.62;
            carvings.push(CarveGroove::line((body.left, top), (body.right, top), 0.0).weighted(CarveWeight::Hair, unit).into());
            carvings.push(CarveText::new(*id, CarveFont::Figures, 11.5 * unit, body.left, baseline).into());
            carvings.push(CarveText::new(*verdict, CarveFont::Text, 12.0 * unit, body.left + body.width() * 0.55, baseline).align(CarveAlign::Centre).depth(0.7).into());
            carvings.push(CarveText::new(*price, CarveFont::Figures, 11.5 * unit, body.right, baseline).align(CarveAlign::End).into());
        }
        carvings
    }
}

#[cfg(test)]
mod tests {
    use super::MempoolCarving;
    use crate::CarveBox;

    #[test]
    fn the_mempool_section_keeps_inside_its_area() {
        let area = CarveBox::new(870.0, 680.0, 1400.0, 860.0);

        assert!(MempoolCarving::new(1.0).carve(area).iter().all(|carving| area.grow(2.0).contains(carving.bounds())));
    }
}
