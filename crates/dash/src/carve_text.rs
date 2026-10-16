use crate::{CarveAlign, CarveBox, CarveCut, CarveFill, CarveFont};

#[derive(Debug, Clone, PartialEq)]
pub struct CarveText {
    pub text: String,
    pub font: CarveFont,
    pub size: f64,
    pub x: f64,
    pub y: f64,
    pub align: CarveAlign,
    pub tracking: f64,
    pub depth: f64,
    pub fill: CarveFill,
    pub cut: CarveCut,
}

impl CarveText {
    const ASCENT: f64 = 0.74;
    const DESCENT: f64 = 0.22;

    #[must_use]
    pub fn new(text: impl Into<String>, font: CarveFont, size: f64, x: f64, y: f64) -> Self {
        Self {
            text: text.into(),
            font,
            size,
            x,
            y,
            align: CarveAlign::Start,
            tracking: 0.0,
            depth: font.depth(),
            fill: CarveFill::Plain,
            cut: CarveCut::for_size(size),
        }
    }

    #[must_use]
    pub fn align(self, align: CarveAlign) -> Self {
        Self { align, ..self }
    }

    #[must_use]
    pub fn tracking(self, tracking: f64) -> Self {
        Self { tracking, ..self }
    }

    #[must_use]
    pub fn depth(self, depth: f64) -> Self {
        Self { depth, ..self }
    }

    #[must_use]
    pub fn gilt(self) -> Self {
        Self { fill: CarveFill::Gilt, ..self }
    }

    #[must_use]
    pub fn incised(self) -> Self {
        Self { cut: CarveCut::Incised, ..self }
    }

    #[must_use]
    pub fn mask_colour(&self) -> String {
        self.fill.mask_colour(self.depth, self.cut)
    }

    #[must_use]
    pub fn width(&self) -> f64 {
        self.text.chars().count() as f64 * self.size * (self.font.advance() + self.tracking)
    }

    #[must_use]
    pub fn bounds(&self) -> CarveBox {
        let left = self.align.left_of(self.x, self.width());
        CarveBox::new(left, self.y - self.size * Self::ASCENT, left + self.width(), self.y + self.size * Self::DESCENT)
    }
}

#[cfg(test)]
mod tests {
    use super::CarveText;
    use crate::{CarveAlign, CarveCut, CarveFill, CarveFont};

    #[test]
    fn text_is_a_full_depth_plain_cut_until_told_otherwise() {
        let text = CarveText::new("DELPHI", CarveFont::Display, 20.0, 0.0, 0.0);

        assert!((text.depth - 1.0).abs() < 1e-9);
        assert!((CarveText::new("903,412", CarveFont::Figures, 12.0, 0.0, 0.0).depth - CarveFont::Figures.depth()).abs() < 1e-9);
        assert_eq!(text.fill, CarveFill::Plain);
        assert_eq!(CarveText::new("1", CarveFont::Figures, 9.0, 0.0, 0.0).gilt().fill, CarveFill::Gilt);
    }

    #[test]
    fn the_bounds_rise_above_the_baseline_and_follow_the_alignment() {
        let text = CarveText::new("0000", CarveFont::Figures, 10.0, 100.0, 50.0).align(CarveAlign::End);
        let bounds = text.bounds();

        assert!((bounds.right - 100.0).abs() < 1e-9);
        assert!((bounds.width() - 24.0).abs() < 1e-9);
        assert!(bounds.top < 50.0 && bounds.bottom > 50.0);
    }

    #[test]
    fn the_size_of_the_letters_decides_how_they_are_cut() {
        assert_eq!(CarveText::new("ΔELPHI", CarveFont::Display, 46.0, 0.0, 0.0).cut, CarveCut::Relief);
        assert_eq!(CarveText::new("HOURLY", CarveFont::Display, 11.0, 0.0, 0.0).mask_colour(), "rgb(255, 0, 255)");
    }

    #[test]
    fn large_figures_can_keep_the_incised_cut() {
        let figures = CarveText::new("104,212", CarveFont::Figures, 30.0, 0.0, 0.0);

        assert_eq!(figures.cut, CarveCut::Relief);
        assert_eq!(figures.incised().cut, CarveCut::Incised);
    }

    #[test]
    fn tracking_widens_the_text() {
        let plain = CarveText::new("ORACLE", CarveFont::Display, 10.0, 0.0, 0.0);
        let tracked = plain.clone().tracking(0.2);

        assert!(tracked.width() > plain.width());
    }
}
