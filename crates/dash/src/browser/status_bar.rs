use std::collections::HashMap;

use nostr_minions::{use_nostr_relay_pool, ReadyState};
use yew::prelude::*;

use crate::{DisplayFormat, OracleStatus};

pub struct ConnectionSummary;

impl ConnectionSummary {
    #[must_use]
    pub fn describe(health: &HashMap<String, ReadyState>) -> (&'static str, &'static str) {
        if health.is_empty() {
            return ("connecting", "Connecting");
        }
        if health.values().any(|state| *state == ReadyState::OPEN) {
            return ("open", "Connected");
        }
        if health.values().any(|state| *state == ReadyState::CONNECTING) {
            return ("connecting", "Connecting");
        }
        ("closed", "Disconnected")
    }
}

#[derive(Properties, PartialEq)]
pub struct StatusBarProps {
    pub now: i64,
    pub live: bool,
    pub status: Option<OracleStatus>,
    pub on_settings: Callback<MouseEvent>,
}

/// The bar across the top: the name, the relay connection, the chain heights the oracle
/// reports, and the clock.
#[function_component(StatusBar)]
pub fn status_bar(props: &StatusBarProps) -> Html {
    let health = use_nostr_relay_pool()
        .map(|pool| pool.relay_health())
        .unwrap_or_default();
    let (state, text) = ConnectionSummary::describe(&health);
    let status = props.status.as_ref();
    let height = |value: Option<u32>| value.map_or_else(|| "—".to_string(), |height| DisplayFormat::count(height as usize));

    html! {
        <header class="topbar">
            <div class="identity">
                <span class="wordmark">{"DELPHI"}</span>
                <span class="tagline">{"Bitcoin price oracle"}</span>
            </div>
            <dl class="vitals">
                <div title="relay connection">
                    <dt>{"Relay"}</dt>
                    <dd><span class={classes!("dot", state)} aria-hidden="true"></span>{text}</dd>
                </div>
                <div title="whether the oracle's live stream has spoken in the last 90 seconds">
                    <dt>{"Stream"}</dt>
                    <dd class={classes!(props.live.then_some("good"))}>{if props.live { "Live" } else { "Quiet" }}</dd>
                </div>
                <div title="newest block the oracle validated">
                    <dt>{"Tip"}</dt>
                    <dd class={classes!(status.and_then(|status| status.tip_stale).unwrap_or(false).then_some("warn"))}>
                        {height(status.and_then(|status| status.tip))}
                    </dd>
                </div>
                <div title="newest block deep enough for a settled price">
                    <dt>{"Settled"}</dt>
                    <dd>{height(status.and_then(|status| status.settled))}</dd>
                </div>
                <div>
                    <dt>{"Time"}</dt>
                    <dd>{DisplayFormat::clock(props.now)}</dd>
                </div>
            </dl>
            <button type="button" class="ghost" onclick={props.on_settings.clone()}>{"Settings"}</button>
        </header>
    }
}
