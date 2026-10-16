#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexColor {
    red: u8,
    green: u8,
    blue: u8,
}

impl HexColor {
    #[must_use]
    pub const fn rgb([red, green, blue]: [u8; 3]) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub const fn channels(self) -> [u8; 3] {
        [self.red, self.green, self.blue]
    }

    #[must_use]
    pub fn mix(self, other: Self, amount: f64) -> Self {
        let amount = amount.clamp(0.0, 1.0);
        let [one, two] = [self.channels(), other.channels()].map(|channels| channels.map(f64::from));
        Self::rgb([0, 1, 2].map(|at| (one[at] + (two[at] - one[at]) * amount).round() as u8))
    }

    #[must_use]
    pub fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.red, self.green, self.blue)
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.is_ascii() {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
        Some(Self {
            red: channel(0)?,
            green: channel(2)?,
            blue: channel(4)?,
        })
    }

    #[must_use]
    pub fn luminance(self) -> f64 {
        0.2126 * Self::linear(self.red) + 0.7152 * Self::linear(self.green) + 0.0722 * Self::linear(self.blue)
    }

    #[must_use]
    pub fn contrast(self, other: Self) -> f64 {
        let (one, two) = (self.luminance(), other.luminance());
        (one.max(two) + 0.05) / (one.min(two) + 0.05)
    }

    fn linear(channel: u8) -> f64 {
        let scaled = f64::from(channel) / 255.0;
        if scaled <= 0.040_45 {
            scaled / 12.92
        } else {
            ((scaled + 0.055) / 1.055).powf(2.4)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HexColor;

    #[test]
    fn parses_six_digit_hex_with_a_hash() {
        assert!(HexColor::parse("#b4823f").is_some());
        assert!(HexColor::parse("b4823f").is_none());
        assert!(HexColor::parse("#b48").is_none());
        assert!(HexColor::parse("#zz823f").is_none());
    }

    #[test]
    fn black_on_white_is_the_maximum_contrast() {
        let black = HexColor::parse("#000000").unwrap();
        let white = HexColor::parse("#ffffff").unwrap();

        assert!((black.contrast(white) - 21.0).abs() < 1e-9);
        assert!((white.contrast(black) - 21.0).abs() < 1e-9);
    }

    #[test]
    fn channels_and_hex_round_trip() {
        let bronze = HexColor::rgb([0xb4, 0x82, 0x3f]);

        assert_eq!(bronze.hex(), "#b4823f");
        assert_eq!(HexColor::parse(&bronze.hex()), Some(bronze));
        assert_eq!(bronze.channels(), [0xb4, 0x82, 0x3f]);
    }

    #[test]
    fn mixing_runs_from_one_colour_to_the_other() {
        let (black, white) = (HexColor::rgb([0, 0, 0]), HexColor::rgb([255, 255, 255]));

        assert_eq!(black.mix(white, 0.0), black);
        assert_eq!(black.mix(white, 1.0), white);
        assert_eq!(black.mix(white, 0.5), HexColor::rgb([128, 128, 128]));
        assert_eq!(black.mix(white, 3.0), white);
    }

    #[test]
    fn a_color_has_no_contrast_with_itself() {
        let bronze = HexColor::parse("#b4823f").unwrap();

        assert!((bronze.contrast(bronze) - 1.0).abs() < 1e-9);
    }
}
