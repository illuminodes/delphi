use yew::prelude::*;

use super::box_size_hook::use_box_size;
use crate::{BlockStrip, ChainBlock, ChainLayout, DisplayFormat, StripBlock};

#[derive(Properties, PartialEq)]
pub struct BlockStripViewProps {
    pub strip: BlockStrip,
    pub selected: u32,
    pub on_select: Callback<u32>,
    pub now: i64,
}

/// The recent blocks as a wall of 3D blocks tiling the space given, newest first. A block rises
/// as high as it was full and is coloured by how many votes it gave the price, from red for few
/// to green for many; blocks still short of settling are dimmed. Each block shows its height and
/// votes; the rest is in its hover note. Clicking a block picks it.
#[function_component(BlockStripView)]
pub fn block_strip_view(props: &BlockStripViewProps) -> Html {
    let area = use_node_ref();
    let (width, height) = use_box_size(&area);
    let blocks = props.strip.blocks();
    let layout = ChainLayout::fit(blocks.len(), width, height);

    html! {
        <div class="chain-area" ref={area}>
            <div class="chain" role="list" aria-label="recent blocks, newest first">
                {for layout.rows().into_iter().map(|row| html! {
                    <div class="chain-row">
                        {for blocks[row].iter().map(|block| ChainCube {
                            block,
                            selected: block.height == props.selected,
                        }.view(&props.on_select, props.now))}
                    </div>
                })}
            </div>
        </div>
    }
}

/// One block drawn as a 3D block in its slot.
struct ChainCube<'a> {
    block: &'a StripBlock,
    selected: bool,
}

impl ChainCube<'_> {
    fn view(&self, on_select: &Callback<u32>, now: i64) -> Html {
        let block = self.block;
        let height = block.height;
        let onclick = on_select.reform(move |_: MouseEvent| height);
        html! {
            <div role="listitem" class="chain-slot" key={height}>
                <button
                    type="button"
                    class={classes!(
                        "cube",
                        (!block.settled).then_some("pending"),
                        block.vote_quality().is_none().then_some("unrated"),
                        self.selected.then_some("selected"),
                    )}
                    style={self.style()}
                    title={self.summary(now)}
                    aria-pressed={self.selected.to_string()}
                    onclick={onclick}
                >
                    <span class="cube-height">{DisplayFormat::count(height as usize)}</span>
                    <span class="cube-votes">{block.votes.map_or_else(|| "—".to_string(), DisplayFormat::count)}</span>
                </button>
            </div>
        }
    }

    fn summary(&self, now: i64) -> String {
        let block = self.block;
        let mut parts = vec![format!("block {}", DisplayFormat::count(block.height as usize))];
        if let Some(votes) = block.votes {
            parts.push(format!("{} votes", DisplayFormat::count(votes)));
        }
        parts.extend([
            format!("{:.0}% full", block.fullness * 100.0),
            format!("{} txs", DisplayFormat::count(block.transactions)),
            block.fee_rate.map_or_else(|| "no fee rate".to_string(), |rate| format!("~{rate:.1} sat/vB")),
            format!("{:.3} ₿ fees", ChainBlock::btc(block.fees)),
            if block.settled {
                "settled".to_string()
            } else {
                format!("{}/{} conf", block.confirmations, BlockStrip::SETTLE_DEPTH)
            },
            DisplayFormat::age(now - block.time),
        ]);
        parts.join(" · ")
    }

    fn style(&self) -> String {
        let fill = format!("--fill: {:.3}", self.block.fullness.max(0.25));
        self.block
            .vote_quality()
            .map_or(fill.clone(), |quality| format!("{fill}; --quality: {quality:.3}"))
    }
}
