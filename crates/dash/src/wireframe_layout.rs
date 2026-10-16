use crate::CarveBox;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WireframeLayout {
    pub unit: f64,
    pub slab: CarveBox,
    pub header: CarveBox,
    pub info: CarveBox,
    pub price: CarveBox,
    pub history: CarveBox,
    pub fit: CarveBox,
    pub chain: CarveBox,
    pub mempool: CarveBox,
}

impl WireframeLayout {
    const DESIGN_WIDTH: f64 = 1440.0;
    const DESIGN_HEIGHT: f64 = 900.0;
    const MIN_UNIT: f64 = 0.6;
    const MAX_UNIT: f64 = 1.6;
    const MARGIN: f64 = 44.0;
    const GUTTER: f64 = 40.0;
    const HEADER: f64 = 96.0;

    #[must_use]
    pub fn new(width: f64, height: f64) -> Self {
        let unit = (width / Self::DESIGN_WIDTH).min(height / Self::DESIGN_HEIGHT).clamp(Self::MIN_UNIT, Self::MAX_UNIT);
        let (margin, gutter) = (Self::MARGIN * unit, Self::GUTTER * unit);
        let slab = CarveBox::new(0.0, 0.0, width, height);
        let inner = CarveBox::new(margin, margin * 0.6, width - margin, height - margin * 0.8);
        let header = CarveBox::new(inner.left, inner.top, inner.right, inner.top + Self::HEADER * unit);
        let body_top = header.bottom + gutter * 0.6;
        let upper_bottom = body_top + (inner.bottom - body_top) * 0.62;
        let columns = Self::split(inner.left, inner.right, &[0.22, 0.52, 0.26], gutter);
        let centre_split = body_top + (upper_bottom - body_top) * 0.36;
        let lower = Self::split(inner.left, inner.right, &[0.6, 0.4], gutter);
        let lower_top = upper_bottom + gutter;
        Self {
            unit,
            slab,
            header,
            info: CarveBox::new(columns[0].0, body_top, columns[0].1, upper_bottom),
            price: CarveBox::new(columns[1].0, body_top, columns[1].1, centre_split - gutter * 0.5),
            history: CarveBox::new(columns[1].0, centre_split + gutter * 0.5, columns[1].1, upper_bottom),
            fit: CarveBox::new(columns[2].0, body_top, columns[2].1, upper_bottom),
            chain: CarveBox::new(lower[0].0, lower_top, lower[0].1, inner.bottom),
            mempool: CarveBox::new(lower[1].0, lower_top, lower[1].1, inner.bottom),
        }
    }

    #[must_use]
    pub fn sections(&self) -> [CarveBox; 7] {
        [self.header, self.info, self.price, self.history, self.fit, self.chain, self.mempool]
    }

    fn split(left: f64, right: f64, shares: &[f64], gutter: f64) -> Vec<(f64, f64)> {
        let usable = right - left - gutter * (shares.len() - 1) as f64;
        let total: f64 = shares.iter().sum();
        let mut cursor = left;
        shares
            .iter()
            .map(|share| {
                let start = cursor;
                cursor += usable * share / total;
                let span = (start, cursor);
                cursor += gutter;
                span
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::WireframeLayout;

    #[test]
    fn every_section_lies_on_the_slab_and_none_overlap() {
        for (width, height) in [(1440.0, 900.0), (1280.0, 720.0), (1920.0, 1080.0), (2560.0, 1440.0)] {
            let layout = WireframeLayout::new(width, height);
            let sections = layout.sections();
            for (index, section) in sections.iter().enumerate() {
                assert!(layout.slab.contains(*section), "{width}x{height} section {index}");
                assert!(section.width() > 0.0 && section.height() > 0.0);
                for other in &sections[index + 1..] {
                    assert!(!section.overlaps(*other), "{width}x{height} section {index} overlaps");
                }
            }
        }
    }

    #[test]
    fn the_design_size_is_drawn_at_unit_scale_and_bigger_screens_scale_up() {
        assert!((WireframeLayout::new(1440.0, 900.0).unit - 1.0).abs() < 1e-9);
        assert!(WireframeLayout::new(2880.0, 1800.0).unit > 1.0);
    }
}
