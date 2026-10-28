use yew::prelude::*;

use super::box_size_hook::use_box_size;
use super::pointer_reader::PointerReader;
use crate::{ChartGeometry, ChartLayout, CivilTime, DisplayFormat, SeriesPoint, WindowLabel};

#[derive(Properties, PartialEq)]
pub struct HistoryChartProps {
    pub window: AttrValue,
    pub points: Vec<SeriesPoint>,
    /// Notes held so far while the history is still loading.
    #[prop_or_default]
    pub loading: Option<usize>,
}

/// The audited price history as a line. Its frame and grid show from the start; the line draws
/// itself in once the history has arrived.
#[function_component(HistoryChart)]
pub fn history_chart(props: &HistoryChartProps) -> Html {
    let label = WindowLabel::of(&props.window);
    let hovered = use_state(|| None::<usize>);
    let plot = use_node_ref();
    let (width, height) = use_box_size(&plot);
    let layout = ChartLayout::for_series(width, height);
    let chart = ChartGeometry::new(&props.points, layout);
    let view_box = format!("0 0 {} {}", layout.width, layout.height);
    let aria = format!("{} price history", label.title(&props.window));

    let (readout, body) = chart.map_or_else(
        || {
            let waiting = props.loading.map_or_else(
                || "waiting for audited blocks".to_string(),
                |held| format!("loading audited blocks · {held} so far"),
            );
            let readout = html! {
                <div class="chart-readout">
                    <span class="readout-price pending">{"$—"}</span>
                    <span class="readout-time">{waiting}</span>
                </div>
            };
            let body = html! {
                <svg key="skeleton" class="chart skeleton" viewBox={view_box.clone()} preserveAspectRatio="none" role="img"
                    aria-label={aria.clone()} aria-busy="true">
                    {for layout.grid_rows(layout.y_tick_count()).into_iter().map(|y| html! {
                        <line class="grid-line" x1={layout.left.to_string()} x2={(layout.width - layout.right).to_string()}
                            y1={format!("{y:.1}")} y2={format!("{y:.1}")} />
                    })}
                </svg>
            };
            (readout, body)
        },
        |chart| {
            let last_index = chart.points().len() - 1;
            let active = hovered.unwrap_or(last_index).min(last_index);
            let selected = chart.points()[active];
            let (marker_x, marker_y) = chart.position(active).unwrap_or((0.0, 0.0));
            let change = chart.change_percent();
            let trend = if change >= 0.0 { "up" } else { "down" };
            let multi_day = chart.span_seconds() > 2 * 86_400;
            let x_ticks = chart.x_ticks(layout.x_tick_count());
            let last_tick = x_ticks.len().saturating_sub(1);
            let on_move = {
                let hovered = hovered.clone();
                let chart = chart.clone();
                let plot = plot.clone();
                Callback::from(move |event: MouseEvent| {
                    if let Some(x) = PointerReader::chart_x(&event, &plot, chart.layout()) {
                        hovered.set(Some(chart.index_at(x)));
                    }
                })
            };
            let on_leave = {
                let hovered = hovered.clone();
                Callback::from(move |_: MouseEvent| hovered.set(None))
            };
            let readout = html! {
                    <div class={classes!("chart-readout", trend)}>
                        <span class="readout-price">{DisplayFormat::usd(selected.price())}</span>
                        <span class="readout-time">
                            {CivilTime::from_unix(selected.stop_time()).full_label()}
                            {selected.stop_height().map_or_else(String::new, |height| {
                                format!(" · #{}", DisplayFormat::count(height as usize))
                            })}
                        </span>
                        <span class="readout-range" title="low – high over the shown history">
                            {format!("{} – {}", DisplayFormat::usd_whole(chart.low()), DisplayFormat::usd_whole(chart.high()))}
                        </span>
                        <span class={classes!("readout-change", trend)}>{DisplayFormat::percent_brief(change)}</span>
                    </div>
            };
            let body = html! {
                    <svg
                        key="chart"
                        class={classes!("chart", trend)}
                        viewBox={view_box.clone()}
                        preserveAspectRatio="none"
                        role="img"
                        aria-label={aria.clone()}
                        onmousemove={on_move}
                        onmouseleave={on_leave}
                    >
                        <defs>
                            <linearGradient id={format!("fill-{}", props.window)} x1="0" y1="0" x2="0" y2="1">
                                <stop offset="0%" class="fill-top" />
                                <stop offset="100%" class="fill-bottom" />
                            </linearGradient>
                        </defs>
                        {for chart.y_ticks(layout.y_tick_count()).iter().map(|tick| html! {
                            <g>
                                <line class="grid-line" x1={layout.left.to_string()} x2={(layout.width - layout.right).to_string()}
                                    y1={format!("{:.1}", tick.position)} y2={format!("{:.1}", tick.position)} />
                                <text class="axis y-axis" x={(layout.left - 6.0).to_string()}
                                    y={format!("{:.1}", tick.position + 4.0)}>
                                    {DisplayFormat::usd_whole(tick.value)}
                                </text>
                            </g>
                        })}
                        {for x_ticks.iter().enumerate().map(|(index, tick)| {
                            let time = CivilTime::from_unix(tick.value as i64);
                            let text = if multi_day { time.date_label() } else { time.time_label() };
                            let anchor = if index == 0 {
                                "start"
                            } else if index == last_tick {
                                "end"
                            } else {
                                "middle"
                            };
                            html! {
                                <text class="axis x-axis" text-anchor={anchor}
                                    x={format!("{:.1}", tick.position)} y={(layout.height - 5.0).to_string()}>
                                    {text}
                                </text>
                            }
                        })}
                        <g class="trace">
                            <path class="area" d={chart.area_path()} fill={format!("url(#fill-{})", props.window)} />
                            <path class="line" d={chart.line_path()} />
                        </g>
                        <g class="pointer">
                            <line class="crosshair" x1={format!("{marker_x:.1}")} x2={format!("{marker_x:.1}")}
                                y1={layout.top.to_string()} y2={layout.plot_bottom().to_string()} />
                            <circle class="marker" cx={format!("{marker_x:.1}")} cy={format!("{marker_y:.1}")} r="3.5" />
                        </g>
                    </svg>
            };
            (readout, body)
        },
    );

    html! {
        <article class="card chart-card">
            <header>
                <h2>{format!("{} history", label.title(&props.window))}</h2>
                <span class="detail">{format!("{} audited blocks", props.points.len())}</span>
            </header>
            {readout}
            <div class="plot" ref={plot}>{body}</div>
        </article>
    }
}
