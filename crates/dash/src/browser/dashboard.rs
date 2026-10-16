use yew::prelude::*;

use super::blockchain_card::BlockchainCard;
use super::board_hook::use_dash_state;
use super::clock_hook::use_unix_seconds;
use super::fit_panel::FitPanel;
use super::history_chart::HistoryChart;
use super::history_range_input::HistoryRangeInput;
use super::mempool_card::MempoolCard;
use super::price_card::PriceCard;
use super::status_bar::StatusBar;
use super::tabbed_card::TabbedCard;
use super::tile_workspace::TileWorkspace;
use crate::{DashSettings, FeedFilter, HistoryRange, PanelId, PublisherKey, WindowLabel, WindowTabs};

#[derive(Properties, PartialEq)]
pub struct DashboardProps {
    pub settings: DashSettings,
    pub publisher: PublisherKey,
    pub on_settings: Callback<MouseEvent>,
}

#[function_component(Dashboard)]
pub fn dashboard(props: &DashboardProps) -> Html {
    let range = use_state(HistoryRange::default);
    let state = use_dash_state(&props.publisher, *range);
    let now = use_unix_seconds(1_000);
    let board = state.prices();
    let notes = board.ordered();
    let fits = state.fits();
    let windows: Vec<&str> = notes.iter().map(|note| note.window()).collect();
    let charted: Vec<&str> = windows
        .iter()
        .copied()
        .filter(|window| *window != "mempool" && board.history(window).len() >= 2)
        .collect();
    let fitted: Vec<&str> = fits.ordered().iter().map(|fit| fit.window()).collect();
    let history_tabs = WindowTabs::longest_first(&charted).led_by(WindowLabel::DAILY);
    let loaded: usize = FeedFilter::WINDOWS.iter().map(|window| board.history(window).len()).sum();
    let on_range = {
        let range = range.clone();
        Callback::from(move |picked: HistoryRange| range.set(picked))
    };
    let fit_tabs = WindowTabs::shortest_first(&fitted);
    let price_tabs = WindowTabs::shortest_first(&windows);
    let blocks = state.blocks().cloned();
    let mempool = state.mempool().clone();
    let oracle_status = state.status().cloned();

    let status = html! {
        <StatusBar
            relay={props.settings.relay.clone()}
            publisher={props.publisher.short()}
            now={now}
            rejected={state.rejected()}
            live={state.is_live(now)}
            status={state.status().cloned()}
            on_settings={props.on_settings.clone()}
        />
    };
    let waiting = notes.is_empty();

    let render = {
        let notes: Vec<_> = notes.iter().map(|note| (*note).clone()).collect();
        let board = board.clone();
        let fits = fits.clone();
        let price = Callback::from(move |window: String| {
            notes.iter().find(|note| note.window() == window).map_or_else(Html::default, |note| {
                let sparkline = if window == "mempool" { board.history(&window) } else { Vec::new() };
                html! {
                    <PriceCard
                        note={note.clone()}
                        sparkline={sparkline}
                        now={now}
                        spread={board.spread_percent(&window, "chain-1h").filter(|_| window != "chain-1h")}
                    />
                }
            })
        });
        let history = {
            let board = state.prices().clone();
            Callback::from(move |window: String| html! {
                <HistoryChart window={window.clone()} points={board.history(&window)} />
            })
        };
        let fit = Callback::from(move |window: String| {
            fits.latest(&window).map_or_else(Html::default, |fit| html! { <FitPanel fit={fit.clone()} /> })
        });
        Callback::from(move |panel: PanelId| match panel {
            PanelId::Info => html! {
                <>
                    {status.clone()}
                    {waiting.then(|| html! { <p class="empty">{"Waiting for the first price from this publisher…"}</p> })}
                </>
            },
            PanelId::Price => html! {
                <TabbedCard tabs={price_tabs.clone()} render={price.clone()} waiting="Waiting for the first price…" />
            },
            PanelId::Fit => html! {
                <TabbedCard tabs={fit_tabs.clone()} render={fit.clone()} waiting="Waiting for a fit…" />
            },
            PanelId::History => html! {
                <div class="history-panel">
                    <HistoryRangeInput range={*range} loaded={loaded} on_change={on_range.clone()} />
                    <TabbedCard tabs={history_tabs.clone()} render={history.clone()} waiting="Waiting for audited blocks…" />
                </div>
            },
            PanelId::Blockchain => html! { <BlockchainCard blocks={blocks.clone()} now={now} /> },
            PanelId::Mempool => html! {
                <MempoolCard stream={mempool.clone()} status={oracle_status.clone()} now={now} />
            },
        })
    };

    html! {
        <div class="shell">
            <main class="workspace">
                <TileWorkspace render={render} />
            </main>
        </div>
    }
}
