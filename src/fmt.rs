//! ANSI colours and the value formatters shared by the status line and the `status` command.
//! Colour constants are lifted from quotaline so the two status lines match visually.

pub const RESET: &str = "\x1b[0m";
pub const DIM: &str = "\x1b[2m";
pub const GRAY: &str = "\x1b[90m";
pub const GREEN: &str = "\x1b[32m";
pub const AMBER: &str = "\x1b[38;5;214m"; // 256-colour amber (warning band)
pub const RED: &str = "\x1b[31m";

// Runway colour bands (hours of balance left at the *total* burn rate). Tunable.
pub const RUNWAY_AMBER_HRS: f64 = 12.0;
pub const RUNWAY_RED_HRS: f64 = 4.0;

/// Colour for a runway in hours: red when nearly dry, amber when low, else green.
/// `None` (unknown balance or zero burn) → gray.
pub fn runway_color(hours: Option<f64>) -> &'static str {
    match hours {
        None => GRAY,
        Some(h) if h <= RUNWAY_RED_HRS => RED,
        Some(h) if h <= RUNWAY_AMBER_HRS => AMBER,
        Some(_) => GREEN,
    }
}

/// Dollars → compact `$1.84`, `$47.20`, `$1.2k`. Two decimals under $1k, k/M above.
pub fn fmt_money(usd: f64) -> String {
    let neg = usd < 0.0;
    let a = usd.abs();
    let body = if a >= 1e6 {
        format!("{:.1}M", a / 1e6)
    } else if a >= 1e4 {
        // $10k+ — drop the cents, they're noise at this scale.
        format!("{:.1}k", a / 1e3)
    } else {
        format!("{a:.2}")
    };
    if neg {
        format!("-${body}")
    } else {
        format!("${body}")
    }
}

/// Dollars-per-hour → `$1.84/hr` (cents kept; burn is usually small).
pub fn fmt_rate(usd_per_hr: f64) -> String {
    format!("${usd_per_hr:.2}/hr")
}

/// Duration in hours → compact runway string. Minutes under 1h (`45m`), whole hours up to two
/// days (`25h`, `47h` — more useful than "~1d" for a burn readout), days+hours beyond (`2d4h`).
pub fn fmt_hours(hours: f64) -> String {
    if !hours.is_finite() || hours < 0.0 {
        return "—".to_string();
    }
    let total_min = (hours * 60.0).round() as i64;
    if total_min <= 0 {
        return "0m".to_string();
    }
    if total_min < 60 {
        return format!("{total_min}m");
    }
    let total_hr = ((total_min as f64) / 60.0).round() as i64;
    if total_hr < 48 {
        return format!("{total_hr}h");
    }
    let days = total_hr / 24;
    let rem = total_hr % 24;
    if rem > 0 {
        format!("{days}d{rem}h")
    } else {
        format!("{days}d")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money() {
        assert_eq!(fmt_money(1.84), "$1.84");
        assert_eq!(fmt_money(47.2), "$47.20");
        assert_eq!(fmt_money(0.0), "$0.00");
        assert_eq!(fmt_money(-3.5), "-$3.50");
        assert_eq!(fmt_money(12_345.0), "$12.3k");
        assert_eq!(fmt_money(2_400_000.0), "$2.4M");
    }

    #[test]
    fn rate() {
        assert_eq!(fmt_rate(1.84), "$1.84/hr");
        assert_eq!(fmt_rate(0.0), "$0.00/hr");
    }

    #[test]
    fn hours() {
        assert_eq!(fmt_hours(22.0), "22h");
        assert_eq!(fmt_hours(0.5), "30m");
        assert_eq!(fmt_hours(24.97), "25h"); // hours, not "~1d", up to two days
        assert_eq!(fmt_hours(28.0), "28h");
        assert_eq!(fmt_hours(48.0), "2d");
        assert_eq!(fmt_hours(52.0), "2d4h");
        assert_eq!(fmt_hours(-1.0), "—");
        assert_eq!(fmt_hours(f64::INFINITY), "—");
    }

    #[test]
    fn runway_bands() {
        assert_eq!(runway_color(None), GRAY);
        assert_eq!(runway_color(Some(48.0)), GREEN);
        assert_eq!(runway_color(Some(8.0)), AMBER);
        assert_eq!(runway_color(Some(2.0)), RED);
    }
}
