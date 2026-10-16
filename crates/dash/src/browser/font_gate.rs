use wasm_bindgen_futures::JsFuture;
use yew::UseStateHandle;

use crate::CarveFont;

pub struct FontGate;

impl FontGate {
    const PROBE_SIZE: f64 = 16.0;

    pub fn open(ready: UseStateHandle<bool>) {
        yew::platform::spawn_local(async move {
            if let Some(fonts) = web_sys::window().and_then(|window| window.document()).map(|document| document.fonts()) {
                for font in CarveFont::ALL {
                    let _ = JsFuture::from(fonts.load(&font.css(Self::PROBE_SIZE))).await;
                }
            }
            ready.set(true);
        });
    }
}
