use mantle::difficulty::BlockTime;
use mantle_oracle::RangeSelection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanWindow {
    start: u32,
    duration: u32,
}

impl SpanWindow {
    #[must_use]
    pub const fn new(start: u32, duration: u32) -> Self {
        Self { start, duration }
    }

    #[must_use]
    pub const fn start(self) -> u32 {
        self.start
    }

    #[must_use]
    pub const fn stop(self) -> u32 {
        self.start + self.duration
    }

    #[must_use]
    pub const fn duration(self) -> u32 {
        self.duration
    }

    #[must_use]
    pub const fn midpoint(self) -> u32 {
        self.start + self.duration / 2
    }

    #[must_use]
    pub const fn selection(self) -> RangeSelection {
        RangeSelection::TimeSpan {
            start: BlockTime::new(self.start),
            stop: BlockTime::new(self.stop()),
        }
    }
}

pub struct SpanPlan {
    duration: u32,
}

impl SpanPlan {
    #[must_use]
    pub const fn of_duration(duration: u32) -> Self {
        Self { duration }
    }

    #[must_use]
    pub const fn duration(&self) -> u32 {
        self.duration
    }

    /// Tile `first..=last` with non-overlapping windows of this duration,
    /// aligned to multiples of the duration since the epoch so windows of the
    /// same length always land on the same boundaries.
    #[must_use]
    pub fn tile(&self, first: u32, last: u32) -> Vec<SpanWindow> {
        if self.duration == 0 || last <= first {
            return Vec::new();
        }
        let mut windows = Vec::new();
        let mut start = first / self.duration * self.duration;
        if start < first {
            start += self.duration;
        }
        while start + self.duration <= last {
            windows.push(SpanWindow::new(start, self.duration));
            start += self.duration;
        }
        windows
    }

    #[must_use]
    pub fn label(&self) -> String {
        let seconds = self.duration;
        if seconds.is_multiple_of(86_400) {
            format!("{}d", seconds / 86_400)
        } else if seconds.is_multiple_of(3_600) {
            format!("{}h", seconds / 3_600)
        } else if seconds.is_multiple_of(60) {
            format!("{}m", seconds / 60)
        } else {
            format!("{seconds}s")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SpanPlan, SpanWindow};

    #[test]
    fn tiles_a_span_into_aligned_windows() {
        let plan = SpanPlan::of_duration(3_600);
        let windows = plan.tile(7_000, 18_000);

        assert_eq!(windows.first().copied(), Some(SpanWindow::new(7_200, 3_600)));
        assert_eq!(windows.len(), 3);
        assert_eq!(windows.last().copied().map(SpanWindow::stop), Some(18_000));
    }

    #[test]
    fn windows_do_not_extend_past_the_end() {
        let plan = SpanPlan::of_duration(3_600);
        let windows = plan.tile(0, 9_000);

        assert_eq!(windows.len(), 2);
        assert!(windows.iter().all(|window| window.stop() <= 9_000));
    }

    #[test]
    fn empty_when_the_span_is_shorter_than_one_window() {
        let plan = SpanPlan::of_duration(86_400);
        assert!(plan.tile(0, 3_600).is_empty());
    }

    #[test]
    fn empty_for_a_zero_duration() {
        assert!(SpanPlan::of_duration(0).tile(0, 86_400).is_empty());
    }

    #[test]
    fn midpoint_is_the_window_centre() {
        assert_eq!(SpanWindow::new(1_000, 400).midpoint(), 1_200);
    }

    #[test]
    fn labels_durations_by_largest_unit() {
        assert_eq!(SpanPlan::of_duration(86_400).label(), "1d");
        assert_eq!(SpanPlan::of_duration(21_600).label(), "6h");
        assert_eq!(SpanPlan::of_duration(1_800).label(), "30m");
        assert_eq!(SpanPlan::of_duration(90).label(), "90s");
    }
}
