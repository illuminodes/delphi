use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
use yew::NodeRef;

use super::plan_mask::PlanMask;
use super::stone_photo::StonePhoto;
use crate::{MarbleKind, StoneEtch, WireframePlan};

pub struct EtchPainter;

impl EtchPainter {
    const MAX_SCALE: f64 = 2.0;
    const SEED: u64 = 7;

    pub async fn paint(node: NodeRef, plan: WireframePlan, kind: MarbleKind) -> Option<()> {
        let photo = StonePhoto::load(kind.photo()?).await?;
        let canvas = node.cast::<HtmlCanvasElement>()?;
        let slab = plan.layout().slab;
        let scale = web_sys::window()?.device_pixel_ratio().clamp(1.0, Self::MAX_SCALE);
        let (width, height) = ((slab.width() * scale).round() as u32, (slab.height() * scale).round() as u32);
        if width == 0 || height == 0 {
            return None;
        }
        let mask = PlanMask::cut(&plan, kind, width, height, scale)?;
        let mut pixels = photo.pixels(width, height)?;
        StoneEtch::new(kind.etch(), Self::SEED).apply(&mut pixels, &mask);
        canvas.set_width(width);
        canvas.set_height(height);
        let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
        let image = ImageData::new_with_u8_clamped_array_and_sh(Clamped(&pixels), width, height).ok()?;
        context.put_image_data(&image, 0.0, 0.0).ok()
    }
}
