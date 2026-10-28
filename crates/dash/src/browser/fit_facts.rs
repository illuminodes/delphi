use yew::prelude::*;

use crate::{DisplayFormat, FitNote};

#[derive(Properties, PartialEq)]
pub struct FitFactsProps {
    pub fit: FitNote,
}

/// The figures behind one fit, for the row beside its tabs.
#[function_component(FitFacts)]
pub fn fit_facts(props: &FitFactsProps) -> Html {
    let fit = &props.fit;
    html! {
        <>
            <span title="outputs whose amounts imply a price">
                {format!("{} implied", DisplayFormat::count(fit.implied_total() as usize))}
            </span>
            <span title="price from the coarse first pass, before the fit">
                {format!("rough {}", DisplayFormat::usd_whole(fit.rough_price()))}
            </span>
            {fit.octave_ratio().map_or_else(Html::default, |ratio| html! {
                <span title="score of the best rival one octave away, relative to the winner">
                    {format!("octave {ratio:.2}")}
                </span>
            })}
        </>
    }
}
