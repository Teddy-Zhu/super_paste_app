use super::*;

struct Directory(std::path::PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) { eprintln!("failed to remove preview test directory: {error}"); }
    }
}

#[tokio::test]
async fn abandoned_blocking_parser_retains_admission_budget_and_shutdown_ownership() {
    let directory = Directory(std::env::temp_dir().join(format!("super-paste-rs-parser-{}", uuid::Uuid::new_v4())));
    std::fs::create_dir(&directory.0).expect("isolated parser directory");
    let storage = Storage::open(directory.0.join("parser.sqlite3")).await.expect("isolated database");
    let service = PreviewService::new(storage.clone(), Handle::current());
    let work = Arc::new(ParseWork {
        _permit: service.workers.clone().acquire_owned().await.expect("parser admission"),
        _task: service.tasks.token(),
    });
    let memory = ByteBudget::new(128, "blocking parser input");
    let lease = memory.reserve(128).expect("input lease");
    let (started, running) = tokio::sync::oneshot::channel();
    let (release, blocked) = std::sync::mpsc::channel();
    let cancel = CancellationToken::new();
    let caller = tokio::spawn({ let service = service.clone(); let cancel = cancel.clone(); async move {
        service.run_parser(work, &cancel, move || {
            let _input_lease = lease;
            let _ = started.send(());
            blocked.recv_timeout(std::time::Duration::from_secs(5)).expect("release native parser simulation");
            Ok(())
        }).await
    }});
    running.await.expect("blocking parser actually running");
    cancel.cancel(); caller.abort();
    assert!(caller.await.expect_err("async waiter abandoned").is_cancelled());
    assert_eq!(service.workers.available_permits(), 1, "running parser keeps its physical admission");
    assert_eq!(service.tasks.len(), 1, "running parser remains in shutdown tracking");
    assert_eq!(memory.used(), 128, "running parser keeps its actual input allocation charged");
    let shutdown = service.shutdown(); tokio::pin!(shutdown);
    assert!(futures_util::poll!(shutdown.as_mut()).is_pending(), "shutdown must wait for actual blocking completion");
    release.send(()).expect("release running parser");
    shutdown.await.expect("parser shutdown completed");
    assert!(service.tasks.is_empty()); assert_eq!(memory.used(), 0);
    storage.close_for_test().await.expect("close database");
}
