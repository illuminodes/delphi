use wasm_bindgen::JsCast;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::HistoryRange;

#[derive(Properties, PartialEq)]
pub struct HistoryRangeInputProps {
    pub range: HistoryRange,
    pub on_change: Callback<HistoryRange>,
}

/// The number of days of audited history to ask the relay for. A new value re-queries the relay
/// when submitted or when the field loses focus.
#[function_component(HistoryRangeInput)]
pub fn history_range_input(props: &HistoryRangeInputProps) -> Html {
    let draft = use_state(|| props.range.day_count().to_string());
    let field = use_node_ref();

    {
        let draft = draft.clone();
        use_effect_with(props.range, move |range| {
            draft.set(range.day_count().to_string());
        });
    }

    let apply = {
        let field = field.clone();
        let draft = draft.clone();
        let current = props.range;
        let on_change = props.on_change.clone();
        Callback::from(move |()| {
            let typed = field
                .cast::<HtmlInputElement>()
                .map(|input| input.value())
                .unwrap_or_default();
            match HistoryRange::parse(&typed) {
                Some(range) if range != current => on_change.emit(range),
                Some(range) => draft.set(range.day_count().to_string()),
                None => draft.set(current.day_count().to_string()),
            }
        })
    };
    let on_submit = {
        let apply = apply.clone();
        Callback::from(move |event: SubmitEvent| {
            event.prevent_default();
            apply.emit(());
        })
    };
    let on_blur = {
        let apply = apply.clone();
        Callback::from(move |_: FocusEvent| apply.emit(()))
    };
    let on_input = {
        let draft = draft.clone();
        Callback::from(move |event: InputEvent| {
            if let Some(input) = event.target().and_then(|target| target.dyn_into::<HtmlInputElement>().ok()) {
                draft.set(input.value());
            }
        })
    };
    let preset = |days: u32| {
        let on_change = props.on_change.clone();
        let active = props.range.day_count() == days;
        html! {
            <button
                type="button"
                class={classes!("range-preset", active.then_some("active"))}
                onclick={Callback::from(move |_: MouseEvent| on_change.emit(HistoryRange::days(days)))}
            >
                {format!("{days}d")}
            </button>
        }
    };

    html! {
        <form class="history-range" onsubmit={on_submit}>
            <label>
                <input
                    ref={field}
                    type="number"
                    inputmode="numeric"
                    min={HistoryRange::MIN_DAYS.to_string()}
                    max={HistoryRange::MAX_DAYS.to_string()}
                    value={(*draft).clone()}
                    oninput={on_input}
                    onblur={on_blur}
                    aria-label="days of history to load from the relay"
                />
                {"days"}
            </label>
            {preset(1)}
            {preset(7)}
            {preset(30)}
            {preset(90)}
        </form>
    }
}
