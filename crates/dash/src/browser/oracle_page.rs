use web_sys::HtmlCanvasElement;
use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::font_gate::FontGate;
use super::oracle_stage::OracleStage;
use crate::{DisplayFormat, OracleScene};

#[function_component(OraclePage)]
pub fn oracle_page() -> Html {
    let frame = use_node_ref();
    let canvas = use_node_ref();
    let fonts_ready = use_state(|| false);
    let newest = use_state(|| None::<u32>);
    let scene = use_mut_ref(|| OracleScene::simulated(1.0, 1.0, 7));
    let (width, height) = use_box_size(&frame);

    use_effect_with((), {
        let fonts_ready = fonts_ready.clone();
        move |()| FontGate::open(fonts_ready)
    });

    use_effect_with((width, height, *fonts_ready), {
        let canvas = canvas.clone();
        let scene = scene.clone();
        let newest = newest.clone();
        move |(width, height, ready): &(f64, f64, bool)| {
            let stage = (*ready && *width > 0.0 && *height > 0.0)
                .then(|| canvas.cast::<HtmlCanvasElement>())
                .flatten()
                .and_then(|canvas| OracleStage::start(canvas, (*width, *height), scene, newest));
            move || drop(stage)
        }
    });

    let label = newest.map_or_else(String::new, |height| format!("block {}", DisplayFormat::count(height as usize)));
    html! {
        <div class="oracle-page" ref={frame}>
            <canvas class="oracle-canvas" ref={canvas} role="img" aria-label="Bitcoin blocks laid as inscribed stones in the polygonal wall at Delphi" />
            <p class="oracle-note" title={format!("simulated chain, shown {}× faster", OracleScene::PACE)}>{label}</p>
        </div>
    }
}
