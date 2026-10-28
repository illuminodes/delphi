use yew::prelude::*;

use super::box_size_hook::use_box_size;
use crate::{BlockStrip, ChainBlock, ChainLayout, CubeDetail, DisplayFormat, StripBlock};

#[derive(Properties, PartialEq)]
pub struct BlockStripViewProps {
    pub strip: BlockStrip,
    pub selected: u32,
    pub on_select: Callback<u32>,
    pub now: i64,
}

/// The recent blocks as a chain of cubes, newest first, flowed into rows that fill the space
/// given without scrolling. A cube is as big as its block was full and coloured by how many
/// votes the block gave the price, from red for few to green for many. Clicking a cube picks
/// that block.
#[function_component(BlockStripView)]
pub fn block_strip_view(props: &BlockStripViewProps) -> Html {
    let area = use_node_ref();
    let (width, height) = use_box_size(&area);
    let blocks = props.strip.blocks();
    let fullness: Vec<f64> = blocks.iter().map(|block| block.fullness).collect();
    let layout = ChainLayout::fit(&fullness, width, height);
    let style = format!("--link: {:.0}px; --row-gap: {:.0}px", ChainLayout::LINK, ChainLayout::ROW_GAP);

    html! {
        <div class="chain-area" ref={area}>
            <div class="chain" style={style} role="list" aria-label="recent blocks, newest first">
                {for layout.rows().iter().map(|row| html! {
                    <div class="chain-row">
                        {for row.iter().enumerate().map(|(place, index)| ChainCube {
                            block: &blocks[*index],
                            side: layout.side(*index),
                            selected: blocks[*index].height == props.selected,
                            linked: place > 0,
                        }.view(&props.on_select, props.now))}
                    </div>
                })}
            </div>
        </div>
    }
}

/// One block drawn as a cube.
struct ChainCube<'a> {
    block: &'a StripBlock,
    side: f64,
    selected: bool,
    linked: bool,
}

impl ChainCube<'_> {
    fn view(&self, on_select: &Callback<u32>, now: i64) -> Html {
        let block = self.block;
        let height = block.height;
        let onclick = on_select.reform(move |_: MouseEvent| height);
        let depth = if block.settled {
            "settled".to_string()
        } else {
            format!("{}/{} conf", block.confirmations, BlockStrip::SETTLE_DEPTH)
        };
        html! {
            <div role="listitem" class="chain-slot" key={height} style={format!("--cube: {:.0}px", self.side)}>
                {if self.linked { html! { <span class="chain-link" aria-hidden="true"></span> } } else { html! {} }}
                <span class="cube-height">{DisplayFormat::count(height as usize)}</span>
                <button
                    type="button"
                    class={classes!(
                        "cube",
                        if block.settled { "settled" } else { "pending" },
                        block.vote_quality().is_none().then_some("unrated"),
                        self.selected.then_some("selected"),
                    )}
                    style={block.vote_quality().map(|quality| format!("--quality: {quality:.3}"))}
                    title={self.summary(now)}
                    aria-pressed={self.selected.to_string()}
                    onclick={onclick}
                >
                    <span class="cube-text">{self.text(now)}</span>
                </button>
                <span class={classes!("cube-depth", block.settled.then_some("good"))}>{depth}</span>
            </div>
        }
    }

    fn text(&self, now: i64) -> Html {
        let block = self.block;
        let detail = CubeDetail::for_side(self.side);
        let votes = html! {
            <span class="cube-votes">
                {block.votes.map_or_else(|| "—".to_string(), |votes| format!("{} votes", DisplayFormat::count(votes)))}
            </span>
        };
        if detail == CubeDetail::Least {
            return votes;
        }
        html! {
            <>
                {votes}
                <span>{format!("{:.0}% full", block.fullness * 100.0)}</span>
                <span>{format!("{} txs", DisplayFormat::count(block.transactions))}</span>
                {if detail == CubeDetail::All {
                    html! {
                        <>
                            <span>{block.fee_rate.map_or_else(|| "—".to_string(), |rate| format!("~{rate:.0} sat/vB"))}</span>
                            <span class="cube-age">{DisplayFormat::age(now - block.time)}</span>
                        </>
                    }
                } else {
                    html! {}
                }}
            </>
        }
    }

    /// Every figure on the cube, for the hover note when the cube is too small to show them.
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
            DisplayFormat::age(now - block.time),
        ]);
        parts.join(" · ")
    }
}
