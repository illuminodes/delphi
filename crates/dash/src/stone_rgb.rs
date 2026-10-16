use crate::HexColor;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StoneRgb([f64; 3]);

impl StoneRgb {
    #[must_use]
    pub const fn new(channels: [f64; 3]) -> Self {
        Self(channels)
    }

    #[must_use]
    pub const fn channels(self) -> [f64; 3] {
        self.0
    }

    #[must_use]
    pub fn mix(self, other: Self, amount: f64) -> Self {
        Self([0, 1, 2].map(|channel| self.0[channel] + (other.0[channel] - self.0[channel]) * amount))
    }

    #[must_use]
    pub fn lit(self, factor: f64) -> Self {
        Self(self.0.map(|channel| channel * factor))
    }

    #[must_use]
    pub fn frosted(self, frost: Self) -> Self {
        Self([0, 1, 2].map(|channel| self.0[channel] + frost.0[channel] * (1.0 - self.0[channel] / 255.0)))
    }

    #[must_use]
    pub fn dulled(self, amount: f64) -> Self {
        let grey = 0.2126 * self.0[0] + 0.7152 * self.0[1] + 0.0722 * self.0[2];
        self.mix(Self([grey; 3]), amount)
    }

    #[must_use]
    pub fn shouldered(self, knee: f64) -> Self {
        let room = 255.0 - knee;
        Self(self.0.map(|channel| if channel <= knee { channel } else { knee + room * (1.0 - (-(channel - knee) / room).exp()) }))
    }

    #[must_use]
    pub fn lightness(self) -> f64 {
        (0.2126 * self.0[0] + 0.7152 * self.0[1] + 0.0722 * self.0[2]) / 255.0
    }

    #[must_use]
    pub fn rgba(self) -> [u8; 4] {
        let [red, green, blue] = self.0.map(|channel| channel.round().clamp(0.0, 255.0) as u8);
        [red, green, blue, 255]
    }
}

impl From<HexColor> for StoneRgb {
    fn from(colour: HexColor) -> Self {
        Self(colour.channels().map(f64::from))
    }
}

#[cfg(test)]
mod tests {
    use super::StoneRgb;
    use crate::HexColor;

    #[test]
    fn mixing_runs_from_one_colour_to_the_other() {
        let (black, white) = (StoneRgb::new([0.0; 3]), StoneRgb::new([255.0; 3]));

        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, 0.5).rgba(), [128, 128, 128, 255]);
    }

    #[test]
    fn frosting_lightens_dark_stone_and_keeps_a_vein_brighter_than_its_ground() {
        let frost = StoneRgb::new([60.0; 3]);
        let (ground, vein) = (StoneRgb::new([20.0; 3]).frosted(frost), StoneRgb::new([200.0; 3]).frosted(frost));

        assert!(ground.channels()[0] > 20.0);
        assert!(vein.channels()[0] > ground.channels()[0] + 100.0);
        assert_eq!(StoneRgb::new([255.0; 3]).frosted(frost), StoneRgb::new([255.0; 3]));
    }

    #[test]
    fn dulling_pulls_a_colour_towards_its_own_grey() {
        let red = StoneRgb::new([200.0, 40.0, 40.0]);
        let [r, g, b] = red.dulled(1.0).channels();

        assert!((r - g).abs() < 1e-9 && (g - b).abs() < 1e-9);
        assert_eq!(red.dulled(0.0), red);
    }

    #[test]
    fn a_shoulder_keeps_mid_tones_and_bends_overbright_ones_under_white_in_order() {
        let shoulder = |value: f64| StoneRgb::new([value; 3]).shouldered(200.0).channels()[0];

        assert!((shoulder(150.0) - 150.0).abs() < 1e-9);
        assert!(shoulder(260.0) < 255.0 && shoulder(400.0) < 255.0);
        assert!(shoulder(230.0) < shoulder(260.0) && shoulder(260.0) < shoulder(400.0));
    }

    #[test]
    fn pixels_are_rounded_and_clamped_and_opaque() {
        assert_eq!(StoneRgb::new([-4.0, 300.0, 12.6]).rgba(), [0, 255, 13, 255]);
    }

    #[test]
    fn a_hex_colour_keeps_its_channels() {
        let bronze = StoneRgb::from(HexColor::rgb([180, 130, 63]));

        assert_eq!(bronze.rgba(), [180, 130, 63, 255]);
        assert!((StoneRgb::new([255.0; 3]).lightness() - 1.0).abs() < 1e-9);
    }
}
