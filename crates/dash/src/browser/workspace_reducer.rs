use std::rc::Rc;

use yew::Reducible;

use crate::{WorkspaceAction, WorkspaceState};

impl Reducible for WorkspaceState {
    type Action = WorkspaceAction;

    fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
        Rc::new(Rc::unwrap_or_clone(self).applied(action))
    }
}
