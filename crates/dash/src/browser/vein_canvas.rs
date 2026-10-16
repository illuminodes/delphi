use yew::prelude::*;

use super::vein_painter::VeinPainter;
use crate::{ChalkPlot, MarbleKind, MarbleStone};

#[derive(Properties, PartialEq)]
pub struct VeinCanvasProps {
    pub kind: MarbleKind,
    pub plot: ChalkPlot,
    pub seed: u64,
}

#[function_component(VeinCanvas)]
pub fn vein_canvas(props: &VeinCanvasProps) -> Html {
    let canvas = use_node_ref();
    let painted = use_state(|| false);
    use_effect_with((props.kind, props.seed, props.plot.clone()), {
        let canvas = canvas.clone();
        let painted = painted.clone();
        move |(kind, seed, plot): &(MarbleKind, u64, ChalkPlot)| {
            let stone = MarbleStone::new(kind.recipe(), *seed);
            painted.set(VeinPainter::paint(&canvas, &stone, plot).is_some());
        }
    });
    html! { <canvas class={classes!("vein", painted.then_some("ready"))} ref={canvas} aria-hidden="true" /> }
}
