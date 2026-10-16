use crate::BrandTypeface;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarveFont {
    Display,
    Text,
    Figures,
}

impl CarveFont {
    pub const ALL: [Self; 3] = [Self::Display, Self::Text, Self::Figures];

    #[must_use]
    pub const fn face(self) -> BrandTypeface {
        match self {
            Self::Display => BrandTypeface::ALL[0],
            Self::Text => BrandTypeface::ALL[1],
            Self::Figures => BrandTypeface::ALL[2],
        }
    }

    #[must_use]
    pub fn css(self, size: f64) -> String {
        format!("{size:.1}px {}", self.face().stack)
    }

    #[must_use]
    pub const fn depth(self) -> f64 {
        match self {
            Self::Display | Self::Text => 1.0,
            Self::Figures => 0.6,
        }
    }

    #[must_use]
    pub const fn advance(self) -> f64 {
        match self {
            Self::Display => 0.74,
            Self::Text => 0.5,
            Self::Figures => 0.6,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CarveFont;

    #[test]
    fn each_cut_font_is_the_brand_face_for_its_role() {
        assert_eq!(CarveFont::Display.face().role, "Display");
        assert_eq!(CarveFont::Text.face().role, "Text");
        assert_eq!(CarveFont::Figures.face().role, "Figures");
    }

    #[test]
    fn figures_are_cut_shallower_than_lettering() {
        assert!(CarveFont::Figures.depth() < CarveFont::Display.depth());
        assert!((CarveFont::Text.depth() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn the_canvas_font_names_the_size_and_the_brand_stack() {
        assert_eq!(CarveFont::Figures.css(12.0), "12.0px \"IBM Plex Mono\", ui-monospace, monospace");
    }
}
