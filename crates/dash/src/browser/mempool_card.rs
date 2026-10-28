use yew::prelude::*;

use super::box_size_hook::use_box_size;
use crate::{BlockFilter, DisplayFormat, GridTile, MempoolGrid, MempoolStream, MempoolTransaction, OracleStatus, TransactionStage};

#[derive(Properties, PartialEq)]
pub struct MempoolCardProps {
    pub stream: MempoolStream,
    pub status: Option<OracleStatus>,
    pub now: i64,
}

/// The streamed mempool transactions as squares, newest at the top left, each sized by its
/// dollar value and coloured by how far it got into the price. Hovering a square describes it;
/// clicking opens it on mempool.space. The legend dims every stage but the one picked.
/// Before the first transaction, the header and legend show with dashes over a pulsing empty grid.
#[function_component(MempoolCard)]
pub fn mempool_card(props: &MempoolCardProps) -> Html {
    let selected = use_state(|| None::<TransactionStage>);
    let hovered = use_state(|| None::<String>);
    let area = use_node_ref();
    let (width, height) = use_box_size(&area);
    let stream = &props.stream;
    let waiting = stream.is_empty();
    let transactions = stream.recent(None);
    let sides: Vec<u32> = transactions.iter().map(|transaction| MempoolGrid::side_for(transaction.usd_value())).collect();
    let grid = MempoolGrid::pack(&sides, MempoolCardParts::cells(width), MempoolCardParts::cells(height));
    let total = stream.total();
    let shown = hovered
        .as_ref()
        .and_then(|txid| transactions.iter().find(|transaction| &transaction.txid == txid).copied());

    let legend = TransactionStage::ALL.into_iter().map(|stage| {
        let count = stream.stage_count(stage);
        let active = *selected == Some(stage);
        let onclick = {
            let selected = selected.clone();
            Callback::from(move |_: MouseEvent| selected.set(if active { None } else { Some(stage) }))
        };
        html! {
            <button
                type="button"
                class={classes!("legend-item", stage.css_class(), active.then_some("active"))}
                title={stage.hint()}
                onclick={onclick}
            >
                <span class="swatch" aria-hidden="true"></span>
                {stage.label()}
                <span class="legend-count">{if waiting { "—".to_string() } else { DisplayFormat::count(count) }}</span>
            </button>
        }
    });
    let tiles = grid.tiles().iter().map(|tile| {
        let transaction = transactions[tile.index];
        let dimmed = selected.is_some_and(|stage| stage != transaction.stage);
        let enter = {
            let hovered = hovered.clone();
            let txid = transaction.txid.clone();
            Callback::from(move |_: MouseEvent| hovered.set(Some(txid.clone())))
        };
        html! {
            <a
                key={transaction.txid.clone()}
                class={classes!("tx-square", transaction.stage.css_class(), dimmed.then_some("dimmed"))}
                style={MempoolCardParts::place(tile)}
                href={format!("https://mempool.space/tx/{}", transaction.txid)}
                target="_blank"
                rel="noopener noreferrer"
                aria-label={format!("{} transaction {}", transaction.stage.label(), transaction.short_txid())}
                onmouseenter={enter}
                onfocus={
                    let hovered = hovered.clone();
                    let txid = transaction.txid.clone();
                    Callback::from(move |_: FocusEvent| hovered.set(Some(txid.clone())))
                }
            />
        }
    });
    let leave = {
        let hovered = hovered.clone();
        Callback::from(move |_: MouseEvent| hovered.set(None))
    };

    html! {
        <article class="card mempool-card">
            <header>
                {if waiting {
                    html! {
                        <>
                            <h2>{"— transactions"}</h2>
                            <span class="detail">{"waiting for the oracle to stream a mempool transaction"}</span>
                        </>
                    }
                } else {
                    html! {
                        <>
                            <h2>{format!("{} transactions", DisplayFormat::count(total))}</h2>
                            <span class="detail">
                                {format!(
                                    "{} entered the model ({:.0}%) · showing newest {}",
                                    DisplayFormat::count(stream.entered_model()),
                                    BlockFilter::share(stream.entered_model(), total) * 100.0,
                                    DisplayFormat::count(grid.tiles().len()),
                                )}
                            </span>
                            {MempoolCardParts::dropped(props.status.as_ref())}
                        </>
                    }
                }}
            </header>
            <nav class="tx-legend">{for legend}</nav>
            <div class={classes!("tx-grid-area", waiting.then_some("empty"))} ref={area} onmouseleave={leave} aria-busy={waiting.to_string()}>
                <div class="tx-grid" style={MempoolCardParts::extent(&grid)}>
                    {for tiles}
                </div>
                {MempoolCardParts::inspector(shown, props.now)}
            </div>
        </article>
    }
}

/// The pieces of the mempool card around the grid.
struct MempoolCardParts;

impl MempoolCardParts {
    /// One cell plus the gap after it, in CSS pixels.
    const CELL_PITCH: f64 = 12.0;
    const GAP: f64 = 2.0;

    fn span(cells: u32) -> f64 {
        f64::from(cells) * Self::CELL_PITCH - Self::GAP
    }

    /// A square's place as a transform, so a move between places can be animated.
    fn place(tile: &GridTile) -> String {
        format!(
            "--size:{:.0}px;transform:translate({:.0}px,{:.0}px)",
            Self::span(tile.side),
            f64::from(tile.x) * Self::CELL_PITCH,
            f64::from(tile.y) * Self::CELL_PITCH,
        )
    }

    fn extent(grid: &MempoolGrid) -> String {
        format!("width:{:.0}px;height:{:.0}px", Self::span(grid.columns()), Self::span(grid.rows()))
    }

    /// How many whole cells fit along a length, counting no gap after the last one.
    fn cells(length: f64) -> u32 {
        ((length + Self::GAP) / Self::CELL_PITCH).floor().max(1.0) as u32
    }

    fn dropped(status: Option<&OracleStatus>) -> Html {
        let Some(dropped) = status.map(|status| status.transaction_notes_dropped).filter(|dropped| *dropped > 0) else {
            return html! {};
        };
        html! {
            <span class="detail warn" title="the daemon's per-second note limit held these back; the model still saw them">
                {format!("{} not streamed", DisplayFormat::count(usize::try_from(dropped).unwrap_or(usize::MAX)))}
            </span>
        }
    }

    fn inspector(transaction: Option<&MempoolTransaction>, now: i64) -> Html {
        let Some(transaction) = transaction else {
            return html! {};
        };
        let outcome = transaction.rejection().map_or_else(|| Self::reading(transaction), |(label, hint)| {
            html! { <span title={hint}>{format!("rejected: {label}")}</span> }
        });
        html! {
            <div class={classes!("tx-inspector", transaction.stage.css_class())}>
                <span class="tx-stage">{transaction.stage.label()}</span>
                <span class="tx-id">{transaction.short_txid()}</span>
                <span class="tx-value">
                    {transaction.usd_value().map_or_else(|| "—".to_string(), DisplayFormat::usd_whole)}
                </span>
                <span class="detail">
                    {format!(
                        "{} in · {} out · {} B",
                        transaction.inputs,
                        transaction.outputs.len(),
                        DisplayFormat::count(transaction.size)
                    )}
                </span>
                <span class="tx-outcome">{outcome}</span>
                <span class="detail tx-age">{DisplayFormat::age(now - transaction.received_at)}</span>
            </div>
        }
    }

    fn reading(transaction: &MempoolTransaction) -> Html {
        match transaction.best_output() {
            Some(output) => html! {
                <span title={format!("{} sats", DisplayFormat::count(usize::try_from(output.sats).unwrap_or(usize::MAX)))}>
                    {output.usd.map_or_else(String::new, |usd| format!("${usd} output → "))}
                    {output.implied.map_or_else(|| "—".to_string(), |implied| format!("implies {}", DisplayFormat::usd_whole(implied)))}
                </span>
            },
            None => html! { <span>{transaction.stage.hint()}</span> },
        }
    }
}
