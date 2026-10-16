use std::collections::HashMap;

use nostr_minions::{use_nostr_relay_pool, ReadyState};
use yew::prelude::*;

use crate::{DisplayFormat, OracleStatus};

pub struct ConnectionSummary;

impl ConnectionSummary {
    #[must_use]
    pub fn describe(health: &HashMap<String, ReadyState>) -> (&'static str, &'static str) {
        if health.is_empty() {
            return ("connecting", "Connecting…");
        }
        if health.values().any(|state| *state == ReadyState::OPEN) {
            return ("open", "Connected");
        }
        if health.values().any(|state| *state == ReadyState::CONNECTING) {
            return ("connecting", "Connecting…");
        }
        ("closed", "Disconnected")
    }
}

#[derive(Properties, PartialEq)]
pub struct StatusBarProps {
    pub relay: AttrValue,
    pub publisher: AttrValue,
    pub now: i64,
    pub rejected: usize,
    pub live: bool,
    pub status: Option<OracleStatus>,
    pub on_settings: Callback<MouseEvent>,
}

#[function_component(StatusBar)]
pub fn status_bar(props: &StatusBarProps) -> Html {
    let health = use_nostr_relay_pool()
        .map(|pool| pool.relay_health())
        .unwrap_or_default();
    let (state, text) = ConnectionSummary::describe(&health);

    html! {
        <article class="card status">
            <div class="brand">
                <span class={classes!("dot", state)} aria-hidden="true"></span>
                <a class="wordmark-link" href="#delphi" title="Delphi design system"><strong>{"Mantle oracle"}</strong></a>
                <span class="state">{text}</span>
                <span
                    class={classes!("live-badge", props.live.then_some("on"))}
                    title="Notes the oracle streams while it works; they are not stored by the relay"
                >
                    {if props.live { "● live" } else { "○ live stream quiet" }}
                </span>
                {props.status.as_ref().map_or_else(|| html! {}, |status| html! {
                    <span class="oracle-status">
                        {status.tip.map_or_else(String::new, |tip| format!("tip {} · ", DisplayFormat::count(tip as usize)))}
                        {format!(
                            "{} mempool outputs · {} audits sent",
                            DisplayFormat::count(status.mempool_outputs),
                            DisplayFormat::count(usize::try_from(status.audits_published).unwrap_or(usize::MAX)),
                        )}
                    </span>
                })}
            </div>
            <div class="meta">
                <span title="relay">{props.relay.clone()}</span>
                <span title="publisher">{props.publisher.clone()}</span>
                <span>{DisplayFormat::clock(props.now)}</span>
                {if props.rejected > 0 {
                    html! { <span class="warn">{format!("{} unreadable", props.rejected)}</span> }
                } else {
                    html! {}
                }}
                <button type="button" onclick={props.on_settings.clone()}>{"Settings"}</button>
            </div>
        </article>
    }
}
