use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

#[wasm_bindgen(module = "/src/browser/carved_slab.js")]
extern "C" {
    pub type CarvedSlab;

    #[wasm_bindgen(static_method_of = CarvedSlab, catch)]
    pub async fn create(canvas: &HtmlCanvasElement) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(method)]
    pub fn frame(this: &CarvedSlab, width: f64, height: f64, pixel_ratio: f64);

    #[wasm_bindgen(method)]
    pub fn carve(this: &CarvedSlab, width: f64, height: f64, columns: u32, rows: u32, heights: &[f32]);

    #[wasm_bindgen(method)]
    pub fn finish(this: &CarvedSlab, map_width: u32, map_height: u32, albedo: &[u8], normals: &[u8], surface: &[u8], environment: f64);

    #[wasm_bindgen(method)]
    pub fn light(this: &CarvedSlab, x: f64, y: f64, z: f64);

    #[wasm_bindgen(method)]
    pub fn render(this: &CarvedSlab);

    #[wasm_bindgen(method)]
    pub fn dispose(this: &CarvedSlab);
}
