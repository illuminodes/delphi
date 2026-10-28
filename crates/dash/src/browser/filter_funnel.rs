use yew::prelude::*;

use super::meter_row::MeterRow;
use crate::{BlockFilter, DisplayFormat, SplitSegment};

#[derive(Properties, PartialEq)]
pub struct FilterFunnelProps {
    pub filter: BlockFilter,
}

/// Where a block's transactions went on their way into the price, as meter rows: the verdict on
/// each, then how many of the accepted ones' outputs were kept by the amount limits, sat near a
/// round dollar amount, and agreed with the price.
#[function_component(FilterFunnel)]
pub fn filter_funnel(props: &FilterFunnelProps) -> Html {
    let filter = &props.filter;
    if !filter.has_verdicts() {
        return html! { <li class="detail">{"This daemon does not report filter verdicts yet."}</li> };
    }
    let mut stages = vec![FunnelStage::new(
        "kept",
        filter.admitted_outputs,
        filter.accepted_outputs,
        "outputs of accepted transactions between 1,000 sats and 100,000 BTC",
    )];
    if let Some(near) = filter.near_round_usd {
        stages.push(FunnelStage::new(
            "round $",
            near,
            filter.admitted_outputs,
            "within ±25% of a round dollar amount ($5 … $1,000) at the hourly price, and not a round number of sats",
        ));
    }
    if let Some(inside) = filter.inside_band {
        stages.push(FunnelStage::new(
            "votes",
            inside,
            filter.admitted_outputs,
            "outputs whose implied price lies within ±10% of the hourly price: the ones that vote",
        ));
    }

    html! {
        <>
            <MeterRow
                label="verdict"
                value={format!("{:.0}% ok", BlockFilter::share(filter.accepted(), filter.total()) * 100.0)}
                hint="the verdict on each transaction; hover a colour for the rule"
                segments={filter.verdicts()}
            />
            {for stages.iter().map(FunnelStage::view)}
        </>
    }
}

/// One step of the funnel: how many survived it, out of how many reached it.
struct FunnelStage {
    label: &'static str,
    count: usize,
    of: usize,
    hint: &'static str,
}

impl FunnelStage {
    const fn new(label: &'static str, count: usize, of: usize, hint: &'static str) -> Self {
        Self { label, count, of, hint }
    }

    fn view(&self) -> Html {
        let segment = SplitSegment {
            key: "stage",
            label: self.label,
            amount: DisplayFormat::count(self.count),
            share: BlockFilter::share(self.count, self.of),
            hint: self.hint,
        };
        html! {
            <MeterRow
                label={self.label}
                value={DisplayFormat::count(self.count)}
                hint={self.hint}
                segments={vec![segment]}
            />
        }
    }
}
