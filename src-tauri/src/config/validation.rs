use serde_json::Value;

pub(crate) fn validate_patch(store_id: &str, patch: &Value) -> Result<Vec<String>, String> {
    if store_id != "settings" && store_id != "sync" {
        return Err("配置分组无效".to_owned());
    }
    let patch = patch
        .as_object()
        .ok_or_else(|| "配置变更必须是对象".to_owned())?;
    if patch.is_empty() {
        return Ok(Vec::new());
    }
    let defaults = data_format::default_preferences();
    let schema = defaults
        .get(store_id)
        .and_then(Value::as_object)
        .ok_or_else(|| "配置默认结构缺失".to_owned())?;
    for (field, value) in patch {
        let field_schema = schema
            .get(field)
            .ok_or_else(|| format!("配置字段无效: {field}"))?;
        validate_partial_shape(value, field_schema, field)?;
    }
    Ok(patch.keys().cloned().collect())
}

pub(crate) fn validate_roots(settings: &Value, sync: &Value) -> Result<(), String> {
    let defaults = data_format::default_preferences();
    validate_shape(
        settings,
        defaults
            .get("settings")
            .ok_or_else(|| "配置默认结构缺失: settings".to_owned())?,
        "settings",
    )?;
    validate_shape(
        sync,
        defaults
            .get("sync")
            .ok_or_else(|| "配置默认结构缺失: sync".to_owned())?,
        "sync",
    )?;

    validate_settings(settings)?;
    validate_sync(sync)
}

fn validate_shape(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    match schema {
        Value::Object(schema_fields) => {
            let fields = value
                .as_object()
                .ok_or_else(|| format!("配置字段类型无效: {path}"))?;
            if fields.len() != schema_fields.len()
                || schema_fields.keys().any(|key| !fields.contains_key(key))
            {
                return Err(format!("配置字段结构无效: {path}"));
            }
            for (key, field_schema) in schema_fields {
                validate_shape(
                    fields
                        .get(key)
                        .ok_or_else(|| format!("配置字段缺失: {path}.{key}"))?,
                    field_schema,
                    &format!("{path}.{key}"),
                )?;
            }
            Ok(())
        }
        Value::Array(_) => {
            if value.is_array() {
                Ok(())
            } else {
                Err(format!("配置字段类型无效: {path}"))
            }
        }
        Value::String(_) if value.is_string() => Ok(()),
        Value::Bool(_) if value.is_boolean() => Ok(()),
        Value::Number(_) if value.as_f64().is_some_and(f64::is_finite) => Ok(()),
        _ => Err(format!("配置字段类型无效: {path}")),
    }
}

fn validate_partial_shape(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    match schema {
        Value::Object(schema_fields) => {
            let fields = value
                .as_object()
                .ok_or_else(|| format!("配置字段类型无效: {path}"))?;
            for (key, value) in fields {
                let field_schema = schema_fields
                    .get(key)
                    .ok_or_else(|| format!("配置字段无效: {path}.{key}"))?;
                validate_partial_shape(value, field_schema, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
        _ => validate_shape(value, schema, path),
    }
}

fn validate_settings(settings: &Value) -> Result<(), String> {
    let string = |field: &str| {
        settings
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("配置字段类型无效: settings.{field}"))
    };
    let bool_value = |field: &str| {
        settings
            .get(field)
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("配置字段类型无效: settings.{field}"))
    };

    if !matches!(string("colorMode")?, "system" | "light" | "dark") {
        return Err("颜色模式无效".to_owned());
    }
    if !matches!(
        string("overlayPosition")?,
        "top" | "bottom" | "left" | "right"
    ) {
        return Err("浮层位置无效".to_owned());
    }
    let ocr_mode = string("ocrMode")?;
    if !matches!(ocr_mode, "local" | "remote" | "openai" | "claude") {
        return Err("OCR 模式无效".to_owned());
    }
    if !matches!(string("ocrRemoteRequestFormat")?, "json" | "form") {
        return Err("远程 OCR 请求格式无效".to_owned());
    }
    for field in ["maxHistorySize", "maxKeepTime"] {
        let value = settings
            .get(field)
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("配置字段类型无效: settings.{field}"))?;
        if value < -1 {
            return Err(format!("配置字段范围无效: settings.{field}"));
        }
    }

    let local_ocr_enabled = bool_value("enableLocalOCR")?;
    let remote_url = string("ocrRemoteUrl")?.trim();
    if ocr_mode == "remote" {
        let params = string("ocrRemoteExtraParams")?;
        let parsed: Value = serde_json::from_str(params)
            .map_err(|_| "远程 OCR 额外参数必须是 JSON 对象".to_owned())?;
        if !parsed.is_object() {
            return Err("远程 OCR 额外参数必须是 JSON 对象".to_owned());
        }
        if remote_url.is_empty() && local_ocr_enabled {
            return Err("远程 OCR 地址不能为空".to_owned());
        }
        if !remote_url.is_empty() {
            let url =
                reqwest::Url::parse(remote_url).map_err(|_| "远程 OCR 地址无效".to_owned())?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err("远程 OCR 地址必须使用 HTTP 或 HTTPS".to_owned());
            }
        }
    }

    let configs = settings
        .get("ocrAIConfigs")
        .and_then(Value::as_object)
        .ok_or_else(|| "AI OCR 配置无效".to_owned())?;
    for provider in ["openai", "claude"] {
        let config = configs
            .get(provider)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("AI OCR 配置无效: {provider}"))?;
        let base_url = config
            .get("baseUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("AI OCR Base URL 无效: {provider}"))?
            .trim();
        config
            .get("apiKey")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("AI OCR API Key 无效: {provider}"))?;
        let _model = config
            .get("model")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("AI OCR 模型无效: {provider}"))?
            .trim();
        let api = config
            .get("api")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("AI OCR API 类型无效: {provider}"))?;
        if provider == "openai" && !matches!(api, "chat" | "responses") {
            return Err("OpenAI API 类型无效".to_owned());
        }
        if provider == "claude" && api != "chat" {
            return Err("Claude API 类型无效".to_owned());
        }
        if !base_url.is_empty() {
            let url = reqwest::Url::parse(base_url)
                .map_err(|_| format!("AI OCR Base URL 格式无效: {provider}"))?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(format!("AI OCR Base URL 无效: {provider}"));
            }
        }
    }
    Ok(())
}

fn validate_sync(sync: &Value) -> Result<(), String> {
    let direction = sync
        .get("direction")
        .and_then(Value::as_str)
        .ok_or_else(|| "同步方向无效".to_owned())?;
    if !matches!(direction, "upload" | "download" | "bidirectional") {
        return Err("同步方向无效".to_owned());
    }
    let interval = sync
        .get("interval")
        .and_then(Value::as_i64)
        .ok_or_else(|| "同步间隔无效".to_owned())?;
    if interval <= 0 {
        return Err("同步间隔必须大于零".to_owned());
    }
    let webdav = sync
        .pointer("/params/webdav")
        .and_then(Value::as_object)
        .ok_or_else(|| "WebDAV 配置无效".to_owned())?;
    let server_url = webdav
        .get("serverUrl")
        .and_then(Value::as_str)
        .ok_or_else(|| "WebDAV 地址无效".to_owned())?
        .trim();
    if !server_url.is_empty() {
        let url = reqwest::Url::parse(server_url).map_err(|_| "WebDAV 地址无效".to_owned())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("WebDAV 地址必须是无凭据的 HTTP 或 HTTPS URL".to_owned());
        }
    }
    let timeout = webdav
        .get("timeout")
        .and_then(Value::as_i64)
        .ok_or_else(|| "WebDAV 超时设置无效".to_owned())?;
    if timeout <= 0 {
        return Err("WebDAV 超时设置必须大于零".to_owned());
    }
    Ok(())
}
