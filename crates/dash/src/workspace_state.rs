use crate::{TileGesture, TileLayout, TileSize, WorkspacePlan};

/// What the workspace is asked to do.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceAction {
    Viewport(TileSize),
    Begin(TileGesture),
    Follow { x: f64, y: f64 },
    Finish,
}

/// The tile layout and the gesture in progress.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceState {
    layout: TileLayout,
    gesture: Option<TileGesture>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            layout: TileLayout::new(WorkspacePlan::cards(), TileSize::ZERO),
            gesture: None,
        }
    }
}

impl WorkspaceState {
    #[must_use]
    pub const fn layout(&self) -> &TileLayout {
        &self.layout
    }

    #[must_use]
    pub const fn gesture(&self) -> Option<&TileGesture> {
        self.gesture.as_ref()
    }

    #[must_use]
    pub fn applied(mut self, action: WorkspaceAction) -> Self {
        match action {
            WorkspaceAction::Viewport(size) => self.layout.set_viewport(size),
            WorkspaceAction::Begin(gesture) => self.gesture = Some(gesture),
            WorkspaceAction::Follow { x, y } => {
                if let Some(gesture) = self.gesture.as_mut() {
                    gesture.follow(&mut self.layout, x, y);
                }
            }
            WorkspaceAction::Finish => {
                if let Some(gesture) = self.gesture.take() {
                    gesture.finish(&mut self.layout);
                }
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{WorkspaceAction, WorkspaceState};
    use crate::{PanelId, TileGesture, TileSize};

    struct StateFixture;

    impl StateFixture {
        fn sized() -> WorkspaceState {
            WorkspaceState::default().applied(WorkspaceAction::Viewport(TileSize::new(4000.0, 2000.0)))
        }
    }

    #[test]
    fn every_card_is_shown_from_the_start() {
        assert_eq!(WorkspaceState::default().layout().panels(), PanelId::ALL.to_vec());
    }

    #[test]
    fn a_resize_gesture_moves_the_card_edge() {
        let mut state = StateFixture::sized();
        let edge = state.layout().dividers().remove(0);
        state = state
            .applied(WorkspaceAction::Begin(TileGesture::Resize(edge)))
            .applied(WorkspaceAction::Follow { x: 0.2, y: 0.1 })
            .applied(WorkspaceAction::Finish);

        let (first, rect) = state.layout().tiles().remove(0);
        assert_eq!(first, PanelId::History);
        assert!((rect.width - 0.2).abs() < 1e-9);
        assert!(state.gesture().is_none());
    }

    #[test]
    fn moving_without_a_gesture_changes_nothing() {
        let state = StateFixture::sized();
        let after = state.clone().applied(WorkspaceAction::Follow { x: 0.2, y: 0.2 });

        assert_eq!(after, state);
    }
}
