use yew::prelude::*;

use super::block_strip_view::BlockStripView;
use super::filter_funnel::FilterFunnel;
use super::split_bar::SplitBar;
use crate::{BlockScale, BlockStrip, ChainBlock, ChainBlocks, CivilTime, DisplayFormat, OracleStatus};

#[derive(Properties, PartialEq)]
pub struct BlockchainCardProps {
    pub blocks: Option<ChainBlocks>,
    pub status: Option<OracleStatus>,
    pub now: i64,
}

/// The newest block the oracle read, in detail, and the few before it as rows of bars measured
/// against each other.
#[function_component(BlockchainCard)]
pub fn blockchain_card(props: &BlockchainCardProps) -> Html {
    let Some(blocks) = props.blocks.as_ref() else {
        return html! { <div class="card placeholder">{"Waiting for the oracle to read a block…"}</div> };
    };
    let Some(tip) = blocks.latest() else {
        return html! { <div class="card placeholder">{"No blocks yet"}</div> };
    };
    let status = props.status.as_ref();
    let strip = BlockStrip::new(blocks, status.and_then(|status| status.tip), status.and_then(|status| status.settled));
    let scale = BlockScale::of(blocks);

    html! {
        <article class="card blockchain-card">
            <header>
                <h2>{format!("Block {}", DisplayFormat::count(tip.height as usize))}</h2>
                <span class="detail">{CivilTime::from_unix(tip.time).full_label()}</span>
                <span class="age">{DisplayFormat::age(props.now - tip.time)}</span>
            </header>
            <p class="block-hash" title={tip.hash.clone()}>{tip.hash.clone()}</p>
            <BlockStripView strip={strip} />
            <ul class="facts">
                {for BlockchainFacts::of(tip, blocks).into_iter().map(|(label, value, hint)| html! {
                    <li title={hint}>
                        <span class="fact-label">{label}</span>
                        {value}
                    </li>
                })}
            </ul>
            <div class="block-meter" title="weight as a share of the 4 MWU limit">
                <span class="fact-label">{"full"}</span>
                <span class="meter-track"><span class="meter-bar" style={BlockchainFacts::width(tip.fullness())}></span></span>
                <span class="meter-value">{format!("{:.1}%", tip.fullness() * 100.0)}</span>
            </div>
            <SplitBar title="Miner reward" segments={tip.reward()} />
            <FilterFunnel filter={tip.filter.clone()} />
            <section class="block-list">
                <h3>{"Recent blocks"}</h3>
                <div class="block-row head" aria-hidden="true">
                    <span>{"Height"}</span>
                    <span>{"Txs"}</span>
                    <span>{"Full"}</span>
                    <span>{"sat/vB"}</span>
                    <span title="outputs whose implied price agrees with the hourly price">{"Votes"}</span>
                </div>
                <ul>
                    {for blocks.blocks().iter().map(|block| BlockchainFacts::row(block, &scale, props.now))}
                </ul>
            </section>
        </article>
    }
}

/// The parts of the blockchain card drawn from one block.
struct BlockchainFacts;

impl BlockchainFacts {
    fn of(tip: &ChainBlock, blocks: &ChainBlocks) -> Vec<(&'static str, String, &'static str)> {
        let mut facts = vec![
            ("txs", DisplayFormat::count(tip.transactions), "transactions in the block, coinbase included"),
            ("outputs", DisplayFormat::count(tip.outputs), "outputs created by the block"),
            ("size", format!("{:.2} MB", tip.size as f64 / 1_000_000.0), "serialized size"),
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
            facts.push(("interval", format!("{:.1} min", interval / 60.0), "mean time between the blocks listed"));
        }
        facts
    }

    fn row(block: &ChainBlock, scale: &BlockScale, now: i64) -> Html {
        let votes = block.filter.inside_band;
        html! {
            <li class="block-row" key={block.height} title={block.hash.clone()}>
                <span class="row-height">
                    {DisplayFormat::count(block.height as usize)}
                    <span class="detail">{DisplayFormat::age(now - block.time)}</span>
                </span>
                {Self::cell(DisplayFormat::count(block.transactions), scale.transactions(block), "txs")}
                {Self::cell(format!("{:.0}%", block.fullness() * 100.0), block.fullness(), "full")}
                {Self::cell(
                    block.fee_rate().map_or_else(|| "—".to_string(), |rate| format!("{rate:.1}")),
                    scale.fee_rate(block),
                    "fees",
                )}
                {Self::cell(votes.map_or_else(|| "—".to_string(), DisplayFormat::count), scale.votes(block), "votes")}
            </li>
        }
    }

    fn cell(value: String, share: f64, kind: &'static str) -> Html {
        html! {
            <span class={classes!("row-cell", kind)}>
                <span class="row-value">{value}</span>
                <span class="meter-track"><span class="meter-bar" style={Self::width(share)}></span></span>
            </span>
        }
    }

    fn width(share: f64) -> String {
        format!("width: {:.1}%", share.clamp(0.0, 1.0) * 100.0)
    }
}
