use nostr_minions::{NostrRelayPoolProvider, UserRelay};
use yew::prelude::*;

use super::dashboard::Dashboard;
use super::settings_panel::SettingsPanel;
use super::settings_store::SettingsStore;

#[function_component(App)]
pub fn app() -> Html {
    let settings = use_memo((), |()| SettingsStore::load());
    let editing = use_state(|| false);

    let validated = settings.validate().ok();
    let on_open = {
        let editing = editing.clone();
        Callback::from(move |_| editing.set(true))
    };
    let on_close = {
        let editing = editing.clone();
        Callback::from(move |()| editing.set(false))
    };

    let Some((relay, publisher)) = validated else {
        return html! {
            <SettingsPanel initial={(*settings).clone()} dismissible={false} on_close={on_close} />
        };
    };
    let relays = vec![UserRelay {
        url: relay,
        read: true,
        write: false,
    }];

    html! {
        <NostrRelayPoolProvider relays={relays}>
            <Dashboard settings={(*settings).clone()} publisher={publisher} on_settings={on_open} />
            {if *editing {
                html! {
                    <SettingsPanel initial={(*settings).clone()} dismissible={true} on_close={on_close} />
                }
            } else {
                html! {}
            }}
        </NostrRelayPoolProvider>
    }
}
