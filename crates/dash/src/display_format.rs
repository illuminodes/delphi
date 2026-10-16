pub struct DisplayFormat;

impl DisplayFormat {
    #[must_use]
    pub fn usd(value: f64) -> String {
        if !value.is_finite() {
            return "—".to_string();
        }
        let cents = (value.abs() * 100.0).round();
        let whole = (cents / 100.0).trunc();
        let fraction = cents - whole * 100.0;
        let sign = if value < 0.0 { "-" } else { "" };
        format!("{sign}${}.{:02}", Self::grouped(whole), fraction)
    }

    #[must_use]
    pub fn usd_whole(value: f64) -> String {
        if !value.is_finite() {
            return "—".to_string();
        }
        let sign = if value < 0.0 { "-" } else { "" };
        format!("{sign}${}", Self::grouped(value.abs().round()))
    }

    #[must_use]
    pub fn count(value: usize) -> String {
        Self::grouped(value as f64)
    }

    #[must_use]
    pub fn age(seconds: i64) -> String {
        match seconds {
            ..=1 => "just now".to_string(),
            2..=59 => format!("{seconds}s ago"),
            60..=3_599 => format!("{}m ago", seconds / 60),
            3_600..=86_399 => format!("{}h ago", seconds / 3_600),
            _ => format!("{}d ago", seconds / 86_400),
        }
    }

    #[must_use]
    pub fn percent(value: f64) -> String {
        if value.is_finite() {
            format!("{value:+.3}%")
        } else {
            "—".to_string()
        }
    }

    #[must_use]
    pub fn percent_brief(value: f64) -> String {
        if value.is_finite() {
            format!("{value:+.2}%")
        } else {
            "—".to_string()
        }
    }

    #[must_use]
    pub fn clock(unix_seconds: i64) -> String {
        let seconds_of_day = unix_seconds.rem_euclid(86_400);
        format!(
            "{:02}:{:02}:{:02} UTC",
            seconds_of_day / 3_600,
            seconds_of_day % 3_600 / 60,
            seconds_of_day % 60
        )
    }

    fn grouped(whole: f64) -> String {
        let digits = format!("{whole:.0}");
        let mut grouped = String::with_capacity(digits.len() + digits.len() / 3);
        for (index, digit) in digits.chars().enumerate() {
            if index > 0 && (digits.len() - index) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(digit);
        }
        grouped
    }
}

#[cfg(test)]
mod tests {
    use super::DisplayFormat;

    #[test]
    fn dollars_are_grouped_and_carry_two_decimals() {
        assert_eq!(DisplayFormat::usd(83_551.5), "$83,551.50");
        assert_eq!(DisplayFormat::usd(1_234_567.891), "$1,234,567.89");
        assert_eq!(DisplayFormat::usd(999.0), "$999.00");
        assert_eq!(DisplayFormat::usd(0.0), "$0.00");
    }

    #[test]
    fn rounding_up_to_the_next_dollar_carries() {
        assert_eq!(DisplayFormat::usd(99.999), "$100.00");
        assert_eq!(DisplayFormat::usd(999_999.999), "$1,000,000.00");
    }

    #[test]
    fn a_negative_amount_keeps_its_sign_and_a_non_finite_one_is_a_dash() {
        assert_eq!(DisplayFormat::usd(-12.5), "-$12.50");
        assert_eq!(DisplayFormat::usd(f64::NAN), "—");
        assert_eq!(DisplayFormat::usd(f64::INFINITY), "—");
    }

    #[test]
    fn whole_dollars_drop_the_cents() {
        assert_eq!(DisplayFormat::usd_whole(83_551.5), "$83,552");
        assert_eq!(DisplayFormat::usd_whole(-12.4), "-$12");
        assert_eq!(DisplayFormat::usd_whole(f64::NAN), "—");
    }

    #[test]
    fn counts_are_grouped() {
        assert_eq!(DisplayFormat::count(0), "0");
        assert_eq!(DisplayFormat::count(1_008), "1,008");
        assert_eq!(DisplayFormat::count(7_336), "7,336");
        assert_eq!(DisplayFormat::count(1_000_000), "1,000,000");
    }

    #[test]
    fn ages_pick_the_largest_whole_unit() {
        assert_eq!(DisplayFormat::age(0), "just now");
        assert_eq!(DisplayFormat::age(1), "just now");
        assert_eq!(DisplayFormat::age(2), "2s ago");
        assert_eq!(DisplayFormat::age(59), "59s ago");
        assert_eq!(DisplayFormat::age(60), "1m ago");
        assert_eq!(DisplayFormat::age(3_599), "59m ago");
        assert_eq!(DisplayFormat::age(3_600), "1h ago");
        assert_eq!(DisplayFormat::age(86_400), "1d ago");
        assert_eq!(DisplayFormat::age(-5), "just now");
    }

    #[test]
    fn percentages_show_their_sign() {
        assert_eq!(DisplayFormat::percent(0.0512), "+0.051%");
        assert_eq!(DisplayFormat::percent(-0.2), "-0.200%");
        assert_eq!(DisplayFormat::percent(f64::NAN), "—");
    }

    #[test]
    fn brief_percentages_keep_two_decimals() {
        assert_eq!(DisplayFormat::percent_brief(0.0512), "+0.05%");
        assert_eq!(DisplayFormat::percent_brief(-1.239), "-1.24%");
        assert_eq!(DisplayFormat::percent_brief(f64::NAN), "—");
    }

    #[test]
    fn the_clock_wraps_at_midnight() {
        assert_eq!(DisplayFormat::clock(0), "00:00:00 UTC");
        assert_eq!(DisplayFormat::clock(86_399), "23:59:59 UTC");
        assert_eq!(DisplayFormat::clock(86_400 + 3_661), "01:01:01 UTC");
    }
}
