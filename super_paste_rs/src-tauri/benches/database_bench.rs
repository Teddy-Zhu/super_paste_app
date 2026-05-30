use criterion::{black_box, criterion_group, criterion_main, Criterion};
use rusqlite::Connection;

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
        text_content: Some(format!("测试内容 {} - 这是一段用于性能测试的文本", id)),
        html_content: None,
        rtf_content: None,
        image_data: None,
        files_json: None,
        metadata_json: None,
        compression_type: None,
    };

    (item, content)
}

fn benchmark_insert(c: &mut Criterion) {
    c.bench_function("insert_single", |b| {
        b.iter_batched(
            || {
                let conn = setup_test_db();
                let (item, content) = create_test_item(1);
                (conn, item, content)
            },
            |(conn, item, content)| ClipboardRepository::insert(&conn, &item, &content).unwrap(),
            criterion::BatchSize::SmallInput,
        );
    });
}

fn benchmark_query(c: &mut Criterion) {
    let conn = setup_test_db();
    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }

    c.bench_function("list_50_items", |b| {
        b.iter(|| {
            let options = QueryOptions {
                offset: 0,
                limit: 50,
                ..Default::default()
            };
            ClipboardRepository::list(&conn, &options).unwrap()
        });
    });
}

fn benchmark_search(c: &mut Criterion) {
    let conn = setup_test_db();
    for i in 0..1000 {
        let (item, content) = create_test_item(i);
        ClipboardRepository::insert(&conn, &item, &content).unwrap();
    }

    c.bench_function("search_1000_items", |b| {
        b.iter(|| ClipboardRepository::search(&conn, black_box("测试"), 50).unwrap());
    });
}

criterion_group!(benches, benchmark_insert, benchmark_query, benchmark_search);
criterion_main!(benches);
