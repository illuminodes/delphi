#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrandTypeface {
    pub family: &'static str,
    pub role: &'static str,
    pub stack: &'static str,
    pub css_var: &'static str,
    pub specimen: &'static str,
    pub note: &'static str,
}

impl BrandTypeface {
    pub const ALL: [Self; 3] = [
        Self {
            family: "Marcellus SC",
            role: "Display",
            stack: "\"Marcellus SC\", \"Trajan Pro\", serif",
            css_var: "--delphi-font-display",
            specimen: "DELPHI · THE ORACLE SPEAKS",
            note: "Flared small caps in a single weight. Wordmark, section heads, window names, buttons. Tracked wide.",
        },
        Self {
            family: "Alegreya Sans",
            role: "Text",
            stack: "\"Alegreya Sans\", \"Gill Sans\", sans-serif",
            css_var: "--delphi-font-text",
            specimen: "The oracle reads the chain and the mempool, and publishes one price per window.",
            note: "Humanist sans with calligraphic strokes. Labels, body copy, controls. Legible down to 12px.",
        },
        Self {
            family: "IBM Plex Mono",
            role: "Figures",
            stack: "\"IBM Plex Mono\", ui-monospace, monospace",
            css_var: "--delphi-font-figures",
            specimen: "0123456789  $00,000.00  #000000  0f1e2d3c",
            note: "Tabular figures. Prices, block heights, hashes, timestamps.",
        },
    ];

    #[must_use]
    pub fn font_query(&self) -> String {
        self.family.replace(' ', "+")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::BrandTypeface;

    const STYLESHEET: &str = include_str!("../styles/delphi.css");

    #[test]
    fn every_face_is_loaded_by_the_stylesheet() {
        for face in &BrandTypeface::ALL {
            let query = format!("family={}", face.font_query());
            assert!(STYLESHEET.contains(&query), "{} is not imported", face.family);
        }
    }

    #[test]
    fn every_face_is_bound_to_its_css_variable() {
        for face in &BrandTypeface::ALL {
            let declaration = format!("{}: {};", face.css_var, face.stack);
            assert!(STYLESHEET.contains(&declaration), "missing `{declaration}` in delphi.css");
        }
    }

    #[test]
    fn each_role_has_one_face() {
        let roles: HashSet<&str> = BrandTypeface::ALL.iter().map(|face| face.role).collect();

        assert_eq!(roles.len(), BrandTypeface::ALL.len());
    }

    #[test]
    fn every_stack_starts_with_its_family() {
        for face in &BrandTypeface::ALL {
            assert!(face.stack.starts_with(&format!("\"{}\"", face.family)));
        }
    }

    #[test]
    fn the_stylesheet_imports_no_unused_faces() {
        let imported = STYLESHEET.matches("family=").count();

        assert_eq!(imported, BrandTypeface::ALL.len());
    }
}
