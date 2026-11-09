use chrono::{Local, TimeZone, Timelike};
use rusqlite::{OptionalExtension, params};
use serde_json::{Value, json};

use super::{Storage, now_ms};

const SUM_FIELDS: &str = "SUM(requests),SUM(failures),SUM(unknown_usage),SUM(input_tokens),SUM(output_tokens),SUM(cached_input_tokens),SUM(cache_write_tokens),SUM(total_tokens),SUM(duration_ms),MIN(first_used_at),MAX(last_used_at)";

pub(super) async fn record_ai_usage(storage: &Storage, entry: &Value) -> Result<bool, String> {
    let entry = entry.clone();
    storage.call(move |connection| {
        let id=required_string(&entry,"id")?;
        let provider=required_string(&entry,"provider")?;
        if !["openai","claude"].contains(&provider){return Err(rusqlite::Error::InvalidQuery);}
        let base_url=required_string(&entry,"baseUrl")?;
        let model=required_string(&entry,"model")?;
        let purpose=required_string(&entry,"purpose")?;
        if !["ocr","connection-test"].contains(&purpose){return Err(rusqlite::Error::InvalidQuery);}
        let started=required_nonnegative(&entry,"startedAt")?;
        let finished=required_nonnegative(&entry,"finishedAt")?;
        let duration=required_nonnegative(&entry,"durationMs")?;
        let success=entry.get("success").and_then(Value::as_bool).ok_or(rusqlite::Error::InvalidQuery)?;
        let input=optional_nonnegative(&entry,"inputTokens")?;
        let output=optional_nonnegative(&entry,"outputTokens")?;
        let cached=required_nonnegative(&entry,"cachedInputTokens")?;
        let cache_write=required_nonnegative(&entry,"cacheWriteTokens")?;
        let total=optional_nonnegative(&entry,"totalTokens")?;
        let transaction=connection.transaction()?;
        let cleared_at=transaction.query_row("SELECT value FROM ai_ocr_usage_meta WHERE key='clearedAt'",[],|row|row.get::<_,i64>(0)).optional()?;
        if cleared_at.is_some_and(|cleared|started<=cleared){return Ok(false);}
        let inserted=transaction.execute("INSERT OR IGNORE INTO ai_ocr_usage_recent(id,provider,base_url,model,purpose,started_at,finished_at,duration_ms,success,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![id,provider,base_url,model,purpose,started,finished,duration,success,input,output,cached,cache_write,total])?;
        if inserted==0{return Ok(false);}
        transaction.execute("INSERT OR IGNORE INTO ai_ocr_models(provider,base_url,model) VALUES(?1,?2,?3)",params![provider,base_url,model])?;
        let model_id:i64=transaction.query_row("SELECT id FROM ai_ocr_models WHERE provider=?1 AND base_url=?2 AND model=?3",params![provider,base_url,model],|row|row.get(0))?;
        let hour=local_hour(started).ok_or(rusqlite::Error::InvalidQuery)?;
        let unknown=i64::from(total.is_none());
        transaction.execute("INSERT INTO ai_ocr_usage_hourly(hour,model_id,requests,failures,unknown_usage,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens,duration_ms,first_used_at,last_used_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(hour,model_id) DO UPDATE SET requests=requests+1,failures=failures+excluded.failures,unknown_usage=unknown_usage+excluded.unknown_usage,input_tokens=input_tokens+excluded.input_tokens,output_tokens=output_tokens+excluded.output_tokens,cached_input_tokens=cached_input_tokens+excluded.cached_input_tokens,cache_write_tokens=cache_write_tokens+excluded.cache_write_tokens,total_tokens=total_tokens+excluded.total_tokens,duration_ms=duration_ms+excluded.duration_ms,first_used_at=MIN(first_used_at,excluded.first_used_at),last_used_at=MAX(last_used_at,excluded.last_used_at)",params![hour,model_id,i64::from(!success),unknown,input.unwrap_or(0),output.unwrap_or(0),cached,cache_write,total.unwrap_or(0),duration,started,finished])?;
        transaction.execute("DELETE FROM ai_ocr_usage_recent WHERE id NOT IN (SELECT id FROM ai_ocr_usage_recent ORDER BY started_at DESC,id DESC LIMIT 50)",[])?;
        transaction.commit()?;
        Ok(true)
    }).await
}

pub(super) async fn get_ai_usage_stats(storage: &Storage, query: &Value) -> Result<Value, String> {
    let query = query.clone();
    storage.call(move |connection|{
        let from=query.get("from").map(required_timestamp).transpose()?.unwrap_or(0);
        let to=query.get("to").map(required_timestamp).transpose()?.unwrap_or(i64::MAX);
        if from>to{return Err(rusqlite::Error::InvalidQuery);}
        let from_hour=local_hour(from).ok_or(rusqlite::Error::InvalidQuery)?;
        let to_hour=if to==i64::MAX{i64::MAX}else{local_hour(to).ok_or(rusqlite::Error::InvalidQuery)?.saturating_add(3_600_000)};
        let cleared_at=connection.query_row("SELECT value FROM ai_ocr_usage_meta WHERE key='clearedAt'",[],|row|row.get::<_,i64>(0)).optional()?;
        let mut totals=empty_totals();
        let mut models=std::collections::BTreeMap::<(String,String,String),UsageTotals>::new();
        let mut days=std::collections::BTreeMap::<String,UsageTotals>::new();
        {
            let mut statement=connection.prepare(&format!("SELECT hour,provider,base_url,model,{SUM_FIELDS} FROM ai_ocr_usage_hourly h JOIN ai_ocr_models m ON m.id=h.model_id WHERE hour>=?1 AND hour<?2 GROUP BY hour,model_id ORDER BY hour"))?;
            let rows=statement.query_map(params![from_hour,to_hour],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,read_totals(row,4)?)))?;
            for row in rows {
                let (hour,provider,base_url,model,value)=row?;
                merge(&mut totals,&value);
                merge(models.entry((provider,base_url,model)).or_insert_with(empty_totals),&value);
                let day=Local.timestamp_millis_opt(hour).single().ok_or(rusqlite::Error::InvalidQuery)?.format("%Y-%m-%d").to_string();
                merge(days.entry(day).or_insert_with(empty_totals),&value);
            }
        }
        let recent = {
            let mut statement=connection.prepare("SELECT id,provider,base_url,model,purpose,started_at,finished_at,duration_ms,success,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens FROM ai_ocr_usage_recent WHERE started_at>=?1 AND started_at<?2 ORDER BY started_at DESC,id DESC LIMIT 50")?;
            let rows=statement.query_map(params![from_hour,to_hour],|row|Ok(json!({"id":row.get::<_,String>(0)?,"provider":row.get::<_,String>(1)?,"baseUrl":row.get::<_,String>(2)?,"model":row.get::<_,String>(3)?,"purpose":row.get::<_,String>(4)?,"startedAt":row.get::<_,i64>(5)?,"finishedAt":row.get::<_,i64>(6)?,"durationMs":row.get::<_,i64>(7)?,"success":row.get::<_,bool>(8)?,"inputTokens":row.get::<_,Option<i64>>(9)?,"outputTokens":row.get::<_,Option<i64>>(10)?,"cachedInputTokens":row.get::<_,i64>(11)?,"cacheWriteTokens":row.get::<_,i64>(12)?,"totalTokens":row.get::<_,Option<i64>>(13)?})))?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut model_values=models.into_iter().map(|((provider,base_url,model),totals)|{let mut row=totals.to_value();row["provider"]=Value::String(provider);row["baseUrl"]=Value::String(base_url);row["model"]=Value::String(model);row}).collect::<Vec<_>>();
        model_values.sort_by(|left,right|right["totalTokens"].as_i64().cmp(&left["totalTokens"].as_i64()));
        let mut day_values=days.into_iter().map(|(day,totals)|{let mut row=totals.to_value();row["day"]=Value::String(day);row}).collect::<Vec<_>>();
        day_values.sort_by(|left,right|right["day"].as_str().cmp(&left["day"].as_str()));
        Ok(json!({"success":true,"data":{"clearedAt":cleared_at,"totals":totals.to_value(),"models":model_values,"days":day_values,"recent":recent}}))
    }).await
}

pub(super) async fn clear_ai_usage(storage: &Storage) -> Result<Value, String> {
    let cleared_at = now_ms();
    storage.call(move |connection|{
        let transaction=connection.transaction()?;
        transaction.execute("DELETE FROM ai_ocr_usage_recent",[])?;
        transaction.execute("DELETE FROM ai_ocr_usage_hourly",[])?;
        transaction.execute("DELETE FROM ai_ocr_models",[])?;
        transaction.execute("INSERT INTO ai_ocr_usage_meta(key,value) VALUES('clearedAt',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[cleared_at])?;
        transaction.commit()?;
        Ok(json!({"success":true,"data":{"clearedAt":cleared_at,"totals":empty_totals().to_value(),"models":[],"days":[],"recent":[]}}))
    }).await
}

fn read_totals(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<UsageTotals> {
    Ok(UsageTotals {
        requests: row.get(start)?,
        failures: row.get(start + 1)?,
        unknown: row.get(start + 2)?,
        input: row.get(start + 3)?,
        output: row.get(start + 4)?,
        cached: row.get(start + 5)?,
        cache_write: row.get(start + 6)?,
        total: row.get(start + 7)?,
        duration: row.get(start + 8)?,
        first: row.get(start + 9)?,
        last: row.get(start + 10)?,
    })
}
#[derive(Clone)]
struct UsageTotals {
    requests: i64,
    failures: i64,
    unknown: i64,
    input: i64,
    output: i64,
    cached: i64,
    cache_write: i64,
    total: i64,
    duration: i64,
    first: Option<i64>,
    last: Option<i64>,
}
impl UsageTotals {
    fn to_value(&self) -> Value {
        json!({"requests":self.requests,"failures":self.failures,"unknownUsage":self.unknown,"inputTokens":self.input,"outputTokens":self.output,"cachedInputTokens":self.cached,"cacheWriteTokens":self.cache_write,"totalTokens":self.total,"durationMs":self.duration,"firstUsedAt":self.first,"lastUsedAt":self.last})
    }
}
fn empty_totals() -> UsageTotals {
    UsageTotals {
        requests: 0,
        failures: 0,
        unknown: 0,
        input: 0,
        output: 0,
        cached: 0,
        cache_write: 0,
        total: 0,
        duration: 0,
        first: None,
        last: None,
    }
}
fn merge(target: &mut UsageTotals, value: &UsageTotals) {
    target.requests += value.requests;
    target.failures += value.failures;
    target.unknown += value.unknown;
    target.input += value.input;
    target.output += value.output;
    target.cached += value.cached;
    target.cache_write += value.cache_write;
    target.total += value.total;
    target.duration += value.duration;
    target.first = match (target.first, value.first) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    target.last = match (target.last, value.last) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}
fn required_string<'a>(value: &'a Value, key: &str) -> rusqlite::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn required_nonnegative(value: &Value, key: &str) -> rusqlite::Result<i64> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn optional_nonnegative(value: &Value, key: &str) -> rusqlite::Result<Option<i64>> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|value| *value >= 0)
            .map(Some)
            .ok_or(rusqlite::Error::InvalidQuery),
    }
}
fn required_timestamp(value: &Value) -> rusqlite::Result<i64> {
    value
        .as_i64()
        .filter(|value| *value >= 0 && *value <= 8_640_000_000_000_000)
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn local_hour(timestamp: i64) -> Option<i64> {
    Some(
        Local
            .timestamp_millis_opt(timestamp)
            .single()?
            .with_minute(0)?
            .with_second(0)?
            .with_nanosecond(0)?
            .timestamp_millis(),
    )
}
