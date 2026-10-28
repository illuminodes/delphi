use yew::prelude::*;

/// Icon links to the project's code, its X account and its website.
pub struct ProjectLinks;

impl ProjectLinks {
    const GITHUB: &'static str = "https://github.com/illuminodes/delphi";
    const X: &'static str = "https://x.com/openlabs21m";
    const WEBSITE: &'static str = "https://www.openlabs.lat/";

    const GITHUB_PATH: &'static str = "M12 .3a12 12 0 0 0-3.8 23.38c.6.11.82-.26.82-.58v-2.04c-3.34.73-4.04-1.61-4.04-1.61-.55-1.39-1.33-1.76-1.33-1.76-1.09-.74.08-.73.08-.73 1.2.08 1.84 1.24 1.84 1.24 1.07 1.83 2.81 1.3 3.5 1 .1-.78.42-1.31.76-1.61-2.67-.3-5.47-1.33-5.47-5.93 0-1.31.47-2.38 1.24-3.22-.14-.3-.54-1.52.1-3.18 0 0 1.01-.32 3.3 1.23a11.5 11.5 0 0 1 6 0c2.28-1.55 3.29-1.23 3.29-1.23.64 1.66.24 2.88.12 3.18a4.65 4.65 0 0 1 1.23 3.22c0 4.61-2.8 5.63-5.48 5.92.43.37.82 1.1.82 2.22v3.29c0 .32.21.7.82.58A12 12 0 0 0 12 .3";
    const X_PATH: &'static str = "M18.9 1.15h3.68l-8.04 9.19L24 22.85h-7.4l-5.8-7.59-6.64 7.59H.47l8.6-9.83L0 1.15h7.6l5.24 6.93zm-1.29 19.5h2.04L6.49 3.24H4.3z";

    pub fn render() -> Html {
        html! {
            <nav class="project-links">
                {Self::link(Self::GITHUB, "Source code on GitHub", html! {
                    <path d={Self::GITHUB_PATH} fill="currentColor" />
                })}
                {Self::link(Self::X, "OpenLabs on X", html! {
                    <path d={Self::X_PATH} fill="currentColor" />
                })}
                {Self::link(Self::WEBSITE, "OpenLabs website", html! {
                    <g fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
                        <circle cx="12" cy="12" r="10" />
                        <path d="M2 12h20M12 2a15.3 15.3 0 0 1 0 20M12 2a15.3 15.3 0 0 0 0 20" />
                    </g>
                })}
            </nav>
        }
    }

    fn link(href: &'static str, label: &'static str, icon: Html) -> Html {
        html! {
            <a href={href} target="_blank" rel="noopener noreferrer" aria-label={label} title={label}>
                <svg viewBox="0 0 24 24" aria-hidden="true">{icon}</svg>
            </a>
        }
    }
}
