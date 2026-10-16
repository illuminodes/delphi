#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhotoCrop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PhotoCrop {
    #[must_use]
    pub fn cover(photo: (f64, f64), target: (f64, f64)) -> Self {
        let (photo_width, photo_height) = photo;
        let (target_width, target_height) = target;
        if photo_width <= 0.0 || photo_height <= 0.0 || target_width <= 0.0 || target_height <= 0.0 {
            return Self { x: 0.0, y: 0.0, width: photo_width.max(0.0), height: photo_height.max(0.0) };
        }
        let zoom = (target_width / photo_width).max(target_height / photo_height);
        let (width, height) = (target_width / zoom, target_height / zoom);
        Self { x: (photo_width - width) / 2.0, y: (photo_height - height) / 2.0, width, height }
    }
}

#[cfg(test)]
mod tests {
    use super::PhotoCrop;

    #[test]
    fn a_wider_photo_is_trimmed_at_both_sides_and_keeps_its_full_height() {
        let crop = PhotoCrop::cover((2000.0, 1000.0), (1000.0, 1000.0));

        assert_eq!(crop, PhotoCrop { x: 500.0, y: 0.0, width: 1000.0, height: 1000.0 });
    }

    #[test]
    fn a_taller_target_takes_a_centred_column_of_the_photo() {
        let crop = PhotoCrop::cover((1900.0, 1200.0), (600.0, 1200.0));

        assert!((crop.width - 600.0).abs() < 1e-9 && (crop.height - 1200.0).abs() < 1e-9);
        assert!((crop.x - 650.0).abs() < 1e-9 && crop.y.abs() < 1e-9);
    }

    #[test]
    fn the_crop_keeps_the_target_aspect() {
        let crop = PhotoCrop::cover((1900.0, 1200.0), (1440.0, 900.0));

        assert!((crop.width / crop.height - 1440.0 / 900.0).abs() < 1e-9);
        assert!(crop.x >= 0.0 && crop.y >= 0.0 && crop.x + crop.width <= 1900.0 && crop.y + crop.height <= 1200.0);
    }
}
