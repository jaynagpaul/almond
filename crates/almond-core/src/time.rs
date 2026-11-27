//! Time helpers over [`jiff`]. Instants are [`Timestamp`]s; anything about
//! the user's calendar day goes through their [`TimeZone`].

use jiff::civil::Time;
use jiff::{SignedDuration, Timestamp, tz::TimeZone};

/// When the study day rolls over, local time. 4 AM, so a late-night session
/// still counts as the same day.
const DAY_STARTS_AT: Time = Time::constant(4, 0, 0, 0);

const SECS_PER_DAY: f64 = 86_400.0;

/// Fractional days from `earlier` to `later`, never negative.
pub(crate) fn days_between(later: Timestamp, earlier: Timestamp) -> f32 {
    (later.duration_since(earlier).as_secs_f64() / SECS_PER_DAY).max(0.0) as f32
}

/// `at` plus `days`, saturating at the end of time rather than panicking on
/// a huge or non-finite interval.
pub(crate) fn plus_days(at: Timestamp, days: f32) -> Timestamp {
    SignedDuration::try_from_secs_f64(f64::from(days) * SECS_PER_DAY)
        .ok()
        .and_then(|d| at.saturating_add(d).ok())
        .unwrap_or(Timestamp::MAX)
}

/// Truncates to whole milliseconds, the precision the log stores.
pub(crate) fn to_millis(at: Timestamp) -> Timestamp {
    Timestamp::from_millisecond(at.as_millisecond()).unwrap_or(at)
}

/// Milliseconds from `earlier` to `later`, clamped to `0..=u32::MAX`.
pub(crate) fn millis_between(later: Timestamp, earlier: Timestamp) -> u32 {
    later
        .duration_since(earlier)
        .as_millis()
        .clamp(0, i128::from(u32::MAX)) as u32
}

/// The end of the study day that contains `now`: the next 4 AM in `tz`.
/// Cards due before then are due today.
pub fn end_of_day(now: Timestamp, tz: &TimeZone) -> Timestamp {
    let local = now.to_zoned_datetime(tz.clone());
    let today = local.date();
    // Only fails at the edges of representable time; a day from now is then
    // the closest sensible answer.
    let fallback = plus_days(now, 1.0);
    let rollover = |date: jiff::civil::Date| {
        date.to_datetime(DAY_STARTS_AT)
            .to_zoned_datetime(tz.clone())
            .map_or(fallback, |z| z.timestamp())
    };
    let start_of_next = rollover(today);
    if start_of_next > now {
        start_of_next
    } else {
        today.tomorrow().map_or(fallback, rollover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn the_day_ends_at_the_next_4am_local() {
        let tz = TimeZone::get("Asia/Kolkata").unwrap();
        // 1:48 AM in Kolkata is still yesterday's study day.
        assert_eq!(
            end_of_day(at("2026-11-27T20:18:00Z"), &tz),
            at("2026-11-27T22:30:00Z")
        );
        // 9 AM rolls over at 4 AM tomorrow.
        assert_eq!(
            end_of_day(at("2026-11-28T03:30:00Z"), &tz),
            at("2026-11-28T22:30:00Z")
        );
    }

    #[test]
    fn the_day_end_follows_daylight_saving() {
        let tz = TimeZone::get("America/New_York").unwrap();
        // Noon EST on the day before clocks spring forward: 4 AM EDT next day.
        assert_eq!(
            end_of_day(at("2026-03-07T17:00:00Z"), &tz),
            at("2026-03-08T08:00:00Z")
        );
    }

    #[test]
    #[allow(clippy::float_cmp)] // exact: these values are representable
    fn day_math() {
        let a = at("2026-01-01T00:00:00Z");
        assert_eq!(days_between(plus_days(a, 1.5), a), 1.5);
        assert_eq!(days_between(a, plus_days(a, 1.0)), 0.0);
        assert_eq!(millis_between(plus_days(a, 1.0), a), 86_400_000);
        assert_eq!(plus_days(a, f32::INFINITY), Timestamp::MAX);
        assert_eq!(plus_days(a, f32::NAN), Timestamp::MAX);
        assert_eq!(plus_days(a, f32::MAX), Timestamp::MAX);
    }
}
