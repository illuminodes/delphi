use crate::HexColor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EtchRecipe {
    pub depth_tone: f64,
    pub frost: HexColor,
    pub relief_frost: HexColor,
    pub dullness: f64,
    pub incised_tone: f64,
    pub grain: f64,
    pub ragged: f64,
    pub shadow: f64,
    pub shadow_softness: f64,
    pub shadow_offset: f64,
    pub highlight: f64,
    pub vee_slope: f64,
    pub vee_floor: f64,
    pub ambient: f64,
    pub relief_tone: f64,
    pub line_vee: f64,
    pub parallax: f64,
    pub grain_keep: f64,
    pub thicken: f64,
    pub rim: f64,
    pub light_tint: HexColor,
    pub gilt: HexColor,
    pub gilt_amount: f64,
}
