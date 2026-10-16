use super::carve_letterer::CarveLetterer;
use crate::{CarveMask, Carving, MarbleKind, WireframePlan};

pub struct PlanMask;

impl PlanMask {
    pub fn cut(plan: &WireframePlan, kind: MarbleKind, width: u32, height: u32, scale: f64) -> Option<CarveMask> {
        let letterer = CarveLetterer::new(width, height, scale)?;
        letterer.letter(plan.carvings());
        let mut mask = letterer.mask()?;
        mask.thicken_relief(kind.etch().thicken);
        for carving in plan.carvings() {
            if let Carving::Groove(groove) = carving {
                mask.stamp_groove(groove);
            }
        }
        Some(mask)
    }
}
