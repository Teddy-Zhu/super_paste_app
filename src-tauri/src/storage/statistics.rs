use chrono::{Datelike, Local, TimeZone, Timelike};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::{Storage, now_ms};

const TYPES: &[&str] = &[
    "text", "html", "image", "rtf", "files", "url", "email", "phone", "code", "color",
];

pub(super) async fn record_copy(storage: &Storage, event: &Value) -> Result<bool, String> {
    let event = event.clone();
    storage
        .call(move |connection| {
            let timestamp = as_timestamp(event.get("timestamp").ok_or(rusqlite::Error::InvalidQuery)?)?;
            let kind = event.get("type").and_then(Value::as_str).filter(|value| TYPES.contains(value)).ok_or(rusqlite::Error::InvalidQuery)?;
            let source = event.get("source").and_then(Value::as_str).filter(|value| value.encode_utf16().count() <= 200).ok_or(rusqlite::Error::InvalidQuery)?;
            let bytes = nonnegative(&event,"bytes")?;
            let files = nonnegative(&event,"files")?;
            let cleared_at = connection.query_row("SELECT value FROM copy_statistics_meta WHERE key='clearedAt'",[],|row|row.get::<_,i64>(0)).optional()?;
            if cleared_at.is_some_and(|cleared| timestamp <= cleared) { return Ok(false); }
            let hour = local_hour(timestamp).ok_or(rusqlite::Error::InvalidQuery)?;
            connection.execute("INSERT INTO copy_statistics(hour,type,source,copies,bytes,files,first_copied_at,last_copied_at) VALUES(?1,?2,?3,1,?4,?5,?6,?6) ON CONFLICT(hour,type,source) DO UPDATE SET copies=copies+1,bytes=bytes+excluded.bytes,files=files+excluded.files,first_copied_at=MIN(first_copied_at,excluded.first_copied_at),last_copied_at=MAX(last_copied_at,excluded.last_copied_at)",params![hour,kind,source,bytes,files,timestamp])?;
            Ok(true)
        })
        .await
}

pub(super) async fn get_copy_stats(storage: &Storage, query: &Value) -> Result<Value, String> {
    let query = query.clone();
    storage
        .call(move |connection| {
            let interval = query
                .get("interval")
                .and_then(Value::as_str)
                .unwrap_or("day");
            if !["hour", "day", "month"].contains(&interval) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let from = query
                .get("from")
                .map(as_timestamp)
                .transpose()?
                .unwrap_or(0);
            let to = query
                .get("to")
                .map(as_timestamp)
                .transpose()?
                .unwrap_or(i64::MAX);
            if from >= to {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let kind = query.get("type").and_then(Value::as_str);
            if kind.is_some_and(|value| !TYPES.contains(&value)) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let source = query.get("source").and_then(Value::as_str);
            if source.is_some_and(|value| value.encode_utf16().count() > 200) {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let start_hour = local_hour(from).ok_or(rusqlite::Error::InvalidQuery)?;
            let cleared_at = connection
                .query_row(
                    "SELECT value FROM copy_statistics_meta WHERE key='clearedAt'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            let all = read_buckets(connection, start_hour, to, kind, None)?;
            let available_sources = all
                .iter()
                .map(|bucket| bucket.source.clone())
                .collect::<std::collections::BTreeSet<_>>();
            let selected = if let Some(source) = source {
                all.into_iter()
                    .filter(|bucket| bucket.source == source)
                    .collect()
            } else {
                all
            };
            let mut totals = empty_totals();
            let mut types = std::collections::BTreeMap::<String, Totals>::new();
            let mut sources = std::collections::BTreeMap::<String, Totals>::new();
            let mut days = std::collections::BTreeMap::<String, Totals>::new();
            let mut periods = std::collections::BTreeMap::<String, Totals>::new();
            for bucket in selected {
                let value = bucket.totals();
                merge(&mut totals, &value);
                merge(
                    types
                        .entry(bucket.kind.clone())
                        .or_insert_with(empty_totals),
                    &value,
                );
                merge(
                    sources
                        .entry(bucket.source.clone())
                        .or_insert_with(empty_totals),
                    &value,
                );
                let day = period_key(bucket.hour, "day").ok_or(rusqlite::Error::InvalidQuery)?;
                merge(days.entry(day).or_insert_with(empty_totals), &value);
                let period =
                    period_key(bucket.hour, interval).ok_or(rusqlite::Error::InvalidQuery)?;
                merge(periods.entry(period).or_insert_with(empty_totals), &value);
            }
            let start = query
                .get("from")
                .and_then(Value::as_i64)
                .unwrap_or(totals.first.unwrap_or_else(now_ms));
            let series_end = to.min(now_ms().saturating_add(1));
            let mut cursor = period_start(start, interval).ok_or(rusqlite::Error::InvalidQuery)?;
            let mut series = Vec::new();
            while cursor < series_end {
                if series.len() >= 2000 {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let key = period_key(cursor, interval).ok_or(rusqlite::Error::InvalidQuery)?;
                let value = periods.get(&key).cloned().unwrap_or_else(empty_totals);
                let mut item = value.to_value();
                item["key"] = Value::String(key);
                item["timestamp"] = Value::from(cursor);
                series.push(item);
                cursor = period_next(cursor, interval).ok_or(rusqlite::Error::InvalidQuery)?;
            }
            let active_days = days.len();
            let type_groups = groups(types);
            let source_groups = groups(sources);
            let mut day_totals = empty_totals();
            for day in days.values() {
                merge(&mut day_totals, day);
            }
            Ok(json!({
                "success":true,
                "data":{
                    "copies":day_totals.copies,
                    "bytes":day_totals.bytes,
                    "files":day_totals.files,
                    "firstCopiedAt":day_totals.first,
                    "lastCopiedAt":day_totals.last,
                    "clearedAt":cleared_at,
                    "activeDays":active_days,
                    "types":type_groups,
                    "sources":source_groups,
                    "availableSources":available_sources,
                    "series":series,
                    "interval":interval
                }
            }))
        })
        .await
}

pub(super) async fn clear_copy_stats(storage: &Storage) -> Result<(), String> {
    storage.call(|connection| {
        let transaction=connection.transaction()?;
        transaction.execute("DELETE FROM copy_statistics",[])?;
        transaction.execute("INSERT INTO copy_statistics_meta(key,value) VALUES('clearedAt',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[now_ms()])?;
        transaction.commit()
    }).await
}

fn read_buckets(
    connection: &rusqlite::Connection,
    from: i64,
    to: i64,
    kind: Option<&str>,
    source: Option<&str>,
) -> rusqlite::Result<Vec<Bucket>> {
    let mut sql = String::from(
        "SELECT hour,type,source,copies,bytes,files,first_copied_at,last_copied_at FROM copy_statistics WHERE hour>=?1 AND hour<?2",
    );
    let mut values = vec![
        rusqlite::types::Value::Integer(from),
        rusqlite::types::Value::Integer(to),
    ];
    if let Some(kind) = kind {
        sql.push_str(" AND type=?");
        values.push(rusqlite::types::Value::Text(kind.to_owned()));
    }
    if let Some(source) = source {
        sql.push_str(" AND source=?");
        values.push(rusqlite::types::Value::Text(source.to_owned()));
    }
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
        Ok(Bucket {
            hour: row.get(0)?,
            kind: row.get(1)?,
            source: row.get(2)?,
            copies: row.get(3)?,
            bytes: row.get(4)?,
            files: row.get(5)?,
            first: row.get(6)?,
            last: row.get(7)?,
        })
    })?;
    rows.collect()
}

#[derive(Clone)]
struct Bucket {
    hour: i64,
    kind: String,
    source: String,
    copies: i64,
    bytes: i64,
    files: i64,
    first: i64,
    last: i64,
}
impl Bucket {
    fn totals(&self) -> Totals {
        Totals {
            copies: self.copies,
            bytes: self.bytes,
            files: self.files,
            first: Some(self.first),
            last: Some(self.last),
        }
    }
}
#[derive(Clone)]
struct Totals {
    copies: i64,
    bytes: i64,
    files: i64,
    first: Option<i64>,
    last: Option<i64>,
}
impl Totals {
    fn to_value(&self) -> Value {
        json!({"copies":self.copies,"bytes":self.bytes,"files":self.files,"firstCopiedAt":self.first,"lastCopiedAt":self.last})
    }
}
fn empty_totals() -> Totals {
    Totals {
        copies: 0,
        bytes: 0,
        files: 0,
        first: None,
        last: None,
    }
}
fn merge(target: &mut Totals, value: &Totals) {
    target.copies += value.copies;
    target.bytes += value.bytes;
    target.files += value.files;
    target.first = match (target.first, value.first) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    target.last = match (target.last, value.last) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}
fn groups(values: std::collections::BTreeMap<String, Totals>) -> Vec<Value> {
    let mut rows = values
        .into_iter()
        .map(|(key, value)| {
            let mut value = value.to_value();
            value["key"] = Value::String(key);
            value
        })
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| {
        b["copies"]
            .as_i64()
            .cmp(&a["copies"].as_i64())
            .then_with(|| a["key"].as_str().cmp(&b["key"].as_str()))
    });
    rows
}
fn nonnegative(value: &Value, key: &str) -> rusqlite::Result<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| (0..=9_007_199_254_740_991).contains(value))
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn as_timestamp(value: &Value) -> rusqlite::Result<i64> {
    value
        .as_i64()
        .filter(|value| *value >= 0 && *value <= 8_640_000_000_000_000)
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn local_hour(timestamp: i64) -> Option<i64> {
    Local
        .timestamp_millis_opt(timestamp)
        .single()?
        .with_minute(0)?
        .with_second(0)?
        .with_nanosecond(0)?
        .timestamp_millis()
        .into()
}
fn period_key(timestamp: i64, interval: &str) -> Option<String> {
    let dt = Local.timestamp_millis_opt(timestamp).single()?;
    Some(match interval {
        "hour" => timestamp.to_string(),
        "day" => dt.format("%Y-%m-%d").to_string(),
        "month" => dt.format("%Y-%m").to_string(),
        _ => return None,
    })
}
fn period_start(timestamp: i64, interval: &str) -> Option<i64> {
    let dt = Local.timestamp_millis_opt(timestamp).single()?;
    let date = match interval {
        "hour" => dt.with_minute(0)?.with_second(0)?.with_nanosecond(0)?,
        "day" => dt
            .date_naive()
            .and_hms_opt(0, 0, 0)?
            .and_local_timezone(Local)
            .single()?,
        "month" => dt
            .date_naive()
            .with_day(1)?
            .and_hms_opt(0, 0, 0)?
            .and_local_timezone(Local)
            .single()?,
        _ => return None,
    };
    Some(date.timestamp_millis())
}
fn period_next(timestamp: i64, interval: &str) -> Option<i64> {
    let dt = Local.timestamp_millis_opt(timestamp).single()?;
    let next = match interval {
        "hour" => dt + chrono::Duration::hours(1),
        "day" => (dt.date_naive() + chrono::Duration::days(1))
            .and_hms_opt(0, 0, 0)?
            .and_local_timezone(Local)
            .single()?,
        "month" => {
            let date = dt
                .date_naive()
                .with_day(1)?
                .checked_add_months(chrono::Months::new(1))?;
            date.and_hms_opt(0, 0, 0)?
                .and_local_timezone(Local)
                .single()?
        }
        _ => return None,
    };
    Some(next.timestamp_millis())
}
