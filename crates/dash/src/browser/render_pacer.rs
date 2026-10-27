use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use yew::functional::UseForceUpdateHandle;

/// Redraws the dashboard at most once per [`Self::PERIOD_MILLIS`], however fast notes arrive.
#[derive(Clone)]
pub struct RenderPacer {
    refresh: UseForceUpdateHandle,
    pending: Rc<RefCell<Option<Timeout>>>,
}

impl RenderPacer {
    pub const PERIOD_MILLIS: u32 = 250;

    pub fn new(refresh: UseForceUpdateHandle) -> Self {
        Self {
            refresh,
            pending: Rc::new(RefCell::new(None)),
        }
    }

    /// Redraws within one period, unless a redraw is already waiting.
    pub fn soon(&self) {
        if self.pending.borrow().is_some() {
            return;
        }
        let pacer = self.clone();
        let timeout = Timeout::new(Self::PERIOD_MILLIS, move || {
            pacer.pending.borrow_mut().take();
            pacer.refresh.force_update();
        });
        *self.pending.borrow_mut() = Some(timeout);
    }

    /// Redraws now, dropping any redraw that was waiting.
    pub fn now(&self) {
        if let Some(timeout) = self.pending.borrow_mut().take() {
            timeout.cancel();
        }
        self.refresh.force_update();
    }
}
