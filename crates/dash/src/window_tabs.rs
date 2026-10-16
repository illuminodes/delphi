use crate::WindowLabel;

/// The windows a card can switch between, in tab order, and which one is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowTabs {
    windows: Vec<String>,
}

impl WindowTabs {
    /// Tabs ordered live first, then by window length.
    #[must_use]
    pub fn shortest_first<S: AsRef<str>>(windows: &[S]) -> Self {
        let mut windows: Vec<String> = windows.iter().map(|window| window.as_ref().to_string()).collect();
        windows.sort_by_key(|window| (WindowLabel::of(window).rank(), window.clone()));
        windows.dedup();
        Self { windows }
    }

    /// Tabs ordered longest window first.
    #[must_use]
    pub fn longest_first<S: AsRef<str>>(windows: &[S]) -> Self {
        let mut tabs = Self::shortest_first(windows);
        tabs.windows.reverse();
        tabs
    }

    /// The same tabs with `window` offered first, added if it has not arrived yet.
    #[must_use]
    pub fn led_by(mut self, window: &str) -> Self {
        self.windows.retain(|held| held != window);
        self.windows.insert(0, window.to_string());
        self
    }

    #[must_use]
    pub fn windows(&self) -> &[String] {
        &self.windows
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// The chosen window while it is still offered, otherwise the first tab.
    #[must_use]
    pub fn showing<'a>(&'a self, chosen: Option<&str>) -> Option<&'a str> {
        chosen
            .and_then(|chosen| self.windows.iter().find(|window| *window == chosen))
            .or_else(|| self.windows.first())
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::WindowTabs;

    #[test]
    fn tabs_are_ordered_by_window_length_either_way() {
        let windows = ["chain-1d", "mempool", "chain-1h"];

        assert_eq!(WindowTabs::shortest_first(&windows).windows(), ["mempool", "chain-1h", "chain-1d"]);
        assert_eq!(WindowTabs::longest_first(&windows).windows(), ["chain-1d", "chain-1h", "mempool"]);
    }

    #[test]
    fn the_chosen_tab_shows_while_offered_and_the_first_otherwise() {
        let tabs = WindowTabs::longest_first(&["chain-1h", "chain-1d"]);

        assert_eq!(tabs.showing(Some("chain-1h")), Some("chain-1h"));
        assert_eq!(tabs.showing(Some("mempool")), Some("chain-1d"));
        assert_eq!(tabs.showing(None), Some("chain-1d"));
        assert_eq!(WindowTabs::shortest_first::<&str>(&[]).showing(None), None);
    }

    #[test]
    fn a_leading_window_shows_first_even_before_it_arrives() {
        let present = WindowTabs::shortest_first(&["chain-1h", "chain-1d"]).led_by("chain-1d");
        let absent = WindowTabs::shortest_first(&["chain-1h"]).led_by("chain-1d");

        assert_eq!(present.windows(), ["chain-1d", "chain-1h"]);
        assert_eq!(absent.windows(), ["chain-1d", "chain-1h"]);
        assert_eq!(absent.showing(None), Some("chain-1d"));
    }
}
