use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement};

use crate::PhotoCrop;

pub struct StonePhoto {
    image: HtmlImageElement,
}

impl StonePhoto {
    pub async fn load(source: &str) -> Option<Self> {
        let image = HtmlImageElement::new().ok()?;
        image.set_src(source);
        JsFuture::from(image.decode()).await.ok()?;
        Some(Self { image })
    }

    pub fn pixels(&self, width: u32, height: u32) -> Option<Vec<u8>> {
        let canvas: HtmlCanvasElement = web_sys::window()?.document()?.create_element("canvas").ok()?.dyn_into().ok()?;
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        let photo = (f64::from(self.image.natural_width()), f64::from(self.image.natural_height()));
        let crop = PhotoCrop::cover(photo, (f64::from(width), f64::from(height)));
        context
            .draw_image_with_html_image_element_and_sw_and_sh_and_dx_and_dy_and_dw_and_dh(
                &self.image,
                crop.x,
                crop.y,
                crop.width,
                crop.height,
                0.0,
                0.0,
                f64::from(width),
                f64::from(height),
            )
            .ok()?;
        Some(context.get_image_data(0.0, 0.0, f64::from(width), f64::from(height)).ok()?.data().0)
    }
}
