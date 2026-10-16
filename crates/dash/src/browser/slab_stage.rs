use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use super::three_slab::CarvedSlab;
use crate::{RakingLight, ReliefGrid, SlabSculpt};

pub struct SlabStage {
    slab: CarvedSlab,
    light: RakingLight,
}

impl SlabStage {
    const ELEVATION: f64 = 24.0;
    const ENVIRONMENT: f64 = 0.4;
    const MAX_RATIO: f64 = 2.0;
    const VERTEX_BUDGET: usize = 1_500_000;

    pub async fn open(canvas: &HtmlCanvasElement) -> Option<Self> {
        let slab = CarvedSlab::create(canvas).await.ok()?.unchecked_into::<CarvedSlab>();
        Some(Self { slab, light: RakingLight::new(Self::ELEVATION) })
    }

    pub fn show(&self, sculpt: &SlabSculpt) {
        let (width, height) = sculpt.size();
        let ratio = web_sys::window().map_or(1.0, |window| window.device_pixel_ratio()).clamp(1.0, Self::MAX_RATIO);
        let relief = sculpt.relief();
        let grid = ReliefGrid::sample(relief, (width, height), Self::VERTEX_BUDGET);
        self.slab.frame(width, height, ratio);
        self.slab.carve(width, height, grid.columns() as u32, grid.rows() as u32, grid.heights());
        self.slab.finish(relief.width() as u32, relief.height() as u32, sculpt.albedo(), sculpt.normals(), sculpt.surface(), Self::ENVIRONMENT);
        self.aim(None);
    }

    pub fn aim(&self, pointer: Option<(f64, f64)>) {
        let [x, y, z] = self.light.toward(pointer);
        self.slab.light(x, y, z);
        self.slab.render();
    }
}

impl Drop for SlabStage {
    fn drop(&mut self) {
        self.slab.dispose();
    }
}
