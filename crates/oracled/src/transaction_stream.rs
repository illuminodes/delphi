use std::time::{Duration, Instant};

/// Decides which per-transaction notes go out: at most `per_second` in each second, the rest
/// dropped and counted, so a burst of mempool traffic cannot get the oracle's key rate-limited
/// by the relay. A limit of zero turns the stream off.
#[derive(Debug, Clone)]
pub struct TransactionStream {
    per_second: u32,
    second_start: Option<Instant>,
    sent: u32,
    dropped: u64,
}

impl TransactionStream {
    pub const DEFAULT_PER_SECOND: u32 = 20;
    const SECOND: Duration = Duration::from_secs(1);

    #[must_use]
    pub const fn new(per_second: u32) -> Self {
        Self {
            per_second,
            second_start: None,
            sent: 0,
            dropped: 0,
        }
    }

    /// Whether a note may go out at `now`; a refusal is counted as dropped.
    pub fn admit(&mut self, now: Instant) -> bool {
        if self.per_second == 0 {
            return false;
        }
        if self
            .second_start
            .is_none_or(|start| now.saturating_duration_since(start) >= Self::SECOND)
        {
            self.second_start = Some(now);
            self.sent = 0;
        }
        if self.sent < self.per_second {
            self.sent += 1;
            return true;
        }
        self.dropped += 1;
        false
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.per_second > 0
    }

    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl Default for TransactionStream {
    fn default() -> Self {
        Self::new(Self::DEFAULT_PER_SECOND)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::TransactionStream;

    #[test]
    fn a_second_holds_at_most_the_limit_and_the_next_one_starts_over() {
        let mut stream = TransactionStream::new(3);
        let start = Instant::now();
        let first: Vec<bool> = (0..5).map(|_| stream.admit(start)).collect();

        assert_eq!(first, vec![true, true, true, false, false]);
        assert_eq!(stream.dropped(), 2);
        assert!(stream.admit(start + Duration::from_secs(1)));
    }

    #[test]
    fn a_zero_limit_turns_the_stream_off_without_counting_drops() {
        let mut stream = TransactionStream::new(0);

        assert!(!stream.is_enabled());
        assert!(!stream.admit(Instant::now()));
        assert_eq!(stream.dropped(), 0);
    }
}
