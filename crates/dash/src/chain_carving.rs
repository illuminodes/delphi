use crate::{CarveBox, CarveFont, CarveGroove, CarveHeading, CarveText, CarveWeight, Carving};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChainCarving {
    unit: f64,
}

impl ChainCarving {
    const BLOCKS: [(&'static str, &'static str, &'static str, f64); 7] = [
        ("903,412", "0000…a3f1", "18.4 sat/vB", 0.96),
        ("903,411", "0000…71c0", "11.2 sat/vB", 0.99),
        ("903,410", "0000…09bd", "9.7 sat/vB", 0.88),
        ("903,409", "0000…e244", "7.1 sat/vB", 0.73),
        ("903,408", "0000…5a1e", "8.9 sat/vB", 0.91),
        ("903,407", "0000…c7d2", "6.4 sat/vB", 0.64),
        ("903,406", "0000…3e90", "12.0 sat/vB", 0.98),
    ];

    #[must_use]
    pub const fn new(unit: f64) -> Self {
        Self { unit }
    }

    #[must_use]
    pub fn carve(&self, area: CarveBox) -> Vec<Carving> {
        let unit = self.unit;
        let (mut carvings, body) = CarveHeading::new("BLOCKCHAIN", "newest first", unit).carve(area);
        let tiles = body.columns(Self::BLOCKS.len(), 14.0 * unit);
        for (index, (tile, (height, hash, fee, fill))) in tiles.iter().zip(Self::BLOCKS).enumerate() {
            let pad = 9.0 * unit;
            carvings.push(CarveGroove::frame(tile.left, tile.top, tile.right, tile.bottom, 0.0).weighted(CarveWeight::Hair, unit).into());
            let mut height_text = CarveText::new(height, CarveFont::Figures, 13.0 * unit, tile.left + pad, tile.top + pad + 12.0 * unit);
            if index == 0 {
                height_text = height_text.gilt();
            }
            carvings.push(height_text.into());
            carvings.push(CarveText::new(hash, CarveFont::Figures, 9.5 * unit, tile.left + pad, tile.top + pad + 30.0 * unit).depth(0.6).into());
            carvings.push(CarveText::new(fee, CarveFont::Figures, 9.5 * unit, tile.left + pad, tile.top + pad + 46.0 * unit).depth(0.6).into());
            let bar_y = tile.bottom - pad - 3.0 * unit;
            let (bar_left, bar_right) = (tile.left + pad, tile.right - pad);
            carvings.push(CarveGroove::line((bar_left, bar_y), (bar_right, bar_y), 6.0 * unit).depth(0.3).into());
            carvings.push(CarveGroove::line((bar_left, bar_y), (bar_left + (bar_right - bar_left) * fill, bar_y), 4.0 * unit).depth(0.9).into());
        }
        carvings
    }
}

#[cfg(test)]
mod tests {
    use super::ChainCarving;
    use crate::CarveBox;

    #[test]
    fn the_blockchain_section_keeps_inside_its_area() {
        let area = CarveBox::new(44.0, 680.0, 830.0, 860.0);

        assert!(ChainCarving::new(1.0).carve(area).iter().all(|carving| area.grow(2.0).contains(carving.bounds())));
    }
}
