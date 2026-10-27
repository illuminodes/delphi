use yew::prelude::*;

use crate::{DisplayFormat, PriceFact, PriceNote, SeriesPoint, SparklinePath, WindowLabel};

#[derive(Properties, PartialEq)]
pub struct PriceCardProps {
    pub note: PriceNote,
    pub sparkline: Vec<SeriesPoint>,
    pub now: i64,
    pub spread: Option<f64>,
}

#[function_component(PriceCard)]
pub fn price_card(props: &PriceCardProps) -> Html {
    let note = &props.note;
    let label = WindowLabel::of(note.window());
    let stale = note.is_stale(props.now);
    let live = note.window() == "mempool";
    let class = classes!("card", "price-card", if live { "live" } else { "chain" }, stale.then_some("stale"));
    let pairs: Vec<(i64, f64)> = props
        .sparkline
        .iter()
        .map(|point| (point.stop_time(), point.price()))
        .collect();
    let spark = SparklinePath::new(160.0, 36.0, 2.0).points(&pairs);
    let age = DisplayFormat::age(note.age_seconds(props.now));

    html! {
        <article class={class}>
            <header>
                <span class="source-tag">{if live { "Live · mempool" } else { "Settled · chain" }}</span>
                <span class="detail">{format!("{} · {}", label.title(note.window()), label.detail())}</span>
                <span class={classes!("age", stale.then_some("warn"))}>
                    {if stale { format!("stale · {age}") } else { age }}
                </span>
            </header>
            <div class="price-row">
                <p class="price">{DisplayFormat::usd(note.price())}</p>
                {spark.map_or_else(|| html! {}, |points| html! {
                    <svg class="spark" viewBox="0 0 160 36" preserveAspectRatio="none" aria-hidden="true">
                        <polyline points={points} />
                    </svg>
                })}
            </div>
            <ul class="facts">
                {for PriceFact::list(note, props.spread).into_iter().map(|fact| html! {
                    <li title={fact.hint}>
                        <span class="fact-label">{fact.label}</span>
                        {fact.value}
                    </li>
                })}
            </ul>
        </article>
    }
}
