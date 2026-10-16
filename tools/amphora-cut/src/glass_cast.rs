use image::{imageops, Rgba, Rgba32FImage, RgbaImage};

use crate::backdrop_mask::BackdropMask;

pub struct GlassCast<'a> {
    mask: &'a BackdropMask,
    frame: (usize, usize, usize, usize),
}

impl<'a> GlassCast<'a> {
    const TINT: [f32; 3] = [0.42, 0.47, 0.5];
    const OPACITY: f32 = 0.12;
    const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

    pub fn new(mask: &'a BackdropMask, frame: (usize, usize, usize, usize)) -> Self {
        Self { mask, frame }
    }

    pub fn render(&self, out_width: u32) -> RgbaImage {
        self.fill(out_width, Self::TINT, Self::OPACITY)
    }

    pub fn silhouette(&self, out_width: u32) -> RgbaImage {
        self.fill(out_width, Self::WHITE, 1.0)
    }

    fn fill(&self, out_width: u32, tint: [f32; 3], opacity: f32) -> RgbaImage {
        let (left, top, width, height) = self.frame;
        let glass = Rgba32FImage::from_fn(width as u32, height as u32, |x, y| {
            let alpha = self.mask.coverage(left + x as usize, top + height - 1 - y as usize) as f32 * opacity;
            Rgba([tint[0] * alpha, tint[1] * alpha, tint[2] * alpha, alpha])
        });
        let out_height = (f64::from(out_width) * height as f64 / width as f64).round() as u32;
        let small = imageops::resize(&glass, out_width, out_height, imageops::FilterType::Lanczos3);
        RgbaImage::from_fn(out_width, out_height, |x, y| {
            let [red, green, blue, alpha] = small.get_pixel(x, y).0.map(|channel| channel.clamp(0.0, 1.0));
            let straight = |channel: f32| if alpha > 0.0 { (channel / alpha * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
            Rgba([straight(red), straight(green), straight(blue), (alpha * 255.0).round() as u8])
        })
    }
}

#[cfg(test)]
mod tests {
    use image::RgbImage;

    use super::GlassCast;
    use crate::backdrop_mask::BackdropMask;

    #[test]
    fn the_glass_is_one_even_faint_tint_across_the_whole_body() {
        let photo = RgbImage::from_fn(60, 60, |x, y| if (10..50).contains(&x) && (10..50).contains(&y) { image::Rgb([40, 30, 20]) } else { image::Rgb([230, 230, 230]) });
        let mask = BackdropMask::flood_keeping(&photo, BackdropMask::BRIGHT, BackdropMask::POCKET_SHARE);
        let (left, top, right, bottom) = mask.bounds().unwrap();
        let glass = GlassCast::new(&mask, (left, top, right - left + 1, bottom - top + 1)).render(40);

        let middle = glass.get_pixel(20, 20).0;
        let side = glass.get_pixel(6, 20).0;
        assert_eq!(middle, side);
        assert!((25..36).contains(&middle[3]));
    }

    #[test]
    fn the_silhouette_is_solid_white_inside_and_clear_outside_on_the_same_outline() {
        let photo = RgbImage::from_fn(60, 60, |x, y| if (10..50).contains(&x) && (10..50).contains(&y) { image::Rgb([40, 30, 20]) } else { image::Rgb([230, 230, 230]) });
        let mask = BackdropMask::flood_keeping(&photo, BackdropMask::BRIGHT, BackdropMask::POCKET_SHARE);
        let (left, top, right, bottom) = mask.bounds().unwrap();
        let cast = GlassCast::new(&mask, (left, top, right - left + 1, bottom - top + 1));
        let (glass, silhouette) = (cast.render(40), cast.silhouette(40));

        assert_eq!(silhouette.get_pixel(20, 20).0, [255, 255, 255, 255]);
        assert_eq!(silhouette.dimensions(), glass.dimensions());
        for (shape, tint) in silhouette.pixels().zip(glass.pixels()) {
            assert_eq!(shape.0[3] == 0, tint.0[3] == 0);
        }
    }
}
