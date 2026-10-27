use std::collections::HashMap;

use crate::{FitNote, WindowLabel};

/// The newest fit per price window.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FitBoard {
    fits: HashMap<String, FitNote>,
    rejected: usize,
}

impl FitBoard {
    pub fn accept(&mut self, note: &nostr_minions::NostrNote) -> bool {
        match FitNote::from_note(note) {
            Ok(fit) => self.record(fit),
            Err(_) => {
                self.rejected += 1;
                false
            }
        }
    }

    pub fn record(&mut self, fit: FitNote) -> bool {
        let newer = self
            .fits
            .get(fit.window())
            .is_none_or(|current| fit.created_at() > current.created_at());
        if newer {
            self.fits.insert(fit.window().to_string(), fit);
        }
        newer
    }

    #[must_use]
    pub fn latest(&self, window: &str) -> Option<&FitNote> {
        self.fits.get(window)
    }

    #[must_use]
    pub const fn rejected(&self) -> usize {
        self.rejected
    }

    /// Every window the oracle fits, plus any other window a fit arrived for. Fits are live
    /// notes, so a fresh page holds none until the oracle's next repeat.
    #[must_use]
    pub fn windows(&self) -> Vec<&str> {
        FitNote::WINDOWS
            .into_iter()
            .chain(self.ordered().into_iter().map(FitNote::window))
            .collect()
    }

    #[must_use]
    pub fn ordered(&self) -> Vec<&FitNote> {
        let mut fits: Vec<&FitNote> = self
            .fits
            .values()
            .filter(|fit| !WindowLabel::is_retired(fit.window()))
            .collect();
        fits.sort_by(|left, right| {
            WindowLabel::of(left.window())
                .rank()
                .cmp(&WindowLabel::of(right.window()).rank())
                .then_with(|| left.window().cmp(right.window()))
        });
        fits
    }
}

#[cfg(test)]
mod tests {
    use nostr_minions::{NostrNote, NostrNoteBuilder};

    use super::FitBoard;
    use crate::NoteKind;

    struct BoardFixture;

    impl BoardFixture {
        fn note(window: &str, price: f64, created_at: i64) -> NostrNote {
            let content = format!(
                r#"{{"source":"chain","price":{price},"curve":{{"first_price":100.0,"step":0.9,"scores":[0.1,1.0]}},"implied":{{"low":80.0,"high":125.0,"counts":[1]}}}}"#
            );
            let mut note = NostrNoteBuilder::new()
                .kind(NoteKind::LIVE)
                .content(content)
                .tag("w", &format!("fit-{window}"))
                .build();
            note.created_at = created_at;
            note
        }
    }

    #[test]
    fn only_a_newer_fit_replaces_the_current_one() {
        let mut board = FitBoard::default();

        assert!(board.accept(&BoardFixture::note("chain-1h", 100.0, 20)));
        assert!(!board.accept(&BoardFixture::note("chain-1h", 90.0, 10)));

        assert!((board.latest("chain-1h").expect("fit").price() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn fits_are_ordered_like_the_price_cards() {
        let mut board = FitBoard::default();
        for window in ["chain-1d", "mempool", "chain-1h"] {
            board.accept(&BoardFixture::note(window, 1.0, 1));
        }

        let order: Vec<&str> = board.ordered().iter().map(|fit| fit.window()).collect();
        assert_eq!(order, vec!["mempool", "chain-1h", "chain-1d"]);
    }

    #[test]
    fn every_fitted_window_is_offered_before_its_first_fit_arrives() {
        let mut board = FitBoard::default();
        assert_eq!(board.windows(), vec!["mempool", "chain-1h", "chain-1d"]);

        board.accept(&BoardFixture::note("chain-4h", 1.0, 1));
        assert!(board.windows().contains(&"chain-4h"));
    }

    #[test]
    fn an_unreadable_fit_is_counted() {
        let mut board = FitBoard::default();
        let mut bad = BoardFixture::note("mempool", 1.0, 1);
        bad.content = "nope".to_string();

        assert!(!board.accept(&bad));
        assert_eq!(board.rejected(), 1);
    }
}
