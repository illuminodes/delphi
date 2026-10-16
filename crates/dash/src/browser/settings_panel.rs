use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use super::settings_store::SettingsStore;
use crate::DashSettings;

pub struct InputText;

impl InputText {
    #[must_use]
    pub fn of(event: &InputEvent) -> String {
        event
            .target()
            .and_then(|target| target.dyn_into::<HtmlInputElement>().ok())
            .map(|input| input.value())
            .unwrap_or_default()
    }
}

pub struct PageReload;

impl PageReload {
    pub fn now() {
        if let Some(window) = web_sys::window() {
            let _ = window.location().reload();
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct SettingsPanelProps {
    pub initial: DashSettings,
    pub dismissible: bool,
    pub on_close: Callback<()>,
}

#[function_component(SettingsPanel)]
pub fn settings_panel(props: &SettingsPanelProps) -> Html {
    let draft = use_state(|| props.initial.clone());
    let error = use_state(|| Option::<String>::None);

    let on_relay = {
        let draft = draft.clone();
        Callback::from(move |event: InputEvent| {
            let mut next = (*draft).clone();
            next.relay = InputText::of(&event);
            draft.set(next);
        })
    };
    let on_publisher = {
        let draft = draft.clone();
        Callback::from(move |event: InputEvent| {
            let mut next = (*draft).clone();
            next.publisher = InputText::of(&event);
            draft.set(next);
        })
    };
    let on_save = {
        let draft = draft.clone();
        let error = error.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            match draft.validate() {
                Ok(_) => {
                    SettingsStore::save(&draft);
                    PageReload::now();
                }
                Err(problem) => error.set(Some(problem.to_string())),
            }
        })
    };
    let on_cancel = {
        let on_close = props.on_close.clone();
        Callback::from(move |_| on_close.emit(()))
    };

    html! {
        <div class="scrim">
            <form class="panel" onsubmit={on_save}>
                <h2>{"Feed settings"}</h2>
                <label>
                    {"Relay"}
                    <input type="text" value={draft.relay.clone()} oninput={on_relay}
                        placeholder="wss://relay.example" autocomplete="off" spellcheck="false" />
                </label>
                <label>
                    {"Publisher key"}
                    <input type="text" value={draft.publisher.clone()} oninput={on_publisher}
                        placeholder="npub1… or 64-digit hex" autocomplete="off" spellcheck="false" />
                </label>
                {error.as_ref().map_or_else(|| html! {}, |message| html! {
                    <p class="warn" role="alert">{message.clone()}</p>
                })}
                <div class="actions">
                    {if props.dismissible {
                        html! { <button type="button" class="ghost" onclick={on_cancel}>{"Cancel"}</button> }
                    } else {
                        html! {}
                    }}
                    <button type="submit">{"Save and connect"}</button>
                </div>
            </form>
        </div>
    }
}
