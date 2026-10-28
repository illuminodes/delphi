use yew::prelude::*;

use crate::{WindowLabel, WindowTabs};

#[derive(Properties, PartialEq)]
pub struct TabbedCardProps {
    pub tabs: WindowTabs,
    pub render: Callback<String, Html>,
    pub waiting: AttrValue,
    /// Controls shown before the tabs, in the same row.
    #[prop_or_default]
    pub lead: Html,
    /// Facts about the showing window, shown after the tabs, in the same row.
    #[prop_or_default]
    pub aside: Option<Callback<String, Html>>,
}

/// A card that shows one window at a time, chosen from a tab strip along its top.
/// The choice survives windows coming and going; if the chosen one disappears, the first tab shows.
/// Switching tabs mounts the window afresh, so its entry animations play again.
#[function_component(TabbedCard)]
pub fn tabbed_card(props: &TabbedCardProps) -> Html {
    let chosen = use_state(|| None::<String>);
    let Some(showing) = props.tabs.showing(chosen.as_deref()).map(ToString::to_string) else {
        return html! { <div class="card placeholder">{props.waiting.clone()}</div> };
    };
    let several = props.tabs.windows().len() > 1;
    let tabs = if several {
        html! {
            <div class="window-tabs" role="tablist">
                {for props.tabs.windows().iter().map(|window| {
                    let active = *window == showing;
                    let pick = {
                        let chosen = chosen.clone();
                        let window = window.clone();
                        Callback::from(move |_: MouseEvent| chosen.set(Some(window.clone())))
                    };
                    html! {
                        <button
                            type="button"
                            role="tab"
                            aria-selected={active.to_string()}
                            class={classes!("window-tab", active.then_some("active"))}
                            onclick={pick}
                        >
                            {WindowLabel::of(window).title(window)}
                        </button>
                    }
                })}
            </div>
        }
    } else {
        Html::default()
    };
    let aside = props.aside.as_ref().map(|aside| aside.emit(showing.clone()));
    let row = if several || aside.is_some() || props.lead != Html::default() {
        html! {
            <nav class="tab-row">
                {props.lead.clone()}
                {tabs}
                {aside.map_or_else(Html::default, |aside| html! { <div class="tab-aside">{aside}</div> })}
            </nav>
        }
    } else {
        Html::default()
    };

    html! {
        <div class="tabbed">
            {row}
            <div class="tabbed-body">
                <div class="tabbed-window" key={showing.clone()}>{props.render.emit(showing)}</div>
            </div>
        </div>
    }
}
