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

/// The recent blocks as a chain of cubes, newest first, sized and wrapped into rows to fill the
/// space it is given without scrolling. Each cube fills as high as the block is full, in a
/// colour set by its fee rate, and carries as much of its figures as its size allows. Clicking
/// a cube picks that block.
#[function_component(BlockStripView)]
pub fn block_strip_view(props: &BlockStripViewProps) -> Html {
    let area = use_node_ref();
    let (width, height) = use_box_size(&area);
    let blocks = props.strip.blocks();
    let layout = ChainLayout::fit(blocks.len(), width, height);
    let style = format!(
        "--cube: {:.0}px; --columns: {}; --link: {:.0}px; --row-gap: {:.0}px",
        layout.cube,
        layout.columns,
        ChainLayout::LINK,
        ChainLayout::ROW_GAP
    );

    html! {
        <div class="chain-area" ref={area}>
            <div
                class={classes!("chain", layout.detail().css_class())}
                style={style}
                role="list"
                aria-label="recent blocks, newest first"
            >
                {for blocks.iter().take(layout.shown).enumerate().map(|(index, block)| ChainCube {
                    block,
                    selected: block.height == props.selected,
                    linked: layout.linked(index),
                    detail: layout.detail(),
                }.view(&props.on_select, props.now))}
            </div>
        </div>
    }
}

/// One block drawn as a cube.
struct ChainCube<'a> {
    block: &'a StripBlock,
    selected: bool,
    linked: bool,
    detail: CubeDetail,
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
            <div role="listitem" class="chain-slot" key={height}>
                {if self.linked { html! { <span class="chain-link" aria-hidden="true"></span> } } else { html! {} }}
                <span class="cube-height">{DisplayFormat::count(height as usize)}</span>
                <button
                    type="button"
                    class={classes!(
                        "cube",
                        if block.settled { "settled" } else { "pending" },
                        (block.fullness >= 0.97).then_some("full"),
                        self.selected.then_some("selected"),
                    )}
                    style={self.style()}
                    title={self.summary(now)}
                    aria-pressed={self.selected.to_string()}
                    onclick={onclick}
                >
                    <span class="cube-fill" aria-hidden="true"></span>
                    <span class="cube-text">{self.text(now)}</span>
                </button>
                <span class={classes!("cube-depth", block.settled.then_some("good"))}>{depth}</span>
            </div>
        }
    }

    fn text(&self, now: i64) -> Html {
        let block = self.block;
        let rate = html! {
            <span class="cube-rate">
                {block.fee_rate.map_or_else(|| "—".to_string(), |rate| format!("~{rate:.0} sat/vB"))}
            </span>
        };
        if self.detail == CubeDetail::Least {
            return rate;
        }
        html! {
            <>
                {rate}
                <span class="cube-fees">{format!("{:.3} ₿", ChainBlock::btc(block.fees))}</span>
                <span>{format!("{} txs", DisplayFormat::count(block.transactions))}</span>
                {if self.detail == CubeDetail::All {
                    html! {
                        <>
                            {block.votes.map_or_else(|| html! {}, |votes| html! {
                                <span>{format!("{} votes", DisplayFormat::count(votes))}</span>
                            })}
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
        let mut parts = vec![
            format!("block {}", DisplayFormat::count(block.height as usize)),
            block.fee_rate.map_or_else(|| "no fee rate".to_string(), |rate| format!("~{rate:.1} sat/vB")),
            format!("{:.3} ₿ fees", ChainBlock::btc(block.fees)),
            format!("{} txs", DisplayFormat::count(block.transactions)),
            format!("{:.0}% full", block.fullness * 100.0),
        ];
        if let Some(votes) = block.votes {
            parts.push(format!("{} votes", DisplayFormat::count(votes)));
        }
        parts.push(DisplayFormat::age(now - block.time));
        parts.join(" · ")
    }

    fn style(&self) -> String {
        let fill = format!("--fill: {:.1}%", self.block.fullness * 100.0);
        self.block
            .fee_hue()
            .map_or(fill.clone(), |hue| format!("{fill}; --tone: hsl({hue:.0} 58% 38%)"))
    }
}
