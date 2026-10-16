use crate::PhotoCrop;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LimestonePatch {
    pub x: f64,
    pub y: f64,
    pub texel: f64,
    pub mirrored: bool,
    pub flipped: bool,
}

impl LimestonePatch {
    #[must_use]
    pub fn crop(&self, photo: (f64, f64), target: (f64, f64)) -> PhotoCrop {
        let width = (target.0 * self.texel).min(photo.0);
        let height = (target.1 * self.texel).min(photo.1);
        let x = (self.x * photo.0).min(photo.0 - width).max(0.0);
        let y = (self.y * photo.1).min(photo.1 - height).max(0.0);
        PhotoCrop { x, y, width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::LimestonePatch;

    #[test]
    fn the_crop_keeps_a_fixed_texel_size_and_stays_inside_the_photo() {
        let patch = LimestonePatch { x: 0.9, y: 0.2, texel: 1.5, mirrored: false, flipped: false };
        let small = patch.crop((1000.0, 1000.0), (100.0, 50.0));
        let large = patch.crop((1000.0, 1000.0), (300.0, 150.0));

        assert!((small.width - 150.0).abs() < 1e-9 && (large.width - 450.0).abs() < 1e-9);
        assert!(large.x + large.width <= 1000.0 + 1e-9 && small.y >= 0.0);
    }

    #[test]
    fn a_target_larger_than_the_photo_takes_the_whole_photo() {
        let patch = LimestonePatch { x: 0.5, y: 0.5, texel: 4.0, mirrored: false, flipped: false };
        let crop = patch.crop((800.0, 600.0), (400.0, 300.0));

        assert!(crop.x.abs() < 1e-9 && (crop.width - 800.0).abs() < 1e-9 && (crop.height - 600.0).abs() < 1e-9);
    }
}
