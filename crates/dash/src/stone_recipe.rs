use crate::HexColor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneRecipe {
    pub base: HexColor,
    pub bands: &'static [HexColor],
    pub band_spacing: f64,
    pub band_softness: f64,
    pub clouding: f64,
    pub cloudiness: f64,
    pub turbulence: f64,
    pub branch_colour: HexColor,
    pub branch_count: usize,
    pub branch_width: f64,
    pub vein_core: HexColor,
    pub vein_rim: HexColor,
    pub vein_width: f64,
    pub rim_strength: f64,
}
