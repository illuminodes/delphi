use crate::{EtchRecipe, HexColor, StoneRecipe};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarbleKind {
    Nero,
    Parian,
    Aurum,
}

impl MarbleKind {
    pub const ALL: [Self; 3] = [Self::Nero, Self::Parian, Self::Aurum];

    const NERO_BANDS: [HexColor; 4] = [
        HexColor::rgb([0x17, 0x17, 0x19]),
        HexColor::rgb([0x1f, 0x1f, 0x22]),
        HexColor::rgb([0x12, 0x12, 0x14]),
        HexColor::rgb([0x26, 0x25, 0x27]),
    ];
    const PARIAN_BANDS: [HexColor; 4] = [
        HexColor::rgb([0xf4, 0xf1, 0xec]),
        HexColor::rgb([0xe9, 0xe4, 0xdc]),
        HexColor::rgb([0xf8, 0xf6, 0xf2]),
        HexColor::rgb([0xdf, 0xd9, 0xd0]),
    ];
    const AURUM_BANDS: [HexColor; 8] = [
        HexColor::rgb([0xc9, 0x93, 0x4f]),
        HexColor::rgb([0xe0, 0xb8, 0x7c]),
        HexColor::rgb([0xa8, 0x6c, 0x30]),
        HexColor::rgb([0xd4, 0xa5, 0x62]),
        HexColor::rgb([0xee, 0xd5, 0xa4]),
        HexColor::rgb([0xb5, 0x7a, 0x3a]),
        HexColor::rgb([0x8e, 0x52, 0x22]),
        HexColor::rgb([0xd8, 0xab, 0x6a]),
    ];

    #[must_use]
    pub const fn photo(self) -> Option<&'static str> {
        match self {
            Self::Nero => Some("stone/nero.jpg"),
            Self::Parian => Some("stone/parian.jpg"),
            Self::Aurum => None,
        }
    }

    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Nero => "nero",
            Self::Parian => "parian",
            Self::Aurum => "aurum",
        }
    }

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Nero => "Nero",
            Self::Parian => "Parian",
            Self::Aurum => "Aurum",
        }
    }

    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::Nero => "Black marble with ash-grey veins",
            Self::Parian => "White temple marble with faint veins",
            Self::Aurum => "Banded honey onyx",
        }
    }

    #[must_use]
    pub const fn etch(self) -> EtchRecipe {
        match self {
            Self::Nero => EtchRecipe {
                depth_tone: 1.0,
                frost: HexColor::rgb([0x24, 0x24, 0x26]),
                relief_frost: HexColor::rgb([0x10, 0x11, 0x14]),
                dullness: 0.1,
                incised_tone: 1.5,
                grain: 0.12,
                ragged: 0.35,
                shadow: 0.85,
                shadow_softness: 1.0,
                shadow_offset: 1.3,
                highlight: 0.6,
                vee_slope: 1.4,
                vee_floor: 6.0,
                ambient: 0.35,
                relief_tone: 1.6,
                line_vee: 0.0,
                parallax: 1.5,
                grain_keep: 1.5,
                thicken: 1.2,
                rim: 0.06,
                light_tint: HexColor::rgb([0xd8, 0xd2, 0xc6]),
                gilt: HexColor::rgb([0x96, 0x74, 0x34]),
                gilt_amount: 1.0,
            },
            Self::Parian => EtchRecipe {
                depth_tone: 0.97,
                frost: HexColor::rgb([0x00, 0x00, 0x00]),
                relief_frost: HexColor::rgb([0x00, 0x00, 0x00]),
                dullness: 0.15,
                incised_tone: 0.4,
                grain: 0.06,
                ragged: 0.35,
                shadow: 0.75,
                shadow_softness: 1.0,
                shadow_offset: 1.3,
                highlight: 0.7,
                vee_slope: 1.4,
                vee_floor: 6.0,
                ambient: 0.35,
                relief_tone: 0.95,
                line_vee: 0.6,
                parallax: 1.5,
                grain_keep: 2.5,
                thicken: 1.2,
                rim: 0.6,
                light_tint: HexColor::rgb([0xff, 0xff, 0xff]),
                gilt: HexColor::rgb([0xa8, 0x7e, 0x34]),
                gilt_amount: 1.0,
            },
            Self::Aurum => EtchRecipe {
                depth_tone: 0.75,
                frost: HexColor::rgb([0x00, 0x00, 0x00]),
                relief_frost: HexColor::rgb([0x00, 0x00, 0x00]),
                dullness: 0.3,
                incised_tone: 0.55,
                grain: 0.08,
                ragged: 0.3,
                shadow: 0.7,
                shadow_softness: 1.0,
                shadow_offset: 1.4,
                highlight: 0.4,
                vee_slope: 1.0,
                vee_floor: 6.0,
                ambient: 0.3,
                relief_tone: 0.9,
                line_vee: 0.0,
                parallax: 1.5,
                grain_keep: 1.0,
                thicken: 1.2,
                rim: 0.4,
                light_tint: HexColor::rgb([0xff, 0xf2, 0xd6]),
                gilt: HexColor::rgb([0xf0, 0xd6, 0x8a]),
                gilt_amount: 0.8,
            },
        }
    }

    #[must_use]
    pub const fn recipe(self) -> StoneRecipe {
        match self {
            Self::Nero => StoneRecipe {
                base: HexColor::rgb([0x1a, 0x1a, 0x1c]),
                bands: &Self::NERO_BANDS,
                band_spacing: 15.0,
                band_softness: 1.0,
                clouding: 0.25,
                cloudiness: 1.0,
                turbulence: 16.0,
                branch_colour: HexColor::rgb([0xb8, 0xb2, 0xa8]),
                branch_count: 7,
                branch_width: 1.3,
                vein_core: HexColor::rgb([0xec, 0xe6, 0xda]),
                vein_rim: HexColor::rgb([0x05, 0x05, 0x06]),
                vein_width: 3.2,
                rim_strength: 0.7,
            },
            Self::Parian => StoneRecipe {
                base: HexColor::rgb([0xf3, 0xf1, 0xef]),
                bands: &Self::PARIAN_BANDS,
                band_spacing: 17.0,
                band_softness: 1.0,
                clouding: 0.03,
                cloudiness: 1.0,
                turbulence: 18.0,
                branch_colour: HexColor::rgb([0x8a, 0x84, 0x7c]),
                branch_count: 7,
                branch_width: 1.1,
                vein_core: HexColor::rgb([0x4d, 0x46, 0x40]),
                vein_rim: HexColor::rgb([0xff, 0xff, 0xff]),
                vein_width: 2.8,
                rim_strength: 0.6,
            },
            Self::Aurum => StoneRecipe {
                base: HexColor::rgb([0xc4, 0x96, 0x5a]),
                bands: &Self::AURUM_BANDS,
                band_spacing: 7.0,
                band_softness: 0.95,
                clouding: 0.08,
                cloudiness: 0.95,
                turbulence: 7.0,
                branch_colour: HexColor::rgb([0xf3, 0xe2, 0xbd]),
                branch_count: 6,
                branch_width: 1.1,
                vein_core: HexColor::rgb([0xf8, 0xee, 0xd8]),
                vein_rim: HexColor::rgb([0x4a, 0x26, 0x0c]),
                vein_width: 3.4,
                rim_strength: 0.8,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::MarbleKind;
    use crate::{HexColor, StoneRgb};

    const STYLESHEET: &str = include_str!("../styles/marble.css");

    #[test]
    fn keys_are_unique() {
        let keys: HashSet<&str> = MarbleKind::ALL.iter().map(|kind| kind.key()).collect();

        assert_eq!(keys.len(), MarbleKind::ALL.len());
    }

    #[test]
    fn the_slab_shows_the_stone_base_colour_before_the_canvas_paints() {
        for kind in MarbleKind::ALL {
            let rule = format!(".marble-slab.{} {{\n    --slab-base: {};", kind.key(), kind.recipe().base.hex());
            assert!(STYLESHEET.contains(&rule), "missing `{rule}`");
        }
    }

    #[test]
    fn the_carved_stones_have_a_photo_shipped_with_the_app() {
        for kind in [MarbleKind::Nero, MarbleKind::Parian] {
            let photo = kind.photo().unwrap_or_else(|| panic!("{} has no photo", kind.key()));
            assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("public").join(photo).is_file(), "{photo}");
        }
    }

    #[test]
    fn every_stone_has_bands() {
        assert!(MarbleKind::ALL.iter().all(|kind| kind.recipe().bands.len() >= 2));
    }

    #[test]
    fn the_vein_core_stands_out_from_the_stone_as_a_whole() {
        for kind in MarbleKind::ALL {
            let recipe = kind.recipe();
            let total = recipe.bands.iter().fold([0.0; 3], |sum, band| {
                let channels = StoneRgb::from(*band).channels();
                [sum[0] + channels[0], sum[1] + channels[1], sum[2] + channels[2]]
            });
            let mean = StoneRgb::new(total.map(|sum| sum / recipe.bands.len() as f64)).rgba();
            let gap = recipe.vein_core.contrast(HexColor::rgb([mean[0], mean[1], mean[2]]));
            assert!(gap >= 1.8, "{}: {gap:.2}", kind.key());
        }
    }

    #[test]
    fn the_vein_core_or_its_rim_stands_out_from_every_band() {
        for kind in MarbleKind::ALL {
            let recipe = kind.recipe();
            for band in recipe.bands {
                let gap = recipe.vein_core.contrast(*band).max(recipe.vein_rim.contrast(*band));
                assert!(gap >= 3.0, "{} against {}: {gap:.2}", kind.key(), band.hex());
            }
        }
    }
}
