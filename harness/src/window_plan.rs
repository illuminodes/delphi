use mantle::difficulty::BlockTime;
use mantle_oracle::{RangeSelection, UtcDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPlan {
    pub date: UtcDate,
}

impl WindowPlan {
    #[must_use]
    pub fn ending_on(last: UtcDate, count: usize) -> Vec<Self> {
        if count == 0 {
            return Vec::new();
        }
        (0..count)
            .map(|offset| Self::from_days_before(last, count - 1 - offset))
            .collect()
    }

    fn from_days_before(last: UtcDate, offset: usize) -> Self {
        let last_seconds = u64::from(last.midnight().expect("window date in range").to_u32());
        let seconds = last_seconds - u64::try_from(offset).expect("offset fits u64") * 86_400;
        Self {
            date: UtcDate::containing(BlockTime::new(u32::try_from(seconds).expect("window date in range"))),
        }
    }

    #[must_use]
    pub fn selection(self) -> RangeSelection {
        RangeSelection::UtcDay(self.date.midnight().expect("window date in range"))
    }

    #[must_use]
    pub fn label(self) -> String {
        self.date.to_string()
    }
}

#[cfg(test)]
mod tests {
    use mantle_oracle::{RangeSelection, UtcDate};

    use super::WindowPlan;

    #[test]
    fn plans_consecutive_days_backwards_from_last() {
        let last = UtcDate::parse("2026-08-29").expect("valid date");
        let plan = WindowPlan::ending_on(last, 3);
        let labels: Vec<String> = plan.iter().map(|window| window.label()).collect();
        assert_eq!(labels, ["2026-08-27", "2026-08-28", "2026-08-29"]);
    }

    #[test]
    fn empty_plan_for_zero_windows() {
        let last = UtcDate::parse("2026-08-29").expect("valid date");
        assert!(WindowPlan::ending_on(last, 0).is_empty());
    }

    #[test]
    fn selection_is_the_utc_day() {
        let window = WindowPlan::ending_on(UtcDate::parse("2026-08-29").expect("valid date"), 1)
            .pop()
            .expect("one window");
        let midnight = window.date.midnight().expect("window date in range");
        assert_eq!(window.selection(), RangeSelection::UtcDay(midnight));
    }
}
