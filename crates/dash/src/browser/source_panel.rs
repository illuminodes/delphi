use nostr_minions::use_nostr_relay_pool;
use yew::prelude::*;

use super::connection_summary::ConnectionSummary;
use crate::{DisplayFormat, OracleStatus};

#[derive(Properties, PartialEq)]
pub struct SourcePanelProps {
    pub publisher: AttrValue,
    pub relay: AttrValue,
    pub verified: usize,
    pub refused: usize,
    pub unreadable: usize,
    pub last_note: Option<i64>,
    pub live: bool,
    pub status: Option<OracleStatus>,
    pub now: i64,
    pub on_settings: Callback<MouseEvent>,
}

/// Where the numbers come from and why to believe them: the relay connection and live stream,
/// the signing key, how many notes passed the signature check, and what the oracle reports
/// about its own work.
#[function_component(SourcePanel)]
pub fn source_panel(props: &SourcePanelProps) -> Html {
    let health = use_nostr_relay_pool().map(|pool| pool.relay_health()).unwrap_or_default();
    let (connection, connection_text) = ConnectionSummary::describe(&health);
    let status = props.status.as_ref();
    let row = |label: &'static str, value: String, hint: &'static str| {
        html! {
            <div title={hint}>
                <dt>{label}</dt>
                <dd>{value}</dd>
            </div>
        }
    };
    let verdict = if props.verified == 0 {
        html! { <p class="verdict pending">{"Waiting for a signed note"}</p> }
    } else {
        html! {
            <p class="verdict good" title="every note shown passed the publisher check and a schnorr signature check in this browser">
                {format!("{} notes verified in this browser", DisplayFormat::count(props.verified))}
            </p>
        }
    };

    html! {
        <article class="card source-card">
            <header>
                {verdict}
                <button type="button" class="ghost" onclick={props.on_settings.clone()}>{"Settings"}</button>
            </header>
            <dl class="source-facts">
                <div title="the relay this page reads from; it cannot forge notes">
                    <dt>{"Relay"}</dt>
                    <dd>
                        <span class={classes!("dot", connection)} aria-hidden="true"></span>
                        {format!("{connection_text} · {}", props.relay)}
                    </dd>
                </div>
                <div title="whether the oracle's live stream has spoken in the last 90 seconds">
                    <dt>{"Stream"}</dt>
                    <dd class={classes!(props.live.then_some("good"))}>{if props.live { "Live" } else { "Quiet" }}</dd>
                </div>
                <div title="the oracle's public key; every note is signed by it">
                    <dt>{"Signed by"}</dt>
                    <dd class="key">{props.publisher.clone()}</dd>
                </div>
                {row(
                    "Last note",
                    props.last_note.map_or_else(|| "—".to_string(), |at| DisplayFormat::age(props.now - at)),
                    "age of the newest signed note",
                )}
                {if props.refused > 0 {
                    row("Refused", DisplayFormat::count(props.refused), "notes from another key or with a bad signature, dropped")
                } else {
                    html! {}
                }}
                {if props.unreadable > 0 {
                    row(
                        "Unreadable",
                        DisplayFormat::count(props.unreadable),
                        "signed notes this page could not parse; a newer oracle may send fields it does not know",
                    )
                } else {
                    html! {}
                }}
                {status.map(|status| html! {
                    <>
                        {row(
                            "Audits",
                            DisplayFormat::count(usize::try_from(status.audits_published).unwrap_or(usize::MAX)),
                            "settled prices the oracle has published",
                        )}
                        {row(
                            "Mempool",
                            format!("{} outputs", DisplayFormat::count(status.mempool_outputs)),
                            "unconfirmed outputs in the live sample",
                        )}
                        {row("Uptime", DisplayFormat::duration(status.uptime), "time since the oracle started")}
                    </>
                }).unwrap_or_default()}
            </dl>
            <p class="method">
                {"Prices come from Bitcoin transactions only: headers and blocks are fetched from many independent peers and accepted once they agree at 6 confirmations. No exchange feeds and no trusted node."}
            </p>
        </article>
    }
}
