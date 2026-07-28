use std::time::Duration;

/// Parses simple durations shared across component configs: "500ms",
/// "2s", "1m", "1h", or a bare integer (interpreted as seconds).
/// Hand-rolled rather than pulling in a duration-parsing crate for this
/// one concern.
pub fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let split_at = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num_part, unit) = s.split_at(split_at);
    let num: u64 = num_part
        .parse()
        .map_err(|_| format!("'{s}' does not start with a number"))?;
    let multiplier_ms: u64 = match unit {
        "" | "s" => 1000,
        "ms" => 1,
        "m" => 60_000,
        "h" => 3_600_000,
        other => return Err(format!("unknown duration unit '{other}'")),
    };
    Ok(Duration::from_millis(num * multiplier_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_duration_suffixes() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("2s").unwrap(), Duration::from_secs(2));
        assert_eq!(parse_duration("1m").unwrap(), Duration::from_secs(60));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("5").unwrap(), Duration::from_secs(5));
    }

    #[test]
    fn rejects_unknown_unit() {
        assert!(parse_duration("5x").is_err());
    }
}
