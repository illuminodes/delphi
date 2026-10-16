use web_sys::HtmlCanvasElement;
use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::fume_stage::FumeStage;
use crate::{FumeFate, FumeScene, HarvestTimer};

#[function_component(FumePage)]
pub fn fume_page() -> Html {
    let frame = use_node_ref();
    let canvas = use_node_ref();
    let tally = use_state(Vec::<usize>::new);
    let scene = use_mut_ref(|| FumeScene::new(1.0, 1.0, 7));
    let timer = use_mut_ref(|| HarvestTimer::SIMULATED);
    let (width, height) = use_box_size(&frame);

    use_effect_with((width, height), {
        let canvas = canvas.clone();
        let tally = tally.clone();
        let scene = scene.clone();
        let timer = timer.clone();
        move |size: &(f64, f64)| {
            let stage = (size.0 > 0.0 && size.1 > 0.0)
                .then(|| canvas.cast::<HtmlCanvasElement>())
                .flatten()
                .and_then(|canvas| FumeStage::start(canvas, *size, scene, timer, tally));
            move || drop(stage)
        }
    });

    let harvest = {
        let scene = scene.clone();
        let timer = timer.clone();
        Callback::from(move |_: MouseEvent| {
            if scene.borrow_mut().harvest() {
                timer.borrow_mut().restart();
            }
        })
    };
    let counts = (*tally).clone();
    let count = |at: usize| counts.get(at).copied().unwrap_or(0);
    let jars = FumeFate::JARS.iter().enumerate().map(|(slot, fate)| {
        html! {
            <li style={format!("--fume:{}", fate.colour().hex())}>
                <span class="fume-swatch" />
                <span class="fume-name">{fate.label()}</span>
                <span class="fume-count">{count(slot)}</span>
            </li>
        }
    });

    html! {
        <div class="fume-page" ref={frame}>
            <canvas class="fume-canvas" ref={canvas} role="img" aria-label="Mempool transactions rising as fumes from the chasm into the oracle's jars" />
            <ul class="fume-legend">
                {for jars}
                <li class="scattered" style={format!("--fume:{}", FumeFate::Scattered.colour().hex())}>
                    <span class="fume-swatch" />
                    <span class="fume-name">{FumeFate::Scattered.label()}</span>
                    <span class="fume-count">{count(FumeFate::JARS.len())}</span>
                </li>
            </ul>
            <p class="fume-note">
                {format!("simulated mempool · harvests {}", count(FumeFate::JARS.len() + 1))}
                <button class="fume-harvest" type="button" onclick={harvest}>{"harvest now"}</button>
            </p>
        </div>
    }
}
