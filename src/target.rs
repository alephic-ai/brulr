//! Parsing the burn target argument and formatting durations.

use std::time::Duration;

/// What a burn run aims for.
#[derive(Debug, PartialEq)]
pub enum Goal {
    Tokens(u64),
    Duration(Duration),
    Dollars(f64),
}

/// Parse "HH:MM" into seconds-of-day.
pub fn parse_hhmm(s: &str) -> Result<u32, String> {
    let bad = || format!("invalid time: {s} (use HH:MM, 24-hour)");
    let (h, m) = s.split_once(':').ok_or_else(bad)?;
    let h: u32 = h.parse().map_err(|_| bad())?;
    let m: u32 = m.parse().map_err(|_| bad())?;
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(h * 3600 + m * 60)
}

/// Seconds from `now` to the next occurrence of `target` on a 24h clock.
/// Exact match maps to a full day rather than zero (don't burn nothing).
// ponytail: assumes 86400s/day, so a DST change mid-window shifts the stop by
// an hour, which is fine for a burn tool.
pub fn secs_until(now: u32, target: u32) -> u64 {
    const DAY: u32 = 86_400;
    match (target + DAY - now) % DAY {
        0 => DAY as u64,
        n => n as u64,
    }
}

/// Parse a burn target: plain integer = tokens, `N`+s/m/h = duration,
/// `N`+`usd` = dollars.
pub fn parse_target(s: &str) -> Result<Goal, String> {
    if let Some(num) = s.strip_suffix("usd") {
        let d: f64 = num
            .parse()
            .map_err(|_| format!("invalid dollar amount: {s} (use e.g. 5usd, 0.25usd)"))?;
        return Ok(Goal::Dollars(d));
    }
    let (num, unit) = s.split_at(s.len() - s.chars().last().map_or(0, |c| c.len_utf8()));
    let secs_per_unit = match unit {
        "s" => Some(1),
        "m" => Some(60),
        "h" => Some(3600),
        _ => None,
    };
    match secs_per_unit {
        Some(mult) => {
            let n: u64 = num
                .parse()
                .map_err(|_| format!("invalid duration: {s} (use e.g. 90s, 45m, 2h)"))?;
            Ok(Goal::Duration(Duration::from_secs(n * mult)))
        }
        None => {
            let n: u64 = s.parse().map_err(|_| {
                format!("invalid target: {s} (tokens like 100000, 90s/45m/2h, or 5usd)")
            })?;
            Ok(Goal::Tokens(n))
        }
    }
}

/// Compact duration for display: 90s, 45m, 2h, 1h30m.
pub fn fmt_dur(d: Duration) -> String {
    let s = d.as_secs();
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    let mut out = String::new();
    if h > 0 {
        out += &format!("{h}h");
    }
    if m > 0 {
        out += &format!("{m}m");
    }
    if sec > 0 || out.is_empty() {
        out += &format!("{sec}s");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_handles_tokens_durations_and_dollars() {
        assert_eq!(parse_target("100000").unwrap(), Goal::Tokens(100_000));
        assert_eq!(parse_target("90s").unwrap(), Goal::Duration(Duration::from_secs(90)));
        assert_eq!(parse_target("45m").unwrap(), Goal::Duration(Duration::from_secs(45 * 60)));
        assert_eq!(parse_target("2h").unwrap(), Goal::Duration(Duration::from_secs(2 * 3600)));
        assert_eq!(parse_target("5usd").unwrap(), Goal::Dollars(5.0));
        assert_eq!(parse_target("0.25usd").unwrap(), Goal::Dollars(0.25));
        assert!(parse_target("45x").is_err());
        assert!(parse_target("m").is_err());
        assert!(parse_target("usd").is_err());
        assert!(parse_target("").is_err());
    }

    #[test]
    fn fmt_dur_is_compact() {
        assert_eq!(fmt_dur(Duration::from_secs(90)), "1m30s");
        assert_eq!(fmt_dur(Duration::from_secs(45 * 60)), "45m");
        assert_eq!(fmt_dur(Duration::from_secs(2 * 3600)), "2h");
        assert_eq!(fmt_dur(Duration::from_secs(0)), "0s");
    }

    #[test]
    fn parse_hhmm_valid_and_invalid() {
        assert_eq!(parse_hhmm("00:00").unwrap(), 0);
        assert_eq!(parse_hhmm("07:00").unwrap(), 7 * 3600);
        assert_eq!(parse_hhmm("23:59").unwrap(), 23 * 3600 + 59 * 60);
        assert!(parse_hhmm("24:00").is_err());
        assert!(parse_hhmm("07:60").is_err());
        assert!(parse_hhmm("0700").is_err());
        assert!(parse_hhmm("").is_err());
    }

    #[test]
    fn secs_until_covers_before_after_and_exact() {
        // target later today
        assert_eq!(secs_until(6 * 3600, 7 * 3600), 3600);
        // target already passed, so next day
        assert_eq!(secs_until(8 * 3600, 7 * 3600), 23 * 3600);
        // exact match gives a full day, never zero
        assert_eq!(secs_until(7 * 3600, 7 * 3600), 86_400);
    }
}
