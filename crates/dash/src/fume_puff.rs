use crate::HexColor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FumePuff {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    pub alpha: f64,
    pub colour: HexColor,
}

impl FumePuff {
    pub const FAINT: f64 = 0.004;

    #[must_use]
    pub fn visible(&self) -> bool {
        self.alpha > Self::FAINT && self.radius > 0.0
    }

    #[must_use]
    pub fn css_colour(&self, alpha: f64) -> String {
        let [red, green, blue] = self.colour.channels();
        format!("rgba({red},{green},{blue},{:.3})", (self.alpha * alpha).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::FumePuff;
    use crate::HexColor;

    #[test]
    fn a_faint_or_empty_puff_is_not_drawn() {
        let puff = FumePuff { x: 0.0, y: 0.0, radius: 4.0, alpha: 0.5, colour: HexColor::rgb([1, 2, 3]) };

        assert!(puff.visible());
        assert!(!FumePuff { alpha: 0.0, ..puff }.visible());
        assert!(!FumePuff { radius: 0.0, ..puff }.visible());
    }

    #[test]
    fn the_css_colour_scales_the_puffs_alpha() {
        let puff = FumePuff { x: 0.0, y: 0.0, radius: 4.0, alpha: 0.5, colour: HexColor::rgb([90, 171, 163]) };

        assert_eq!(puff.css_colour(0.5), "rgba(90,171,163,0.250)");
    }
}
