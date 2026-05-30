pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

pub fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "Invalid timestamp".to_string())
}
