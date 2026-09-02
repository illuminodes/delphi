use std::fmt;

use mantle::difficulty::BlockTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    fn validates_leap_days() {
        assert!(UtcDate::parse("2024-02-29").is_ok());
        assert_eq!(UtcDate::parse("2023-02-29"), Err(UtcDateError::InvalidDate));
    }
}
