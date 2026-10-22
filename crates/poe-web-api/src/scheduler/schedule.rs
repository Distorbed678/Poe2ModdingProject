//! When a task runs: the schedule types, their syntax and the next-run calculation.
//!
//! Four forms, all in the server's local time zone (`TZ`, else the system zone) except the
//! interval:
//!
//! | `type` | fields | meaning |
//! |---|---|---|
//! | `daily` | `time` `"HH:MM"` | every day at that time |
//! | `weekly` | `days` (`mon`..`sun`), `time` | on those days at that time |
//! | `interval` | `every_secs` (60 to 30 days) | every N seconds, counted from the last run (or from the server start) |
//! | `cron` | `expr` | five fields `minute hour day-of-month month day-of-week` |
//!
//! Cron fields take `*`, a number, a range `a-b`, a list `a,b,c`, and steps `*/n` or `a-b/n`
//! or `a/n`. Months and weekdays also take names (`jan`..`dec`, `sun`..`sat`); the weekday
//! `0` and `7` are both Sunday. When both day-of-month and day-of-week are restricted, a day
//! matches if either does (as in Vixie cron). A local time that does not exist (the hour skipped
//! when the clocks go forward) runs at the first instant after the gap; a local time that
//! happens twice (clocks go back) runs the first time only.

use jiff::{
    Timestamp,
    civil::{Date, DateTime, time},
    tz::TimeZone,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Shortest and longest interval.
pub const MIN_INTERVAL_SECS: u64 = 60;
pub const MAX_INTERVAL_SECS: u64 = 30 * 86_400;

/// How often a task runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Every day at `time` (`"HH:MM"`, 24 hours).
    Daily { time: String },
    /// On `days` (`mon`, `tue`, ..., `sun`) at `time`.
    Weekly { days: Vec<String>, time: String },
    /// Every `every_secs` seconds (60 to 2 592 000).
    Interval { every_secs: u64 },
    /// A five-field cron expression: `minute hour day-of-month month day-of-week`.
    Cron { expr: String },
}

const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// The parsed form of the calendar schedules: the allowed values of each field.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Calendar {
    minutes: u64,
    hours: u32,
    /// Bit 1..=31.
    days: u32,
    /// Bit 1..=12.
    months: u16,
    /// Bit 0 (Sunday) ..= 6.
    weekdays: u8,
    days_any: bool,
    weekdays_any: bool,
}

fn parse_time(text: &str) -> Result<(u8, u8), String> {
    let (h, m) = text.trim().split_once(':').ok_or("time must be HH:MM")?;
    let digits = |s: &str| !s.is_empty() && s.len() <= 2 && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(h) || !digits(m) {
        return Err("time must be HH:MM".into());
    }
    let (h, m): (u8, u8) = (h.parse().unwrap_or(99), m.parse().unwrap_or(99));
    if h > 23 || m > 59 {
        return Err("time must be between 00:00 and 23:59".into());
    }
    Ok((h, m))
}

fn day_index(name: &str) -> Option<u8> {
    const FULL: [&str; 7] = [
        "sunday",
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
    ];
    let n = name.trim().to_ascii_lowercase();
    DAY_NAMES
        .iter()
        .zip(FULL)
        .position(|(short, full)| n == *short || n == full)
        .map(|i| i as u8)
}

/// One cron field into a bit set of the values `lo..=hi`. `names` maps words to values.
fn field(
    text: &str,
    lo: u32,
    hi: u32,
    what: &str,
    name_of: &dyn Fn(&str) -> Option<u32>,
) -> Result<(u64, bool), String> {
    let mut bits = 0u64;
    let mut any = false;
    for part in text.split(',') {
        if part.is_empty() {
            return Err(format!("{what}: empty list entry"));
        }
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => {
                let step: u32 = s.parse().map_err(|_| format!("{what}: bad step {s:?}"))?;
                if step == 0 {
                    return Err(format!("{what}: the step must be at least 1"));
                }
                (r, Some(step))
            }
            None => (part, None),
        };
        let value = |s: &str| -> Result<u32, String> {
            name_of(s)
                .or_else(|| s.parse().ok())
                .ok_or_else(|| format!("{what}: bad value {s:?}"))
        };
        let (from, to) = if range == "*" {
            if step.is_none() {
                any = true;
            }
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (value(a)?, value(b)?)
        } else {
            let v = value(range)?;
            // `a/n` means from a to the end.
            (v, if step.is_some() { hi } else { v })
        };
        if from < lo || to > hi || from > to {
            return Err(format!("{what}: {from}-{to} is outside {lo}-{hi}"));
        }
        let step = step.unwrap_or(1);
        let mut v = from;
        while v <= to {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok((bits, any))
}

impl Calendar {
    fn from_cron(expr: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "a cron expression has 5 fields (minute hour day-of-month month day-of-week), this has {}",
                fields.len()
            ));
        }
        let none = |_: &str| None;
        let (minutes, _) = field(fields[0], 0, 59, "minute", &none)?;
        let (hours, _) = field(fields[1], 0, 23, "hour", &none)?;
        let (days, days_any) = field(fields[2], 1, 31, "day of month", &none)?;
        let month_name = |s: &str| {
            MONTH_NAMES
                .iter()
                .position(|m| s.eq_ignore_ascii_case(m))
                .map(|i| i as u32 + 1)
        };
        let (months, _) = field(fields[3], 1, 12, "month", &month_name)?;
        let weekday_name = |s: &str| day_index(s).map(u32::from);
        let (mut weekdays, weekdays_any) = field(fields[4], 0, 7, "day of week", &weekday_name)?;
        // 7 is Sunday too.
        if weekdays & (1 << 7) != 0 {
            weekdays = (weekdays & !(1 << 7)) | 1;
        }
        Ok(Self {
            minutes,
            hours: hours as u32,
            days: days as u32,
            months: months as u16,
            weekdays: weekdays as u8,
            days_any,
            weekdays_any,
        })
    }

    fn at(hour: u8, minute: u8, weekdays: u8) -> Self {
        Self {
            minutes: 1 << minute,
            hours: 1 << hour,
            days: u32::MAX,
            months: u16::MAX,
            weekdays,
            days_any: true,
            weekdays_any: weekdays == 0x7f,
        }
    }

    fn day_matches(&self, date: Date) -> bool {
        if self.months & (1 << date.month()) == 0 {
            return false;
        }
        let dom = self.days & (1 << date.day()) != 0;
        let dow = self.weekdays & (1 << date.weekday().to_sunday_zero_offset()) != 0;
        match (self.days_any, self.weekdays_any) {
            (true, true) => true,
            (false, true) => dom,
            (true, false) => dow,
            (false, false) => dom || dow,
        }
    }

    /// The first matching instant strictly after `after`.
    fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        let start = after.to_zoned(tz.clone()).date();
        let mut date = start;
        // 8 years: enough for "29 February".
        for _ in 0..(8 * 366) {
            if self.day_matches(date) {
                for h in 0..24u8 {
                    if self.hours & (1 << h) == 0 {
                        continue;
                    }
                    for m in 0..60u8 {
                        if self.minutes & (1 << m) == 0 {
                            continue;
                        }
                        let local = DateTime::from_parts(date, time(h as i8, m as i8, 0, 0));
                        if let Ok(ts) = tz.to_ambiguous_timestamp(local).compatible()
                            && ts > after
                        {
                            return Some(ts);
                        }
                    }
                }
            }
            date = date.tomorrow().ok()?;
        }
        None
    }
}

impl Schedule {
    fn calendar(&self) -> Result<Option<Calendar>, String> {
        match self {
            Self::Daily { time } => {
                let (h, m) = parse_time(time)?;
                Ok(Some(Calendar::at(h, m, 0x7f)))
            }
            Self::Weekly { days, time } => {
                let (h, m) = parse_time(time)?;
                if days.is_empty() {
                    return Err("choose at least one day".into());
                }
                let mut set = 0u8;
                for d in days {
                    let i = day_index(d).ok_or_else(|| {
                        format!("unknown day {d:?} (mon, tue, wed, thu, fri, sat, sun)")
                    })?;
                    set |= 1 << i;
                }
                Ok(Some(Calendar::at(h, m, set)))
            }
            Self::Interval { every_secs } => {
                if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(every_secs) {
                    return Err(format!(
                        "the interval is between {MIN_INTERVAL_SECS} seconds and 30 days"
                    ));
                }
                Ok(None)
            }
            Self::Cron { expr } => Calendar::from_cron(expr).map(Some),
        }
    }

    /// Checks the schedule; the message names the problem.
    pub fn validate(&self) -> Result<(), String> {
        self.calendar().map(|_| ())
    }

    /// The next run strictly after `after` (unix seconds). Intervals count from `last_run` (or
    /// from `after` when the task never ran or the interval has already passed). `None`: the
    /// schedule never matches (a cron date that does not exist) or is invalid.
    pub fn next_after(&self, after: i64, last_run: Option<i64>, tz: &TimeZone) -> Option<i64> {
        match self.calendar().ok()? {
            None => {
                let Self::Interval { every_secs } = self else {
                    return None;
                };
                let every = i64::try_from(*every_secs).ok()?;
                let base = last_run
                    .filter(|l| l.saturating_add(every) > after)
                    .unwrap_or(after);
                base.checked_add(every)
            }
            Some(cal) => {
                let ts = Timestamp::from_second(after).ok()?;
                cal.next_after(ts, tz).map(Timestamp::as_second)
            }
        }
    }

    /// The next `count` runs after `after`.
    pub fn upcoming(
        &self,
        after: i64,
        last_run: Option<i64>,
        tz: &TimeZone,
        count: usize,
    ) -> Vec<i64> {
        let mut out = Vec::new();
        let (mut at, mut last) = (after, last_run);
        while out.len() < count {
            let Some(next) = self.next_after(at, last, tz) else {
                break;
            };
            out.push(next);
            (at, last) = (next, Some(next));
        }
        out
    }

    /// A sentence for the page: "Every day at 04:00".
    pub fn describe(&self) -> String {
        match self {
            Self::Daily { time } => format!("Every day at {time}"),
            Self::Weekly { days, time } => {
                let mut names: Vec<(u8, String)> = days
                    .iter()
                    .filter_map(|d| {
                        let i = day_index(d)?;
                        // Monday first.
                        Some(((i + 6) % 7, d3(i)))
                    })
                    .collect();
                names.sort();
                names.dedup();
                let list: Vec<String> = names.into_iter().map(|(_, n)| n).collect();
                format!("Every {} at {time}", list.join(", "))
            }
            Self::Interval { every_secs } => format!("Every {}", duration_text(*every_secs)),
            Self::Cron { expr } => format!(
                "Cron {}",
                expr.split_whitespace().collect::<Vec<_>>().join(" ")
            ),
        }
    }

    /// Normalises what the user typed: lower-case day names, trimmed fields.
    pub fn normalized(self) -> Self {
        match self {
            Self::Daily { time } => Self::Daily {
                time: normalize_time(&time),
            },
            Self::Weekly { days, time } => {
                let mut set: Vec<u8> = days.iter().filter_map(|d| day_index(d)).collect();
                set.sort_by_key(|i| (i + 6) % 7);
                set.dedup();
                Self::Weekly {
                    days: set
                        .into_iter()
                        .map(|i| DAY_NAMES[i as usize].to_owned())
                        .collect(),
                    time: normalize_time(&time),
                }
            }
            Self::Cron { expr } => Self::Cron {
                expr: expr.split_whitespace().collect::<Vec<_>>().join(" "),
            },
            other => other,
        }
    }
}

fn normalize_time(text: &str) -> String {
    parse_time(text).map_or_else(
        |_| text.trim().to_owned(),
        |(h, m)| format!("{h:02}:{m:02}"),
    )
}

fn d3(i: u8) -> String {
    let n = DAY_NAMES[i as usize];
    format!("{}{}", n[..1].to_ascii_uppercase(), &n[1..])
}

/// `3600` -> "1 h", `5400` -> "1 h 30 min", `90` -> "1 min 30 s".
pub fn duration_text(secs: u64) -> String {
    let (d, h, m, s) = (
        secs / 86_400,
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60,
    );
    let parts: Vec<String> = [(d, "d"), (h, "h"), (m, "min"), (s, "s")]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, u)| format!("{n} {u}"))
        .collect();
    if parts.is_empty() {
        "0 s".into()
    } else {
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use jiff::tz::{self, TimeZone};

    use super::*;

    fn utc() -> TimeZone {
        TimeZone::UTC
    }

    fn ts(s: &str) -> i64 {
        s.parse::<Timestamp>().unwrap().as_second()
    }

    fn daily(t: &str) -> Schedule {
        Schedule::Daily { time: t.into() }
    }

    fn cron(e: &str) -> Schedule {
        Schedule::Cron { expr: e.into() }
    }

    #[test]
    fn daily_runs_at_the_local_time() {
        let s = daily("04:00");
        assert_eq!(
            s.next_after(ts("2026-10-22T03:59:59Z"), None, &utc()),
            Some(ts("2026-10-22T04:00:00Z"))
        );
        // Strictly after: at the moment itself the next day is meant.
        assert_eq!(
            s.next_after(ts("2026-10-22T04:00:00Z"), None, &utc()),
            Some(ts("2026-10-23T04:00:00Z"))
        );
        assert_eq!(
            s.next_after(ts("2026-12-31T23:00:00Z"), None, &utc()),
            Some(ts("2027-01-01T04:00:00Z"))
        );
        // In a zone two hours ahead, 04:00 local is 02:00 UTC.
        let plus2 = TimeZone::fixed(tz::offset(2));
        assert_eq!(
            s.next_after(ts("2026-10-22T00:00:00Z"), None, &plus2),
            Some(ts("2026-10-22T02:00:00Z"))
        );
        assert_eq!(
            s.next_after(ts("2026-10-22T02:00:00Z"), None, &plus2),
            Some(ts("2026-10-23T02:00:00Z"))
        );
    }

    #[test]
    fn weekly_runs_on_the_chosen_days() {
        // 2026-10-22 is a Thursday.
        let s = Schedule::Weekly {
            days: vec!["mon".into(), "fri".into()],
            time: "23:30".into(),
        };
        assert_eq!(
            s.next_after(ts("2026-10-22T12:00:00Z"), None, &utc()),
            Some(ts("2026-10-23T23:30:00Z"))
        );
        assert_eq!(
            s.next_after(ts("2026-10-23T23:30:00Z"), None, &utc()),
            Some(ts("2026-10-26T23:30:00Z"))
        );
        let sunday = Schedule::Weekly {
            days: vec!["SUN".into()],
            time: "0:05".into(),
        };
        assert_eq!(
            sunday.next_after(ts("2026-10-22T12:00:00Z"), None, &utc()),
            Some(ts("2026-10-25T00:05:00Z"))
        );
        assert_eq!(
            s.clone().normalized(),
            Schedule::Weekly {
                days: vec!["mon".into(), "fri".into()],
                time: "23:30".into()
            }
        );
        assert_eq!(
            Schedule::Weekly {
                days: vec!["Fri".into(), "monday".into(), "fri".into()],
                time: "7:5".into()
            }
            .normalized(),
            Schedule::Weekly {
                days: vec!["mon".into(), "fri".into()],
                time: "07:05".into()
            }
        );
    }

    #[test]
    fn intervals_count_from_the_last_run() {
        let s = Schedule::Interval { every_secs: 3600 };
        assert_eq!(s.next_after(1000, None, &utc()), Some(4600));
        // The last run was recently: the next is an interval after it, not after `after`.
        assert_eq!(s.next_after(2000, Some(1500), &utc()), Some(5100));
        // The last run is long ago (the server was down): the interval restarts from now.
        assert_eq!(s.next_after(100_000, Some(1500), &utc()), Some(103_600));
        assert_eq!(s.upcoming(0, None, &utc(), 3), vec![3600, 7200, 10_800]);
    }

    #[test]
    fn cron_fields() {
        let after = ts("2026-10-22T10:07:30Z");
        let next = |e: &str| {
            cron(e)
                .next_after(after, None, &utc())
                .map(|t| Timestamp::from_second(t).unwrap().to_string())
        };
        assert_eq!(
            next("*/15 * * * *").as_deref(),
            Some("2026-10-22T10:15:00Z")
        );
        assert_eq!(
            next("0 4,16 * * *").as_deref(),
            Some("2026-10-22T16:00:00Z")
        );
        assert_eq!(
            next("30 9-11 * * *").as_deref(),
            Some("2026-10-22T10:30:00Z")
        );
        assert_eq!(next("0 0 1 * *").as_deref(), Some("2026-11-01T00:00:00Z"));
        assert_eq!(next("0 0 * jan *").as_deref(), Some("2027-01-01T00:00:00Z"));
        assert_eq!(
            next("5/20 * * * *").as_deref(),
            Some("2026-10-22T10:25:00Z")
        );
        // Monday to Friday at 08:00; today is Thursday at 10:07, so tomorrow.
        assert_eq!(
            next("0 8 * * mon-fri").as_deref(),
            Some("2026-10-23T08:00:00Z")
        );
        assert_eq!(next("0 8 * * 0").as_deref(), Some("2026-10-25T08:00:00Z"));
        assert_eq!(next("0 8 * * 7").as_deref(), Some("2026-10-25T08:00:00Z"));
        // Both day fields restricted: either matches (the 1st, or any Sunday).
        assert_eq!(next("0 8 1 * sun").as_deref(), Some("2026-10-25T08:00:00Z"));
        // 29 February exists only in leap years.
        assert_eq!(next("0 0 29 2 *").as_deref(), Some("2028-02-29T00:00:00Z"));
        // A date that never exists.
        assert_eq!(next("0 0 31 2 *"), None);
        assert_eq!(
            cron("0 0 31 2 *").upcoming(0, None, &utc(), 3),
            Vec::<i64>::new()
        );
    }

    #[test]
    fn invalid_schedules_say_what_is_wrong() {
        let err = |s: Schedule| s.validate().unwrap_err();
        assert!(err(daily("4pm")).contains("HH:MM"));
        assert!(err(daily("24:00")).contains("00:00 and 23:59"));
        assert!(err(daily("")).contains("HH:MM"));
        assert!(
            err(Schedule::Weekly {
                days: vec![],
                time: "04:00".into()
            })
            .contains("at least one day")
        );
        assert!(
            err(Schedule::Weekly {
                days: vec!["funday".into()],
                time: "04:00".into()
            })
            .contains("unknown day")
        );
        assert!(err(Schedule::Interval { every_secs: 59 }).contains("between"));
        assert!(
            err(Schedule::Interval {
                every_secs: MAX_INTERVAL_SECS + 1
            })
            .contains("between")
        );
        assert!(err(cron("* * * *")).contains("5 fields"));
        assert!(err(cron("60 * * * *")).contains("minute"));
        assert!(err(cron("* 24 * * *")).contains("hour"));
        assert!(err(cron("* * 0 * *")).contains("day of month"));
        assert!(err(cron("* * * 13 *")).contains("month"));
        assert!(err(cron("* * * * 8")).contains("day of week"));
        assert!(err(cron("*/0 * * * *")).contains("step"));
        assert!(err(cron("5-1 * * * *")).contains("minute"));
        assert!(err(cron("a * * * *")).contains("bad value"));
        assert!(err(cron("1,,2 * * * *")).contains("empty"));
        // Valid ones pass.
        for ok in [
            "0 4 * * *",
            "*/5 * * * *",
            "0 0 1 jan,jul *",
            "0 12 * * mon-fri",
            "59 23 31 12 sun",
        ] {
            cron(ok).validate().unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        daily("00:00").validate().unwrap();
        Schedule::Interval { every_secs: 60 }.validate().unwrap();
        // An invalid schedule never runs.
        assert_eq!(daily("99:99").next_after(0, None, &utc()), None);
    }

    #[test]
    fn schedules_are_described_for_people() {
        assert_eq!(daily("04:00").describe(), "Every day at 04:00");
        assert_eq!(
            Schedule::Weekly {
                days: vec!["fri".into(), "mon".into()],
                time: "23:30".into()
            }
            .describe(),
            "Every Mon, Fri at 23:30"
        );
        assert_eq!(
            Schedule::Interval { every_secs: 5400 }.describe(),
            "Every 1 h 30 min"
        );
        assert_eq!(
            Schedule::Interval {
                every_secs: 86_400 * 2
            }
            .describe(),
            "Every 2 d"
        );
        assert_eq!(cron(" 0  4 * *   mon").describe(), "Cron 0 4 * * mon");
        assert_eq!(duration_text(0), "0 s");
        assert_eq!(duration_text(61), "1 min 1 s");
    }

    #[test]
    fn json_shape_is_stable() {
        let s: Schedule =
            serde_json::from_str(r#"{"type":"weekly","days":["mon"],"time":"04:00"}"#).unwrap();
        assert_eq!(
            s,
            Schedule::Weekly {
                days: vec!["mon".into()],
                time: "04:00".into()
            }
        );
        assert_eq!(
            serde_json::to_string(&daily("04:00")).unwrap(),
            r#"{"type":"daily","time":"04:00"}"#
        );
        assert_eq!(
            serde_json::to_string(&Schedule::Interval { every_secs: 60 }).unwrap(),
            r#"{"type":"interval","every_secs":60}"#
        );
        assert!(serde_json::from_str::<Schedule>(r#"{"type":"hourly"}"#).is_err());
    }

    /// Daylight saving time, with the real zone database when this machine has one.
    #[test]
    fn daylight_saving_changes_are_handled() {
        let Ok(berlin) = TimeZone::get("Europe/Berlin") else {
            eprintln!("no tz database: skipping");
            return;
        };
        // 2026-03-29: 02:00 -> 03:00 (clocks go forward). A task at 02:30 runs right after
        // the gap, at 03:30 local = 01:30 UTC... jiff's "compatible" shifts forward by the gap.
        let spring = daily("02:30");
        let next = spring
            .next_after(ts("2026-03-28T12:00:00Z"), None, &berlin)
            .unwrap();
        assert_eq!(
            Timestamp::from_second(next)
                .unwrap()
                .to_zoned(berlin.clone())
                .strftime("%F %H:%M")
                .to_string(),
            "2026-03-29 03:30"
        );
        // The day after it is back to 02:30 local.
        let next2 = spring.next_after(next, None, &berlin).unwrap();
        assert_eq!(
            Timestamp::from_second(next2)
                .unwrap()
                .to_zoned(berlin.clone())
                .strftime("%F %H:%M")
                .to_string(),
            "2026-03-30 02:30"
        );
        // 2026-10-25: 03:00 -> 02:00 (clocks go back). 02:30 happens twice: only the first.
        let autumn = daily("02:30");
        let a = autumn
            .next_after(ts("2026-10-24T12:00:00Z"), None, &berlin)
            .unwrap();
        assert_eq!(a, ts("2026-10-25T00:30:00Z"));
        let b = autumn.next_after(a, None, &berlin).unwrap();
        assert_eq!(
            b,
            ts("2026-10-26T01:30:00Z"),
            "not the second 02:30 of the same night"
        );
    }
}
