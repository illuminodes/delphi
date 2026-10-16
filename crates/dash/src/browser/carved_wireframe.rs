use std::cell::RefCell;
use std::rc::Rc;

use web_sys::{HtmlCanvasElement, PointerEvent};
use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::font_gate::FontGate;
use super::slab_carver::SlabCarver;
use super::slab_stage::SlabStage;
use super::stone_preference::StonePreference;
use crate::{HeaderCarving, MarbleKind, WireframePlan};

#[function_component(CarvedWireframe)]
pub fn carved_wireframe() -> Html {
    const SEED: u64 = 7;
    let kind = use_state(StonePreference::initial);
    let fonts_ready = use_state(|| false);
    let slab = use_node_ref();
    let canvas = use_node_ref();
    let stage: Rc<RefCell<Option<SlabStage>>> = use_mut_ref(|| None);
    let ready = use_state(|| false);
    let (width, height) = use_box_size(&slab);

    use_effect_with((), {
        let fonts_ready = fonts_ready.clone();
        move |()| FontGate::open(fonts_ready)
    });

    let plan = WireframePlan::new(width.max(1.0), height.max(1.0), *kind, SEED);
    use_effect_with((*kind, width, height, *fonts_ready), {
        let canvas = canvas.clone();
        let stage = stage.clone();
        let ready = ready.clone();
        let plan = plan.clone();
        move |(kind, width, height, fonts): &(MarbleKind, f64, f64, bool)| {
            if *fonts && *width > 0.0 && *height > 0.0 {
                let kind = *kind;
                yew::platform::spawn_local(async move {
                    let Some(element) = canvas.cast::<HtmlCanvasElement>() else { return };
                    if stage.borrow().is_none() {
                        let opened = SlabStage::open(&element).await;
                        *stage.borrow_mut() = opened;
                    }
                    let Some(sculpt) = SlabCarver::carve(&plan, kind).await else { return };
                    if let Some(stage) = stage.borrow().as_ref() {
                        stage.show(&sculpt);
                        ready.set(true);
                    }
                });
            }
        }
    });

    let on_move = {
        let stage = stage.clone();
        Callback::from(move |event: PointerEvent| {
            let Some(target) = event.current_target().and_then(|target| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(target).ok()) else { return };
            let rect = target.get_bounding_client_rect();
            let pointer = (
                (f64::from(event.client_x()) - rect.left()) / rect.width().max(1.0),
                (f64::from(event.client_y()) - rect.top()) / rect.height().max(1.0),
            );
            if let Some(stage) = stage.borrow().as_ref() {
                stage.aim(Some(pointer));
            }
        })
    };
    let on_leave = {
        let stage = stage.clone();
        Callback::from(move |_: PointerEvent| {
            if let Some(stage) = stage.borrow().as_ref() {
                stage.aim(None);
            }
        })
    };

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
        <div class={classes!("etched-wireframe", "carved", kind.key())} ref={slab} style={format!("background:{}", kind.recipe().base.hex())}
            onpointermove={on_move} onpointerleave={on_leave}>
            <canvas class={classes!("slab", ready.then_some("ready"))} ref={canvas} role="img" aria-label="Delphi dashboard wireframe carved into a marble slab" />
            <button class="stone-switch" style={switch_style} onclick={on_switch} aria-label="Switch between Nero and Parian marble" />
        </div>
    }
}
