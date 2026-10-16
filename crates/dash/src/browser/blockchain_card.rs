use yew::prelude::*;

use super::filter_funnel::FilterFunnel;
use crate::{BlockFilter, ChainBlock, ChainBlocks, CivilTime, DisplayFormat};

#[derive(Properties, PartialEq)]
pub struct BlockchainCardProps {
    pub blocks: Option<ChainBlocks>,
    pub now: i64,
}

/// The newest block the oracle read, in detail, and the few before it.
#[function_component(BlockchainCard)]
pub fn blockchain_card(props: &BlockchainCardProps) -> Html {
    let Some(blocks) = props.blocks.as_ref() else {
        return html! { <div class="card placeholder">{"Waiting for the oracle to read a block…"}</div> };
    };
    let Some(tip) = blocks.latest() else {
        return html! { <div class="card placeholder">{"No blocks yet"}</div> };
    };
    let age = DisplayFormat::age(props.now - tip.time);
    let facts = BlockchainFacts::of(tip, blocks);

    html! {
        <article class="card blockchain-card">
            <header>
                <h2>{format!("Block {}", DisplayFormat::count(tip.height as usize))}</h2>
                <span class="detail">{CivilTime::from_unix(tip.time).full_label()}</span>
                <span class="age">{age}</span>
            </header>
            <p class="block-hash" title={tip.hash.clone()}>{tip.hash.clone()}</p>
            <ul class="facts">
                {for facts.into_iter().map(|(label, value, hint)| html! {
                    <li title={hint}>
                        <span class="fact-label">{label}</span>
                        {value}
                    </li>
                })}
            </ul>
            <FilterFunnel filter={tip.filter.clone()} />
            <table class="recent-blocks">
                <thead>
                    <tr>
                        <th>{"Height"}</th><th>{"Hash"}</th><th>{"Txs"}</th>
                        <th title="transactions the filter accepted">{"Accepted"}</th>
                        <th title="outputs whose implied price agrees with the hourly price">{"Votes"}</th>
                        <th>{"Full"}</th><th>{"Fees"}</th>
                    </tr>
                </thead>
                <tbody>
                    {for blocks.blocks().iter().map(|block| html! {
                        <tr>
                            <td>{DisplayFormat::count(block.height as usize)}</td>
                            <td class="hash" title={block.hash.clone()}>{block.short_hash()}</td>
                            <td>{DisplayFormat::count(block.transactions)}</td>
                            <td>{BlockchainFacts::accepted(block)}</td>
                            <td>{block.filter.inside_band.map_or_else(|| "—".to_string(), DisplayFormat::count)}</td>
                            <td>{format!("{:.0}%", block.fullness() * 100.0)}</td>
                            <td>{format!("{:.3} ₿", ChainBlock::btc(block.fees))}</td>
                        </tr>
                    })}
                </tbody>
            </table>
        </article>
    }
}

/// The labelled facts shown for the newest block.
struct BlockchainFacts;

impl BlockchainFacts {
    fn accepted(block: &ChainBlock) -> String {
        let filter = &block.filter;
        if !filter.has_verdicts() {
            return "—".to_string();
        }
        format!(
            "{} ({:.0}%)",
            DisplayFormat::count(filter.accepted()),
            BlockFilter::share(filter.accepted(), filter.total()) * 100.0
        )
    }

    fn of(tip: &ChainBlock, blocks: &ChainBlocks) -> Vec<(&'static str, String, &'static str)> {
        let mut facts = vec![
            ("transactions", DisplayFormat::count(tip.transactions), "transactions in the block, coinbase included"),
            ("outputs", DisplayFormat::count(tip.outputs), "outputs created by the block"),
            (
                "size",
                format!("{:.2} MB · {:.1}% full", tip.size as f64 / 1_000_000.0, tip.fullness() * 100.0),
                "serialized size, and weight as a share of the 4 MWU limit",
            ),
            (
                "fees",
                format!("{:.4} ₿", ChainBlock::btc(tip.fees)),
                "what the coinbase paid beyond the subsidy",
            ),
            ("subsidy", format!("{:.4} ₿", ChainBlock::btc(tip.subsidy)), "new coins issued by this block"),
            (
                "moved",
                format!("{} ₿", DisplayFormat::count(ChainBlock::btc(tip.output_value).round() as usize)),
                "value of every non-coinbase output",
            ),
        ];
        if let Some(rate) = tip.fee_rate() {
            facts.push(("fee rate", format!("{rate:.1} sat/vB"), "average fee per virtual byte"));
        }
        if let Some(difficulty) = tip.difficulty {
            facts.push(("difficulty", format!("{:.2} T", difficulty / 1e12), "how much harder than the easiest target"));
        }
        if let Some(interval) = blocks.mean_interval() {
            facts.push((
                "interval",
                format!("{:.1} min", interval / 60.0),
                "mean time between the blocks listed below",
            ));
        }
        facts
    }
}
