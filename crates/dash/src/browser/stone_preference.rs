use crate::MarbleKind;

pub struct StonePreference;

impl StonePreference {
    const LIGHT_QUERY: &'static str = "(prefers-color-scheme: light)";

    pub fn initial() -> MarbleKind {
        let light = web_sys::window()
            .and_then(|window| window.match_media(Self::LIGHT_QUERY).ok().flatten())
            .is_some_and(|query| query.matches());
        if light { MarbleKind::Parian } else { MarbleKind::Nero }
    }

    pub const fn other(kind: MarbleKind) -> MarbleKind {
        match kind {
            MarbleKind::Nero => MarbleKind::Parian,
            _ => MarbleKind::Nero,
        }
    }
}
