use crate::{BoxBlur, CarveRelief};

#[derive(Debug, Clone, PartialEq)]
pub struct CarveCavity {
    width: usize,
    occlusion: Vec<f32>,
}

impl CarveCavity {
    const REACH: f64 = 3.0;
    const PASSES: usize = 2;
    const DEPTH_SCALE: f64 = 2.5;
    const STRENGTH: f64 = 0.55;

    #[must_use]
    pub fn new(relief: &CarveRelief, scale: f64) -> Self {
        let (width, height) = (relief.width(), relief.height());
        let radius = (Self::REACH * scale).round().max(1.0) as usize;
        let around = BoxBlur::new(width, height, radius, Self::PASSES).blur(relief.depths());
        let occlusion = relief
            .depths()
            .iter()
            .zip(&around)
            .map(|(depth, around)| {
                let sunk = (f64::from(*depth) - f64::from(*around)).max(0.0) + f64::from(*depth) * 0.25;
                (1.0 - Self::STRENGTH * (1.0 - (-sunk / Self::DEPTH_SCALE).exp())) as f32
            })
            .collect();
        Self { width, occlusion }
    }

    #[must_use]
    pub fn at(&self, column: usize, row: usize) -> f64 {
        self.occlusion.get(row * self.width + column).map_or(1.0, |occlusion| f64::from(*occlusion))
    }

    pub fn shade(&self, pixels: &mut [u8]) {
        for (pixel, occlusion) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(&self.occlusion) {
            for channel in &mut pixel[..3] {
                *channel = (f64::from(*channel) * f64::from(*occlusion)).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CarveCavity;
    use crate::{CarveGroove, CarveMask, CarveRelief};

    struct CavityFixture;

    impl CavityFixture {
        fn groove() -> CarveCavity {
            let mut mask = CarveMask::new(60, 60, 1.0);
            mask.stamp_groove(&CarveGroove::line((30.0, 5.0), (30.0, 55.0), 10.0));
            CarveCavity::new(&CarveRelief::new(&mask, 1.8, 14.0), 1.0)
        }
    }

    #[test]
    fn polished_stone_is_not_darkened() {
        assert!((CavityFixture::groove().at(5, 30) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_floor_of_a_cut_is_darker_than_its_rim() {
        let cavity = CavityFixture::groove();

        assert!(cavity.at(30, 30) < 0.75, "{}", cavity.at(30, 30));
        assert!(cavity.at(30, 30) + 0.1 < cavity.at(26, 30), "floor {} rim {}", cavity.at(30, 30), cavity.at(26, 30));
    }

    #[test]
    fn shading_darkens_colour_and_keeps_alpha() {
        let cavity = CavityFixture::groove();
        let mut pixels = vec![200; 60 * 60 * 4];
        cavity.shade(&mut pixels);
        let floor = (30 * 60 + 30) * 4;

        assert!(pixels[floor] < 150, "{}", pixels[floor]);
        assert_eq!(pixels[floor + 3], 200);
        assert_eq!(pixels[(30 * 60 + 5) * 4], 200);
    }
}
