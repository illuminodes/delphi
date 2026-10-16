use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::{CarveMask, CarveText, Carving};

pub struct CarveLetterer {
    context: CanvasRenderingContext2d,
    width: u32,
    height: u32,
    scale: f64,
}

impl CarveLetterer {
    pub fn new(width: u32, height: u32, scale: f64) -> Option<Self> {
        let canvas: HtmlCanvasElement = web_sys::window()?.document()?.create_element("canvas").ok()?.dyn_into().ok()?;
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        context.scale(scale, scale).ok()?;
        context.set_text_baseline("alphabetic");
        Some(Self { context, width, height, scale })
    }

    pub fn letter(&self, carvings: &[Carving]) {
        for carving in carvings {
            if let Carving::Text(text) = carving {
                self.draw(text);
            }
        }
    }

    pub fn mask(&self) -> Option<CarveMask> {
        let image = self.context.get_image_data(0.0, 0.0, f64::from(self.width), f64::from(self.height)).ok()?;
        Some(CarveMask::from_rgba(self.width as usize, self.height as usize, self.scale, &image.data()))
    }

    fn draw(&self, text: &CarveText) {
        self.context.set_font(&text.font.css(text.size));
        self.context.set_text_align(text.align.css());
        let spacing = format!("{:.2}px", text.tracking * text.size);
        let _ = js_sys::Reflect::set(&self.context, &JsValue::from_str("letterSpacing"), &JsValue::from_str(&spacing));
        self.context.set_fill_style_str(&text.mask_colour());
        let _ = self.context.fill_text(&text.text, text.x, text.y);
    }
}
