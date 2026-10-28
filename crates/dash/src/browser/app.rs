use nostr_minions::{NostrRelayPoolProvider, UserRelay};
use yew::prelude::*;

use super::boot::Boot;
use super::dashboard::Dashboard;
use super::settings_panel::SettingsPanel;

/// Reads the saved state first, then draws the dashboard as it was left.
#[function_component(App)]
pub fn app() -> Html {
    let boot = use_state(|| None::<Boot>);
    {
        let boot = boot.clone();
        use_effect_with((), move |()| {
            wasm_bindgen_futures::spawn_local(async move { boot.set(Some(Boot::load().await)) });
        });
    }
    (*boot).clone().map_or_else(Html::default, |boot| html! { <Loaded boot={boot} /> })
}

#[derive(Properties, PartialEq)]
struct LoadedProps {
    boot: Boot,
}

#[function_component(Loaded)]
fn loaded(props: &LoadedProps) -> Html {
    let boot = &props.boot;
    let editing = use_state(|| false);
    let on_open = {
        let editing = editing.clone();
        Callback::from(move |_| editing.set(true))
    };
    let on_close = {
        let editing = editing.clone();
        Callback::from(move |()| editing.set(false))
    };

    let Ok((relay, publisher)) = boot.settings.validate() else {
        return html! {
            <SettingsPanel db={boot.db.clone()} initial={boot.settings.clone()} dismissible={false} on_close={on_close} />
        };
    };
    let relays = vec![UserRelay {
        url: relay,
        read: true,
        write: false,
    }];

    html! {
        <NostrRelayPoolProvider relays={relays}>
            <Dashboard boot={boot.clone()} publisher={publisher} on_settings={on_open} />
            {if *editing {
                html! {
                    <SettingsPanel db={boot.db.clone()} initial={boot.settings.clone()} dismissible={true} on_close={on_close} />
                }
            } else {
                html! {}
            }}
        </NostrRelayPoolProvider>
    }
}
