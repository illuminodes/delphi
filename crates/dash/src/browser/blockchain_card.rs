use yew::prelude::*;

use super::block_strip_view::BlockStripView;
use super::filter_funnel::FilterFunnel;
use super::split_bar::SplitBar;
use crate::{BlockStrip, ChainBlock, ChainBlocks, CivilTime, DisplayFormat, OracleStatus};

#[derive(Properties, PartialEq)]
pub struct BlockchainCardProps {
    pub blocks: Option<ChainBlocks>,
    pub status: Option<OracleStatus>,
    pub now: i64,
}

/// The recent blocks as a chain of cubes, and below it the picked block in detail: the newest
/// one until another cube is clicked.
#[function_component(BlockchainCard)]
pub fn blockchain_card(props: &BlockchainCardProps) -> Html {
    let picked = use_state(|| None::<u32>);
    let Some(blocks) = props.blocks.as_ref() else {
        return html! { <div class="card placeholder">{"Waiting for the oracle to read a block…"}</div> };
    };
    let Some(tip) = blocks.latest() else {
        return html! { <div class="card placeholder">{"No blocks yet"}</div> };
    };
    let block = picked
        .and_then(|height| blocks.blocks().iter().find(|block| block.height == height))
        .unwrap_or(tip);
    let status = props.status.as_ref();
    let strip = BlockStrip::new(blocks, status.and_then(|status| status.tip), status.and_then(|status| status.settled));
    let on_select = {
        let picked = picked.clone();
        let tip = tip.height;
        Callback::from(move |height: u32| picked.set((height != tip).then_some(height)))
    };

    html! {
        <article class="card blockchain-card">
            <BlockStripView strip={strip} selected={block.height} on_select={on_select} now={props.now} />
            <section class="block-detail">
                <header>
                    <h2>{format!("Block {}", DisplayFormat::count(block.height as usize))}</h2>
                    <span class="detail">{CivilTime::from_unix(block.time).full_label()}</span>
                    <span class="age">{DisplayFormat::age(props.now - block.time)}</span>
                </header>
                <a
                    class="block-hash"
                    href={format!("https://mempool.space/block/{}", block.hash)}
                    target="_blank"
                    rel="noopener noreferrer"
                    title="open on mempool.space"
                >
                    {block.hash.clone()}
                </a>
                <ul class="facts">
                    {for BlockchainFacts::of(block, blocks).into_iter().map(|(label, value, hint)| html! {
                        <li title={hint}>
                            <span class="fact-label">{label}</span>
                            {value}
                        </li>
                    })}
                </ul>
                <SplitBar title="Miner reward" segments={block.reward()} />
                <FilterFunnel filter={block.filter.clone()} />
            </section>
        </article>
    }
}

/// The labelled facts shown for the picked block.
struct BlockchainFacts;

impl BlockchainFacts {
    fn of(block: &ChainBlock, blocks: &ChainBlocks) -> Vec<(&'static str, String, &'static str)> {
        let mut facts = vec![
            ("txs", DisplayFormat::count(block.transactions), "transactions in the block, coinbase included"),
            ("outputs", DisplayFormat::count(block.outputs), "outputs created by the block"),
            (
                "size",
                format!("{:.2} MB · {:.1}% full", block.size as f64 / 1_000_000.0, block.fullness() * 100.0),
                "serialized size, and weight as a share of the 4 MWU limit",
            ),
            (
                "moved",
                format!("{} ₿", DisplayFormat::count(ChainBlock::btc(block.output_value).round() as usize)),
                "value of every non-coinbase output",
            ),
        ];
        if let Some(difficulty) = block.difficulty {
            facts.push(("difficulty", format!("{:.2} T", difficulty / 1e12), "how much harder than the easiest target"));
        }
        if let Some(interval) = blocks.mean_interval() {
            facts.push(("interval", format!("{:.1} min", interval / 60.0), "mean time between the blocks in the chain"));
        }
        facts
    }
}
