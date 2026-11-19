use std::{path::PathBuf, sync::Arc};

use image::ImageEncoder;
use super_paste_format::{Digest, Formats, HistoryLimits, HistoryQuery, RecordId, TextContent};
use tokio_util::sync::CancellationToken;

use super::{PreparedRecord, Storage};
use crate::{Error, clipboard::RestoreMode, images::{ImageBudgets, ImageService, ImageSource}, preview::PreviewScope, resources::ByteBudget};

struct DatabaseDirectory(PathBuf);

#[tokio::test]
async fn history_reuses_unchanged_rows_and_reconciles_revisions_and_boundaries() {
    use std::collections::HashMap;
    use super_paste_format::{HistoryEntry, HistoryPage, SourceApp, SourceId};
    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("incremental-history.sqlite3")).await.expect("isolated database");
    let cancel = CancellationToken::new();
    let limits = HistoryLimits { max_records: -1, keep_days: -1 };
    let source = Arc::new(SourceApp { id: SourceId::new(), application: "source".into(), path: "/source".into(), icon: Some(bytes::Bytes::from_static(b"first icon")) });
    let mut ids = Vec::new();
    for index in 0..3u8 {
        let (record, _) = storage.upsert_record(PreparedRecord {
            digest: Digest([index; 32]), content: TextContent { text: Some(format!("entry-{index}").into()), ..Default::default() },
            image: None, timestamp: 100 * (i64::from(index) + 1), source: Some(source.clone()),
        }, limits).await.expect("seed history");
        ids.push(record.id);
    }
    let query = HistoryQuery { limit: 2, ..Default::default() };
    let known = |page: &HistoryPage| Arc::new(page.entries.iter().map(|entry| (entry.id, entry.clone())).collect::<HashMap<RecordId, Arc<HistoryEntry>>>());
    let first = storage.history(query.clone(), &cancel).await.expect("initial window");
    let unchanged = storage.history_reusing(query.clone(), known(&first), &cancel).await.expect("unchanged window");
    assert_eq!(first.next_cursor, unchanged.next_cursor);
    assert!(first.entries.iter().zip(&unchanged.entries).all(|(old, new)| Arc::ptr_eq(old, new)));

    let tag = storage.create_tag("tag".into(), "#123456".into(), None, 350).await.expect("create tag");
    storage.bind_tag(ids[1], tag, 350).await.expect("bind tag");
    let tagged = storage.history_reusing(query.clone(), known(&first), &cancel).await.expect("metadata delta");
    assert!(Arc::ptr_eq(&first.entries[0], &tagged.entries[0]));
    assert!(!Arc::ptr_eq(&first.entries[1], &tagged.entries[1]));
    assert_eq!(first.entries[1].content_revision, tagged.entries[1].content_revision);
    assert_eq!(tagged.entries[1].tags[0].id, tag);
    let filtered = storage.history_reusing(HistoryQuery { search: "entry-1".into(), tag_ids: vec![tag], ..query.clone() }, known(&tagged), &cancel).await.expect("filtered window");
    assert_eq!(filtered.entries.len(), 1);
    assert!(Arc::ptr_eq(&tagged.entries[1], &filtered.entries[0]));

    storage.touch(ids[0], 400).await.expect("move oldest to head");
    let moved = storage.history_reusing(query.clone(), known(&tagged), &cancel).await.expect("order delta");
    assert_eq!(moved.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(), vec![ids[0], ids[2]]);
    assert!(Arc::ptr_eq(&tagged.entries[0], &moved.entries[1]));
    let tail = storage.history_reusing(HistoryQuery { cursor: moved.next_cursor.clone(), ..query.clone() }, known(&tagged), &cancel).await.expect("cursor after reorder");
    assert_eq!(tail.entries.len(), 1);
    assert!(Arc::ptr_eq(&tagged.entries[1], &tail.entries[0]));
    storage.delete_record(ids[0]).await.expect("delete head");
    let deleted = storage.history_reusing(query.clone(), known(&moved), &cancel).await.expect("refill boundary");
    assert_eq!(deleted.entries.iter().map(|entry| entry.id).collect::<Vec<_>>(), vec![ids[2], ids[1]]);
    assert!(Arc::ptr_eq(&moved.entries[1], &deleted.entries[0]));
    assert!(deleted.next_cursor.is_none());

    // An OCR-only revision must replace that summary even when content and
    // metadata revisions do not change, including search membership.
    let ocr_id = ids[1];
    storage.call(move |connection| {
        let transaction = connection.transaction()?;
        let revision = super::revision(&transaction)?;
        transaction.execute("UPDATE history SET ocr_revision=?1,ocr_has_text=1 WHERE id=?2", rusqlite::params![revision, ocr_id.to_string()])?;
        transaction.execute("UPDATE payloads SET ocr='recognized text' WHERE record_id=?1", [ocr_id.to_string()])?;
        transaction.commit()?; Ok(())
    }).await.expect("OCR revision fixture");
    let ocr = storage.history_reusing(HistoryQuery { search: "recognized".into(), ..query.clone() }, known(&deleted), &cancel).await.expect("OCR search delta");
    assert_eq!(ocr.entries.len(), 1);
    assert!(!Arc::ptr_eq(&deleted.entries[1], &ocr.entries[0]));
    assert!(ocr.entries[0].has_ocr);

    let updated_source = Arc::new(SourceApp { icon: Some(bytes::Bytes::from_static(b"updated icon")), ..source.as_ref().clone() });
    let mut source_window = deleted;
    for (digest, timestamp) in [(4, 50), (5, 40)] {
        storage.upsert_record(PreparedRecord {
            digest: Digest([digest; 32]), content: TextContent { text: Some(format!("outside loaded window {digest}").into()), ..Default::default() },
            image: None, timestamp, source: Some(updated_source.clone()),
        }, limits).await.expect("capture same source");
        let current = storage.history_reusing(query.clone(), known(&source_window), &cancel).await.expect("shared source invalidation");
        assert!(current.entries.iter().all(|entry| entry.source.as_ref().expect("source").icon == updated_source.icon));
        if digest == 5 {
            assert!(source_window.entries.iter().zip(&current.entries).all(|(old, new)| Arc::ptr_eq(old, new)), "an insertion outside the window with unchanged source must reuse every row");
        }
        source_window = current;
    }
}

#[tokio::test]
async fn ocr_presence_uses_complete_result_and_preserves_original() {
    use crate::ocr::OcrAdmission;
    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("ocr-presence.sqlite3")).await.expect("isolated database");
    let cancel = CancellationToken::new();
    let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default()).expect("image service");
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8).expect("tiny source image");
    let image = images.import_encoded(png, &cancel).await.expect("image ingress");
    let (saved, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([89; 32]), content: TextContent::default(), image: Some(image), timestamp: 100, source: None,
    }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save image");
    assert!(!storage.record(saved.id, &cancel).await.expect("initial record").entry.has_ocr);
    let cases = [
        (format!("{}识别文字\r\n", " \u{feff}".repeat(5000)), true),
        ("\u{feff} \t\r\n\u{a0}\u{3000}".repeat(1800), false),
        ("\u{85}".into(), true),
        ("\0".into(), true),
        (String::new(), false),
    ];
    for (text, expected) in cases {
        let record = storage.record(saved.id, &cancel).await.expect("current revision");
        let OcrAdmission::Started(ticket) = storage.begin_ocr(saved.id, record.entry.content_revision, 0).await.expect("OCR admission") else { panic!("image must be admitted"); };
        let original: Arc<str> = text.into();
        let revision = storage.update_ocr(ticket, original.clone()).await.expect("save OCR").expect("accepted OCR revision");
        let page = storage.history(HistoryQuery::default(), &cancel).await.expect("hot metadata");
        assert_eq!(page.entries[0].has_ocr, expected);
        assert_eq!(page.entries[0].ocr_revision, revision);
        let complete = storage.record(saved.id, &cancel).await.expect("complete record");
        assert_eq!(complete.entry.has_ocr, expected);
        assert_eq!(complete.content.ocr.as_ref(), Some(&original));
        assert_eq!(storage.restore_input(saved.id, RestoreMode::Ocr, &cancel).await.expect("raw OCR restoration").content.text, Some(original));
        assert!(storage.update_ocr(ticket, "stale result".into()).await.expect("stale completion").is_none());
        assert_eq!(storage.record(saved.id, &cancel).await.expect("unchanged completion").entry.has_ocr, expected);
    }
    images.shutdown().await.expect("release image work");
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn ocr_presence_upgrade_preserves_payloads_and_rolls_back_on_invalid_data() {
    let directory = DatabaseDirectory::new();
    let path = directory.0.join("ocr-upgrade.sqlite3");
    let storage = Storage::open(&path).await.expect("isolated database");
    let cancel = CancellationToken::new();
    let mut ids = Vec::new();
    for index in 0..3u8 {
        let (saved, _) = storage.upsert_record(PreparedRecord {
            digest: Digest([90 + index; 32]), content: TextContent { text: Some(format!("source {index}").into()), ..Default::default() },
            image: None, timestamp: i64::from(index), source: None,
        }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save original record");
        ids.push(saved.id);
    }
    storage.close_for_test().await.expect("close before legacy fixture");
    // v1 differs only in these derived columns. Keep the complete independent
    // app schema, IDs, payloads and configuration while restoring its old shape.
    const V1_COLUMNS: &str = "ALTER TABLE history DROP COLUMN ocr_has_text; ALTER TABLE history ADD COLUMN ocr_excerpt TEXT; ALTER TABLE history ADD COLUMN ocr_truncated INTEGER NOT NULL DEFAULT 0; PRAGMA user_version=1;";
    let text = format!("{}识别文字\0原文", " \u{feff}".repeat(5000));
    let blank = " \u{feff}\r\n".repeat(3000);
    {
        let connection = rusqlite::Connection::open(&path).expect("legacy fixture");
        connection.execute_batch(V1_COLUMNS).expect("v1 OCR columns");
        for (id, original) in [(ids[0], &text), (ids[1], &blank)] {
            connection.execute("UPDATE payloads SET ocr=?2 WHERE record_id=?1", rusqlite::params![id.to_string(), original]).expect("legacy original OCR");
            connection.execute("UPDATE history SET ocr_excerpt=' ',ocr_truncated=1,ocr_revision=7 WHERE id=?1", [id.to_string()]).expect("misleading legacy excerpt");
        }
    }
    let storage = Storage::open(&path).await.expect("upgrade independent v1 database");
    for (id, expected, original) in [(ids[0], true, Some(text.as_str())), (ids[1], false, Some(blank.as_str())), (ids[2], false, None)] {
        let record = storage.record(id, &cancel).await.expect("upgraded record");
        assert_eq!(record.entry.has_ocr, expected);
        assert_eq!(record.content.ocr.as_deref(), original);
        if original.is_some() { assert_eq!(record.entry.ocr_revision, 7); }
    }
    storage.close_for_test().await.expect("close upgraded database");
    {
        let connection = rusqlite::Connection::open(&path).expect("inspect upgrade");
        assert_eq!(connection.pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0)).expect("version"), 2);
        let columns: Vec<String> = connection.prepare("PRAGMA table_info(history)").expect("columns").query_map([], |row| row.get(1)).expect("column rows").collect::<rusqlite::Result<_>>().expect("column names");
        assert!(!columns.iter().any(|column| column == "ocr_excerpt" || column == "ocr_truncated"));
        connection.execute_batch(V1_COLUMNS).expect("another legacy fixture");
        connection.execute("UPDATE payloads SET ocr=CAST(x'ff' AS TEXT) WHERE record_id=?1", [ids[2].to_string()]).expect("invalid legacy OCR");
    }
    assert!(matches!(Storage::open(&path).await, Err(Error::Database(message)) if message.contains("UTF-8")));
    {
        let connection = rusqlite::Connection::open(&path).expect("inspect rollback");
        assert_eq!(connection.pragma_query_value::<i32, _>(None, "user_version", |row| row.get(0)).expect("version"), 1);
        assert!(connection.prepare("SELECT ocr_has_text FROM history").is_err(), "failed DDL must roll back");
        connection.prepare("SELECT ocr_excerpt,ocr_truncated FROM history").expect("legacy schema retained");
        connection.execute("UPDATE payloads SET ocr=NULL WHERE record_id=?1", [ids[2].to_string()]).expect("repair invalid fixture");
    }
    let storage = Storage::open(&path).await.expect("retry rolled-back upgrade");
    assert!(storage.record(ids[0], &cancel).await.expect("preserved record").entry.has_ocr);
    storage.close_for_test().await.expect("close retried upgrade");
    let storage = Storage::open(&path).await.expect("reopen current schema without reapplying DDL");
    assert_eq!(storage.history_count().await.expect("retained rows"), 3);
    storage.close_for_test().await.expect("close current schema");
}

impl DatabaseDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("super-paste-rs-storage-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create isolated database directory");
        Self(path)
    }
}

impl Drop for DatabaseDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            eprintln!("failed to remove test database directory: {error}");
        }
    }
}

#[tokio::test]
async fn history_mcp_qr_and_image_cancellation_skip_sql_while_queued() {
    use std::{future::Future, sync::atomic::{AtomicUsize, Ordering}, task::Poll, time::Duration};
    use rusqlite::hooks::{AuthContext, Authorization};

    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("cancel-queued.sqlite3")).await.expect("isolated database");
    let count = Arc::new(AtomicUsize::new(0)); let observed = count.clone();
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let worker = tokio::spawn({ let storage = storage.clone(); async move {
        storage.call(move |connection| {
            connection.authorizer(Some(move |_: AuthContext<'_>| {
                observed.fetch_add(1, Ordering::Relaxed); Authorization::Allow
            }))?;
            let _ = entered.send(());
            blocked.recv_timeout(Duration::from_secs(5)).expect("release held database worker");
            Ok(())
        }).await
    }});
    ready.await.expect("database worker held");
    let cancel = CancellationToken::new();
    let mut query = Box::pin(storage.history(HistoryQuery::default(), &cancel));
    let mut qr = Box::pin(storage.qr_text_input(RecordId::new(), &cancel));
    let mut restore_original = Box::pin(storage.restore_input(RecordId::new(), RestoreMode::Original, &cancel));
    let mut restore_plain = Box::pin(storage.restore_input(RecordId::new(), RestoreMode::PlainText, &cancel));
    let mut restore_ocr = Box::pin(storage.restore_input(RecordId::new(), RestoreMode::Ocr, &cancel));
    let mut mcp_search = Box::pin(storage.search_records(String::new(), 20, 0, &cancel));
    let mut mcp_record = Box::pin(storage.record(RecordId::new(), &cancel));
    let mut mcp_image = Box::pin(storage.image_metadata(RecordId::new(), &cancel));
    let mut copy_stats = Box::pin(storage.copy_stats(super_paste_format::CopyStatsQuery::default(), 100, &cancel));
    let mut ai_usage = Box::pin(storage.ai_usage(super_paste_format::AiUsageQuery::default(), &cancel));
    let mut tags = Box::pin(storage.tags(&cancel));
    let image_budget = ByteBudget::new(16, "queued image read");
    let mut image = Box::pin(storage.read_image(Digest([0; 32]), 16, image_budget.reserve(16).expect("image admission"), &cancel));
    std::future::poll_fn(|cx| {
        assert!(query.as_mut().poll(cx).is_pending(), "query must be queued behind the held worker");
        assert!(qr.as_mut().poll(cx).is_pending(), "QR source read must be queued behind the held worker");
        assert!(restore_original.as_mut().poll(cx).is_pending(), "original restore must be queued");
        assert!(restore_plain.as_mut().poll(cx).is_pending(), "plain restore must be queued");
        assert!(restore_ocr.as_mut().poll(cx).is_pending(), "OCR restore must be queued");
        assert!(mcp_search.as_mut().poll(cx).is_pending(), "MCP search must be queued");
        assert!(mcp_record.as_mut().poll(cx).is_pending(), "MCP record read must be queued");
        assert!(mcp_image.as_mut().poll(cx).is_pending(), "MCP image metadata must be queued");
        assert!(copy_stats.as_mut().poll(cx).is_pending(), "copy statistics must be queued");
        assert!(ai_usage.as_mut().poll(cx).is_pending(), "AI usage must be queued");
        assert!(tags.as_mut().poll(cx).is_pending(), "tag counts must be queued");
        assert!(image.as_mut().poll(cx).is_pending(), "image read must be queued behind the held worker");
        Poll::Ready(())
    }).await;
    cancel.cancel(); drop(image);
    assert_eq!(image_budget.used(), 16, "queued physical read still owns its admission after the async observer leaves");
    let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default()).expect("image service");
    let mut shutdown = Box::pin(images.shutdown());
    std::future::poll_fn(|cx| {
        assert!(shutdown.as_mut().poll(cx).is_pending(), "image shutdown must drain physical SQLite reads");
        Poll::Ready(())
    }).await;
    release.send(()).expect("release database worker");
    worker.await.expect("join held worker").expect("held database call");
    shutdown.await.expect("drain image reads");
    assert_eq!(image_budget.used(), 0, "shutdown waits until the physical read releases admission");
    assert!(matches!(query.await, Err(Error::Cancelled)));
    assert!(matches!(qr.await, Err(Error::Cancelled)));
    assert!(matches!(restore_original.await, Err(Error::Cancelled)));
    assert!(matches!(restore_plain.await, Err(Error::Cancelled)));
    assert!(matches!(restore_ocr.await, Err(Error::Cancelled)));
    assert!(matches!(mcp_search.await, Err(Error::Cancelled)));
    assert!(matches!(mcp_record.await, Err(Error::Cancelled)));
    assert!(matches!(mcp_image.await, Err(Error::Cancelled)));
    assert!(matches!(copy_stats.await, Err(Error::Cancelled)));
    assert!(matches!(ai_usage.await, Err(Error::Cancelled)));
    assert!(matches!(tags.await, Err(Error::Cancelled)));
    assert_eq!(count.load(Ordering::Relaxed), 0, "obsolete read must not prepare SQL");
    storage.call(|connection| {
        connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?; Ok(())
    }).await.expect("clear test authorizer");
    storage.clear_copy_stats(101).await.expect("accepted copy statistics clear after cancellation");
    storage.clear_ai_usage(101).await.expect("accepted AI usage clear after cancellation");
    let live = CancellationToken::new();
    let copy = storage.copy_stats(super_paste_format::CopyStatsQuery::default(), 102, &live).await.expect("next live copy statistics read");
    let ai = storage.ai_usage(super_paste_format::AiUsageQuery::default(), &live).await.expect("next live AI usage read");
    assert_eq!(copy.cleared_at, Some(101));
    assert_eq!(ai.cleared_at, Some(101));
    assert_eq!(copy.totals.copies, 0);
    assert_eq!(ai.totals.requests, 0);
    let tag = storage.create_tag("取消后的标签".into(), "#aabbcc".into(), None, 103).await.expect("tag creation after cancellation");
    let tags = storage.tags(&live).await.expect("next live tag read");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].id, tag);
    assert_eq!(tags[0].count, 0);
    assert!(storage.delete_tag(tag).await.expect("tag deletion after cancellation"));
    assert!(storage.tags(&live).await.expect("tag read after deletion").is_empty());
    assert_eq!(image_budget.used(), 0, "cancelled queued image read releases admission without allocating a BLOB");
    assert!(storage.history(HistoryQuery::default(), &CancellationToken::new()).await.expect("next live page").entries.is_empty());
    storage.close_for_test().await.expect("close database");
    assert!(matches!(storage.history(HistoryQuery::default(), &cancel).await, Err(Error::Cancelled)), "already-cancelled read must not enter a closed worker");
    assert!(matches!(storage.qr_text_input(RecordId::new(), &cancel).await, Err(Error::Cancelled)), "already-cancelled QR read must not enter a closed worker");
    for mode in [RestoreMode::Original, RestoreMode::PlainText, RestoreMode::Ocr] {
        assert!(matches!(storage.restore_input(RecordId::new(), mode, &cancel).await, Err(Error::Cancelled)), "already-cancelled restoration must not enter a closed worker");
    }
    assert!(matches!(storage.search_records(String::new(), 20, 0, &cancel).await, Err(Error::Cancelled)));
    assert!(matches!(storage.record(RecordId::new(), &cancel).await, Err(Error::Cancelled)));
    assert!(matches!(storage.image_metadata(RecordId::new(), &cancel).await, Err(Error::Cancelled)));
    assert!(matches!(storage.copy_stats(super_paste_format::CopyStatsQuery::default(), 100, &cancel).await, Err(Error::Cancelled)));
    assert!(matches!(storage.ai_usage(super_paste_format::AiUsageQuery::default(), &cancel).await, Err(Error::Cancelled)));
    assert!(matches!(storage.tags(&cancel).await, Err(Error::Cancelled)));
}

#[tokio::test]
async fn qr_source_keeps_plain_text_priority_and_literal_html_fallback() {
    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("qr-source.sqlite3")).await.expect("isolated database");
    let cancel = CancellationToken::new();
    for (index, text, html, expected) in [
        (40, Some("原文\0🙂"), "<p>不应替代原文</p>", "原文\0🙂"),
        (41, None, "<p>汉字&amp;🙂</p>\r\n<b>保留实体</b>", "汉字&amp;🙂\r\n保留实体"),
        (42, Some(""), "<i>空文本回退</i>", "空文本回退"),
    ] {
        let (record, _) = storage.upsert_record(PreparedRecord {
            digest: Digest([index; 32]), content: TextContent { text: text.map(Arc::from), html: Some(html.into()), ..Default::default() },
            image: None, timestamp: 100, source: None,
        }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save QR source");
        assert_eq!(&*storage.qr_text_input(record.id, &cancel).await.expect("read QR source"), expected);
    }
    let (record, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([43; 32]), content: TextContent { text: Some("0".repeat(crate::qr::MAX_TEXT_BYTES + 1).into()), html: Some("<p>不能绕过文本长度限制</p>".into()), ..Default::default() },
        image: None, timestamp: 100, source: None,
    }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save oversized source");
    assert!(matches!(storage.qr_text_input(record.id, &cancel).await, Err(Error::Invalid(_))));
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn history_cancellation_interrupts_active_read_without_poisoning_later_commands() {
    use rusqlite::hooks::{AuthContext, Authorization};

    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("cancel-active.sqlite3")).await.expect("isolated database");
    let (started, running) = tokio::sync::oneshot::channel();
    storage.call(move |connection| {
        let mut started = Some(started);
        connection.authorizer(Some(move |_: AuthContext<'_>| {
            if let Some(started) = started.take() { let _ = started.send(()); }
            Authorization::Allow
        }))?;
        Ok(())
    }).await.expect("observe query preparation");
    let cancel = CancellationToken::new();
    let task = tokio::spawn({ let storage = storage.clone(); let cancel = cancel.clone(); async move {
        storage.call_read(&cancel, |connection| {
            let _: i64 = connection.query_row("WITH RECURSIVE counter(x) AS (VALUES(0) UNION ALL SELECT x+1 FROM counter WHERE x<1000000000) SELECT sum(x) FROM counter", [], |row| row.get(0))?;
            Ok(())
        }).await
    }});
    running.await.expect("query entered SQLite with its progress handler installed");
    cancel.cancel();
    assert!(matches!(tokio::time::timeout(std::time::Duration::from_secs(5), task).await.expect("active read interrupted").expect("join read"), Err(Error::Cancelled)));
    storage.call(|connection| {
        connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?; Ok(())
    }).await.expect("clear test authorizer");
    let content = TextContent { text: Some("retained after cancellation".into()), ..Default::default() };
    let (record, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([37; 32]), content, image: None, timestamp: 100, source: None,
    }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("later write must not inherit cancellation");
    let live = CancellationToken::new();
    let failed: crate::Result<()> = storage.call_read(&live, |connection| {
        connection.execute_batch("SELECT * FROM missing_table")?; Ok(())
    }).await;
    assert!(matches!(failed, Err(Error::Database(_))), "real SQL errors remain visible");
    let page = storage.history(HistoryQuery { search: "retained".into(), limit: 1, ..Default::default() }, &live).await.expect("next live search after cancellation and SQL failure");
    assert_eq!(page.entries.len(), 1); assert_eq!(page.entries[0].id, record.id);
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn preview_cancellation_rolls_back_read_and_live_scopes_share_document() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use crate::preview::{PreviewBody, PreviewService};

    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("preview-cancel.sqlite3")).await.expect("isolated database");
    storage.upsert_record(PreparedRecord {
        digest: Digest([38; 32]), content: TextContent { html: Some("<p>共享<b>文档</b></p>".into()), ..Default::default() },
        image: None, timestamp: 100, source: None,
    }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save rich source");
    let live = CancellationToken::new();
    let entry = storage.history(HistoryQuery::default(), &live).await.expect("summary").entries.remove(0);
    let cancel = CancellationToken::new(); let requested = cancel.clone();
    let selects = Arc::new(AtomicUsize::new(0)); let observed = selects.clone();
    storage.call(move |connection| {
        connection.authorizer(Some(move |context: AuthContext<'_>| {
            // Header and file sizing have completed and the input lease is
            // held when the payload SELECT is prepared. Cancel at that point.
            if matches!(context.action, AuthAction::Select) && observed.fetch_add(1, Ordering::Relaxed) == 2 { requested.cancel(); }
            Authorization::Allow
        }))?;
        Ok(())
    }).await.expect("observe payload read");
    let memory = ByteBudget::new(1024 * 1024, "preview cancellation");
    assert!(matches!(storage.preview_input(entry.clone(), PreviewScope::Details, memory.clone(), &cancel).await, Err(Error::Cancelled)));
    assert!(selects.load(Ordering::Relaxed) >= 3); assert_eq!(memory.used(), 0, "cancelled input releases its payload lease");
    storage.call(|connection| {
        assert!(connection.is_autocommit(), "cancelled read must close its snapshot transaction");
        connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?; Ok(())
    }).await.expect("clear observer and inspect transaction");
    let service = PreviewService::new(storage.clone(), tokio::runtime::Handle::current());
    let (card, details) = tokio::join!(service.get(entry.clone(), PreviewScope::Card, &live), service.get(entry, PreviewScope::Details, &live));
    let card = card.expect("live card after cancelled SQL"); let details = details.expect("live details after cancelled SQL");
    match (&card.body, &details.body) {
        (PreviewBody::Rich(card), PreviewBody::Rich(details)) => {
            assert!(Arc::ptr_eq(card, details), "card and details reuse the same parsed source");
            assert!(card.text.contains("共享文档"));
        }
        _ => panic!("both scopes must preserve the rich document"),
    }
    drop(card); drop(details);
    service.shutdown().await.expect("drain preview work");
    storage.close_for_test().await.expect("close database");
}

#[tokio::test]
async fn external_link_projection_reads_full_text_without_loading_other_formats() {
    let directory = DatabaseDirectory::new();
    let storage = Storage::open(&directory.0.join("links.sqlite3")).await.expect("isolated database");
    let original: Arc<str> = format!(" \tHTTPS://example.com/{}?q=%2F#end\r\n", "path".repeat(6000)).into();
    let (record, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([61; 32]), content: TextContent { text: Some(original.clone()), ..Default::default() }, image: None, timestamp: 1, source: None,
    }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save full long URL");
    let id = record.id;
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET html=x'ff',rtf=x'ff',ocr=x'ff' WHERE record_id=?1", [id.to_string()])?;
        Ok(())
    }).await.expect("unrelated formats must not be read");
    let service = crate::preview::PreviewService::new(storage.clone(), tokio::runtime::Handle::current());
    let link = service.external_link(id).await.expect("read and validate full text only");
    assert_eq!(link.href(), original.trim()); assert!(link.href().len() > 20_000);
    drop(link);
    let memory = ByteBudget::new(1, "external link");
    assert!(matches!(storage.external_link_input(id, memory.clone()).await, Err(Error::ResourceLimit(_))));
    assert_eq!(memory.used(), 0);
    let memory = ByteBudget::new(1024 * 1024, "external link");
    storage.call(move |connection| { connection.execute("UPDATE payloads SET text=CAST(x'ff' AS TEXT) WHERE record_id=?1", [id.to_string()])?; Ok(()) }).await.expect("invalid text encoding");
    assert!(matches!(storage.external_link_input(id, memory.clone()).await, Err(Error::Database(_))));
    assert_eq!(memory.used(), 0);
    storage.delete_record(id).await.expect("remove record");
    assert!(matches!(service.external_link(id).await, Err(Error::NotFound)));
    service.shutdown().await.expect("drain reads");
    assert!(matches!(service.external_link(id).await, Err(Error::Cancelled)));
    storage.close_for_test().await.expect("close database");
}

// Exercise the disk schema and real image ingress rather than a serialized DTO
// or warmed preview cache. No window, clipboard, OCR runtime or user data is used.
#[tokio::test]
async fn raw_formats_survive_reopen_and_shared_images_follow_record_lifetimes() {
    let directory = DatabaseDirectory::new();
    let path = directory.0.join("history.sqlite3");
    let storage = Storage::open(&path).await.expect("open empty database");
    let initial = storage.diagnostic_counts().await.expect("initial counts");
    assert_eq!((initial.records, initial.tags, initial.images), (0, 0, 0));

    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png)
        .write_image(&[255, 0, 0, 255, 0, 0, 255, 128], 2, 1, image::ExtendedColorType::Rgba8)
        .expect("encode fixture once");
    let cancel = CancellationToken::new();
    let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default())
        .expect("image service");
    let image = images.import_encoded(png.clone(), &cancel).await.expect("import original image");
    let metadata = image.metadata();
    let content = TextContent {
        text: Some(format!("  {}\r\n\0尾部", "中文👩🏽‍💻e\u{301}\t".repeat(3000)).into()),
        html: Some(format!("<div data-original=\"yes\">  {} &amp; &#x1f600; </div>", "<b>段落</b>\r\n".repeat(1800)).into()),
        rtf: Some(format!("{{\\rtf1\\ansi\\ansicpg936 {}\\u20013?}}", "\\b preserved\\b0 ".repeat(1800)).into()),
        files: vec!["/资料/含 空格.txt".into(), "C:\\资料\\原件.png".into(), "/资料/含 空格.txt".into()].into(),
    };
    let unlimited = HistoryLimits { max_records: -1, keep_days: -1 };
    let (first, removed) = storage.upsert_record(PreparedRecord {
        digest: Digest([1; 32]), content: content.clone(), image: Some(image), timestamp: 100, source: None,
    }, unlimited).await.expect("save all original formats");
    assert!(first.inserted && removed.is_empty());
    let before = storage.record(first.id, &cancel).await.expect("full original record");
    assert_eq!(before.content.content, content);
    {
        let records = storage.search_records(String::new(), 1, 0, &cancel).await.expect("live MCP search");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].entry.id, first.id);
        assert_eq!(records[0].content.content, content);
        assert_eq!(storage.image_metadata(first.id, &cancel).await.expect("live MCP image metadata").map(|image| image.id), Some(metadata.id));
    }
    let excerpt = storage.history(HistoryQuery::default(), &cancel).await.expect("summary page");
    assert_eq!(excerpt.entries.len(), 1);
    assert!(excerpt.entries[0].truncated.contains(Formats::TEXT | Formats::HTML | Formats::RTF));
    assert_ne!(excerpt.entries[0].excerpt.text, content.text);

    // Closing the worker and replacing the image service excludes a cache hit
    // from proving persistence or hiding a changed original byte stream.
    storage.checkpoint().await.expect("checkpoint original data");
    storage.connection.clone().close().await.expect("close database");
    drop(images);
    drop(storage);
    let storage = Storage::open(&path).await.expect("reopen database");
    let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default())
        .expect("fresh image service");
    let image = images.load_original(metadata.clone(), &cancel).await.expect("load persisted image");
    assert_eq!(image.bytes(), png);
    assert_eq!(image.dimensions(), Some((2, 1)));
    assert_eq!(storage.content(first.id).await.expect("reopened formats").content, content);

    let (duplicate, removed) = storage.upsert_record(PreparedRecord {
        digest: Digest([1; 32]), content: content.clone(), image: Some(image.clone()), timestamp: 200, source: None,
    }, unlimited).await.expect("deduplicate record");
    assert!(!duplicate.inserted && removed.is_empty());
    assert_eq!(duplicate.id, first.id);
    let after = storage.record(first.id, &cancel).await.expect("deduplicated record");
    assert_eq!(after.entry.timestamp, 100);
    assert_eq!(after.entry.last_used, 200);
    assert_eq!(after.entry.content_revision, before.entry.content_revision);
    assert!(after.entry.metadata_revision > before.entry.metadata_revision);
    assert_eq!(after.content.content, content);

    let (protected, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([2; 32]), content: TextContent { text: Some("保留图片".into()), ..Default::default() },
        image: Some(image.clone()), timestamp: 300, source: None,
    }, unlimited).await.expect("save second image reference");
    let tag = storage.create_tag("保留".into(), "#3399ff".into(), None, 300).await.expect("create tag");
    assert!(storage.bind_tag(protected.id, tag, 300).await.expect("protect record"));
    let (newest, removed) = storage.upsert_record(PreparedRecord {
        digest: Digest([3; 32]), content: TextContent { text: Some("最新图片".into()), ..Default::default() },
        image: Some(image), timestamp: 400, source: None,
    }, HistoryLimits { max_records: 1, keep_days: -1 }).await.expect("apply retention");
    assert_eq!(removed, vec![first.id]);
    let counts = storage.diagnostic_counts().await.expect("shared image counts");
    assert_eq!((counts.records, counts.images, counts.encoded_image_bytes), (2, 1, png.len() as u64));

    assert_eq!(storage.clear_history().await.expect("clear unprotected history"), vec![newest.id]);
    assert_eq!(storage.history_count().await.expect("protected history count"), 1);
    let read_budget = ByteBudget::new(metadata.encoded_bytes, "stored image read");
    assert_eq!(storage.read_image(metadata.id, metadata.encoded_bytes, read_budget.reserve(metadata.encoded_bytes).expect("image admission"), &CancellationToken::new()).await.expect("image remains referenced").0, png);
    assert!(storage.delete_record(protected.id).await.expect("delete final image reference"));
    let counts = storage.diagnostic_counts().await.expect("orphan collection counts");
    assert_eq!((counts.records, counts.images, counts.encoded_image_bytes), (0, 0, 0));
    assert!(matches!(storage.read_image(metadata.id, metadata.encoded_bytes, read_budget.reserve(metadata.encoded_bytes).expect("image admission"), &CancellationToken::new()).await, Err(crate::Error::NotFound)));
    storage.connection.clone().close().await.expect("close database after verification");
}

#[tokio::test]
async fn restoration_projects_only_required_formats_and_preserves_fallbacks() {
    let cancel = CancellationToken::new();
    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("restore.sqlite3")).await.expect("isolated database");
    let images = ImageService::new(tokio::runtime::Handle::current(), storage.clone(), ImageBudgets::default()).expect("image service");
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(&[1, 2, 3, 255], 1, 1, image::ExtendedColorType::Rgba8).expect("tiny original image");
    let image = images.import_encoded(png, &CancellationToken::new()).await.expect("image ingress");
    let metadata = image.metadata();
    let content = TextContent {
        text: Some("\0原文 \t👩🏽‍💻\r\n ".into()), html: Some("<b>HTML 原文</b>".into()), rtf: Some(r"{\rtf1 RTF original}".into()),
        files: vec!["/资料/a b.txt".into(), "/资料/a b.txt".into()].into(),
    };
    let limits = HistoryLimits { max_records: -1, keep_days: -1 };
    let (record, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([20; 32]), content: content.clone(), image: Some(image), timestamp: 100, source: None,
    }, limits).await.expect("store original formats");
    let id = record.id; let ocr: Arc<str> = "  OCR\r\n\0识别文字  ".into(); let saved_ocr = ocr.to_string();
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET ocr=?2 WHERE record_id=?1", rusqlite::params![id.to_string(), saved_ocr])?;
        Ok(())
    }).await.expect("set authoritative OCR");
    let interrupted = CancellationToken::new(); let requested = interrupted.clone();
    storage.call(move |connection| {
        use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
        connection.authorizer(Some(move |context: AuthContext<'_>| {
            // The snapshot has begun when SQLite prepares its first SELECT.
            if matches!(context.action, AuthAction::Select) { requested.cancel(); }
            Authorization::Allow
        }))?;
        Ok(())
    }).await.expect("cancel an active restoration snapshot");
    assert!(matches!(storage.restore_input(id, RestoreMode::Original, &interrupted).await, Err(Error::Cancelled)));
    storage.call(|connection| {
        use rusqlite::hooks::{AuthContext, Authorization};
        connection.authorizer(None::<fn(AuthContext<'_>) -> Authorization>)?;
        assert!(connection.is_autocommit(), "cancelled restoration must release its read transaction");
        Ok(())
    }).await.expect("clear restoration observer");
    let original = storage.restore_input(id, RestoreMode::Original, &cancel).await.expect("all original formats");
    assert_eq!(original.content, content);
    assert!(matches!(original.image, Some(ImageSource::Stored(image)) if image.id == metadata.id && image.encoded_bytes == metadata.encoded_bytes));
    let expected_ocr = TextContent { text: Some(ocr), ..Default::default() };
    let recognized = storage.restore_input(id, RestoreMode::Ocr, &cancel).await.expect("OCR-only input");
    assert_eq!(recognized.content, expected_ocr);
    assert!(recognized.image.is_none());

    // Invalid UTF-8 in irrelevant columns makes an accidental Rust payload read
    // fail deterministically, without allocating giant test payloads or timing IO.
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET html=CAST(x'80' AS TEXT),rtf=CAST(x'80' AS TEXT) WHERE record_id=?1", [id.to_string()])?;
        connection.execute("UPDATE files SET path=CAST(x'80' AS TEXT) WHERE record_id=?1", [id.to_string()])?;
        Ok(())
    }).await.expect("make unused formats unreadable");
    let plain = storage.restore_input(id, RestoreMode::PlainText, &cancel).await.expect("raw text skips unrelated formats");
    assert_eq!(plain.content, TextContent { text: content.text, ..Default::default() });
    assert!(plain.image.is_none());
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET text=CAST(x'80' AS TEXT) WHERE record_id=?1", [id.to_string()])?;
        Ok(())
    }).await.expect("make original text unreadable");
    assert_eq!(storage.restore_input(id, RestoreMode::Ocr, &cancel).await.expect("OCR skips all original formats").content, expected_ocr);
    assert!(matches!(storage.restore_input(id, RestoreMode::PlainText, &cancel).await, Err(Error::Database(_))));

    let (fallback, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([21; 32]), content: TextContent {
            text: Some("\u{85}\u{a0}\u{2003}\r\n\t".into()), html: Some("<script>hidden</script><p> </p>".into()),
            rtf: Some(r"{\rtf1 fallback}".into()), files: vec!["/资料/a b.txt".into(), "/资料/a b.txt".into()].into(),
        }, image: None, timestamp: 200, source: None,
    }, limits).await.expect("store Unicode whitespace and format fallbacks");
    let input = storage.restore_input(fallback.id, RestoreMode::PlainText, &cancel).await.expect("format fallbacks");
    assert!(input.content.text.is_none());
    assert_eq!(input.content.plain_text().expect("RTF fallback").as_ref(), "fallback");
    let fallback_id = fallback.id;
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET rtf=?2 WHERE record_id=?1", rusqlite::params![fallback_id.to_string(), r"{\rtf1 }"])?;
        Ok(())
    }).await.expect("empty textual fallbacks");
    let input = storage.restore_input(fallback.id, RestoreMode::PlainText, &cancel).await.expect("file fallback");
    assert_eq!(input.content.plain_text().expect("ordered duplicate file paths").as_ref(), "/资料/a b.txt\n/资料/a b.txt");
    assert!(storage.restore_input(fallback.id, RestoreMode::Ocr, &cancel).await.expect("existing record without OCR").is_empty());
    let missing = RecordId::new();
    for mode in [RestoreMode::Original, RestoreMode::PlainText, RestoreMode::Ocr] {
        assert!(matches!(storage.restore_input(missing, mode, &cancel).await, Err(Error::NotFound)));
    }
    storage.close_for_test().await.expect("close isolated database");
}

#[tokio::test]
async fn previews_preserve_nul_and_bound_utf16_before_sharing() {
    let directory = DatabaseDirectory::new();
    let storage = Storage::open(directory.0.join("preview.sqlite3")).await.expect("isolated database");
    // The bounded SQL byte prefix ends inside 尾, while the preceding emoji
    // crosses the UTF-16 display boundary. Neither may corrupt the preview.
    let expected_card = format!("前\0{}", "中".repeat(19_997));
    let content = TextContent {
        text: Some(format!("{expected_card}😀尾部").into()),
        html: Some(format!("\0{}x", "😀".repeat(99_999)).into()),
        rtf: Some(format!("\0{}", "😀".repeat(100_000)).into()),
        files: vec!["/资料/a b.txt".into(), "/资料/a b.txt".into()].into(),
    };
    let (record, _) = storage.upsert_record(PreparedRecord {
        digest: Digest([30; 32]), content: content.clone(), image: None, timestamp: 100, source: None,
    }, HistoryLimits { max_records: -1, keep_days: -1 }).await.expect("save Unicode sources");
    let entry = storage.history(HistoryQuery::default(), &CancellationToken::new()).await.expect("summary").entries.remove(0);
    assert!(entry.truncated.contains(Formats::TEXT | Formats::HTML | Formats::RTF));
    let memory = ByteBudget::new(2 * 1024 * 1024, "preview regression");
    for scope in [PreviewScope::Card, PreviewScope::Details] {
        let input = storage.preview_input(entry.clone(), scope, memory.clone(), &CancellationToken::new()).await.expect("bounded preview");
        assert_eq!(input.content.text.as_deref(), if scope == PreviewScope::Card { Some(expected_card.as_str()) } else { content.text.as_deref() });
        assert_eq!(input.text_clipped, scope == PreviewScope::Card);
        assert_eq!(input.content.html, content.html);
        assert!(!input.html_omitted);
        assert!(input.content.rtf.is_none() && input.rtf_omitted);
        assert_eq!(input.content.files, content.files);
        drop(input);
        assert_eq!(memory.used(), 0);
    }
    assert_eq!(storage.content(record.id).await.expect("untouched originals").content, content);

    let id = record.id;
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET rtf=?2 WHERE record_id=?1", rusqlite::params![id.to_string(), format!("\0{}", "中".repeat(200_000))])?;
        Ok(())
    }).await.expect("source beyond the coarse byte bound");
    let small_memory = ByteBudget::new(500_000, "small preview regression");
    let input = storage.preview_input(entry.clone(), PreviewScope::Card, small_memory.clone(), &CancellationToken::new()).await.expect("oversized rich source excluded before admission");
    assert!(input.content.rtf.is_none() && input.rtf_omitted);
    drop(input);
    assert_eq!(small_memory.used(), 0);

    for sql in [
        "UPDATE payloads SET text=CAST(x'80' AS TEXT) WHERE record_id=?1",
        "UPDATE payloads SET text=x'61' WHERE record_id=?1",
        "UPDATE payloads SET text=CAST(zeroblob(59999)||x'f0' AS TEXT) WHERE record_id=?1",
    ] {
        storage.call(move |connection| { connection.execute(sql, [id.to_string()])?; Ok(()) }).await.expect("corrupt source");
        for scope in [PreviewScope::Card, PreviewScope::Details] {
            assert!(matches!(storage.preview_input(entry.clone(), scope, memory.clone(), &CancellationToken::new()).await, Err(Error::Database(_))));
            assert_eq!(memory.used(), 0);
        }
    }
    storage.call(move |connection| {
        connection.execute("UPDATE payloads SET text=NULL,html='',rtf=NULL WHERE record_id=?1", [id.to_string()])?;
        Ok(())
    }).await.expect("NULL and empty sources");
    let input = storage.preview_input(entry.clone(), PreviewScope::Card, memory.clone(), &CancellationToken::new()).await.expect("empty projection");
    assert!(input.content.text.is_none() && input.content.rtf.is_none());
    assert_eq!(input.content.html.as_deref(), Some(""));
    assert!(!input.html_omitted && !input.rtf_omitted && !input.text_clipped);
    drop(input);
    let mut stale = (*entry).clone(); stale.content_revision += 1;
    assert!(matches!(storage.preview_input(Arc::new(stale), PreviewScope::Card, memory.clone(), &CancellationToken::new()).await, Err(Error::Cancelled)));
    assert_eq!(memory.used(), 0);
    storage.close_for_test().await.expect("close isolated database");
}
