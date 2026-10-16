#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChalkTone {
    Chalk,
    Ochre,
    Vapour,
    Hearth,
}

impl ChalkTone {
    pub const ALL: [Self; 4] = [Self::Chalk, Self::Ochre, Self::Vapour, Self::Hearth];

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Chalk => "chalk",
            Self::Ochre => "ochre",
            Self::Vapour => "vapour",
            Self::Hearth => "hearth",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Chalk => "Chalk",
            Self::Ochre => "Ochre",
            Self::Vapour => "Vapour",
            Self::Hearth => "Hearth",
        }
    }

    #[must_use]
    pub fn class(self) -> String {
        format!("tone-{}", self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::ChalkTone;

    const STYLESHEET: &str = include_str!("../styles/marble.css");

    #[test]
    fn every_tone_sets_the_chalk_ink() {
        for tone in ChalkTone::ALL {
            let rule = format!(".marble-slab.{} {{ --chalk-ink:", tone.class());
            assert!(STYLESHEET.contains(&rule), "missing `{rule}`");
        }
    }

    #[test]
    fn every_tone_has_a_colour_on_every_stone() {
        for tone in ChalkTone::ALL {
            let variable = format!("--slab-{}:", tone.key());
            assert_eq!(STYLESHEET.matches(&variable).count(), 3, "{variable}");
        }
    }
}
