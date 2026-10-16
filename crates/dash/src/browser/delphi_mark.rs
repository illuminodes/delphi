use yew::prelude::*;

#[derive(Properties, PartialEq)]
pub struct DelphiMarkProps {
    #[prop_or_default]
    pub class: Classes,
}

#[function_component(DelphiMark)]
pub fn delphi_mark(props: &DelphiMarkProps) -> Html {
    html! {
        <svg class={props.class.clone()} viewBox="0 0 120 120" role="img" aria-label="Delphi">
            <defs>
                <radialGradient id="delphi-mark-chasm" cx="50%" cy="85%" r="75%">
                    <stop offset="0%" stop-color="var(--delphi-pneuma)" />
                    <stop offset="100%" stop-color="var(--delphi-chasm)" />
                </radialGradient>
            </defs>
            <circle cx="60" cy="60" r="56" fill="var(--delphi-aurum)" />
            <circle cx="60" cy="60" r="51" fill="url(#delphi-mark-chasm)" />
            <path class="wisp" d="M54 72 C48 62 60 56 54 46 C50 39 56 34 54 28" fill="none" stroke="var(--delphi-vapour)" stroke-width="2.5" stroke-linecap="round" />
            <path class="wisp late" d="M66 72 C72 63 60 56 66 47 C70 40 64 35 66 30" fill="none" stroke="var(--delphi-vapour)" stroke-width="2.5" stroke-linecap="round" />
            <path d="M38 72 H82" stroke="var(--delphi-helios)" stroke-width="5" stroke-linecap="square" />
            <path d="M44 72 L36 96 M60 72 V98 M76 72 L84 96" stroke="var(--delphi-aurum)" stroke-width="4" stroke-linecap="square" />
        </svg>
    }
}
