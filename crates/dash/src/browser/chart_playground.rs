use yew::prelude::*;

use super::marble_chart::MarbleChart;
use crate::{ChalkTone, MarbleKind, SampleSeries};

#[function_component(ChartPlayground)]
pub fn chart_playground() -> Html {
    let kind = use_state(|| MarbleKind::Nero);
    let tone = use_state(|| ChalkTone::Chalk);
    let seed = use_state(|| 7_u64);
    let points = use_memo(*seed, |seed| SampleSeries::new(*seed, 144).points());
    let short = use_memo(*seed, |seed| SampleSeries::new(seed.wrapping_add(1), 36).points());

    let redraw = {
        let seed = seed.clone();
        Callback::from(move |_: MouseEvent| seed.set(seed.wrapping_add(1)))
    };

    html! {
        <section>
            <h2>{"Line chart"}</h2>
            <p class="lede">
                {"The price line is the main vein of the stone. The slab is generated from a base colour, \
                  and its bands follow the line: they bend with it near the vein and settle into the \
                  long-run trend further away. The prices are generated sample data, not oracle output. \
                  Hover the slab to move the ring along the line."}
            </p>
            <div class="chart-playground">
                <div class="controls">
                    <div class="choice">
                        <span class="t-label">{"Stone"}</span>
                        {for MarbleKind::ALL.into_iter().map(|option| {
                            let kind = kind.clone();
                            let active = *kind == option;
                            html! {
                                <button type="button" class={classes!(active.then_some("active"))}
                                    onclick={Callback::from(move |_| kind.set(option))}>
                                    {option.title()}
                                </button>
                            }
                        })}
                    </div>
                    <div class="choice">
                        <span class="t-label">{"Chalk"}</span>
                        {for ChalkTone::ALL.into_iter().map(|option| {
                            let tone = tone.clone();
                            let active = *tone == option;
                            html! {
                                <button type="button" class={classes!(active.then_some("active"))}
                                    onclick={Callback::from(move |_| tone.set(option))}>
                                    {option.title()}
                                </button>
                            }
                        })}
                    </div>
                    <button type="button" class="btn ghost" onclick={redraw}>{"New sample"}</button>
                </div>
                <MarbleChart
                    title="Daily"
                    detail={kind.summary()}
                    points={(*points).clone()}
                    kind={*kind}
                    tone={*tone}
                    seed={*seed}
                />
                <div class="trio">
                    {for MarbleKind::ALL.into_iter().map(|option| html! {
                        <MarbleChart
                            title={option.title()}
                            detail="6 hours"
                            points={(*short).clone()}
                            kind={option}
                            tone={*tone}
                            seed={seed.wrapping_add(option as u64)}
                        />
                    })}
                </div>
            </div>
        </section>
    }
}
