use crate::{EtchRecipe, HexColor, LimestonePatch, SplitMix};

pub struct Limestone;

impl Limestone {
    pub const PHOTO: &'static str = "oracle/limestone-albedo.jpg";
    pub const JOINT: HexColor = HexColor::rgb([0x14, 0x10, 0x0d]);
    const TEXEL: (f64, f64) = (0.9, 1.4);
    const COOL: [f64; 3] = [0.97, 1.0, 1.04];
    const WARM: [f64; 3] = [1.08, 0.98, 0.88];
    const WARM_SHARE: f64 = 0.35;
    pub const ETCH: EtchRecipe = EtchRecipe {
        depth_tone: 0.82,
        frost: HexColor::rgb([0x00, 0x00, 0x00]),
        relief_frost: HexColor::rgb([0x00, 0x00, 0x00]),
        dullness: 0.2,
        incised_tone: 0.5,
        grain: 0.1,
        ragged: 0.4,
        shadow: 0.8,
        shadow_softness: 1.0,
        shadow_offset: 1.3,
        highlight: 0.45,
        vee_slope: 1.3,
        vee_floor: 6.0,
        ambient: 0.35,
        relief_tone: 0.85,
        line_vee: 0.0,
        parallax: 1.5,
        grain_keep: 1.5,
        thicken: 1.0,
        rim: 0.2,
        light_tint: HexColor::rgb([0xff, 0xf4, 0xe6]),
        gilt: HexColor::rgb([0x8c, 0x6a, 0x34]),
        gilt_amount: 0.55,
    };

    #[must_use]
    pub fn patch(seed: u64) -> LimestonePatch {
        let mut random = SplitMix::new(seed);
        let texel = Self::TEXEL.0 + (Self::TEXEL.1 - Self::TEXEL.0) * random.unit();
        LimestonePatch { x: random.unit(), y: random.unit(), texel, mirrored: random.unit() < 0.5, flipped: random.unit() < 0.5 }
    }

    #[must_use]
    pub fn tone(seed: u64) -> f64 {
        let mut random = SplitMix::new(seed ^ 0x70E5);
        0.86 + 0.22 * random.unit()
    }

    #[must_use]
    pub fn tint(seed: u64) -> [f64; 3] {
        let mut random = SplitMix::new(seed ^ 0x7147);
        let warmth = if random.unit() < Self::WARM_SHARE { 0.5 + 0.5 * random.unit() } else { 0.3 * random.unit() };
        [0, 1, 2].map(|channel| Self::COOL[channel] + (Self::WARM[channel] - Self::COOL[channel]) * warmth)
    }
}

#[cfg(test)]
mod tests {
    use super::Limestone;

    #[test]
    fn stones_take_different_places_and_orientations_in_the_photo() {
        let patches: Vec<_> = (0..40).map(Limestone::patch).collect();

        assert!(patches.iter().all(|patch| (0.0..=1.0).contains(&patch.x) && (0.0..=1.0).contains(&patch.y)));
        assert!(patches.iter().any(|patch| (patch.x - patches[0].x).abs() > 0.2));
        assert!(patches.iter().any(|patch| patch.mirrored) && patches.iter().any(|patch| !patch.mirrored));
        assert!(patches.iter().any(|patch| patch.flipped) && patches.iter().any(|patch| !patch.flipped));
    }

    #[test]
    fn the_tone_varies_a_little_around_the_photo() {
        let tones: Vec<f64> = (0..40).map(Limestone::tone).collect();

        assert!(tones.iter().all(|tone| (0.86..=1.08).contains(tone)));
        assert!(tones.iter().any(|tone| (tone - tones[0]).abs() > 0.02));
    }

    #[test]
    fn some_stones_are_warm_and_most_are_cool() {
        let tints: Vec<[f64; 3]> = (0..60).map(Limestone::tint).collect();
        let warm = tints.iter().filter(|tint| tint[0] > tint[2] + 0.08).count();

        assert!((8..=35).contains(&warm), "{warm}");
    }

    #[test]
    fn the_photo_ships_with_the_app() {
        assert!(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("public").join(Limestone::PHOTO).is_file());
    }
}
