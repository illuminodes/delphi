use yew::prelude::*;

#[function_component(MeanderRule)]
pub fn meander_rule() -> Html {
    html! {
        <svg class="meander" aria-hidden="true" preserveAspectRatio="none">
            <defs>
                <pattern id="delphi-meander" width="28" height="14" patternUnits="userSpaceOnUse">
                    <path
                        d="M0 13 H24 V1 H6 V9 H16 V5"
                        fill="none"
                        stroke="currentColor"
                        stroke-width="1.6"
                        stroke-linecap="square"
                    />
                </pattern>
            </defs>
            <rect width="100%" height="14" fill="url(#delphi-meander)" />
        </svg>
    }
}
