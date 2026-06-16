//! Small time helpers. All timestamps in this crate are Unix time in
//! milliseconds (`u64`), matching the original progress database format.

use std::time::{SystemTime, UNIX_EPOCH};

/// Returns the current Unix time in milliseconds.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is set before the Unix epoch")
        .as_millis() as u64
}

/// Formats a duration given in milliseconds as a short human-readable string,
/// e.g. "42s", "5m", "3h", "2d", "1w".
pub fn humanize_ms(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        86400..604800 => format!("{}d", secs / 86400),
        _ => format!("{}w", secs / 604800),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize() {
        assert_eq!("0s", humanize_ms(0));
        assert_eq!("59s", humanize_ms(59_000));
        assert_eq!("1m", humanize_ms(60_000));
        assert_eq!("59m", humanize_ms(3_599_000));
        assert_eq!("1h", humanize_ms(3_600_000));
        assert_eq!("23h", humanize_ms(86_399_000));
        assert_eq!("1d", humanize_ms(86_400_000));
        assert_eq!("6d", humanize_ms(604_799_000));
        assert_eq!("1w", humanize_ms(604_800_000));
        assert_eq!("4w", humanize_ms(4 * 604_800_000));
    }
}
