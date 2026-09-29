use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Decides when a window's price is worth publishing again.
///
/// A price goes out when it has moved by at least `min_change` since the last
/// one published, or when `heartbeat` has passed without any publication, so
/// a subscriber can tell a quiet market from a dead daemon.
#[derive(Debug, Clone)]
pub struct PublishPolicy {
    default_rule: Rule,
    rules: HashMap<String, Rule>,
    published: HashMap<String, Published>,
}

#[derive(Debug, Clone, Copy)]
struct Rule {
    min_change: f64,
    heartbeat: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Published {
    price: f64,
    at: Instant,
}

/// Why a publication was allowed, or that it was not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    First,
    Moved,
    Heartbeat,
    Hold,
}

impl Verdict {
    #[must_use]
    pub const fn publishes(self) -> bool {
        !matches!(self, Self::Hold)
    }
}

impl PublishPolicy {
    #[must_use]
    pub fn new(min_change: f64, heartbeat: Duration) -> Self {
        Self {
            default_rule: Rule {
                min_change,
                heartbeat,
            },
            rules: HashMap::new(),
            published: HashMap::new(),
        }
    }

    /// Give one window its own threshold and heartbeat.
    #[must_use]
    pub fn with_rule(mut self, window: &str, min_change: f64, heartbeat: Duration) -> Self {
        self.rules.insert(
            window.to_string(),
            Rule {
                min_change,
                heartbeat,
            },
        );
        self
    }

    /// Every update goes out, whatever changed.
    #[must_use]
    pub fn always() -> Self {
        Self::new(0.0, Duration::ZERO)
    }

    #[must_use]
    pub fn judge(&self, window: &str, price: f64, now: Instant) -> Verdict {
        let Some(last) = self.published.get(window) else {
            return Verdict::First;
        };
        let rule = self.rules.get(window).copied().unwrap_or(self.default_rule);
        let change = (price - last.price).abs() / last.price;
        if change >= rule.min_change && price.to_bits() != last.price.to_bits() {
            return Verdict::Moved;
        }
        if now.saturating_duration_since(last.at) >= rule.heartbeat {
            return Verdict::Heartbeat;
        }
        Verdict::Hold
    }

    pub fn record(&mut self, window: &str, price: f64, now: Instant) {
        self.published
            .insert(window.to_string(), Published { price, at: now });
    }

    /// Forget a window, so its next price counts as the first.
    pub fn reset(&mut self, window: &str) {
        self.published.remove(window);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{PublishPolicy, Verdict};

    struct PolicyFixture;

    impl PolicyFixture {
        fn policy() -> PublishPolicy {
            PublishPolicy::new(0.001, Duration::from_mins(1))
        }
    }

    #[test]
    fn the_first_price_of_a_window_is_published() {
        let policy = PolicyFixture::policy();

        assert_eq!(policy.judge("w", 77_000.0, Instant::now()), Verdict::First);
    }

    #[test]
    fn a_move_past_the_threshold_is_published() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);

        assert_eq!(policy.judge("w", 77_100.0, start), Verdict::Moved);
    }

    #[test]
    fn a_move_below_the_threshold_is_held() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);

        assert_eq!(policy.judge("w", 77_010.0, start), Verdict::Hold);
    }

    #[test]
    fn a_downward_move_counts_like_an_upward_one() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);

        assert_eq!(policy.judge("w", 76_900.0, start), Verdict::Moved);
    }

    #[test]
    fn a_still_price_is_published_once_the_heartbeat_passes() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);

        assert_eq!(
            policy.judge("w", 77_000.0, start + Duration::from_secs(59)),
            Verdict::Hold
        );
        assert_eq!(
            policy.judge("w", 77_000.0, start + Duration::from_mins(1)),
            Verdict::Heartbeat
        );
    }

    #[test]
    fn a_publication_restarts_the_heartbeat() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);
        let later = start + Duration::from_secs(50);
        policy.record("w", 77_500.0, later);

        assert_eq!(
            policy.judge("w", 77_500.0, later + Duration::from_secs(20)),
            Verdict::Hold
        );
    }

    #[test]
    fn windows_are_judged_independently() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("a", 77_000.0, start);

        assert_eq!(policy.judge("b", 1.0, start), Verdict::First);
    }

    #[test]
    fn a_reset_window_publishes_again_at_once() {
        let mut policy = PolicyFixture::policy();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);
        policy.reset("w");

        assert_eq!(policy.judge("w", 77_000.0, start), Verdict::First);
    }

    #[test]
    fn a_window_rule_overrides_the_default() {
        let mut policy = PolicyFixture::policy().with_rule("chain", 0.0, Duration::ZERO);
        let start = Instant::now();
        policy.record("chain", 77_000.0, start);
        policy.record("other", 77_000.0, start);

        assert!(policy.judge("chain", 77_000.0, start).publishes());
        assert_eq!(policy.judge("other", 77_000.0, start), Verdict::Hold);
    }

    #[test]
    fn the_always_policy_publishes_every_update() {
        let mut policy = PublishPolicy::always();
        let start = Instant::now();
        policy.record("w", 77_000.0, start);

        assert!(policy.judge("w", 77_000.0, start).publishes());
    }

    #[test]
    fn a_verdict_reports_whether_it_publishes() {
        assert!(Verdict::First.publishes());
        assert!(Verdict::Moved.publishes());
        assert!(Verdict::Heartbeat.publishes());
        assert!(!Verdict::Hold.publishes());
    }
}
