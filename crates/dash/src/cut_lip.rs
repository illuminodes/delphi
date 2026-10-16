use crate::CarveMask;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutLip {
    reach: usize,
}

impl CutLip {
    #[must_use]
    pub fn new(mask: &CarveMask) -> Self {
        Self { reach: mask.scale().round().max(1.0) as usize }
    }

    #[must_use]
    pub fn at(&self, mask: &CarveMask, column: usize, row: usize) -> f64 {
        if column < self.reach || row < self.reach {
            return 0.0;
        }
        let behind = mask.cut_at(column - self.reach, row - self.reach) - mask.grooved_at(column - self.reach, row - self.reach);
        (behind - mask.cut_at(column, row)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::CutLip;
    use crate::CarveMask;

    struct LipFixture;

    impl LipFixture {
        fn letter() -> CarveMask {
            let rgba: Vec<u8> = (0..20 * 20).flat_map(|index| if (5..10).contains(&(index % 20)) { [255, 0, 0, 255] } else { [0, 0, 0, 255] }).collect();
            CarveMask::from_rgba(20, 20, 1.0, &rgba)
        }
    }

    #[test]
    fn the_polished_edge_on_the_far_side_of_a_letter_from_the_light_catches_a_rim() {
        let mask = LipFixture::letter();
        let lip = CutLip::new(&mask);

        assert!((lip.at(&mask, 10, 8) - 1.0).abs() < 1e-6);
        assert!(lip.at(&mask, 4, 8).abs() < 1e-6);
        assert!(lip.at(&mask, 12, 8).abs() < 1e-6);
        assert!(lip.at(&mask, 7, 8).abs() < 1e-6);
    }
}
