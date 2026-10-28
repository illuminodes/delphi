use yew::prelude::*;

use crate::{BlockStrip, ChainBlock, DisplayFormat, StripBlock};

#[derive(Properties, PartialEq)]
pub struct BlockStripViewProps {
    pub strip: BlockStrip,
    pub selected: u32,
    pub on_select: Callback<u32>,
    pub now: i64,
}

/// The recent blocks as a chain of cubes, newest on the left. Each cube fills as high as the
/// block is full, in a colour set by its fee rate, and carries its fee rate, fees, transactions
/// and votes. Clicking a cube shows that block below.
#[function_component(BlockStripView)]
pub fn block_strip_view(props: &BlockStripViewProps) -> Html {
    let blocks = props.strip.blocks();
    html! {
        <div class="chain" role="list" aria-label="recent blocks, newest first">
            {for blocks.iter().enumerate().map(|(index, block)| html! {
                <>
                    {if index > 0 { html! { <span class="chain-link" aria-hidden="true"></span> } } else { html! {} }}
                    {ChainCube { block, selected: block.height == props.selected }.view(&props.on_select, props.now)}
                </>
            })}
        </div>
    }
}

/// One block drawn as a cube.
struct ChainCube<'a> {
    block: &'a StripBlock,
    selected: bool,
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
            <div role="listitem" class="chain-slot">
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
                    aria-pressed={self.selected.to_string()}
                    onclick={onclick}
                >
                    <span class="cube-fill" aria-hidden="true"></span>
                    <span class="cube-text">
                        <span class="cube-rate">
                            {block.fee_rate.map_or_else(|| "—".to_string(), |rate| format!("~{rate:.1} sat/vB"))}
                        </span>
                        <span class="cube-fees">{format!("{:.3} ₿", ChainBlock::btc(block.fees))}</span>
                        <span>{format!("{} txs", DisplayFormat::count(block.transactions))}</span>
                        {block.votes.map_or_else(|| html! {}, |votes| html! {
                            <span class="cube-votes" title="outputs whose implied price agrees with the hourly price">
                                {format!("{} votes", DisplayFormat::count(votes))}
                            </span>
                        })}
                        <span class="cube-age">{DisplayFormat::age(now - block.time)}</span>
                    </span>
                </button>
                <span class={classes!("cube-depth", block.settled.then_some("good"))}>{depth}</span>
            </div>
        }
    }

    fn style(&self) -> String {
        let fill = format!("--fill: {:.1}%", self.block.fullness * 100.0);
        self.block
            .fee_hue()
            .map_or(fill.clone(), |hue| format!("{fill}; --tone: hsl({hue:.0} 62% 46%)"))
    }
}
