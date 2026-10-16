use yew::prelude::*;

use crate::{BlockFilter, DisplayFormat, MempoolStream, MempoolTransaction, OracleStatus, TransactionStage};

#[derive(Properties, PartialEq)]
pub struct MempoolCardProps {
    pub stream: MempoolStream,
    pub status: Option<OracleStatus>,
    pub now: i64,
}

/// Every mempool transaction the oracle streamed since this page subscribed, sorted by how far
/// it got into the price, and the newest ones with their outputs read in dollars.
#[function_component(MempoolCard)]
pub fn mempool_card(props: &MempoolCardProps) -> Html {
    let selected = use_state(|| None::<TransactionStage>);
    let stream = &props.stream;
    if stream.is_empty() {
        return html! {
            <div class="card placeholder">
                {"Waiting for the oracle to stream a mempool transaction…"}
                <p class="detail">{"Daemons older than the transaction stream, or started with --tx-notes 0, send none."}</p>
            </div>
        };
    }
    let total = stream.total();
    let stages = TransactionStage::ALL.into_iter().map(|stage| {
        let count = stream.stage_count(stage);
        let share = BlockFilter::share(count, total);
        let active = *selected == Some(stage);
        let onclick = {
            let selected = selected.clone();
            Callback::from(move |_: MouseEvent| selected.set(if active { None } else { Some(stage) }))
        };
        html! {
            <li class={classes!(stage.css_class(), active.then_some("active"))} title={stage.hint()} onclick={onclick}>
                <span class="funnel-label">{stage.label()}</span>
                <span class="funnel-track">
                    <span class="funnel-bar" style={format!("width: {:.1}%", share * 100.0)}></span>
                </span>
                <span class="funnel-count">
                    {DisplayFormat::count(count)}
                    <span class="detail">{format!(" {:.0}%", share * 100.0)}</span>
                </span>
            </li>
        }
    });

    html! {
        <article class="card mempool-card">
            <header>
                <h2>{format!("{} transactions", DisplayFormat::count(total))}</h2>
                <span class="detail">
                    {format!(
                        "{} entered the model ({:.0}%)",
                        DisplayFormat::count(stream.entered_model()),
                        BlockFilter::share(stream.entered_model(), total) * 100.0
                    )}
                </span>
                {MempoolCardParts::dropped(props.status.as_ref())}
            </header>
            <section class="filter-funnel">
                <ul class="funnel-stages stage-picker">{for stages}</ul>
                <h3>{"Turned away"}</h3>
                <ul class="funnel-rejections">
                    {for stream.rejections().into_iter().map(|(verdict, count)| {
                        let (label, hint) = BlockFilter::rejection(verdict).unwrap_or((verdict, ""));
                        html! {
                            <li title={hint}>
                                <span class="fact-label">{label}</span>
                                {DisplayFormat::count(count)}
                                <span class="detail">{format!(" {:.0}%", BlockFilter::share(count, total) * 100.0)}</span>
                            </li>
                        }
                    })}
                </ul>
            </section>
            <h3 class="list-title">
                {selected.map_or_else(|| "Newest".to_string(), |stage| format!("Newest · {}", stage.label()))}
            </h3>
            <ul class="tx-list">
                {for stream.recent(*selected).into_iter().map(|transaction| MempoolCardParts::row(transaction, props.now))}
            </ul>
        </article>
    }
}

/// The pieces of the mempool card that are not the stage picker.
struct MempoolCardParts;

impl MempoolCardParts {
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

    fn row(transaction: &MempoolTransaction, now: i64) -> Html {
        let reason = transaction.rejection().map_or_else(
            || Self::reading(transaction),
            |(label, hint)| html! { <span class="tx-reason" title={hint}>{label}</span> },
        );
        let outputs: Vec<String> = transaction
            .outputs
            .iter()
            .map(|output| format!("{:.8} ₿{}", output.btc(), if output.admitted { "" } else { " (not kept)" }))
            .collect();
        html! {
            <li class={classes!("tx-row", transaction.stage.css_class())}>
                <span class="tx-stage" title={transaction.stage.hint()}>{transaction.stage.label()}</span>
                <a
                    class="tx-id"
                    href={format!("https://mempool.space/tx/{}", transaction.txid)}
                    target="_blank"
                    rel="noopener noreferrer"
                    title={transaction.txid.clone()}
                >
                    {transaction.short_txid()}
                </a>
                <span class="tx-shape" title={outputs.join("\n")}>
                    {format!("{} in · {} out · {} B", transaction.inputs, transaction.outputs.len(), DisplayFormat::count(transaction.size))}
                </span>
                {reason}
                <span class="detail">{DisplayFormat::age(now - transaction.received_at)}</span>
            </li>
        }
    }

    fn reading(transaction: &MempoolTransaction) -> Html {
        match transaction.best_output() {
            Some(output) => html! {
                <span class="tx-reason" title={format!("{} sats", DisplayFormat::count(usize::try_from(output.sats).unwrap_or(usize::MAX)))}>
                    {output.usd.map_or_else(String::new, |usd| format!("${usd} → "))}
                    {output.implied.map_or_else(|| "—".to_string(), DisplayFormat::usd_whole)}
                </span>
            },
            None => html! { <span class="tx-reason">{"—"}</span> },
        }
    }
}
