use nostr_minions::use_nostr_relay_pool;
use yew::prelude::*;

use super::connection_summary::ConnectionSummary;
use super::project_links::ProjectLinks;
use crate::DisplayFormat;

#[derive(Properties, PartialEq)]
pub struct SourcePanelProps {
    pub publisher: AttrValue,
    pub relay: AttrValue,
    pub verified: usize,
    pub refused: usize,
    pub unreadable: usize,
    pub live: bool,
    pub saved_only: bool,
    pub on_settings: Callback<MouseEvent>,
}

/// Where the numbers come from and why to believe them: the relay, the signing key, how many
/// notes passed the signature check here, and how the oracle reaches its prices.
#[function_component(SourcePanel)]
pub fn source_panel(props: &SourcePanelProps) -> Html {
    let health = use_nostr_relay_pool().map(|pool| pool.relay_health()).unwrap_or_default();
    let (connection, connection_text) = ConnectionSummary::describe(&health);
    let quiet = connection == "open" && !props.live;
    let relay = props.relay.trim_start_matches("wss://").trim_start_matches("ws://");
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
                <div class="identity">
                    <span class="wordmark">{"DELPHI"}</span>
                    <span class="tagline">{"Bitcoin price oracle"}</span>
                </div>
                <button type="button" class="ghost" onclick={props.on_settings.clone()}>{"Settings"}</button>
            </header>
            {verdict}
            <dl class="source-facts">
                <div title={format!("{connection_text}; the relay this page reads from, which cannot forge notes")}>
                    <dt>{"Relay"}</dt>
                    <dd>
                        <span class={classes!("dot", if quiet { "connecting" } else { connection })} aria-hidden="true"></span>
                        {if quiet { format!("{relay} · quiet") } else { relay.to_string() }}
                    </dd>
                </div>
                <div title="the oracle's public key; every note is signed by it">
                    <dt>{"Signed by"}</dt>
                    <dd class="key">{props.publisher.clone()}</dd>
                </div>
                {if props.saved_only {
                    row("Showing", "Saved from the last visit".to_string(), "status, blocks and fits saved in this browser, signatures checked again; replaced as fresh notes arrive")
                } else {
                    html! {}
                }}
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
            </dl>
            <p class="method">
                {"Prices come only from Bitcoin transactions, in blocks that many independent peers agree on at 6 confirmations. No exchanges, no trusted node."}
            </p>
            {ProjectLinks::render()}
        </article>
    }
}
