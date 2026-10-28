use yew::prelude::*;

use super::split_bar::SplitBar;
use crate::{BlockFilter, DisplayFormat};

#[derive(Properties, PartialEq)]
pub struct FilterFunnelProps {
    pub filter: BlockFilter,
}

/// Where a block's transactions went on their way into the price: the verdict on each, then how
/// many of the accepted ones' outputs were kept by the amount limits, sat near a round dollar
/// amount, and agreed with the price.
#[function_component(FilterFunnel)]
pub fn filter_funnel(props: &FilterFunnelProps) -> Html {
    let filter = &props.filter;
    if !filter.has_verdicts() {
        return html! { <p class="detail">{"This daemon does not report filter verdicts yet."}</p> };
    }
    let mut stages = vec![
        FunnelStage::new(
            "accepted",
            filter.accepted(),
            filter.total(),
            "passed every rule: up to 5 inputs, exactly 2 outputs, no OP_RETURN, small witness, not chained or a double spend",
        ),
        FunnelStage::new(
            "outputs kept",
            filter.admitted_outputs,
            filter.accepted_outputs,
            "outputs of accepted transactions between 1,000 sats and 100,000 BTC",
        ),
    ];
    if let Some(near) = filter.near_round_usd {
        stages.push(FunnelStage::new(
            "near round $",
            near,
            filter.admitted_outputs,
            "within ±25% of a round dollar amount ($5 … $1,000) at the hourly price, and not a round number of sats",
        ));
    }
    if let Some(inside) = filter.inside_band {
        stages.push(FunnelStage::new(
            "agree ±10%",
            inside,
            filter.admitted_outputs,
            "outputs whose implied price lies within ±10% of the hourly price: the ones that vote",
        ));
    }

    html! {
        <section class="filter-funnel">
            <SplitBar title="Verdict on each transaction" segments={filter.verdicts()} />
            <h3>{"How the oracle read this block"}</h3>
            <ul class="funnel-stages">
                {for stages.iter().map(FunnelStage::view)}
            </ul>
            {filter.price.map(|price| html! {
                <p class="detail">{format!("Measured against the hourly price {}.", DisplayFormat::usd_whole(price))}</p>
            })}
        </section>
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
        let share = BlockFilter::share(self.count, self.of);
        html! {
            <li title={self.hint}>
                <span class="funnel-label">{self.label}</span>
                <span class="funnel-track">
                    <span class="funnel-bar" style={format!("width: {:.1}%", share * 100.0)}></span>
                </span>
                <span class="funnel-count">
                    {DisplayFormat::count(self.count)}
                    <span class="detail">{format!(" {:.0}%", share * 100.0)}</span>
                </span>
            </li>
        }
    }
}
