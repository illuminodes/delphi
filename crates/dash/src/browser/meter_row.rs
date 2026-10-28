use yew::prelude::*;

use crate::SplitSegment;

#[derive(Properties, PartialEq)]
pub struct MeterRowProps {
    pub label: AttrValue,
    pub value: AttrValue,
    pub hint: AttrValue,
    pub segments: Vec<SplitSegment>,
}

/// One labelled bar on a single line: the parts of a whole, each with its own colour and a
/// hover note, and the figure that matters most at the end.
#[function_component(MeterRow)]
pub fn meter_row(props: &MeterRowProps) -> Html {
    html! {
        <li class="meter-row" title={props.hint.clone()}>
            <span class="meter-label">{props.label.clone()}</span>
            <span class="meter-bar" role="img" aria-label={props.label.clone()}>
                {for props.segments.iter().map(|segment| html! {
                    <span
                        class={format!("seg-{}", segment.key)}
                        style={format!("width: {:.2}%", segment.share * 100.0)}
                        title={format!("{}: {} ({:.1}%) — {}", segment.label, segment.amount, segment.share * 100.0, segment.hint)}
                    ></span>
                })}
            </span>
            <span class="meter-value">{props.value.clone()}</span>
        </li>
    }
}
