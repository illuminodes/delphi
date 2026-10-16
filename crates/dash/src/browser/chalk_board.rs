use yew::prelude::*;

use super::pointer_reader::PointerReader;
use crate::{ChalkPlot, CivilTime, DisplayFormat, ElementId};

#[derive(Properties, PartialEq)]
pub struct ChalkBoardProps {
    pub plot: ChalkPlot,
    pub active: usize,
    pub on_hover: Callback<Option<usize>>,
}

#[function_component(ChalkBoard)]
pub fn chalk_board(props: &ChalkBoardProps) -> Html {
    let chalk_id = use_memo((), |()| ElementId::next("delphi-chalk"));
    let chalk_id = (*chalk_id).clone();
    let plot = &props.plot;
    let chart = plot.chart();
    let layout = *chart.layout();
    let chalk = format!("url(#{chalk_id})");

    let on_move = {
        let on_hover = props.on_hover.clone();
        let chart = chart.clone();
        Callback::from(move |event: MouseEvent| {
            if let Some(x) = PointerReader::chart_x(&event, chart.layout()) {
                on_hover.emit(Some(chart.index_at(x)));
            }
        })
    };
    let on_leave = {
        let on_hover = props.on_hover.clone();
        Callback::from(move |_: MouseEvent| on_hover.emit(None))
    };

    html! {
        <svg
            class="board"
            viewBox={format!("0 0 {} {}", layout.width, layout.height)}
            preserveAspectRatio="none"
            role="img"
            aria-label="price line chart"
            onmousemove={on_move}
            onmouseleave={on_leave}
        >
            <defs>
                <filter id={chalk_id} x="-5%" y="-5%" width="110%" height="110%">
                    <feTurbulence type="fractalNoise" baseFrequency="0.85" numOctaves="2" seed="5" result="grain" />
                    <feDisplacementMap in="SourceGraphic" in2="grain" scale="2.4" xChannelSelector="R" yChannelSelector="G" result="rough" />
                    <feColorMatrix in="grain" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 0 0 0  2.8 0 0 0 -0.62" result="gaps" />
                    <feComposite in="rough" in2="gaps" operator="in" />
                </filter>
            </defs>
            <g filter={chalk.clone()}>
                {for plot.rules().iter().map(|rule| html! { <path class="rule" d={rule.path.clone()} /> })}
            </g>
            {for plot.rules().iter().map(|rule| html! {
                <text class="label y" x={format!("{:.1}", layout.left - 10.0)} y={format!("{:.1}", rule.tick.position + 3.5)}>
                    {DisplayFormat::usd_whole(rule.tick.value)}
                </text>
            })}
            {BoardAxis::time_labels(plot)}
            <g filter={chalk}>
                {plot.cursor(props.active).map_or_else(|| html! {}, |path| html! { <path class="cursor" d={path} /> })}
                {plot.ring(props.active).map_or_else(|| html! {}, |path| html! { <path class="ring" d={path} /> })}
            </g>
        </svg>
    }
}

struct BoardAxis;

impl BoardAxis {
    const MULTI_DAY_SECONDS: i64 = 2 * 86_400;

    fn time_labels(plot: &ChalkPlot) -> Html {
        let chart = plot.chart();
        let layout = chart.layout();
        let ticks = chart.x_ticks(layout.x_tick_count());
        let last = ticks.len().saturating_sub(1);
        let multi_day = chart.span_seconds() > Self::MULTI_DAY_SECONDS;
        html! {
            {for ticks.iter().enumerate().map(|(index, tick)| {
                let time = CivilTime::from_unix(tick.value as i64);
                let anchor = match index {
                    0 => "start",
                    _ if index == last => "end",
                    _ => "middle",
                };
                html! {
                    <text class="label x" text-anchor={anchor} x={format!("{:.1}", tick.position)} y={format!("{:.1}", layout.height - 9.0)}>
                        {if multi_day { time.date_label() } else { time.time_label() }}
                    </text>
                }
            })}
        }
    }
}
