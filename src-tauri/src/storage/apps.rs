use super::{Storage, now_ms};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

pub(super) async fn upsert_app(storage: &Storage, value: &Value) -> Result<String, String> {
    let value = value.clone();
    storage
        .call(move |connection| {
            let application = value.get("application").and_then(Value::as_str).ok_or(rusqlite::Error::InvalidQuery)?;
            let path = value.get("path").and_then(Value::as_str).ok_or(rusqlite::Error::InvalidQuery)?;
            let icon = value.get("icon").and_then(Value::as_str);
            let now = now_ms();
            if let Some((id, old_icon)) = connection
                .query_row(
                    "SELECT id,icon FROM apps WHERE application=?1 AND path=?2",
                    params![application,path],
                    |row| Ok((row.get::<_,String>(0)?,row.get::<_,Option<String>>(1)?)),
                )
                .optional()?
            {
                if icon.is_some_and(|icon| crate::platform::source_icon::should_replace(old_icon.as_deref(), icon)) && icon != old_icon.as_deref() {
                    connection.execute("UPDATE apps SET icon=?1,updated_at=?2 WHERE id=?3",params![icon,now,id])?;
                }
                return Ok(id);
            }
            let id = value.get("id").and_then(Value::as_str).filter(|id| !id.is_empty()).map(str::to_owned).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let created_at=value.get("createdAt").and_then(Value::as_i64).unwrap_or(now);
            let updated_at=value.get("updatedAt").and_then(Value::as_i64).unwrap_or(now);
            connection.execute("INSERT INTO apps(id,application,path,icon,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,application,path,icon,created_at,updated_at])?;
            Ok(id)
        })
        .await
}
