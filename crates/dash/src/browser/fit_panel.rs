use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::pointer_reader::PointerReader;
use crate::{ChartLayout, DisplayFormat, FitGeometry, FitNote, WindowLabel};

#[derive(Properties, PartialEq)]
pub struct FitPanelProps {
    pub window: AttrValue,
    /// `None` until the window's first fit arrives; the frame shows meanwhile.
    pub fit: Option<FitNote>,
}

/// The outputs behind one fit as rising bars, with the score curve drawn in over them.
/// Before the fit arrives, the baseline and pulsing axis stubs hold its place.
#[function_component(FitPanel)]
pub fn fit_panel(props: &FitPanelProps) -> Html {
    let label = WindowLabel::of(&props.window);
    let hovered = use_state(|| None::<f64>);
    let plot = use_node_ref();
    let (width, height) = use_box_size(&plot);
    let layout = ChartLayout::for_histogram(width, height);
    let drawn = props
        .fit
        .as_ref()
        .and_then(|fit| FitGeometry::new(fit, layout).map(|chart| (fit, chart)));

    let body = drawn.map_or_else(
        || FitPanelParts::frame(layout, &label.title(&props.window)),
        |(fit, chart)| {
            let layout = *chart.layout();
            let price_x = chart.x_of(fit.price());
            let rough_x = chart.x_of(fit.rough_price());
            let bottom = layout.plot_bottom();
            let ticks = chart.ticks(layout.x_tick_count());
            let last_tick = ticks.len().saturating_sub(1);
            let on_move = {
                let hovered = hovered.clone();
                let chart = chart.clone();
                let plot = plot.clone();
                Callback::from(move |event: MouseEvent| {
                    if let Some(x) = PointerReader::chart_x(&event, &plot, chart.layout()) {
                        hovered.set(Some(chart.price_at(x)));
                    }
                })
            };
            let on_leave = {
                let hovered = hovered.clone();
                Callback::from(move |_: MouseEvent| hovered.set(None))
            };
            let band_rect = fit.band().map(|(low, high)| {
                let left = chart.x_of(low);
                let right = chart.x_of(high);
                html! {
                    <rect class="fit-band" x={format!("{left:.1}")} y={layout.top.to_string()}
                        width={format!("{:.1}", (right - left).max(0.0))} height={layout.inner_height().to_string()}>
                        <title>{"chain anchor band"}</title>
                    </rect>
                }
            });
            let deviation_rect = fit.deviation().map(|deviation| {
                let spread = fit.price() * deviation * 0.2;
                let left = chart.x_of(fit.price() - spread);
                let right = chart.x_of(fit.price() + spread);
                html! {
                    <rect class="fit-spread" x={format!("{left:.1}")} y={layout.top.to_string()}
                        width={format!("{:.1}", (right - left).max(1.0))} height={layout.inner_height().to_string()} />
                }
            });
            html! {
                <svg
                    class="chart fit-chart"
                    viewBox={format!("0 0 {} {}", layout.width, layout.height)}
                    preserveAspectRatio="none"
                    role="img"
                    aria-label={format!("{} fit", label.title(fit.window()))}
                    onmousemove={on_move}
                    onmouseleave={on_leave}
                >
                    {band_rect.unwrap_or_default()}
                    {deviation_rect.unwrap_or_default()}
                    {for chart.bars().iter().filter(|bar| bar.count > 0).map(|bar| html! {
                        <rect class="fit-bar" x={format!("{:.1}", bar.x + 0.5)} y={format!("{:.1}", bar.y)}
                            width={format!("{:.1}", (bar.width - 1.0).max(0.5))} height={format!("{:.1}", bar.height)}>
                            <title>{format!("{} outputs", bar.count)}</title>
                        </rect>
                    })}
                    <g class="trace"><path class="fit-curve" d={chart.curve_path()} /></g>
                    {if chart.contains(fit.rough_price()) {
                        html! {
                            <line class="fit-rough" x1={format!("{rough_x:.1}")} x2={format!("{rough_x:.1}")}
                                y1={layout.top.to_string()} y2={bottom.to_string()} />
                        }
                    } else {
                        html! {}
                    }}
                    <line class="fit-price" x1={format!("{price_x:.1}")} x2={format!("{price_x:.1}")}
                        y1={layout.top.to_string()} y2={bottom.to_string()} />
                    <line class="grid-line" x1={layout.left.to_string()} x2={(layout.width - layout.right).to_string()}
                        y1={bottom.to_string()} y2={bottom.to_string()} />
                    {for ticks.iter().enumerate().map(|(index, (x, price))| {
                        let anchor = if index == 0 {
                            "start"
                        } else if index == last_tick {
                            "end"
                        } else {
                            "middle"
                        };
                        html! {
                            <text class="axis x-axis" text-anchor={anchor}
                                x={format!("{x:.1}")} y={(layout.height - 5.0).to_string()}>
                                {DisplayFormat::usd_whole(*price)}
                            </text>
                        }
                    })}
                    {hovered.map_or_else(|| html! {}, |price| {
                        let x = chart.x_of(price);
                        let offset = (price - fit.price()) / fit.price() * 100.0;
                        let (anchor, label_x) = if x > layout.width / 2.0 { ("end", x - 6.0) } else { ("start", x + 6.0) };
                        html! {
                            <>
                                <line class="crosshair" x1={format!("{x:.1}")} x2={format!("{x:.1}")}
                                    y1={layout.top.to_string()} y2={bottom.to_string()} />
                                <text class="axis hover-price" text-anchor={anchor}
                                    x={format!("{label_x:.1}")} y={(layout.top + 10.0).to_string()}>
                                    {format!("{} ({})", DisplayFormat::usd_whole(price), DisplayFormat::percent_brief(offset))}
                                </text>
                            </>
                        }
                    })}
                </svg>
            }
        },
    );

    html! {
        <article class="card fit-card">
            <div class="plot" ref={plot}>{body}</div>
        </article>
    }
}

struct FitPanelParts;

impl FitPanelParts {
    fn frame(layout: ChartLayout, title: &str) -> Html {
        let bottom = layout.plot_bottom();
        html! {
            <svg
                class="chart fit-chart empty"
                viewBox={format!("0 0 {} {}", layout.width, layout.height)}
                preserveAspectRatio="none"
                role="img"
                aria-label={format!("{title} fit, waiting for the oracle")}
                aria-busy="true"
            >
                <line class="grid-line" x1={layout.left.to_string()} x2={(layout.width - layout.right).to_string()}
                    y1={bottom.to_string()} y2={bottom.to_string()} />
                <g class="axis-stubs">
                    {for layout.grid_columns(layout.x_tick_count()).into_iter().map(|x| {
                        let left = (x - 20.0).clamp(layout.left, layout.width - layout.right - 40.0);
                        html! {
                            <rect x={format!("{left:.1}")} y={(layout.height - 11.0).to_string()} width="40" height="6" rx="3" />
                        }
                    })}
                </g>
            </svg>
        }
    }
}
