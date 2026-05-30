use rusqlite::Connection;
use std::time::Instant;

use super_paste_rs_lib::infrastructure::database::migrations::MigrationManager;
use super_paste_rs_lib::infrastructure::database::repositories::ClipboardRepository;
use super_paste_rs_lib::types::clipboard::{ClipboardContent, ClipboardItem, QueryOptions};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    MigrationManager::run_migrations(&conn).unwrap();
    conn
}

fn create_test_item(id: usize) -> (ClipboardItem, ClipboardContent) {
    let content_id = format!("test-{}", id);
    let now = chrono::Utc::now().timestamp();

    let item = ClipboardItem {
        id: None,
        content_id: content_id.clone(),
        content_hash: format!("hash-{}", id),
        primary_type: "text".to_string(),
        data_size: 100,
        available_formats: vec!["text".to_string()],
        source_app_id: None,
        is_pinned: false,
        is_favorite: false,
        created_at: now,
        last_used_at: now,
        use_count: 1,
    };

    let content = ClipboardContent {
        content_id,
        text_content: Some(format!("测试内容 {}", id)),
        html_content: None,
        rtf_content: None,
        image_data: None,
        files_json: None,
        metadata_json: None,
        compression_type: None,
    };

    (item, content)
}

#[test]
fn test_search_performance() {
    let conn = setup_test_db();

    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }

    let start = Instant::now();
    let results = ClipboardRepository::search(&conn, "测试", 50).unwrap();
    let duration = start.elapsed();

    println!("搜索 1000 条记录耗时: {:?}", duration);
    assert!(duration.as_millis() < 100, "搜索性能未达标: {:?}", duration);
    assert!(!results.is_empty(), "搜索结果为空");
}

#[test]
fn test_insert_performance() {
    let conn = setup_test_db();
    let (item, content) = create_test_item(1);

    let start = Instant::now();
    ClipboardRepository::insert(&conn, &item, &content).unwrap();
    let duration = start.elapsed();

    println!("插入单条记录耗时: {:?}", duration);
    assert!(duration.as_millis() < 10, "插入性能未达标: {:?}", duration);
}

#[test]
fn test_query_performance() {
    let conn = setup_test_db();

    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }

    let options = QueryOptions {
        offset: 0,
        limit: 50,
        ..Default::default()
    };

    let start = Instant::now();
    let results = ClipboardRepository::list(&conn, &options).unwrap();
    let duration = start.elapsed();

    println!("分页查询 50 条耗时: {:?}", duration);
    assert!(duration.as_millis() < 50, "查询性能未达标: {:?}", duration);
    assert_eq!(results.len(), 50, "查询结果数量不正确");
}
