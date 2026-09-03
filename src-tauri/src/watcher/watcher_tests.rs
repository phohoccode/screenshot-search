use notify::event::{CreateKind, ModifyKind, RemoveKind, RenameMode};
use notify::{Event, EventKind};
use rusqlite::Connection;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

use super::debouncer::check_file_stability;
use super::event::{
    is_supported_screenshot_path, is_temporary_file, normalize_notify_event, NormalizedFsEvent,
};
use super::WatcherManager;
use crate::db::connection::Database;
use crate::db::jobs::{self, JOB_TYPE_DELETE, JOB_TYPE_UPSERT};
use crate::db::screenshots;
use crate::indexing::service::IndexingService;
use crate::ocr::mock::MockOcrEngine;
use crate::search::query::{search_screenshots, SearchRequest};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    crate::db::migrations::run_migrations(&conn).unwrap();
    conn
}

#[test]
fn test_event_normalization_and_filtering() {
    let p_png = PathBuf::from("C:\\Screenshots\\test.png");
    let p_jpg = PathBuf::from("C:\\Screenshots\\image.jpg");
    let p_webp = PathBuf::from("C:\\Screenshots\\photo.webp");
    let p_txt = PathBuf::from("C:\\Screenshots\\notes.txt");
    let p_tmp = PathBuf::from("C:\\Screenshots\\temp.tmp");
    let p_crdownload = PathBuf::from("C:\\Screenshots\\shot.png.crdownload");
    let p_hidden = PathBuf::from("C:\\Screenshots\\.DS_Store");

    // Extensions & temporary file check
    assert!(is_supported_screenshot_path(&p_png));
    assert!(is_supported_screenshot_path(&p_jpg));
    assert!(is_supported_screenshot_path(&p_webp));
    assert!(!is_supported_screenshot_path(&p_txt));
    assert!(!is_supported_screenshot_path(&p_tmp));
    assert!(!is_supported_screenshot_path(&p_crdownload));
    assert!(!is_supported_screenshot_path(&p_hidden));
    assert!(is_temporary_file(&p_tmp));
    assert!(is_temporary_file(&p_crdownload));

    // 1. Create event for valid image
    let ev_create = Event {
        kind: EventKind::Create(CreateKind::File),
        paths: vec![p_png.clone(), p_txt.clone()],
        attrs: Default::default(),
    };
    let norm_create = normalize_notify_event(1, &ev_create);
    assert_eq!(norm_create.len(), 1);
    assert!(matches!(
        norm_create[0],
        NormalizedFsEvent::Upsert { folder_id: 1, .. }
    ));

    // 2. Rename event: valid -> valid
    let ev_rename = Event {
        kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        paths: vec![p_png.clone(), p_jpg.clone()],
        attrs: Default::default(),
    };
    let norm_rename = normalize_notify_event(1, &ev_rename);
    assert_eq!(norm_rename.len(), 1);
    assert!(matches!(
        norm_rename[0],
        NormalizedFsEvent::Rename { folder_id: 1, .. }
    ));

    // 3. Rename event: temp -> valid image (common screenshot save pattern)
    let ev_rename_temp = Event {
        kind: EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
        paths: vec![p_tmp.clone(), p_png.clone()],
        attrs: Default::default(),
    };
    let norm_rename_temp = normalize_notify_event(1, &ev_rename_temp);
    assert_eq!(norm_rename_temp.len(), 1);
    assert!(matches!(
        norm_rename_temp[0],
        NormalizedFsEvent::Upsert { folder_id: 1, .. }
    ));

    // 4. Remove event
    let ev_remove = Event {
        kind: EventKind::Remove(RemoveKind::File),
        paths: vec![p_png],
        attrs: Default::default(),
    };
    let norm_remove = normalize_notify_event(1, &ev_remove);
    assert_eq!(norm_remove.len(), 1);
    assert!(matches!(
        norm_remove[0],
        NormalizedFsEvent::Remove { folder_id: 1, .. }
    ));
}

#[test]
fn test_file_stability_check() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("stable.png");
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"fake png content").unwrap();
    }

    // Static file should be verified stable
    let stable = check_file_stability(&file_path).unwrap();
    assert!(stable);

    // Non-existent file
    let non_existent = dir.path().join("deleted.png");
    let missing_stable = check_file_stability(&non_existent).unwrap();
    assert!(!missing_stable);
}

#[test]
fn test_durable_queue_lifecycle() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    // 1. Enqueue job
    let path = "C:\\Screenshots\\card.png";
    let dedupe_key = "UPSERT:1:card.png:hash123";
    let job_id = jobs::enqueue_job(&conn, 1, path, JOB_TYPE_UPSERT, dedupe_key)
        .unwrap()
        .unwrap();

    // 2. Verify deduplication: second enqueue with same dedupe_key returns None
    let duplicate = jobs::enqueue_job(&conn, 1, path, JOB_TYPE_UPSERT, dedupe_key).unwrap();
    assert_eq!(duplicate, None);

    // Verify stats
    let stats = jobs::get_job_stats(&conn).unwrap();
    assert_eq!(stats.pending, 1);
    assert_eq!(stats.processing, 0);

    // 3. Atomically claim job with lease
    let claimed = jobs::claim_next_job(&conn, 60)
        .unwrap()
        .expect("Should claim pending job");
    assert_eq!(claimed.id, job_id);
    assert_eq!(claimed.status, "PROCESSING");
    assert!(claimed.lease_until.is_some());

    // 4. Complete job
    jobs::complete_job(&conn, job_id, None).unwrap();
    let stats_after = jobs::get_job_stats(&conn).unwrap();
    assert_eq!(stats_after.pending, 0);
    assert_eq!(stats_after.processing, 0);
    assert_eq!(stats_after.succeeded, 1);

    // 5. Cleanup completed jobs
    // Fast forward completed_at to test retention
    conn.execute(
        "UPDATE index_jobs SET completed_at = datetime('now', '-48 hours') WHERE id = ?1",
        [job_id],
    )
    .unwrap();
    let cleaned = jobs::cleanup_completed_jobs(&conn, 24).unwrap();
    assert_eq!(cleaned, 1);

    let stats_final = jobs::get_job_stats(&conn).unwrap();
    assert_eq!(stats_final.total, 0);
}

#[test]
fn test_concurrent_job_claims_exact_one_winner() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let dir = tempdir().unwrap();
    let db_path = dir.path().join("test_concurrent.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    crate::db::migrations::run_migrations(&conn).unwrap();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    // Enqueue single job
    let job_id = jobs::enqueue_job(&conn, 1, "C:\\test.png", JOB_TYPE_UPSERT, "DEDUPE:1")
        .unwrap()
        .unwrap();
    drop(conn);

    let winners = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::new();

    // Spawn 4 concurrent worker threads attempting to claim the exact same single job
    for _ in 0..4 {
        let p = db_path.clone();
        let w = winners.clone();
        handles.push(thread::spawn(move || {
            let thread_conn = Connection::open(&p).unwrap();
            if let Ok(Some(job)) = jobs::claim_next_job(&thread_conn, 60) {
                if job.id == job_id {
                    w.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Invariant: Exactly one worker wins the atomic claim
    assert_eq!(winners.load(Ordering::SeqCst), 1);
}

#[test]
fn test_crash_recovery_stale_leases() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    // Insert a simulated job that was PROCESSING when the app crashed and lease expired
    conn.execute(
        "INSERT INTO index_jobs (folder_id, path, job_type, dedupe_key, status, lease_until)
         VALUES (1, 'C:\\Screenshots\\crashed.png', 'UPSERT_SCREENSHOT', 'DEDUPE:CRASH', 'PROCESSING', datetime('now', '-10 seconds'))",
        [],
    ).unwrap();

    let recovered = jobs::recover_stale_leases(&conn).unwrap();
    assert_eq!(recovered, 1);

    // Verify status is now PENDING and ready for worker claim
    let stats = jobs::get_job_stats(&conn).unwrap();
    assert_eq!(stats.pending, 1);
    assert_eq!(stats.processing, 0);
}

#[test]
fn test_modify_pipeline_invalidates_old_search() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("screenshots");
    fs::create_dir_all(&folder_path).unwrap();

    let file_path = folder_path.join("error_log.png");
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"initial image bytes").unwrap();
    }

    let conn = setup_test_db();
    let folder_path_str = folder_path.to_string_lossy().to_string();
    conn.execute(
        "INSERT INTO folders (id, path, enabled, recursive) VALUES (1, ?1, 1, 1)",
        [&folder_path_str],
    )
    .unwrap();

    let db = Database {
        conn: Arc::new(std::sync::Mutex::new(conn)),
    };

    // Use mock OCR returning "P2028 Transaction closed" for initial version
    let engine = Arc::new(MockOcrEngine::new("P2028 Transaction closed"));
    let watcher = WatcherManager::new(db.clone());
    let service = IndexingService::new(db.clone(), engine.clone(), watcher);

    // Enqueue initial job and run one worker step
    let conn_guard = db.conn.lock().unwrap();
    let file_str = file_path.to_string_lossy().to_string();
    jobs::enqueue_job(&conn_guard, 1, &file_str, JOB_TYPE_UPSERT, "KEY:1").unwrap();
    drop(conn_guard);

    // Run worker for 1 job
    let conn_guard = db.conn.lock().unwrap();
    let job = jobs::claim_next_job(&conn_guard, 60).unwrap().unwrap();
    let id =
        crate::indexing::worker::run_indexing_worker_loop_step(&conn_guard, engine.as_ref(), &job)
            .unwrap();
    jobs::complete_job(&conn_guard, job.id, id).unwrap();

    // Verify search matches "P2028"
    let res = search_screenshots(
        &conn_guard,
        &SearchRequest {
            query: "P2028".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res.total_matches, 1);
    drop(conn_guard);

    // 2. Modify screenshot content
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"completely modified image content with new errors")
            .unwrap();
    }

    // New mock OCR returns "ERR_MODULE_NOT_FOUND"
    let new_engine = Arc::new(MockOcrEngine::new("ERR_MODULE_NOT_FOUND in package"));

    // Enqueue modify job
    let conn_guard = db.conn.lock().unwrap();
    jobs::enqueue_job(&conn_guard, 1, &file_str, JOB_TYPE_UPSERT, "KEY:2").unwrap();
    let mod_job = jobs::claim_next_job(&conn_guard, 60).unwrap().unwrap();
    let new_id = crate::indexing::worker::run_indexing_worker_loop_step(
        &conn_guard,
        new_engine.as_ref(),
        &mod_job,
    )
    .unwrap();
    jobs::complete_job(&conn_guard, mod_job.id, new_id).unwrap();

    // Verify old search query "P2028" ceases to match!
    let old_res = search_screenshots(
        &conn_guard,
        &SearchRequest {
            query: "P2028".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(old_res.total_matches, 0);

    // Verify new search query "ERR_MODULE_NOT_FOUND" matches!
    let new_res = search_screenshots(
        &conn_guard,
        &SearchRequest {
            query: "ERR_MODULE_NOT_FOUND".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(new_res.total_matches, 1);

    service.shutdown();
}

#[test]
fn test_delete_pipeline_sync() {
    let dir = tempdir().unwrap();
    let file_path = dir.path().join("to_delete.png");
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"some image").unwrap();
    }

    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    let file_str = file_path.to_string_lossy().to_string();
    let shot_id = screenshots::insert_screenshot(
        &conn,
        1,
        &file_str,
        "to_delete.png",
        "png",
        100,
        "2026-09-03T12:00:00Z",
        "hash1",
    )
    .unwrap();
    screenshots::save_ocr_success(&conn, shot_id, "Temporary code token", "mock").unwrap();

    // Verify initial search match
    let res = search_screenshots(
        &conn,
        &SearchRequest {
            query: "Temporary".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res.total_matches, 1);

    // Delete file from disk
    fs::remove_file(&file_path).unwrap();

    // Enqueue and process delete job
    jobs::enqueue_job(&conn, 1, &file_str, JOB_TYPE_DELETE, "DEL:1").unwrap();
    let del_job = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
    let mock_engine = MockOcrEngine::new("");
    crate::indexing::worker::run_indexing_worker_loop_step(&conn, &mock_engine, &del_job).unwrap();
    jobs::complete_job(&conn, del_job.id, None).unwrap();

    // Verify search match is gone and metadata removed
    let res_after = search_screenshots(
        &conn,
        &SearchRequest {
            query: "Temporary".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_after.total_matches, 0);

    let detail = screenshots::get_screenshot_by_id(&conn, shot_id).unwrap();
    assert!(detail.is_none());
}

#[test]
fn test_startup_reconciliation_offline_changes() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("offline_test");
    fs::create_dir_all(&folder_path).unwrap();

    // Create 3 screenshot files in the directory
    for i in 1..=3 {
        let p = folder_path.join(format!("shot_{i}.png"));
        let mut f = File::create(p).unwrap();
        f.write_all(format!("content {i}").as_bytes()).unwrap();
    }

    let conn = setup_test_db();
    let folder_path_str = folder_path.to_string_lossy().to_string();
    conn.execute(
        "INSERT INTO folders (id, path, enabled, recursive) VALUES (1, ?1, 1, 1)",
        [&folder_path_str],
    )
    .unwrap();

    let db = Database {
        conn: Arc::new(std::sync::Mutex::new(conn)),
    };

    let engine = Arc::new(MockOcrEngine::new("offline text"));
    let watcher = WatcherManager::new(db.clone());
    let service = IndexingService::new(db.clone(), engine, watcher);

    // Run startup reconciliation (simulating app start with 3 new files)
    service.run_startup_reconciliation();

    // Verify that all 3 screenshots were discovered, inserted into screenshots with PENDING,
    // and enqueued into index_jobs table!
    let conn_guard = db.conn.lock().unwrap();
    let count: i64 = conn_guard
        .query_row("SELECT COUNT(*) FROM screenshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 3);

    let job_stats = jobs::get_job_stats(&conn_guard).unwrap();
    assert_eq!(job_stats.pending, 3);

    service.shutdown();
}

#[test]
fn test_full_automatic_pipeline_e2e() {
    // End-to-end integration test with real Windows OCR
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("watched_folder");
    fs::create_dir_all(&folder_path).unwrap();

    // Copy fixture image into watched folder
    let fixture_src = Path::new("tests/fixtures/english.png");
    assert!(fixture_src.exists(), "English fixture must exist");
    let target_file = folder_path.join("english_copy.png");
    fs::copy(fixture_src, &target_file).unwrap();

    let conn = setup_test_db();
    let folder_path_str = folder_path.to_string_lossy().to_string();
    conn.execute(
        "INSERT INTO folders (id, path, enabled, recursive) VALUES (1, ?1, 1, 1)",
        [&folder_path_str],
    )
    .unwrap();

    let db = Database {
        conn: Arc::new(std::sync::Mutex::new(conn)),
    };

    #[cfg(target_os = "windows")]
    let engine: Arc<dyn crate::ocr::engine::OcrEngine> =
        Arc::new(crate::ocr::windows::WindowsMediaOcrEngine::new());
    #[cfg(not(target_os = "windows"))]
    let engine: Arc<dyn crate::ocr::engine::OcrEngine> =
        Arc::new(MockOcrEngine::new("Screenshot Search OCR"));

    let watcher = WatcherManager::new(db.clone());
    let service = IndexingService::new(db.clone(), engine.clone(), watcher);

    // Start service (which launches worker + startup reconciliation + watcher)
    service.start(None);

    // Wait up to 5 seconds for automatic discovery, OCR, and FTS sync
    let mut found = false;
    for _ in 0..50 {
        thread::sleep(Duration::from_millis(100));
        let conn_guard = db.conn.lock().unwrap();
        if let Ok(res) = search_screenshots(
            &conn_guard,
            &SearchRequest {
                query: "Screenshot Search".to_string(),
                ..Default::default()
            },
        ) {
            if res.total_matches > 0 {
                found = true;
                break;
            }
        }
    }

    // Invariant: User query finds screenshot WITHOUT manual Rescan or Start OCR!
    assert!(
        found,
        "Full automatic pipeline should index and make screenshot searchable without user action"
    );

    service.shutdown();
}
