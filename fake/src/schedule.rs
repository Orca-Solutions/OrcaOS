// SPDX-FileCopyrightText: The OrcaOS Authors
// SPDX-License-Identifier: GPL-3.0-or-later

//! Scheduled runs on the household's wall clock (surface 9). A run set for a
//! local time fires once per calendar day or week on the wall clock: a time
//! the clocks skip fires at the next valid minute, and a time the clocks
//! repeat fires once, at its first occurrence.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc, Weekday};
use chrono_tz::Tz;

use crate::model::Schedule;

pub enum Cadence {
    Daily,
    Weekly(Weekday),
}

impl Cadence {
    pub fn parse(s: &str) -> Option<Self> {
        if s == "daily" {
            return Some(Self::Daily);
        }
        let day = s.strip_prefix("weekly:")?;
        let weekday = match day {
            "mon" => Weekday::Mon,
            "tue" => Weekday::Tue,
            "wed" => Weekday::Wed,
            "thu" => Weekday::Thu,
            "fri" => Weekday::Fri,
            "sat" => Weekday::Sat,
            "sun" => Weekday::Sun,
            _ => return None,
        };
        Some(Self::Weekly(weekday))
    }

    pub fn hours(&self) -> i64 {
        match self {
            Self::Daily => 24,
            Self::Weekly(_) => 24 * 7,
        }
    }

    /// The default catch-up window: half the cadence, which the contract
    /// states as 12 hours for a daily run and 3 days for a weekly one.
    pub fn default_catch_up_hours(&self) -> i64 {
        match self {
            Self::Daily => 12,
            Self::Weekly(_) => 72,
        }
    }

    fn matches(&self, date: NaiveDate) -> bool {
        match self {
            Self::Daily => true,
            Self::Weekly(day) => date.weekday() == *day,
        }
    }
}

pub fn parse_time(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s, "%H:%M").ok()
}

/// A tick: when it fires, and the clock-change adjustment, if one applied.
pub struct Tick {
    pub at: DateTime<Utc>,
    pub adjustment: Option<&'static str>,
}

/// The ticks of a schedule in `(after, upto]`, in order.
pub fn ticks_between(schedule: &Schedule, after: DateTime<Utc>, upto: DateTime<Utc>) -> Vec<Tick> {
    let (Ok(zone), Some(cadence), Some(time)) = (
        schedule.zone.parse::<Tz>(),
        Cadence::parse(&schedule.cadence),
        parse_time(&schedule.at),
    ) else {
        return Vec::new();
    };

    let mut date = after.with_timezone(&zone).date_naive() - Duration::days(1);
    let last = upto.with_timezone(&zone).date_naive() + Duration::days(1);
    let mut ticks = Vec::new();

    while date <= last {
        if cadence.matches(date)
            && let Some(tick) = resolve(&zone, date, time)
            && tick.at > after
            && tick.at <= upto
        {
            ticks.push(tick);
        }
        date += Duration::days(1);
    }
    ticks
}

fn resolve(zone: &Tz, date: NaiveDate, time: NaiveTime) -> Option<Tick> {
    let wall = date.and_time(time);
    match zone.from_local_datetime(&wall) {
        LocalResult::Single(t) => Some(Tick { at: t.with_timezone(&Utc), adjustment: None }),
        LocalResult::Ambiguous(first, _) => Some(Tick {
            at: first.with_timezone(&Utc),
            adjustment: Some("the clocks repeated this hour; the run fired once, at its first occurrence"),
        }),
        LocalResult::None => (1..=180).find_map(|minutes| {
            let later = wall + Duration::minutes(minutes);
            match zone.from_local_datetime(&later) {
                LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(Tick {
                    at: t.with_timezone(&Utc),
                    adjustment: Some("the clocks skipped this time; the run fired at the next valid minute"),
                }),
                LocalResult::None => None,
            }
        }),
    }
}

/// The household's wall clock, for ledger lines and daily budgets.
pub fn local(zone: &str, at: DateTime<Utc>) -> String {
    match zone.parse::<Tz>() {
        Ok(tz) => at.with_timezone(&tz).format("%Y-%m-%d %H:%M %Z").to_string(),
        Err(_) => at.format("%Y-%m-%d %H:%M UTC").to_string(),
    }
}

pub fn local_date(zone: &str, at: DateTime<Utc>) -> NaiveDate {
    match zone.parse::<Tz>() {
        Ok(tz) => at.with_timezone(&tz).date_naive(),
        Err(_) => at.date_naive(),
    }
}
