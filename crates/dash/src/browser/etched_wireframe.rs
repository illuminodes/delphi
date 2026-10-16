use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::etch_painter::EtchPainter;
use super::font_gate::FontGate;
use super::stone_preference::StonePreference;
use crate::{HeaderCarving, MarbleKind, WireframePlan};

#[function_component(EtchedWireframe)]
pub fn etched_wireframe() -> Html {
    const SEED: u64 = 7;
    let kind = use_state(StonePreference::initial);
    let fonts_ready = use_state(|| false);
    let slab = use_node_ref();
    let canvas = use_node_ref();
    let painted = use_state(|| false);
    let (width, height) = use_box_size(&slab);

    use_effect_with((), {
        let fonts_ready = fonts_ready.clone();
        move |()| FontGate::open(fonts_ready)
    });

    let plan = WireframePlan::new(width.max(1.0), height.max(1.0), *kind, SEED);
    use_effect_with((*kind, width, height, *fonts_ready), {
        let canvas = canvas.clone();
        let painted = painted.clone();
        let plan = plan.clone();
        move |(kind, width, height, ready): &(MarbleKind, f64, f64, bool)| {
            if *ready && *width > 0.0 && *height > 0.0 {
                let kind = *kind;
                yew::platform::spawn_local(async move {
                    painted.set(EtchPainter::paint(canvas, plan, kind).await.is_some());
                });
            }
        }
    });

    let switch = HeaderCarving::new(plan.layout().unit, *kind).switch(plan.layout().header).bounds().grow(6.0);
    let on_switch = {
        let kind = kind.clone();
        Callback::from(move |_| kind.set(StonePreference::other(*kind)))
    };
    let switch_style = format!(
        "left:{:.0}px;top:{:.0}px;width:{:.0}px;height:{:.0}px",
        switch.left,
        switch.top,
        switch.width(),
        switch.height()
    );

    html! {
        <div class={classes!("etched-wireframe", kind.key())} ref={slab} style={format!("background:{}", kind.recipe().base.hex())}>
            <canvas class={classes!("slab", painted.then_some("ready"))} ref={canvas} role="img" aria-label="Delphi dashboard wireframe carved in marble" />
            <button class="stone-switch" style={switch_style} onclick={on_switch} aria-label="Switch between Nero and Parian marble" />
        </div>
    }
}
