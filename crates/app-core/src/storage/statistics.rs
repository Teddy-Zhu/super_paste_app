use std::collections::{BTreeMap, BTreeSet};
use chrono::{Local, TimeZone};
use rusqlite::{OptionalExtension, params};
use super_paste_format::*;
use tokio_util::sync::CancellationToken;
use crate::{Error, Result};
use super::Storage;
pub(super) use crate::calendar::{period_start, period_next};

impl Storage {
    pub async fn record_copy(&self, event: CopyEvent) -> Result<bool> {
        validate_timestamp(event.timestamp)?;
        if event.kind == PrimaryType::Empty || event.source.encode_utf16().count() > 200 || event.bytes > 9_007_199_254_740_991 || event.files > 9_007_199_254_740_991 {
            return Err(Error::Invalid("复制统计内容无效".into()));
        }
        self.call(move |connection| {
            let cleared_at: Option<i64> = connection.query_row("SELECT value FROM copy_statistics_meta WHERE key='clearedAt'", [], |row| row.get(0)).optional()?;
            if cleared_at.is_some_and(|cleared| event.timestamp <= cleared) { return Ok(false); }
            let hour = period_start(event.timestamp, StatisticsInterval::Hour)?;
            connection.execute("INSERT INTO copy_statistics(hour,type,source,copies,bytes,files,first_copied_at,last_copied_at) VALUES(?1,?2,?3,1,?4,?5,?6,?6) ON CONFLICT(hour,type,source) DO UPDATE SET copies=copies+1,bytes=bytes+excluded.bytes,files=files+excluded.files,first_copied_at=MIN(first_copied_at,excluded.first_copied_at),last_copied_at=MAX(last_copied_at,excluded.last_copied_at)", params![hour, event.kind.as_str(), event.source, event.bytes, event.files, event.timestamp])?;
            Ok(true)
        }).await
    }
    pub async fn copy_stats(&self, query: CopyStatsQuery, now: i64, cancel: &CancellationToken) -> Result<CopyStats> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let from = query.from.unwrap_or(0);
        let to = query.to.unwrap_or(i64::MAX);
        validate_timestamp(from)?;
        if query.to.is_some() { validate_timestamp(to)?; }
        if from >= to || query.kind == Some(PrimaryType::Empty) || query.source.as_ref().is_some_and(|v| v.encode_utf16().count() > 200) {
            return Err(Error::Invalid("复制统计查询参数无效".into()));
        }
        let requested = cancel.clone();
        self.call_read(cancel, move |connection| {
            let mut sql = String::from("SELECT hour,type,source,copies,bytes,files,first_copied_at,last_copied_at FROM copy_statistics WHERE hour>=?1 AND hour<?2");
            let mut values = vec![rusqlite::types::Value::Integer(period_start(from, StatisticsInterval::Hour)?), to.into()];
            if let Some(kind) = query.kind { sql.push_str(" AND type=?"); values.push(kind.as_str().to_owned().into()); }
            let mut statement = connection.prepare(&sql)?;
            let mut rows = statement.query(rusqlite::params_from_iter(values))?;
            let mut totals = CopyTotals::default();
            let mut types = BTreeMap::<String, CopyTotals>::new();
            let mut sources = BTreeMap::<String, CopyTotals>::new();
            let mut periods = BTreeMap::<String, CopyTotals>::new();
            let mut days = BTreeSet::<String>::new();
            let mut available_sources = BTreeSet::<String>::new();
            while let Some(row) = rows.next()? {
                if requested.is_cancelled() { return Err(Error::Cancelled); }
                let hour: i64 = row.get(0)?;
                let kind: String = row.get(1)?;
                let source: String = row.get(2)?;
                available_sources.insert(source.clone());
                if query.source.as_ref().is_some_and(|selected| selected != &source) { continue; }
                let value = CopyTotals { copies: row.get(3)?, bytes: row.get(4)?, files: row.get(5)?, first_copied_at: Some(row.get(6)?), last_copied_at: Some(row.get(7)?) };
                totals.merge(&value);
                types.entry(kind).or_default().merge(&value);
                sources.entry(source).or_default().merge(&value);
                days.insert(period_key(hour, StatisticsInterval::Day)?);
                periods.entry(period_key(hour, query.interval)?).or_default().merge(&value);
            }
            let mut cursor = period_start(query.from.unwrap_or(totals.first_copied_at.unwrap_or(now)), query.interval)?;
            let end = to.min(now.saturating_add(1));
            let mut series = Vec::new();
            while cursor < end {
                if requested.is_cancelled() { return Err(Error::Cancelled); }
                if series.len() >= 2000 { return Err(Error::Invalid("统计时间范围过大".into())); }
                let key = period_key(cursor, query.interval)?;
                let value = periods.remove(&key).unwrap_or_default();
                series.push(CopySeriesPoint { key, timestamp: cursor, totals: value });
                let next = period_next(cursor, query.interval)?;
                if next <= cursor { return Err(Error::Invalid("统计时间分段无效".into())); }
                cursor = next;
            }
            let cleared_at = connection.query_row("SELECT value FROM copy_statistics_meta WHERE key='clearedAt'", [], |row| row.get(0)).optional()?;
            Ok(CopyStats { totals, cleared_at, active_days: days.len(), types: groups(types), sources: groups(sources), available_sources: available_sources.into_iter().collect(), series, interval: query.interval })
        }).await
    }
    pub async fn clear_copy_stats(&self, timestamp: i64) -> Result<()> {
        validate_timestamp(timestamp)?;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute("DELETE FROM copy_statistics", [])?;
            transaction.execute("INSERT INTO copy_statistics_meta(key,value) VALUES('clearedAt',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [timestamp])?;
            transaction.commit()?;
            Ok(())
        }).await
    }
}
fn groups(groups: BTreeMap<String, CopyTotals>) -> Vec<CopyGroup> {
    let mut groups: Vec<_> = groups.into_iter().map(|(key, totals)| CopyGroup { key, totals }).collect();
    groups.sort_by(|a, b| b.totals.copies.cmp(&a.totals.copies).then_with(|| a.key.cmp(&b.key)));
    groups
}
pub(super) fn validate_timestamp(timestamp: i64) -> Result<()> {
    if !(0..=8_640_000_000_000_000).contains(&timestamp) { return Err(invalid_time()); }
    Ok(())
}
fn invalid_time() -> Error { Error::Invalid("统计日期或时区无效".into()) }
pub(super) fn period_key(timestamp: i64, interval: StatisticsInterval) -> Result<String> {
    let dt = Local.timestamp_millis_opt(timestamp).single().ok_or_else(invalid_time)?;
    Ok(match interval { StatisticsInterval::Hour => timestamp.to_string(), StatisticsInterval::Day => dt.format("%Y-%m-%d").to_string(), StatisticsInterval::Month => dt.format("%Y-%m").to_string() })
}
