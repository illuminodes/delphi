use yew::prelude::*;

use crate::{WindowLabel, WindowTabs};

#[derive(Properties, PartialEq)]
pub struct TabbedCardProps {
    pub tabs: WindowTabs,
    pub render: Callback<String, Html>,
    pub waiting: AttrValue,
}

/// A card that shows one window at a time, chosen from a tab strip along its top.
/// The choice survives windows coming and going; if the chosen one disappears, the first tab shows.
#[function_component(TabbedCard)]
pub fn tabbed_card(props: &TabbedCardProps) -> Html {
    let chosen = use_state(|| None::<String>);
    let Some(showing) = props.tabs.showing(chosen.as_deref()).map(ToString::to_string) else {
        return html! { <div class="card placeholder">{props.waiting.clone()}</div> };
    };

    html! {
        <div class="tabbed">
            {if props.tabs.windows().len() > 1 {
                html! {
                    <nav class="window-tabs" role="tablist">
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
                    </nav>
                }
            } else {
                html! {}
            }}
            <div class="tabbed-body">{props.render.emit(showing)}</div>
        </div>
    }
}
