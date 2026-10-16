use super::plan_mask::PlanMask;
use super::stone_photo::StonePhoto;
use crate::{MarbleKind, SlabSculpt, WireframePlan};

pub struct SlabCarver;

impl SlabCarver {
    const MAX_SCALE: f64 = 2.0;

    pub async fn carve(plan: &WireframePlan, kind: MarbleKind) -> Option<SlabSculpt> {
        let photo = StonePhoto::load(kind.photo()?).await?;
        let slab = plan.layout().slab;
        let scale = web_sys::window()?.device_pixel_ratio().clamp(1.0, Self::MAX_SCALE);
        let (width, height) = ((slab.width() * scale).round() as u32, (slab.height() * scale).round() as u32);
        if width == 0 || height == 0 {
            return None;
        }
        let mask = PlanMask::cut(plan, kind, width, height, scale)?;
        let pixels = photo.pixels(width, height)?;
        Some(SlabSculpt::new(&mask, &pixels, kind.etch()))
    }
}
