use serde::{Deserialize, Serialize};

use crate::{PanelFootprint, PanelId, SpiralCard};

/// The reader's own spiral: the cards in order with the share each one asked for, in a form
/// that can be saved and read back on the next visit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceArrangement {
    cards: Vec<(String, f64)>,
}

impl WorkspaceArrangement {
    #[must_use]
    pub fn of(cards: &[SpiralCard]) -> Self {
        Self {
            cards: cards.iter().map(|card| (card.panel().title().to_string(), card.ratio())).collect(),
        }
    }

    /// The saved cards, or `None` unless they name every card exactly once.
    #[must_use]
    pub fn cards(&self) -> Option<Vec<SpiralCard>> {
        let cards: Vec<SpiralCard> = self
            .cards
            .iter()
            .map(|(title, ratio)| {
                let panel = PanelId::titled(title)?;
                Some(SpiralCard::new(panel, *ratio, PanelFootprint::minimum(panel)))
            })
            .collect::<Option<_>>()?;
        let complete = cards.len() == PanelId::ALL.len()
            && PanelId::ALL.iter().all(|panel| cards.iter().any(|card| card.panel() == panel));
        complete.then_some(cards)
    }

    #[must_use]
    pub fn from_json(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceArrangement;
    use crate::{PanelId, WorkspacePlan};

    #[test]
    fn an_arrangement_survives_a_json_round_trip() {
        let mut cards = WorkspacePlan::cards();
        cards.swap(0, 5);
        cards[1].set_ratio(0.3);
        let saved = WorkspaceArrangement::of(&cards).to_json();

        let restored = WorkspaceArrangement::from_json(&saved).and_then(|arrangement| arrangement.cards());

        assert_eq!(restored, Some(cards));
    }

    #[test]
    fn an_arrangement_missing_or_repeating_a_card_is_refused() {
        let mut cards = WorkspacePlan::cards();
        cards.pop();
        assert_eq!(WorkspaceArrangement::of(&cards).cards(), None);

        cards.push(cards[0].clone());
        assert_eq!(WorkspaceArrangement::of(&cards).cards(), None);
    }

    #[test]
    fn unknown_cards_and_bad_json_are_refused() {
        let text = WorkspaceArrangement::of(&WorkspacePlan::cards())
            .to_json()
            .replace(PanelId::Fit.title(), "Gone");

        assert_eq!(WorkspaceArrangement::from_json(&text).and_then(|arrangement| arrangement.cards()), None);
        assert_eq!(WorkspaceArrangement::from_json("{not json"), None);
    }
}
