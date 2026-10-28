use yew::prelude::*;

use crate::{DisplayFormat, PriceNote};

#[derive(Properties, PartialEq)]
pub struct PriceAgeProps {
    pub note: PriceNote,
    pub now: i64,
}

/// How long ago a price was published, flagged once it is older than its window allows.
#[function_component(PriceAge)]
pub fn price_age(props: &PriceAgeProps) -> Html {
    let age = DisplayFormat::age(props.note.age_seconds(props.now));
    if props.note.is_stale(props.now) {
        html! { <span class="warn">{format!("stale · {age}")}</span> }
    } else {
        html! { <span>{age}</span> }
    }
}
