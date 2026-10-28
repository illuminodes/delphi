use yew::prelude::*;

use crate::{DisplayFormat, PriceFact, PriceNote, SeriesPoint, SparklinePath, WindowLabel};

#[derive(Properties, PartialEq)]
pub struct PriceCardProps {
    pub note: PriceNote,
    pub sparkline: Vec<SeriesPoint>,
    pub now: i64,
    pub settled: Option<u32>,
    pub spread: Option<f64>,
}

/// One window's latest price. Settled chain prices carry the strong tone; the mempool price is
/// provisional and shown quieter. The window's recent history fills the space below.
#[function_component(PriceCard)]
pub fn price_card(props: &PriceCardProps) -> Html {
    let note = &props.note;
    let label = WindowLabel::of(note.window());
    let live = note.window() == "mempool";
    let class = classes!(
        "card",
        "price-card",
        if live { "live" } else { "settled" },
        note.is_stale(props.now, props.settled).then_some("stale")
    );
    let pairs: Vec<(i64, f64)> = props
        .sparkline
        .iter()
        .map(|point| (point.stop_time(), point.price()))
        .collect();
    let spark = SparklinePath::new(SparklinePath::WIDE, 40.0, 2.0).points(&pairs);
    let tag = if live { "Live".to_string() } else { format!("Settled · {}", label.detail()) };

    html! {
        <article class={class}>
            <span class="source-tag">{tag}</span>
            <p class="price">{DisplayFormat::usd(note.price())}</p>
            <ul class="facts">
                {for PriceFact::list(note, props.spread).into_iter().map(|fact| html! {
                    <li title={fact.hint}>
                        <span class="fact-label">{fact.label}</span>
                        {fact.value}
                    </li>
                })}
            </ul>
            {spark.map_or_else(Html::default, |points| html! {
                <svg class="spark" viewBox={format!("0 0 {} 40", SparklinePath::WIDE)} preserveAspectRatio="none" aria-hidden="true">
                    <polyline points={points} />
                </svg>
            })}
        </article>
    }
}
