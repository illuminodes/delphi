use web_sys::Storage;

/// The page's light or dark palette: picked by the reader, else the system's preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Dark,
    Light,
}

impl ColorScheme {
    const KEY: &'static str = "delphi-dash.color-scheme";

    fn storage() -> Option<Storage> {
        web_sys::window()?.local_storage().ok()?
    }

    fn system() -> Self {
        let light = web_sys::window()
            .and_then(|window| window.match_media("(prefers-color-scheme: light)").ok().flatten())
            .is_some_and(|query| query.matches());
        if light {
            Self::Light
        } else {
            Self::Dark
        }
    }

    #[must_use]
    pub fn load() -> Self {
        match Self::storage().and_then(|storage| storage.get_item(Self::KEY).ok().flatten()).as_deref() {
            Some("light") => Self::Light,
            Some("dark") => Self::Dark,
            _ => Self::system(),
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Dark => Self::Light,
            Self::Light => Self::Dark,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }

    const FADE_MS: u32 = 400;

    /// Fades the page into this palette and remembers it for the next visit. `index.html` reads it
    /// back before the first paint. The fade is switched on only while the palette changes, so it
    /// never slows hover states.
    pub fn choose(self) {
        if let Some(root) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.document_element())
        {
            let _ = root.set_attribute("data-theme-fade", "");
            let _ = root.set_attribute("data-theme", self.name());
            gloo_timers::callback::Timeout::new(Self::FADE_MS, move || {
                let _ = root.remove_attribute("data-theme-fade");
            })
            .forget();
        }
        if let Some(storage) = Self::storage() {
            let _ = storage.set_item(Self::KEY, self.name());
        }
    }
}
