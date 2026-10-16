#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilTime {
    month: usize,
    day: u32,
    hour: u32,
    minute: u32,
}

impl CivilTime {
    const MONTHS: [&'static str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    #[must_use]
    pub fn from_unix(seconds: i64) -> Self {
        let days = seconds.div_euclid(86_400);
        let second_of_day = seconds.rem_euclid(86_400);
        let shifted = days + 719_468;
        let era_day = shifted.rem_euclid(146_097);
        let year_of_era =
            (era_day - era_day / 1_460 + era_day / 36_524 - era_day / 146_096) / 365;
        let day_of_year = era_day - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_index = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * month_index + 2) / 5 + 1;
        let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
        Self {
            month: usize::try_from(month - 1).unwrap_or(0),
            day: u32::try_from(day).unwrap_or(1),
            hour: u32::try_from(second_of_day / 3_600).unwrap_or(0),
            minute: u32::try_from(second_of_day % 3_600 / 60).unwrap_or(0),
        }
    }

    #[must_use]
    pub fn time_label(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    #[must_use]
    pub fn date_label(&self) -> String {
        format!("{} {}", Self::MONTHS[self.month % 12], self.day)
    }

    #[must_use]
    pub fn full_label(&self) -> String {
        format!("{} {}", self.date_label(), self.time_label())
    }
}

#[cfg(test)]
mod tests {
    use super::CivilTime;

    #[test]
    fn the_epoch_is_the_first_of_january() {
        let time = CivilTime::from_unix(0);

        assert_eq!(time.full_label(), "Jan 1 00:00");
    }

    #[test]
    fn a_recent_timestamp_maps_to_its_calendar_day() {
        let time = CivilTime::from_unix(1_790_719_021);

        assert_eq!(time.date_label(), "Sep 29");
        assert_eq!(time.time_label(), "21:57");
    }

    #[test]
    fn a_leap_day_is_kept() {
        assert_eq!(CivilTime::from_unix(951_782_400).date_label(), "Feb 29");
        assert_eq!(CivilTime::from_unix(951_782_400 + 86_400).date_label(), "Mar 1");
    }

    #[test]
    fn the_last_second_of_the_year_rolls_correctly() {
        assert_eq!(CivilTime::from_unix(946_684_799).full_label(), "Dec 31 23:59");
        assert_eq!(CivilTime::from_unix(946_684_800).full_label(), "Jan 1 00:00");
    }
}
