CREATE TABLE app_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) WITHOUT ROWID;
CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value_json TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision >= 0)) WITHOUT ROWID;
CREATE TABLE config_field_versions (key TEXT PRIMARY KEY NOT NULL, revision INTEGER NOT NULL CHECK(revision >= 0)) WITHOUT ROWID;
CREATE TABLE credential_refs (key TEXT PRIMARY KEY NOT NULL, reference TEXT NOT NULL UNIQUE) WITHOUT ROWID;
-- Contains only non-secret references. Incomplete staging and retired keys survive process exit.
CREATE TABLE credential_garbage (reference TEXT PRIMARY KEY NOT NULL) WITHOUT ROWID;
CREATE TABLE sync_targets (
    target BLOB PRIMARY KEY NOT NULL CHECK(length(target)=32),
    conditionals INTEGER CHECK(conditionals IN (0,1)),
    baseline_revision TEXT, baseline_digest BLOB CHECK(baseline_digest IS NULL OR length(baseline_digest)=32), baseline_etag TEXT,
    pending_document BLOB CHECK(pending_document IS NULL OR length(pending_document)<=1048576),
    conflict_digest BLOB CHECK(conflict_digest IS NULL OR length(conflict_digest)=32),
    last_success INTEGER NOT NULL DEFAULT 0 CHECK(last_success>=0),
    CHECK((baseline_revision IS NULL)=(baseline_digest IS NULL))
) WITHOUT ROWID;
CREATE TABLE sources (
    id TEXT PRIMARY KEY NOT NULL, application TEXT NOT NULL, path TEXT NOT NULL, icon BLOB,
    UNIQUE(application, path)
) WITHOUT ROWID;
CREATE TABLE images (
    id BLOB PRIMARY KEY NOT NULL CHECK(length(id) = 32), format TEXT NOT NULL,
    width INTEGER, height INTEGER, encoded_bytes INTEGER NOT NULL CHECK(encoded_bytes > 0),
    data BLOB NOT NULL CHECK(length(data) = encoded_bytes)
) WITHOUT ROWID;
CREATE TABLE history (
    id TEXT PRIMARY KEY NOT NULL, digest BLOB NOT NULL UNIQUE CHECK(length(digest) = 32),
    primary_type TEXT NOT NULL, formats INTEGER NOT NULL,
    timestamp INTEGER NOT NULL, last_used INTEGER NOT NULL,
    content_revision INTEGER NOT NULL CHECK(content_revision > 0),
    metadata_revision INTEGER NOT NULL CHECK(metadata_revision > 0),
    ocr_revision INTEGER NOT NULL DEFAULT 0 CHECK(ocr_revision >= 0),
    ocr_generation INTEGER NOT NULL DEFAULT 0 CHECK(ocr_generation >= 0),
    source_id TEXT REFERENCES sources(id) ON DELETE SET NULL, image_id BLOB REFERENCES images(id),
    text_characters INTEGER NOT NULL DEFAULT 0, files_count INTEGER NOT NULL DEFAULT 0,
    text_excerpt TEXT, html_excerpt TEXT, rtf_excerpt TEXT,
    truncated INTEGER NOT NULL DEFAULT 0,
    ocr_has_text INTEGER NOT NULL DEFAULT 0 CHECK(ocr_has_text IN (0,1))
) WITHOUT ROWID;
CREATE INDEX history_last_used ON history(last_used DESC, id DESC);
CREATE INDEX history_timestamp ON history(timestamp DESC, id DESC);
CREATE INDEX history_image ON history(image_id);
CREATE TABLE payloads (
    record_id TEXT PRIMARY KEY NOT NULL REFERENCES history(id) ON DELETE CASCADE,
    text TEXT, html TEXT, rtf TEXT, ocr TEXT
) WITHOUT ROWID;
CREATE TABLE files (
    record_id TEXT NOT NULL REFERENCES history(id) ON DELETE CASCADE,
    position INTEGER NOT NULL, path TEXT NOT NULL, PRIMARY KEY(record_id, position)
) WITHOUT ROWID;
CREATE TABLE tags (
    id TEXT PRIMARY KEY NOT NULL, name TEXT NOT NULL UNIQUE, color TEXT NOT NULL,
    description TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE record_tags (
    record_id TEXT NOT NULL REFERENCES history(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL, PRIMARY KEY(record_id, tag_id)
) WITHOUT ROWID;
CREATE INDEX record_tags_by_tag ON record_tags(tag_id, record_id);
CREATE TABLE copy_statistics (
    hour INTEGER NOT NULL, type TEXT NOT NULL, source TEXT NOT NULL,
    copies INTEGER NOT NULL, bytes INTEGER NOT NULL, files INTEGER NOT NULL,
    first_copied_at INTEGER NOT NULL, last_copied_at INTEGER NOT NULL,
    PRIMARY KEY(hour, type, source)
) WITHOUT ROWID;
CREATE TABLE copy_statistics_meta (key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL) WITHOUT ROWID;
CREATE TABLE ai_ocr_models (
    id INTEGER PRIMARY KEY, provider TEXT NOT NULL, base_url TEXT NOT NULL, model TEXT NOT NULL,
    UNIQUE(provider, base_url, model)
);
CREATE TABLE ai_ocr_usage_hourly (
    hour INTEGER NOT NULL, model_id INTEGER NOT NULL REFERENCES ai_ocr_models(id) ON DELETE CASCADE,
    requests INTEGER NOT NULL, failures INTEGER NOT NULL, unknown_usage INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL,
    cached_input_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL,
    total_tokens INTEGER NOT NULL, duration_ms INTEGER NOT NULL,
    first_used_at INTEGER NOT NULL, last_used_at INTEGER NOT NULL, PRIMARY KEY(hour, model_id),
    CHECK(typeof(requests)='integer' AND typeof(failures)='integer' AND typeof(unknown_usage)='integer'
        AND typeof(input_tokens)='integer' AND typeof(output_tokens)='integer' AND typeof(cached_input_tokens)='integer'
        AND typeof(cache_write_tokens)='integer' AND typeof(total_tokens)='integer' AND typeof(duration_ms)='integer')
) WITHOUT ROWID;
CREATE TABLE ai_ocr_usage_recent (
    id TEXT PRIMARY KEY NOT NULL, provider TEXT NOT NULL, base_url TEXT NOT NULL, model TEXT NOT NULL,
    purpose TEXT NOT NULL, started_at INTEGER NOT NULL, finished_at INTEGER NOT NULL,
    duration_ms INTEGER NOT NULL, success INTEGER NOT NULL, input_tokens INTEGER, output_tokens INTEGER,
    cached_input_tokens INTEGER NOT NULL, cache_write_tokens INTEGER NOT NULL, total_tokens INTEGER
) WITHOUT ROWID;
CREATE INDEX ai_ocr_usage_recent_started ON ai_ocr_usage_recent(started_at DESC);
CREATE TABLE ai_ocr_usage_meta (key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL) WITHOUT ROWID;
