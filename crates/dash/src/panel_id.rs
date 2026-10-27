/// One card of the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PanelId {
    History,
    Fit,
    Blockchain,
    Mempool,
    Source,
    Price,
}

impl PanelId {
    pub const ALL: [Self; 6] = [Self::History, Self::Fit, Self::Blockchain, Self::Mempool, Self::Source, Self::Price];

    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::History => "History",
            Self::Fit => "Fit",
            Self::Blockchain => "Blockchain",
            Self::Mempool => "Mempool",
            Self::Source => "Source",
            Self::Price => "Price",
        }
    }
}

#[cfg(test)]
impl PanelId {
    /// A card named by letter in layout tests: `a` is the first in `ALL`, `b` the second, and so on.
    #[must_use]
    pub fn lettered(letter: &str) -> Self {
        let index = letter.bytes().next().map_or(0, |byte| usize::from(byte.saturating_sub(b'a')));
        Self::ALL[index % Self::ALL.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::PanelId;

    #[test]
    fn every_card_has_its_own_title() {
        let mut titles: Vec<&str> = PanelId::ALL.iter().map(PanelId::title).collect();
        titles.sort_unstable();
        titles.dedup();

        assert_eq!(titles.len(), PanelId::ALL.len());
    }
}
