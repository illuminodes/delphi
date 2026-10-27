use web_sys::Element;
use yew::prelude::*;

use super::box_watcher::BoxWatcher;

/// The rendered size of the element behind `node`. The watch follows the element: a component
/// that first renders something else (a placeholder) and attaches the node later is still
/// measured.
#[hook]
pub fn use_box_size(node: &NodeRef) -> (f64, f64) {
    let size = use_state_eq(|| (0.0_f64, 0.0_f64));
    let watch = use_mut_ref(|| None::<(Element, BoxWatcher)>);
    {
        let node = node.clone();
        let size = size.clone();
        let watch = watch.clone();
        use_effect(move || {
            let element = node.cast::<Element>();
            let current = watch.borrow().as_ref().map(|(watched, _)| watched.clone());
            if element != current {
                *watch.borrow_mut() = element.and_then(|element| {
                    BoxWatcher::start(element.clone(), size).map(|watcher| (element, watcher))
                });
            }
        });
    }
    use_effect_with((), move |()| move || drop(watch.borrow_mut().take()));
    *size
}
