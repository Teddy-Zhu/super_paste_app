use std::collections::BTreeMap;
use rusqlite::{OptionalExtension, params};
use super_paste_format::*;
use tokio_util::sync::CancellationToken;
use crate::{Error, Result};
use super::{Storage, statistics::{period_key, period_start, period_next, validate_timestamp}};

const FIELDS: &str = "SUM(requests),SUM(failures),SUM(unknown_usage),SUM(input_tokens),SUM(output_tokens),SUM(cached_input_tokens),SUM(cache_write_tokens),SUM(total_tokens),SUM(duration_ms),MIN(first_used_at),MAX(last_used_at)";
impl Storage {
    pub async fn record_ai_usage(&self, entry: AiUsageEntry) -> Result<bool> {
        validate_timestamp(entry.started_at)?;
        validate_timestamp(entry.finished_at)?;
        let tokens = &entry.tokens;
        if entry.model.is_empty() || entry.base_url.is_empty() || entry.finished_at < entry.started_at ||
            [Some(entry.duration_ms), tokens.input_tokens, tokens.output_tokens, Some(tokens.cached_input_tokens), Some(tokens.cache_write_tokens), tokens.total_tokens]
                .into_iter().flatten().any(|value| value > i64::MAX as u64) {
            return Err(Error::Invalid("AI OCR 用量记录无效".into()));
        }
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let cleared: Option<i64> = transaction.query_row("SELECT value FROM ai_ocr_usage_meta WHERE key='clearedAt'", [], |row| row.get(0)).optional()?;
            if cleared.is_some_and(|cleared| entry.started_at <= cleared) { return Ok(false); }
            let tokens = &entry.tokens;
            let inserted = transaction.execute("INSERT OR IGNORE INTO ai_ocr_usage_recent(id,provider,base_url,model,purpose,started_at,finished_at,duration_ms,success,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![entry.id.to_string(), entry.provider.as_str(), entry.base_url, entry.model, entry.purpose.as_str(), entry.started_at, entry.finished_at, entry.duration_ms, entry.success, tokens.input_tokens, tokens.output_tokens, tokens.cached_input_tokens, tokens.cache_write_tokens, tokens.total_tokens])?;
            if inserted == 0 { return Ok(false); }
            transaction.execute("INSERT OR IGNORE INTO ai_ocr_models(provider,base_url,model) VALUES(?1,?2,?3)", params![entry.provider.as_str(), entry.base_url, entry.model])?;
            let model: i64 = transaction.query_row("SELECT id FROM ai_ocr_models WHERE provider=?1 AND base_url=?2 AND model=?3", params![entry.provider.as_str(), entry.base_url, entry.model], |row| row.get(0))?;
            let hour = period_start(entry.started_at, StatisticsInterval::Hour)?;
            transaction.execute("INSERT INTO ai_ocr_usage_hourly(hour,model_id,requests,failures,unknown_usage,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens,duration_ms,first_used_at,last_used_at) VALUES(?1,?2,1,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12) ON CONFLICT(hour,model_id) DO UPDATE SET requests=requests+1,failures=failures+excluded.failures,unknown_usage=unknown_usage+excluded.unknown_usage,input_tokens=input_tokens+excluded.input_tokens,output_tokens=output_tokens+excluded.output_tokens,cached_input_tokens=cached_input_tokens+excluded.cached_input_tokens,cache_write_tokens=cache_write_tokens+excluded.cache_write_tokens,total_tokens=total_tokens+excluded.total_tokens,duration_ms=duration_ms+excluded.duration_ms,first_used_at=MIN(first_used_at,excluded.first_used_at),last_used_at=MAX(last_used_at,excluded.last_used_at)",
                params![hour, model, !entry.success, tokens.total_tokens.is_none(), tokens.input_tokens.unwrap_or(0), tokens.output_tokens.unwrap_or(0), tokens.cached_input_tokens, tokens.cache_write_tokens, tokens.total_tokens.unwrap_or(0), entry.duration_ms, entry.started_at, entry.finished_at])?;
            transaction.execute("DELETE FROM ai_ocr_usage_recent WHERE id NOT IN (SELECT id FROM ai_ocr_usage_recent ORDER BY started_at DESC,id DESC LIMIT 50)", [])?;
            transaction.commit()?;
            Ok(true)
        }).await
    }
    pub async fn ai_usage(&self, query: AiUsageQuery, cancel: &CancellationToken) -> Result<AiUsageStats> {
        if cancel.is_cancelled() { return Err(Error::Cancelled); }
        let from = query.from.unwrap_or(0);
        let to = query.to.unwrap_or(i64::MAX);
        validate_timestamp(from)?;
        if to != i64::MAX { validate_timestamp(to)?; }
        if from > to { return Err(Error::Invalid("统计日期范围无效".into())); }
        let from = period_start(from, StatisticsInterval::Hour)?;
        let to = if to == i64::MAX { to } else { period_next(period_start(to, StatisticsInterval::Hour)?, StatisticsInterval::Hour)? };
        let requested = cancel.clone();
        self.call_read(cancel, move |connection| {
            let cleared_at = connection.query_row("SELECT value FROM ai_ocr_usage_meta WHERE key='clearedAt'", [], |row| row.get(0)).optional()?;
            let mut totals = AiUsageTotals::default();
            let mut models = BTreeMap::<(String, String, String), AiUsageTotals>::new();
            let mut days = BTreeMap::<String, AiUsageTotals>::new();
            {
                let mut statement = connection.prepare(&format!("SELECT hour,provider,base_url,model,{FIELDS} FROM ai_ocr_usage_hourly h JOIN ai_ocr_models m ON m.id=h.model_id WHERE hour>=?1 AND hour<?2 GROUP BY hour,model_id ORDER BY hour"))?;
                let rows = statement.query_map(params![from, to], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, read_totals(row, 4)?)))?;
                for row in rows {
                    if requested.is_cancelled() { return Err(Error::Cancelled); }
                    let (hour, provider, base_url, model, value) = row?;
                    totals.merge(&value)?;
                    models.entry((provider, base_url, model)).or_default().merge(&value)?;
                    days.entry(period_key(hour, StatisticsInterval::Day)?).or_default().merge(&value)?;
                }
            }
            let recent = {
                let mut statement = connection.prepare("SELECT id,provider,base_url,model,purpose,started_at,finished_at,duration_ms,success,input_tokens,output_tokens,cached_input_tokens,cache_write_tokens,total_tokens FROM ai_ocr_usage_recent WHERE started_at>=?1 AND started_at<?2 ORDER BY started_at DESC,id DESC LIMIT 50")?;
                let mut rows = statement.query(params![from, to])?;
                let mut recent = Vec::new();
                while let Some(row) = rows.next()? {
                    if requested.is_cancelled() { return Err(Error::Cancelled); }
                    let id: String = row.get(0)?;
                    recent.push(AiUsageEntry { id: uuid::Uuid::parse_str(&id).map_err(|_| Error::Database("AI 用量记录 ID 无效".into()))?,
                        provider: AiProvider::parse(&row.get::<_, String>(1)?)?, base_url: row.get(2)?, model: row.get(3)?, purpose: AiPurpose::parse(&row.get::<_, String>(4)?)?,
                        started_at: row.get(5)?, finished_at: row.get(6)?, duration_ms: row.get(7)?, success: row.get(8)?,
                        tokens: AiTokens { input_tokens: row.get(9)?, output_tokens: row.get(10)?, cached_input_tokens: row.get(11)?, cache_write_tokens: row.get(12)?, total_tokens: row.get(13)? } });
                }
                recent
            };
            let mut models = models.into_iter().map(|((provider, base_url, model), totals)| Ok(AiModelUsage { provider: AiProvider::parse(&provider)?, base_url, model, totals })).collect::<Result<Vec<_>>>()?;
            models.sort_by(|a, b| b.totals.total_tokens.cmp(&a.totals.total_tokens).then_with(|| (&a.provider, &a.base_url, &a.model).cmp(&(&b.provider, &b.base_url, &b.model))));
            let days = days.into_iter().rev().map(|(day, totals)| AiDailyUsage { day, totals }).collect();
            Ok(AiUsageStats { cleared_at, totals, models, days, recent })
        }).await
    }
    pub async fn clear_ai_usage(&self, timestamp: i64) -> Result<()> {
        validate_timestamp(timestamp)?;
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute("DELETE FROM ai_ocr_usage_recent", [])?;
            transaction.execute("DELETE FROM ai_ocr_usage_hourly", [])?;
            transaction.execute("DELETE FROM ai_ocr_models", [])?;
            transaction.execute("INSERT INTO ai_ocr_usage_meta(key,value) VALUES('clearedAt',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [timestamp])?;
            transaction.commit()?;
            Ok(())
        }).await
    }
}
fn read_totals(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<AiUsageTotals> {
    Ok(AiUsageTotals { requests: row.get(offset)?, failures: row.get(offset + 1)?, unknown_usage: row.get(offset + 2)?, input_tokens: row.get(offset + 3)?, output_tokens: row.get(offset + 4)?, cached_input_tokens: row.get(offset + 5)?, cache_write_tokens: row.get(offset + 6)?, total_tokens: row.get(offset + 7)?, duration_ms: row.get(offset + 8)?, first_used_at: row.get(offset + 9)?, last_used_at: row.get(offset + 10)? })
}
