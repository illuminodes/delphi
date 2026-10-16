use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

type FrameSlot = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

pub struct FrameLoop {
    handle: Rc<Cell<i32>>,
    slot: FrameSlot,
}

impl FrameLoop {
    pub fn start(mut tick: impl FnMut(f64) + 'static) -> Option<Self> {
        let window = web_sys::window()?;
        let handle = Rc::new(Cell::new(0));
        let slot: FrameSlot = Rc::new(RefCell::new(None));
        let callback = Closure::<dyn FnMut(f64)>::new({
            let (handle, slot, window) = (handle.clone(), Rc::downgrade(&slot), window.clone());
            move |now: f64| {
                tick(now / 1000.0);
                if let Some(slot) = slot.upgrade() {
                    if let Some(next) = slot.borrow().as_ref() {
                        handle.set(window.request_animation_frame(next.as_ref().unchecked_ref()).unwrap_or(0));
                    }
                }
            }
        });
        handle.set(window.request_animation_frame(callback.as_ref().unchecked_ref()).ok()?);
        *slot.borrow_mut() = Some(callback);
        Some(Self { handle, slot })
    }
}

impl Drop for FrameLoop {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            let _ = window.cancel_animation_frame(self.handle.get());
        }
        self.slot.borrow_mut().take();
    }
}
