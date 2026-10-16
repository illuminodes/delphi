use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, ImageData};

use super::carve_letterer::CarveLetterer;
use crate::{Limestone, StoneSketch};

pub struct StoneCutter;

impl StoneCutter {
    pub fn cut(sketch: &StoneSketch, photo: &HtmlImageElement, scale: f64) -> Option<HtmlCanvasElement> {
        let face = sketch.face(scale);
        let (width, height) = face.pixel_size();
        let (wide, tall) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
        if wide == 0 || tall == 0 {
            return None;
        }
        let (canvas, context) = Self::canvas(wide, tall)?;
        let crop = sketch.patch.crop((f64::from(photo.natural_width()), f64::from(photo.natural_height())), (width as f64, height as f64));
        let (flip_x, flip_y) = (if sketch.patch.mirrored { -1.0 } else { 1.0 }, if sketch.patch.flipped { -1.0 } else { 1.0 });
        context.translate(if flip_x < 0.0 { width as f64 } else { 0.0 }, if flip_y < 0.0 { height as f64 } else { 0.0 }).ok()?;
        context.scale(flip_x, flip_y).ok()?;
        context
            .draw_image_with_html_image_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(photo, crop.x, crop.y, crop.width, crop.height, 0.0, 0.0, width as f64, height as f64)
            .ok()?;
        context.set_transform(1.0, 0.0, 0.0, 1.0, 0.0, 0.0).ok()?;
        let mut pixels = context.get_image_data(0.0, 0.0, width as f64, height as f64).ok()?.data().0;
        let carvings = face.local(&sketch.carvings);
        let mask = if carvings.is_empty() {
            None
        } else {
            let letterer = CarveLetterer::new(wide, tall, face.scale())?;
            letterer.letter(&carvings);
            let mut mask = letterer.mask()?;
            mask.thicken_relief(Limestone::ETCH.thicken);
            Some(mask)
        };
        face.dress(&mut pixels, mask.as_ref(), Limestone::ETCH);
        let keep = 1.0 - sketch.shade;
        for pixel in pixels.as_chunks_mut::<4>().0 {
            for channel in &mut pixel[..3] {
                *channel = (f64::from(*channel) * keep).round() as u8;
            }
        }
        let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&pixels), wide, tall).ok()?;
        context.put_image_data(&image, 0.0, 0.0).ok()?;
        Some(canvas)
    }

    fn canvas(width: u32, height: u32) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d)> {
        let canvas: HtmlCanvasElement = web_sys::window()?.document()?.create_element("canvas").ok()?.dyn_into().ok()?;
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        Some((canvas, context))
    }
}
