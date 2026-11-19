//! Local calendar boundaries shared by statistics storage and date filters.
use chrono::{DateTime, Datelike, Duration, Local, LocalResult, NaiveDate, TimeZone, Timelike};
use super_paste_format::StatisticsInterval;
use crate::{Error, Result};

const HOUR: i64 = 3_600_000;
const DAY: i64 = 86_400_000;

fn invalid() -> Error { Error::Invalid("统计日期或时区无效".into()) }

fn at(timestamp: i64) -> Result<DateTime<Local>> {
    let value = Local.timestamp_millis_opt(timestamp).single().ok_or_else(invalid)?;
    // Chrono's UTC range can extend beyond its representable local calendar.
    value.naive_utc().checked_add_signed(Duration::seconds(i64::from(value.offset().local_minus_utc())))
        .ok_or_else(invalid)?;
    Ok(value)
}

fn elapsed_hour(value: &DateTime<Local>) -> i64 {
    i64::from(value.minute()) * 60_000 + i64::from(value.second()) * 1_000
        + i64::from(value.timestamp_subsec_millis())
}

// An offset change inside this hour is itself a bucket boundary. This keeps
// repeated/partial hours distinct, including zones whose DST change is not 1h.
fn transition(mut before: i64, mut after: i64, offset_before: i32) -> Result<i64> {
    while after - before > 1 {
        let middle = before + (after - before) / 2;
        if at(middle)?.offset().local_minus_utc() == offset_before { before = middle; }
        else { after = middle; }
    }
    Ok(after)
}

/// First instant on or after this local date's midnight. A missing midnight
/// advances to the actual clock transition; an entirely skipped date advances
/// to the next real date. Repeated midnight chooses its earliest occurrence.
pub fn day_start(date: NaiveDate) -> Result<i64> {
    let wall = date.and_hms_opt(0, 0, 0).ok_or_else(invalid)?;
    let candidates = match Local.from_local_datetime(&wall) {
        LocalResult::Single(value) => [Some(value), None],
        LocalResult::Ambiguous(first, second) => [Some(first), Some(second)],
        LocalResult::None => [None, None],
    };
    let mut earliest = None;
    for candidate in candidates.into_iter().flatten() {
        let timestamp = candidate.timestamp_millis();
        // The locked local backend can reverse ambiguous candidates or include
        // a stale offset at a transition endpoint. Validate against UTC and
        // compare actual instants rather than trusting LocalResult's order.
        if at(timestamp)?.naive_local() == wall {
            earliest = Some(earliest.map_or(timestamp, |previous: i64| previous.min(timestamp)));
        }
    }
    if let Some(timestamp) = earliest { return Ok(timestamp); }
    // Every supported UTC offset is strictly within +/-24h. A nonexistent
    // wall time lies in a forward gap, so search the corresponding UTC interval
    // for the first real instant after it instead of probing every minute.
    let center = wall.and_utc().timestamp_millis();
    let mut before = center.checked_sub(DAY).ok_or_else(invalid)?;
    let mut after = center.checked_add(DAY).ok_or_else(invalid)?;
    if at(before)?.naive_local() >= wall || at(after)?.naive_local() < wall { return Err(invalid()); }
    while after - before > 1 {
        let middle = before + (after - before) / 2;
        if at(middle)?.naive_local() < wall { before = middle; }
        else { after = middle; }
    }
    Ok(after)
}

pub fn period_start(timestamp: i64, interval: StatisticsInterval) -> Result<i64> {
    let value = at(timestamp)?;
    match interval {
        StatisticsInterval::Hour => {
            let nominal = timestamp.checked_sub(elapsed_hour(&value)).ok_or_else(invalid)?;
            let offset = at(nominal)?.offset().local_minus_utc();
            if offset == value.offset().local_minus_utc() { Ok(nominal) }
            else { transition(nominal, timestamp, offset) }
        }
        StatisticsInterval::Day => day_start(value.date_naive()),
        StatisticsInterval::Month => day_start(value.date_naive().with_day(1).ok_or_else(invalid)?),
    }
}

pub fn period_next(timestamp: i64, interval: StatisticsInterval) -> Result<i64> {
    let value = at(timestamp)?;
    match interval {
        StatisticsInterval::Hour => {
            let nominal = timestamp.checked_add(HOUR - elapsed_hour(&value)).ok_or_else(invalid)?;
            let offset = value.offset().local_minus_utc();
            if at(nominal)?.offset().local_minus_utc() == offset { Ok(nominal) }
            else { transition(timestamp, nominal, offset) }
        }
        StatisticsInterval::Day => day_start(value.date_naive().succ_opt().ok_or_else(invalid)?),
        StatisticsInterval::Month => day_start(value.date_naive().with_day(1)
            .and_then(|date| date.checked_add_months(chrono::Months::new(1))).ok_or_else(invalid)?),
    }
}

#[cfg(all(test, unix))]
mod tests;
