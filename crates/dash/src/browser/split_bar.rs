use yew::prelude::*;

use crate::SplitSegment;

#[derive(Properties, PartialEq)]
pub struct SplitBarProps {
    pub title: AttrValue,
    pub segments: Vec<SplitSegment>,
}

/// A whole split into coloured parts, with a legend giving each part's amount and share.
#[function_component(SplitBar)]
pub fn split_bar(props: &SplitBarProps) -> Html {
    if props.segments.is_empty() {
        return html! {};
    }
    html! {
        <section class="split">
            <h3>{props.title.clone()}</h3>
            <div class="split-bar" role="img" aria-label={props.title.clone()}>
                {for props.segments.iter().map(|segment| html! {
                    <span
                        class={format!("seg-{}", segment.key)}
                        style={format!("flex-grow: {:.4}", segment.share)}
                        title={format!("{}: {} ({:.1}%)", segment.label, segment.amount, segment.share * 100.0)}
                    ></span>
                })}
            </div>
            <ul class="split-legend">
                {for props.segments.iter().map(|segment| html! {
                    <li class={format!("seg-{}", segment.key)} title={segment.hint}>
                        <span class="swatch" aria-hidden="true"></span>
                        <span class="split-label">{segment.label}</span>
                        <span class="split-amount">{segment.amount.clone()}</span>
                        <span class="detail">{format!("{:.0}%", segment.share * 100.0)}</span>
                    </li>
                })}
            </ul>
        </section>
    }
}
