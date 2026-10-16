use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, HtmlImageElement, ImageData};

use crate::{GasCloud, VesselMask};

struct ScratchCanvas {
    canvas: HtmlCanvasElement,
    context: CanvasRenderingContext2d,
}

impl ScratchCanvas {
    fn new() -> Option<Self> {
        let canvas: HtmlCanvasElement = web_sys::window()?.document()?.create_element("canvas").ok()?.dyn_into().ok()?;
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        Some(Self { canvas, context })
    }

    fn fit(&self, width: u32, height: u32) {
        if self.canvas.width() != width || self.canvas.height() != height {
            self.canvas.set_width(width);
            self.canvas.set_height(height);
        }
    }
}

pub struct GasStamp {
    cells: ScratchCanvas,
    shaped: ScratchCanvas,
}

impl GasStamp {
    pub fn new() -> Option<Self> {
        Some(Self { cells: ScratchCanvas::new()?, shaped: ScratchCanvas::new()? })
    }

    pub fn draw(&self, cloud: &GasCloud, shape: &HtmlImageElement, mask: &VesselMask, onto: &CanvasRenderingContext2d) -> Option<()> {
        let (columns, rows) = (u32::try_from(cloud.columns).ok()?, u32::try_from(cloud.rows).ok()?);
        let (wide, tall) = (shape.natural_width(), shape.natural_height());
        if columns == 0 || rows == 0 || wide == 0 || tall == 0 {
            return None;
        }
        self.cells.fit(columns, rows);
        let pixels = cloud.pixels();
        let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&pixels), columns, rows).ok()?;
        self.cells.context.put_image_data(&image, 0.0, 0.0).ok()?;

        self.shaped.fit(wide, tall);
        let (scale_x, scale_y) = (f64::from(wide) / mask.width, f64::from(tall) / mask.height);
        let (left, top, width, height) = cloud.image_bounds();
        let context = &self.shaped.context;
        context.set_global_composite_operation("copy").ok()?;
        context.set_image_smoothing_enabled(true);
        context.draw_image_with_html_canvas_element_and_dw_and_dh(&self.cells.canvas, (left - mask.left) * scale_x, (top - mask.top) * scale_y, width * scale_x, height * scale_y).ok()?;
        context.set_global_composite_operation("destination-in").ok()?;
        context.draw_image_with_html_image_element_and_dw_and_dh(shape, 0.0, 0.0, f64::from(wide), f64::from(tall)).ok()?;

        onto.save();
        onto.set_global_composite_operation("lighter").ok()?;
        onto.set_global_alpha(1.0);
        onto.set_image_smoothing_enabled(true);
        let drawn = onto.draw_image_with_html_canvas_element_and_dw_and_dh(&self.shaped.canvas, mask.left, mask.top, mask.width, mask.height).ok();
        onto.restore();
        drawn
    }
}
