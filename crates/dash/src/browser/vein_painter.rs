use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
use yew::NodeRef;

use crate::{ChalkPlot, MarbleStone};

pub struct VeinPainter;

impl VeinPainter {
    const MAX_SCALE: f64 = 2.0;

    pub fn paint(node: &NodeRef, stone: &MarbleStone, plot: &ChalkPlot) -> Option<()> {
        let canvas = node.cast::<HtmlCanvasElement>()?;
        let layout = plot.chart().layout();
        let scale = web_sys::window()?.device_pixel_ratio().clamp(1.0, Self::MAX_SCALE);
        let (width, height) = ((layout.width * scale).round() as u32, (layout.height * scale).round() as u32);
        if width == 0 || height == 0 {
            return None;
        }
        let branches = plot.vein_branches(stone.recipe());
        let pixels = stone.render(&plot.vein_path()?, &plot.band_flow()?, &branches, width as usize, height as usize, scale);
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&pixels), width, height).ok()?;
        context.put_image_data(&image, 0.0, 0.0).ok()
    }
}
