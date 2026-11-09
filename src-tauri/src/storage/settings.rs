use std::collections::BTreeMap;

use rusqlite::OptionalExtension;
use serde_json::Value;

use super::{Storage, now_ms};

#[derive(Debug, Clone)]
pub struct SettingsSnapshot {
    pub values: BTreeMap<String, Value>,
    pub versions: BTreeMap<String, i64>,
}

impl Storage {
    pub async fn read_setting(&self, key: &str) -> Result<Option<Value>, String> {
        let key = key.to_owned();
        self.call(move |connection| {
            let value = connection
                .query_row(
                    "SELECT value_json FROM app_settings WHERE key=?1",
                    [key.split('.').next().unwrap_or_default()],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|json| {
                    serde_json::from_str::<Value>(&json).map_err(|_| rusqlite::Error::InvalidQuery)
                })
                .transpose()?;
            Ok(value.and_then(|mut value| {
                for field in key.split('.').skip(1) {
                    value = value.get(field)?.clone();
                }
                Some(value)
            }))
        })
        .await
    }

    pub async fn write_settings(&self, entries: &[(String, Value)]) -> Result<(), String> {
        let entries = entries.to_vec();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            for (key, value) in entries {
                if key.trim().is_empty() {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let (root, field) = split_path(&key);
                if !["settings", "sync"].contains(&root)
                    || field.is_some_and(|field| field.is_empty() || field.contains('.'))
                {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let mut current = read_root(&transaction, root)?;
                if let Some(field) = field {
                    let object = current
                        .as_object_mut()
                        .ok_or(rusqlite::Error::InvalidQuery)?;
                    object.insert(field.to_owned(), value);
                    write_root(&transaction, root, &current)?;
                    bump_version(&transaction, &key)?;
                } else if let Some(fields) = value.as_object() {
                    for field in fields.keys() {
                        bump_version(&transaction, &format!("{root}.{field}"))?;
                    }
                    write_root(&transaction, root, &value)?;
                } else {
                    write_root(&transaction, root, &value)?;
                    bump_version(&transaction, &key)?;
                }
            }
            transaction.commit()
        })
        .await
    }

    pub async fn read_settings_snapshot(&self) -> Result<SettingsSnapshot, String> {
        self.call(|connection| {
            let mut values = BTreeMap::new();
            let mut statement = connection.prepare("SELECT key,value_json FROM app_settings")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (key, value) = row?;
                let value =
                    serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery)?;
                values.insert(key, value);
            }
            let mut versions = BTreeMap::new();
            let mut statement =
                connection.prepare("SELECT key,version FROM config_field_versions")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (key, version) = row?;
                versions.insert(key, version);
            }
            Ok(SettingsSnapshot { values, versions })
        })
        .await
    }

    pub async fn write_settings_versioned(
        &self,
        entries: &[(String, Value)],
        expected: &BTreeMap<String, i64>,
    ) -> Result<BTreeMap<String, i64>, String> {
        let entries = entries.to_vec();
        let expected = expected.clone();
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let mut next_versions = BTreeMap::new();
            for (key, value) in &entries {
                let (root, field) = split_path(key);
                let field = field.ok_or(rusqlite::Error::InvalidQuery)?;
                if !["settings", "sync"].contains(&root) || field.is_empty() || field.contains('.') {
                    return Err(rusqlite::Error::InvalidQuery);
                }
                let expected_version = expected.get(key).copied().ok_or(rusqlite::Error::InvalidQuery)?;
                let current_version = transaction
                    .query_row(
                        "SELECT version FROM config_field_versions WHERE key=?1",
                        [key],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                if current_version != expected_version {
                    return Err(rusqlite::Error::QueryReturnedNoRows);
                }
                let mut root_value = read_root(&transaction, root)?;
                root_value
                    .as_object_mut()
                    .ok_or(rusqlite::Error::InvalidQuery)?
                    .insert(field.to_owned(), value.clone());
                write_root(&transaction, root, &root_value)?;
                let next = current_version + 1;
                transaction.execute(
                    "INSERT INTO config_field_versions(key,version) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET version=excluded.version",
                    rusqlite::params![key, next],
                )?;
                next_versions.insert(key.clone(), next);
            }
            transaction.commit()?;
            Ok(next_versions)
        })
        .await
        .map_err(|error| {
            if error == rusqlite::Error::QueryReturnedNoRows.to_string() {
                "配置冲突：提交期间数据已变化".to_owned()
            } else {
                error
            }
        })
    }

    pub async fn read_meta(&self, key: &str) -> Result<Option<Value>, String> {
        let key = key.to_owned();
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT value_json FROM app_meta WHERE key=?1",
                    [key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|value| {
                    serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery)
                })
                .transpose()
        })
        .await
    }

    pub async fn write_meta(&self, key: &str, value: &Value) -> Result<(), String> {
        let (key, value) = (key.to_owned(), value.clone());
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO app_meta(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
                rusqlite::params![key,serde_json::to_string(&value).map_err(|_|rusqlite::Error::InvalidQuery)?],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn compare_and_set_meta(
        &self,
        key: &str,
        expected: Option<&Value>,
        next: &Value,
    ) -> Result<bool, String> {
        let (key, expected, next) = (key.to_owned(), expected.cloned(), next.clone());
        self.call(move |connection| {
            let transaction = connection.transaction()?;
            let existing = transaction
                .query_row("SELECT value_json FROM app_meta WHERE key=?1", [&key], |row| row.get::<_, String>(0))
                .optional()?
                .map(|value| serde_json::from_str::<Value>(&value).map_err(|_| rusqlite::Error::InvalidQuery))
                .transpose()?;
            if existing != expected {
                return Ok(false);
            }
            transaction.execute(
                "INSERT INTO app_meta(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
                rusqlite::params![key,serde_json::to_string(&next).map_err(|_|rusqlite::Error::InvalidQuery)?],
            )?;
            transaction.commit()?;
            Ok(true)
        })
        .await
    }
}

fn split_path(key: &str) -> (&str, Option<&str>) {
    match key.split_once('.') {
        Some((root, field)) => (root, Some(field)),
        None => (key, None),
    }
}

fn read_root(connection: &rusqlite::Connection, root: &str) -> rusqlite::Result<Value> {
    connection
        .query_row(
            "SELECT value_json FROM app_settings WHERE key=?1",
            [root],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value).map_err(|_| rusqlite::Error::InvalidQuery))
        .transpose()
        .map(|value| value.unwrap_or_else(|| Value::Object(serde_json::Map::new())))
}

fn write_root(
    connection: &rusqlite::Connection,
    root: &str,
    value: &Value,
) -> rusqlite::Result<()> {
    if !credentials_are_references(root, value) {
        return Err(rusqlite::Error::InvalidQuery);
    }
    connection.execute(
        "INSERT INTO app_settings(key,value_json,updated_at) VALUES(?1,?2,?3) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json,updated_at=excluded.updated_at",
        rusqlite::params![root,serde_json::to_string(value).map_err(|_|rusqlite::Error::InvalidQuery)?,now_ms()],
    )?;
    Ok(())
}

fn credentials_are_references(root: &str, value: &Value) -> bool {
    let paths: &[&str] = match root {
        "settings" => &["/ocrAIConfigs/openai/apiKey", "/ocrAIConfigs/claude/apiKey"],
        "sync" => &["/params/webdav/password"],
        _ => return false,
    };
    paths.iter().all(|path| match value.pointer(path) {
        None | Some(Value::Null) => true,
        Some(Value::String(secret)) => {
            secret.is_empty() || crate::credentials::is_credential_reference(secret)
        }
        Some(_) => false,
    })
}

fn bump_version(connection: &rusqlite::Connection, key: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO config_field_versions(key,version) VALUES(?1,1) ON CONFLICT(key) DO UPDATE SET version=version+1",
        [key],
    )?;
    Ok(())
}
