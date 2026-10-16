use web_sys::Element;
use yew::prelude::*;

use super::box_watcher::BoxWatcher;

#[hook]
pub fn use_box_size(node: &NodeRef) -> (f64, f64) {
    let size = use_state_eq(|| (0.0_f64, 0.0_f64));
    use_effect_with(node.clone(), {
        let size = size.clone();
        move |node: &NodeRef| {
            let watcher = node
                .cast::<Element>()
                .and_then(|element| BoxWatcher::start(element, size));
            move || drop(watcher)
        }
    });
    *size
}
