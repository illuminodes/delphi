use crate::HexColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrandCamp {
    Ground,
    Apollo,
    Pythia,
}

impl BrandCamp {
    pub const ALL: [Self; 3] = [Self::Ground, Self::Apollo, Self::Pythia];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Ground => "ground",
            Self::Apollo => "apollo",
            Self::Pythia => "pythia",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Ground => "The Ground",
            Self::Apollo => "Apollo",
            Self::Pythia => "Pythia",
        }
    }

    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Ground => "Night, ash and bone. Backgrounds, surfaces, borders and text.",
            Self::Apollo => "Gold, sunlight and the temple hearth. The chain: the answer, settled and final.",
            Self::Pythia => "The chasm, its vapours and the Castalian spring. The mempool: the trance, live and unsettled.",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwatchUse {
    Ink,
    Fill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrandSwatch {
    pub camp: BrandCamp,
    pub name: &'static str,
    pub hex: &'static str,
    pub role: &'static str,
    pub usage: SwatchUse,
}

impl BrandSwatch {
    const fn new(camp: BrandCamp, name: &'static str, hex: &'static str, role: &'static str, usage: SwatchUse) -> Self {
        Self {
            camp,
            name,
            hex,
            role,
            usage,
        }
    }

    #[must_use]
    pub fn css_var(&self) -> String {
        format!("--delphi-{}", self.name)
    }

    #[must_use]
    pub fn color(&self) -> Option<HexColor> {
        HexColor::parse(self.hex)
    }
}

pub struct BrandPalette;

impl BrandPalette {
    pub const SWATCHES: [BrandSwatch; 15] = [
        BrandSwatch::new(BrandCamp::Ground, "obsidian", "#0c0a09", "Page background", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Ground, "iron", "#171412", "Card surface", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Ground, "soot", "#221d1a", "Raised surface, inputs", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Ground, "ash", "#3a312b", "Borders and dividers", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Ground, "dust", "#9c8d7b", "Muted text, labels", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Ground, "bone", "#e8dfcf", "Primary text", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Apollo, "aurum", "#c9a24a", "Primary accent, chain windows", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Apollo, "helios", "#e6c875", "Accent hover, headings", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Apollo, "altar", "#8a1e1e", "Danger fills", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Apollo, "hearth", "#e8604a", "Price down, errors, stale", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Pythia, "chasm", "#2a1d45", "Mempool surfaces, the mark", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Pythia, "pneuma", "#4a2d6b", "Selection, highlight fills", SwatchUse::Fill),
        BrandSwatch::new(BrandCamp::Pythia, "vapour", "#a68fd0", "Mempool accent, live", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Pythia, "castalia", "#5aaba3", "Price up, connected", SwatchUse::Ink),
        BrandSwatch::new(BrandCamp::Pythia, "sulfur", "#cfc04a", "Warnings, connecting", SwatchUse::Ink),
    ];

    pub fn of(camp: BrandCamp) -> impl Iterator<Item = &'static BrandSwatch> {
        Self::SWATCHES.iter().filter(move |swatch| swatch.camp == camp)
    }

    #[must_use]
    pub fn find(name: &str) -> Option<&'static BrandSwatch> {
        Self::SWATCHES.iter().find(|swatch| swatch.name == name)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{BrandCamp, BrandPalette, SwatchUse};

    const STYLESHEET: &str = include_str!("../styles/delphi.css");
    const AA_TEXT: f64 = 4.5;
    const AAA_TEXT: f64 = 7.0;

    #[test]
    fn every_swatch_is_valid_hex() {
        for swatch in &BrandPalette::SWATCHES {
            assert!(swatch.color().is_some(), "{} has bad hex {}", swatch.name, swatch.hex);
        }
    }

    #[test]
    fn swatch_names_are_unique() {
        let names: HashSet<&str> = BrandPalette::SWATCHES.iter().map(|swatch| swatch.name).collect();

        assert_eq!(names.len(), BrandPalette::SWATCHES.len());
    }

    #[test]
    fn every_camp_has_swatches() {
        for camp in BrandCamp::ALL {
            assert!(BrandPalette::of(camp).count() >= 4, "{} is thin", camp.title());
        }
    }

    #[test]
    fn ink_swatches_pass_aa_on_every_ground_surface() {
        let surfaces = ["obsidian", "iron", "soot"].map(|name| BrandPalette::find(name).unwrap().color().unwrap());
        for swatch in BrandPalette::SWATCHES.iter().filter(|swatch| swatch.usage == SwatchUse::Ink) {
            for surface in surfaces {
                let ratio = swatch.color().unwrap().contrast(surface);
                assert!(ratio >= AA_TEXT, "{} reaches only {ratio:.2}", swatch.name);
            }
        }
    }

    #[test]
    fn primary_text_passes_aaa_on_the_page() {
        let bone = BrandPalette::find("bone").unwrap().color().unwrap();
        let obsidian = BrandPalette::find("obsidian").unwrap().color().unwrap();

        assert!(bone.contrast(obsidian) >= AAA_TEXT);
    }

    #[test]
    fn aurum_buttons_keep_dark_label_legible() {
        let aurum = BrandPalette::find("aurum").unwrap().color().unwrap();
        let obsidian = BrandPalette::find("obsidian").unwrap().color().unwrap();

        assert!(aurum.contrast(obsidian) >= AA_TEXT);
    }

    #[test]
    fn the_stylesheet_declares_every_swatch_with_the_same_hex() {
        for swatch in &BrandPalette::SWATCHES {
            let declaration = format!("{}: {};", swatch.css_var(), swatch.hex);
            assert!(STYLESHEET.contains(&declaration), "missing `{declaration}` in delphi.css");
        }
    }
}
