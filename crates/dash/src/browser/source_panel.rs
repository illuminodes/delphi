use nostr_minions::use_nostr_relay_pool;
use yew::prelude::*;

use super::color_scheme::ColorScheme;
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
    let scheme = use_state(ColorScheme::load);
    let on_scheme = {
        let scheme = scheme.clone();
        Callback::from(move |_: MouseEvent| {
            let next = scheme.toggled();
            next.choose();
            scheme.set(next);
        })
    };
    let (scheme_label, scheme_icon) = match *scheme {
        ColorScheme::Dark => ("Light mode", html! {
            <g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                <circle cx="12" cy="12" r="4" />
                <path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4" />
            </g>
        }),
        ColorScheme::Light => ("Dark mode", html! {
            <path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" />
        }),
    };
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
                <div class="header-actions">
                    <button type="button" class="ghost icon" onclick={on_scheme} aria-label={scheme_label} title={scheme_label}>
                        <svg viewBox="0 0 24 24" aria-hidden="true">{scheme_icon}</svg>
                    </button>
                    <button type="button" class="ghost icon" onclick={props.on_settings.clone()} aria-label="Settings" title="Settings">
                        <svg viewBox="0 0 24 24" aria-hidden="true">
                            <g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                                <circle cx="12" cy="12" r="3" />
                                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                            </g>
                        </svg>
                    </button>
                </div>
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
