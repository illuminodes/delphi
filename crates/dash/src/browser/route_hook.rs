use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use yew::prelude::*;

use crate::ViewRoute;

struct HashListener {
    callback: Closure<dyn FnMut()>,
}

impl HashListener {
    const EVENT: &'static str = "hashchange";

    fn current() -> ViewRoute {
        let hash = web_sys::window()
            .and_then(|window| window.location().hash().ok())
            .unwrap_or_default();
        ViewRoute::from_hash(&hash)
    }

    fn start(route: UseStateHandle<ViewRoute>) -> Self {
        let callback = Closure::<dyn FnMut()>::new(move || route.set(Self::current()));
        if let Some(window) = web_sys::window() {
            let _ = window.add_event_listener_with_callback(Self::EVENT, callback.as_ref().unchecked_ref());
        }
        Self { callback }
    }
}

impl Drop for HashListener {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            let _ = window.remove_event_listener_with_callback(Self::EVENT, self.callback.as_ref().unchecked_ref());
        }
    }
}

#[hook]
pub fn use_view_route() -> ViewRoute {
    let route = use_state(HashListener::current);
    use_effect_with((), {
        let route = route.clone();
        move |()| {
            let listener = HashListener::start(route);
            move || drop(listener)
        }
    });
    *route
}
