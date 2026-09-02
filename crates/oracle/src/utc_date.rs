use std::fmt;

use mantle::difficulty::BlockTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct UtcDate {
    year: u32,
    month: u32,
    day: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UtcDateError {
    InvalidFormat,
    InvalidDate,
    TimestampOverflow,
}

impl fmt::Display for UtcDateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFormat => formatter.write_str("date must use YYYY-MM-DD"),
            Self::InvalidDate => formatter.write_str("date is not a valid Gregorian date"),
            Self::TimestampOverflow => formatter.write_str("date is outside the block-time range"),
        }
    }
}

impl fmt::Display for UtcDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

impl std::error::Error for UtcDateError {}

impl UtcDate {
    #[allow(clippy::missing_errors_doc)]
    pub fn parse(value: &str) -> Result<Self, UtcDateError> {
        let mut parts = value.split('-');
        let year = parts
            .next()
            .ok_or(UtcDateError::InvalidFormat)?
            .parse()
            .map_err(|_| UtcDateError::InvalidFormat)?;
        let month = parts
            .next()
            .ok_or(UtcDateError::InvalidFormat)?
            .parse()
            .map_err(|_| UtcDateError::InvalidFormat)?;
        let day = parts
            .next()
            .ok_or(UtcDateError::InvalidFormat)?
            .parse()
            .map_err(|_| UtcDateError::InvalidFormat)?;
        if parts.next().is_some() || year < 1970 || !(1..=12).contains(&month) {
            return Err(UtcDateError::InvalidDate);
        }
        let max_day = Self::days_in_month(year, month);
        if !(1..=max_day).contains(&day) {
            return Err(UtcDateError::InvalidDate);
        }
        Ok(Self { year, month, day })
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn midnight(self) -> Result<BlockTime, UtcDateError> {
        let days_before_year = (1970..self.year)
            .map(|year| {
                if Self::is_leap_year(year) {
                    366_u64
                } else {
                    365
                }
            })
            .sum::<u64>();
        let days_before_month = (1..self.month)
            .map(|month| u64::from(Self::days_in_month(self.year, month)))
            .sum::<u64>();
        let days = days_before_year + days_before_month + u64::from(self.day - 1);
        let seconds = days
            .checked_mul(86_400)
            .ok_or(UtcDateError::TimestampOverflow)?;
        let timestamp = u32::try_from(seconds).map_err(|_| UtcDateError::TimestampOverflow)?;
        Ok(BlockTime::new(timestamp))
    }

    /// Returns the calendar day after this one.
    ///
    /// # Panics
    /// Panics when this date is at the edge of the representable range.
    #[must_use]
    pub fn next(self) -> Self {
        let days = u64::from(self.midnight().expect("date in range").to_u32()) / 86_400;
        Self::from_days(days + 1)
    }

    #[must_use]
    pub fn containing(time: BlockTime) -> Self {
        Self::from_days(u64::from(time.to_u32()) / 86_400)
    }

    fn from_days(days: u64) -> Self {
        let z = days + 719_468;
        let era = z / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + u64::from(month <= 2);
        Self {
            year: u32::try_from(year).expect("year fits u32"),
            month: u32::try_from(month).expect("month fits u32"),
            day: u32::try_from(day).expect("day fits u32"),
        }
    }

    const fn is_leap_year(year: u32) -> bool {
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
    }

    const fn days_in_month(year: u32, month: u32) -> u32 {
        match month {
            2 if Self::is_leap_year(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }
}

#[cfg(test)]
mod tests {
    use mantle::difficulty::BlockTime;

    use super::{UtcDate, UtcDateError};

    #[test]
    fn parses_unix_epoch() {
        assert_eq!(
            UtcDate::parse("1970-01-01").expect("valid date").midnight(),
            Ok(BlockTime::new(0))
        );
    }

    #[test]
    fn converts_known_timestamp() {
        assert_eq!(
            UtcDate::parse("2024-01-01").expect("valid date").midnight(),
            Ok(BlockTime::new(1_704_067_200))
        );
    }

    #[test]
    fn resolves_the_date_containing_a_timestamp() {
        assert_eq!(
            UtcDate::containing(BlockTime::new(86_399)),
            UtcDate::parse("1970-01-01").expect("valid date")
        );
        assert_eq!(
            UtcDate::containing(BlockTime::new(86_400)),
            UtcDate::parse("1970-01-02").expect("valid date")
        );
    }

    #[test]
    fn containing_round_trips_with_midnight() {
        let date = UtcDate::parse("2024-02-29").expect("valid date");
        let midnight = date.midnight().expect("in range");
        assert_eq!(UtcDate::containing(midnight), date);
        assert_eq!(
            UtcDate::containing(BlockTime::new(1_767_225_600)).to_string(),
            "2026-01-01"
        );
    }

    #[test]
    fn displays_iso_format() {
        let date = UtcDate::parse("2026-08-30").expect("valid date");
        assert_eq!(date.to_string(), "2026-08-30");
    }

    #[test]
    fn steps_to_the_next_day_across_month_and_year_ends() {
        assert_eq!(
            UtcDate::parse("2026-08-31").expect("valid date").next(),
            UtcDate::parse("2026-09-01").expect("valid date")
        );
        assert_eq!(
            UtcDate::parse("2026-12-31").expect("valid date").next(),
            UtcDate::parse("2027-01-01").expect("valid date")
        );
        assert_eq!(
            UtcDate::parse("2024-02-28").expect("valid date").next(),
            UtcDate::parse("2024-02-29").expect("valid date")
        );
    }

    #[test]
    fn validates_leap_days() {
        assert!(UtcDate::parse("2024-02-29").is_ok());
        assert_eq!(UtcDate::parse("2023-02-29"), Err(UtcDateError::InvalidDate));
    }
}
