#[cfg(target_arch = "wasm32")]
fn main() {
    if nostr_minions::is_relay_worker() {
        return;
    }
    yew::Renderer::<mantle_dash::browser::App>::new().render();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("mantle-dash runs in a browser: build it with `trunk serve`");
}
