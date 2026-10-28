use yew::prelude::*;

use super::block_strip_view::BlockStripView;
use super::filter_funnel::FilterFunnel;
use super::meter_row::MeterRow;
use crate::{BlockStrip, ChainBlock, ChainBlocks, CivilTime, DisplayFormat, OracleStatus, SplitSegment};

#[derive(Properties, PartialEq)]
pub struct BlockchainCardProps {
    pub blocks: Option<ChainBlocks>,
    pub status: Option<OracleStatus>,
    pub now: i64,
}

/// The recent blocks as a chain of cubes, with the picked block's figures packed around it: one
/// line naming it above, and below a grid of figures beside a stack of one-line bars. The newest
/// block is picked until another cube is clicked.
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
            <header class="block-line">
                <h2>{format!("Block {}", DisplayFormat::count(block.height as usize))}</h2>
                <a
                    class="block-hash"
                    href={format!("https://mempool.space/block/{}", block.hash)}
                    target="_blank"
                    rel="noopener noreferrer"
                    title={format!("{} — open on mempool.space", block.hash)}
                >
                    {block.short_hash()}
                </a>
                <span class="detail">{CivilTime::from_unix(block.time).full_label()}</span>
                <span class="age">{DisplayFormat::age(props.now - block.time)}</span>
            </header>
            <BlockStripView strip={strip} selected={block.height} on_select={on_select} now={props.now} />
            <div class="block-panel">
                <dl class="block-figures">
                    {for BlockFigures::of(block, blocks).into_iter().map(|(label, value, hint)| html! {
                        <div title={hint}>
                            <dt>{label}</dt>
                            <dd>{value}</dd>
                        </div>
                    })}
                </dl>
                <ul class="block-meters">
                    <MeterRow
                        label="full"
                        value={format!("{:.1}%", block.fullness() * 100.0)}
                        hint="weight as a share of the 4 MWU limit"
                        segments={vec![BlockFigures::fullness(block)]}
                    />
                    <MeterRow
                        label="reward"
                        value={format!("{:.3} ₿", ChainBlock::btc(block.subsidy + block.fees))}
                        hint="what the miner was paid: new coins (gold) and fees (teal)"
                        segments={block.reward()}
                    />
                    <FilterFunnel filter={block.filter.clone()} />
                </ul>
            </div>
        </article>
    }
}

/// The figures shown for the picked block.
struct BlockFigures;

impl BlockFigures {
    fn of(block: &ChainBlock, blocks: &ChainBlocks) -> Vec<(&'static str, String, &'static str)> {
        let mut figures = vec![
            ("txs", DisplayFormat::count(block.transactions), "transactions in the block, coinbase included"),
            ("outputs", DisplayFormat::count(block.outputs), "outputs created by the block"),
            ("size", format!("{:.2} MB", block.size as f64 / 1_000_000.0), "serialized size"),
            (
                "moved",
                format!("{} ₿", DisplayFormat::count(ChainBlock::btc(block.output_value).round() as usize)),
                "value of every non-coinbase output",
            ),
        ];
        if let Some(difficulty) = block.difficulty {
            figures.push(("difficulty", format!("{:.2} T", difficulty / 1e12), "how much harder than the easiest target"));
        }
        if let Some(interval) = blocks.mean_interval() {
            figures.push(("interval", format!("{:.1} min", interval / 60.0), "mean time between the blocks in the chain"));
        }
        if let Some(price) = block.filter.price {
            figures.push(("vs price", DisplayFormat::usd_whole(price), "the hourly price the block's outputs were measured against"));
        }
        figures
    }

    fn fullness(block: &ChainBlock) -> SplitSegment {
        SplitSegment {
            key: "full",
            label: "weight",
            amount: format!("{} WU", DisplayFormat::count(block.weight as usize)),
            share: block.fullness().clamp(0.0, 1.0),
            hint: "weight used of the 4 MWU limit",
        }
    }
}
