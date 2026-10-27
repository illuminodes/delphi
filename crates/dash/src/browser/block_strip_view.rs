use yew::prelude::*;

use crate::{BlockStrip, CivilTime, DisplayFormat};

#[derive(Properties, PartialEq)]
pub struct BlockStripViewProps {
    pub strip: BlockStrip,
}

/// The recent blocks left to right, tip last, each filled as high as the block is full.
/// Settled blocks are gold; blocks still short of the settle depth are outlined and show how
/// many confirmations they have.
#[function_component(BlockStripView)]
pub fn block_strip_view(props: &BlockStripViewProps) -> Html {
    let blocks = props.strip.blocks();
    html! {
        <div class="block-strip" role="list" aria-label="recent blocks, oldest first">
            {for blocks.iter().map(|block| {
                let title = format!(
                    "#{} · {} · {} confirmations · {:.0}% full{}",
                    DisplayFormat::count(block.height as usize),
                    CivilTime::from_unix(block.time).full_label(),
                    block.confirmations,
                    block.fullness * 100.0,
                    block.fee_rate.map_or_else(String::new, |rate| format!(" · {rate:.1} sat/vB")),
                );
                html! {
                    <div
                        role="listitem"
                        class={classes!("strip-block", if block.settled { "settled" } else { "pending" })}
                        style={format!("--fill: {:.0}%", block.fullness * 100.0)}
                        title={title}
                    >
                        <span class="strip-height">{DisplayFormat::count(block.height as usize)}</span>
                        <span class="strip-depth">
                            {if block.settled {
                                "settled".to_string()
                            } else {
                                format!("{}/{}", block.confirmations, BlockStrip::SETTLE_DEPTH)
                            }}
                        </span>
                    </div>
                }
            })}
        </div>
    }
}
