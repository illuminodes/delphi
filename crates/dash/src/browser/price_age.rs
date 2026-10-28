use yew::prelude::*;

use crate::{DisplayFormat, PriceNote, PriceOrigin};

#[derive(Properties, PartialEq)]
pub struct PriceAgeProps {
    pub note: PriceNote,
    pub now: i64,
    pub settled: Option<u32>,
}

/// When a price was last updated: for a chain window, when its newest block was mined.
/// Flagged once it is out of date.
#[function_component(PriceAge)]
pub fn price_age(props: &PriceAgeProps) -> Html {
    let updated = match props.note.origin() {
        PriceOrigin::Chain(detail) => i64::from(detail.stop_time),
        PriceOrigin::Mempool(_) => props.note.created_at(),
    };
    let age = DisplayFormat::age((props.now - updated).max(0));
    let stale = props.note.is_stale(props.now, props.settled);
    html! {
        <span class={classes!(stale.then_some("warn"))} title={DisplayFormat::clock(updated)}>
            {if stale { format!("stale · updated {age}") } else { format!("updated {age}") }}
        </span>
    }
}
