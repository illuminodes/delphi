use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{Element, ResizeObserver};
use yew::UseStateHandle;

/// Keeps a state handle set to an element's rendered size until dropped.
pub struct BoxWatcher {
    observer: ResizeObserver,
    _callback: Closure<dyn FnMut()>,
}

impl BoxWatcher {
    pub fn start(element: Element, size: UseStateHandle<(f64, f64)>) -> Option<Self> {
        let target = element.clone();
        let measure = move || {
            let rect = element.get_bounding_client_rect();
            size.set((rect.width().floor(), rect.height().floor()));
        };
        measure.clone()();
        let callback = Closure::<dyn FnMut()>::new(measure);
        let observer = ResizeObserver::new(callback.as_ref().unchecked_ref()).ok()?;
        observer.observe(&target);
        Some(Self {
            observer,
            _callback: callback,
        })
    }
}

impl Drop for BoxWatcher {
    fn drop(&mut self) {
        self.observer.disconnect();
    }
}
