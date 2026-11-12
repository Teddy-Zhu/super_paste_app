use super::{Storage, now_ms};
use rusqlite::params;
use serde_json::{Value, json};

pub(super) async fn get_tagged_record_ids(storage: &Storage) -> Result<Vec<String>, String> {
    storage
        .call(|connection| {
            let mut statement =
                connection.prepare("SELECT DISTINCT content_id FROM clipboard_history_tags")?;
            let rows = statement.query_map([], |row| row.get(0))?;
            rows.collect()
        })
        .await
}

pub(super) async fn get_all_tags(storage: &Storage) -> Result<Vec<Value>, String> {
    storage.call(|connection| {
        let mut statement = connection.prepare("SELECT id,name,color,description,created_at,updated_at FROM tag_groups ORDER BY created_at DESC")?;
        let rows=statement.query_map([],|row|Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"color":row.get::<_,String>(2)?,"description":row.get::<_,Option<String>>(3)?,"createdAt":row.get::<_,i64>(4)?,"updatedAt":row.get::<_,i64>(5)?})))?;
        rows.collect()
    }).await
}

pub(super) async fn create_tag(storage: &Storage, value: &Value) -> Result<String, String> {
    let value = value.clone();
    let result = storage.call(move |connection| {
        let result: rusqlite::Result<String> = (|| {
        let id=value.get("id").and_then(Value::as_str).filter(|value| !value.is_empty()).map(str::to_owned).unwrap_or_else(||uuid::Uuid::new_v4().to_string());
        let name=value.get("name").and_then(Value::as_str).filter(|value| !value.trim().is_empty()).ok_or(rusqlite::Error::InvalidQuery)?;
        let color=value.get("color").and_then(Value::as_str).ok_or(rusqlite::Error::InvalidQuery)?;
        let description=value.get("description").and_then(Value::as_str);
        let now=now_ms();
        connection.execute("INSERT INTO tag_groups(id,name,color,description,created_at,updated_at) VALUES(?1,?2,?3,?4,?5,?6)",params![id,name,color,description,value.get("createdAt").and_then(Value::as_i64).unwrap_or(now),value.get("updatedAt").and_then(Value::as_i64).unwrap_or(now)])?;
        Ok(id)
        })();
        Ok(result)
    }).await.map_err(|error| {
        tracing::error!(error = %error, "failed to create tag");
        format!("创建标签失败: {error}")
    })?;
    result.map_err(|error| {
        if matches!(
            &error,
            rusqlite::Error::SqliteFailure(code, Some(message))
                if code.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                    && message == "UNIQUE constraint failed: tag_groups.name"
        ) {
            return "标签名称已存在".to_owned();
        }
        tracing::error!(error = %error, "failed to create tag");
        format!("创建标签失败: {error}")
    })
}

pub(super) async fn update_tag(
    storage: &Storage,
    tag_id: &str,
    updates: &Value,
) -> Result<bool, String> {
    let (tag_id, updates) = (tag_id.to_owned(), updates.clone());
    storage.call(move |connection| {
        let name=updates.get("name").and_then(Value::as_str);
        let color=updates.get("color").and_then(Value::as_str);
        let description=updates.get("description").and_then(Value::as_str);
        if name.is_none() && color.is_none() && description.is_none() { return Ok(false); }
        Ok(connection.execute("UPDATE tag_groups SET name=COALESCE(?1,name),color=COALESCE(?2,color),description=CASE WHEN ?3 THEN ?4 ELSE description END,updated_at=?5 WHERE id=?6",params![name,color,updates.get("description").is_some(),description,now_ms(),tag_id])?>0)
    }).await
}

pub(super) async fn delete_tag(storage: &Storage, tag_id: &str) -> Result<bool, String> {
    let tag_id = tag_id.to_owned();
    storage
        .call(move |connection| {
            Ok(connection.execute("DELETE FROM tag_groups WHERE id=?1", [tag_id])? > 0)
        })
        .await
}

pub(super) async fn get_tag_stats(storage: &Storage) -> Result<Vec<Value>, String> {
    storage.call(|connection| {
        let mut statement=connection.prepare("SELECT tg.id,tg.name,COUNT(cht.content_id) FROM tag_groups tg LEFT JOIN clipboard_history_tags cht ON tg.id=cht.tag_id GROUP BY tg.id,tg.name ORDER BY COUNT(cht.content_id) DESC,tg.name ASC")?;
        let rows=statement.query_map([],|row|Ok(json!({"tagId":row.get::<_,String>(0)?,"tagName":row.get::<_,String>(1)?,"recordCount":row.get::<_,i64>(2)?})))?;
        rows.collect()
    }).await
}

pub(super) async fn bind_tag(
    storage: &Storage,
    content_id: &str,
    tag_id: &str,
) -> Result<bool, String> {
    let (content_id, tag_id) = (content_id.to_owned(), tag_id.to_owned());
    storage.call(move |connection|Ok(connection.execute("INSERT OR IGNORE INTO clipboard_history_tags(content_id,tag_id,created_at) VALUES(?1,?2,?3)",params![content_id,tag_id,now_ms()])?>0)).await
}

pub(super) async fn unbind_tag(
    storage: &Storage,
    content_id: &str,
    tag_id: &str,
) -> Result<bool, String> {
    let (content_id, tag_id) = (content_id.to_owned(), tag_id.to_owned());
    storage.call(move |connection|{
        let transaction=connection.transaction()?;
        let changed=transaction.execute("DELETE FROM clipboard_history_tags WHERE content_id=?1 AND tag_id=?2",params![content_id,tag_id])?>0;
        if changed {
            let has_tags:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM clipboard_history_tags WHERE content_id=?1)",[&content_id],|row|row.get(0))?;
            if !has_tags { let now=now_ms(); transaction.execute("UPDATE clipboard_history SET timestamp=?1,last_used=?1,updated_at=?1 WHERE content_id=?2",params![now,content_id])?; }
        }
        transaction.commit()?;
        Ok(changed)
    }).await
}
pub(super) fn record_tags_sync(
    connection: &rusqlite::Connection,
    id: &str,
) -> rusqlite::Result<Vec<Value>> {
    let mut statement = connection.prepare("SELECT tg.id,tg.name,tg.color,tg.description,tg.created_at,tg.updated_at FROM tag_groups tg JOIN clipboard_history_tags cht ON cht.tag_id=tg.id WHERE cht.content_id=?1 ORDER BY cht.created_at ASC")?;
    let rows = statement.query_map([id], |row| {
        Ok(json!({"id":row.get::<_,String>(0)?,"name":row.get::<_,String>(1)?,"color":row.get::<_,String>(2)?,"description":row.get::<_,Option<String>>(3)?,"createdAt":row.get::<_,i64>(4)?,"updatedAt":row.get::<_,i64>(5)?}))
    })?;
    rows.collect()
}

pub(super) async fn get_record_tags(storage: &Storage, id: &str) -> Result<Vec<Value>, String> {
    let id = id.to_owned();
    storage
        .call(move |connection| record_tags_sync(connection, &id))
        .await
}
