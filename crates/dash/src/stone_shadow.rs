use crate::{BoxBlur, StoneRelief};

#[derive(Debug, Clone, PartialEq)]
pub struct StoneShadow {
    alpha: Vec<f32>,
}

impl StoneShadow {
    const DROP: (f64, f64) = (2.5, 4.0);
    const SOFTNESS: f64 = 2.5;
    const DEPTH: f32 = 0.85;

    #[must_use]
    pub fn cast(relief: &StoneRelief, scale: f64) -> Self {
        let (width, height) = relief.size();
        let (dx, dy) = ((Self::DROP.0 * scale).round() as usize, (Self::DROP.1 * scale).round() as usize);
        let mut moved = vec![0.0_f32; width * height];
        for row in dy..height {
            for column in dx..width {
                moved[row * width + column] = relief.cover((row - dy) * width + column - dx) as f32;
            }
        }
        let radius = (Self::SOFTNESS * scale).round().max(1.0) as usize;
        let alpha = BoxBlur::new(width, height, radius, 2).blur(&moved).into_iter().map(|value| value * Self::DEPTH).collect();
        Self { alpha }
    }

    #[must_use]
    pub const fn reach() -> f64 {
        Self::DROP.1 + Self::SOFTNESS * 2.0
    }

    #[must_use]
    pub fn at(&self, index: usize) -> f64 {
        self.alpha.get(index).map_or(0.0, |alpha| f64::from(*alpha))
    }
}

#[cfg(test)]
mod tests {
    use super::StoneShadow;
    use crate::{FumePolygon, StoneRelief};

    #[test]
    fn the_shadow_falls_below_and_right_of_the_stone_and_not_above_or_left() {
        let outline = FumePolygon::new(vec![(10.0, 10.0), (90.0, 10.0), (90.0, 50.0), (10.0, 50.0)]);
        let relief = StoneRelief::carve(&outline, (110, 70), 1.0, 2);
        let shadow = StoneShadow::cast(&relief, 1.0);

        let edge = (0..70).rev().find(|row| relief.cover(row * 110 + 50) > 0.5).unwrap_or(0);
        let under = shadow.at((edge + 2) * 110 + 50);

        assert!(under > 0.35, "{under}");
        assert!(under > shadow.at((edge + 9) * 110 + 50));
        assert!(shadow.at(30 * 110 + 92) > 0.2, "{}", shadow.at(30 * 110 + 92));
        assert!(shadow.at(5 * 110 + 50) < 0.02);
        assert!(shadow.at(30 * 110 + 5) < 0.02);
    }
}
