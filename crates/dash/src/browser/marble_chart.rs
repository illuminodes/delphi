use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::chalk_board::ChalkBoard;
use super::vein_canvas::VeinCanvas;
use crate::{ChalkPlot, ChalkTone, CivilTime, DisplayFormat, MarbleKind, SeriesPoint};

#[derive(Properties, PartialEq)]
pub struct MarbleChartProps {
    pub title: AttrValue,
    #[prop_or_default]
    pub detail: AttrValue,
    pub points: Vec<SeriesPoint>,
    pub kind: MarbleKind,
    pub tone: ChalkTone,
    #[prop_or(1)]
    pub seed: u64,
}

#[function_component(MarbleChart)]
pub fn marble_chart(props: &MarbleChartProps) -> Html {
    let hovered = use_state(|| None::<usize>);
    let slab = use_node_ref();
    let (width, height) = use_box_size(&slab);
    let plot = ChalkPlot::new(&props.points, ChalkPlot::layout(width, height), props.seed);

    let on_hover = {
        let hovered = hovered.clone();
        Callback::from(move |index: Option<usize>| hovered.set(index))
    };
    let (readout, board) = plot.map_or_else(
        || (html! {}, html! {}),
        |plot| {
            let active = hovered.unwrap_or(plot.last_index()).min(plot.last_index());
            let point = plot.chart().points()[active];
            let change = plot.chart().change_percent();
            let readout = html! {
                <div class="readout">
                    <span class="price">{DisplayFormat::usd(point.price())}</span>
                    <span class="when">{CivilTime::from_unix(point.stop_time()).full_label()}</span>
                    <span class={if change >= 0.0 { "up" } else { "down" }}>{DisplayFormat::percent_brief(change)}</span>
                </div>
            };
            let board = html! {
                <>
                    <VeinCanvas kind={props.kind} plot={plot.clone()} seed={props.seed} />
                    <ChalkBoard plot={plot} active={active} on_hover={on_hover} />
                </>
            };
            (readout, board)
        },
    );

    html! {
        <figure class="marble-chart" style="margin: 0">
            <header>
                <h4>{props.title.clone()}</h4>
                <span class="detail">{props.detail.clone()}</span>
                {readout}
            </header>
            <div class={classes!("marble-slab", props.kind.key(), props.tone.class())} ref={slab}>
                {board}
            </div>
        </figure>
    }
}
