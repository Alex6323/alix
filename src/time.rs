use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is set before the Unix epoch")
        .as_millis() as u64
}

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

pub fn local_date(now_ms: u64) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp_millis(now_ms as i64)
        .unwrap_or_default()
        .with_timezone(&chrono::Local)
        .date_naive()
}

/// Falls back to the naive UTC end on an unmappable local time (DST edge);
/// errs by at most hours, never days.
pub fn end_of_local_day_ms(date: chrono::NaiveDate) -> u64 {
    use chrono::TimeZone;
    let end = date.and_hms_milli_opt(23, 59, 59, 999).unwrap_or_default();
    match chrono::Local.from_local_datetime(&end) {
        chrono::LocalResult::Single(dt) | chrono::LocalResult::Ambiguous(dt, _) => {
            dt.timestamp_millis().max(0) as u64
        }
        chrono::LocalResult::None => end.and_utc().timestamp_millis().max(0) as u64,
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

    #[test]
    fn local_date_and_end_of_day_are_consistent() {
        let now = crate::time::now_ms();
        let today = local_date(now);
        let end = end_of_local_day_ms(today);
        assert!(end >= now);
        assert!(end - now < 86_400_000);
        assert_eq!(
            today,
            local_date(end),
            "the ceiling is still the same local day"
        );
    }
}
