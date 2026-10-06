/// How a relay treats a note, which decides its kind and window tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteClass {
    /// Stored and never replaced: the oracle's final price for a window.
    Audit,
    /// Forwarded to subscribers and never stored.
    Live,
    /// Stored, one per window, each replacing the last.
    Addressable,
}

impl NoteClass {
    #[must_use]
    pub const fn kind(self) -> u32 {
        match self {
            Self::Audit => 8_078,
            Self::Live => 20_078,
            Self::Addressable => 30_078,
        }
    }

    /// The class a relay gives `kind`, by the NIP-01 ranges.
    #[must_use]
    pub const fn of_kind(kind: u32) -> Self {
        match kind {
            20_000..30_000 => Self::Live,
            30_000..40_000 => Self::Addressable,
            _ => Self::Audit,
        }
    }

    /// The tag carrying the window name. Addressable notes must use `d`.
    #[must_use]
    pub const fn window_tag(self) -> &'static str {
        match self {
            Self::Addressable => "d",
            Self::Audit | Self::Live => "w",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NoteClass;

    #[test]
    fn kinds_fall_in_the_ranges_relays_treat_differently() {
        assert!((1_000..10_000).contains(&NoteClass::Audit.kind()));
        assert!((20_000..30_000).contains(&NoteClass::Live.kind()));
        assert!((30_000..40_000).contains(&NoteClass::Addressable.kind()));
    }

    #[test]
    fn a_kind_maps_back_to_its_class() {
        for class in [NoteClass::Audit, NoteClass::Live, NoteClass::Addressable] {
            assert_eq!(NoteClass::of_kind(class.kind()), class);
        }
    }

    #[test]
    fn only_addressable_notes_use_the_d_tag() {
        assert_eq!(NoteClass::Addressable.window_tag(), "d");
        assert_eq!(NoteClass::Audit.window_tag(), "w");
        assert_eq!(NoteClass::Live.window_tag(), "w");
    }
}
