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

#[test]
fn test_dedupe_key_allows_reindexing_after_succeeded_within_24h() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("dedupe_test");
    fs::create_dir_all(&folder_path).unwrap();

    let file_path = folder_path.join("reindex.png");
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"version 1 content").unwrap();
    }

    let conn = setup_test_db();
    let folder_path_str = folder_path.to_string_lossy().to_string();
    conn.execute(
        "INSERT INTO folders (id, path, enabled, recursive) VALUES (1, ?1, 1, 1)",
        [&folder_path_str],
    )
    .unwrap();

    let file_str = file_path.to_string_lossy().to_string();

    // 1. Enqueue job for v1 and process to SUCCEEDED
    let hash_v1 = "hash_v1_content";
    let key_v1 = format!("UPSERT:1:{file_str}:{hash_v1}");
    let job1_id = jobs::enqueue_job(&conn, 1, &file_str, JOB_TYPE_UPSERT, &key_v1)
        .unwrap()
        .expect("Initial job should be enqueued");

    let claimed1 = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
    assert_eq!(claimed1.id, job1_id);
    let shot1_id = screenshots::insert_screenshot(
        &conn,
        1,
        &file_str,
        "reindex.png",
        "png",
        100,
        "2026-09-04T00:00:00Z",
        hash_v1,
    )
    .unwrap();
    screenshots::save_ocr_success(&conn, shot1_id, "text version one", "mock").unwrap();
    jobs::complete_job(&conn, job1_id, Some(shot1_id)).unwrap();

    // Verify job is SUCCEEDED and retained for 24h
    let stats = jobs::get_job_stats(&conn).unwrap();
    assert_eq!(stats.succeeded, 1);
    assert_eq!(stats.pending, 0);

    // 2. Modify same path to v2 within 24h -> new job must enqueue and complete
    {
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"version 2 modified content").unwrap();
    }
    let hash_v2 = "hash_v2_content";
    let key_v2 = format!("UPSERT:1:{file_str}:{hash_v2}");
    let job2_id = jobs::enqueue_job(&conn, 1, &file_str, JOB_TYPE_UPSERT, &key_v2)
        .unwrap()
        .expect(
            "Modified file v2 should successfully enqueue even while v1 is retained in SUCCEEDED",
        );

    let claimed2 = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
    assert_eq!(claimed2.id, job2_id);
    screenshots::update_screenshot(&conn, shot1_id, 200, "2026-09-04T01:00:00Z", hash_v2).unwrap();
    screenshots::save_ocr_success(&conn, shot1_id, "text version two", "mock").unwrap();
    jobs::complete_job(&conn, job2_id, Some(shot1_id)).unwrap();

    // Verify search matches v2 and not v1
    let res_v2 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "two".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_v2.total_matches, 1);
    let res_v1 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "one".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_v1.total_matches, 0);

    // 3. Modify same path BACK to v1 within 24h -> must safely re-open job and complete
    let job3_id = jobs::enqueue_job(&conn, 1, &file_str, JOB_TYPE_UPSERT, &key_v1)
        .unwrap()
        .expect("Modifying back to v1 should re-open previously SUCCEEDED job");

    assert_eq!(
        job3_id, job1_id,
        "Re-opened job should re-use existing job record ID via ON CONFLICT"
    );
    let claimed3 = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
    assert_eq!(claimed3.id, job1_id);
    screenshots::update_screenshot(&conn, shot1_id, 100, "2026-09-04T02:00:00Z", hash_v1).unwrap();
    screenshots::save_ocr_success(&conn, shot1_id, "text version one restored", "mock").unwrap();
    jobs::complete_job(&conn, job1_id, Some(shot1_id)).unwrap();

    let res_restored = search_screenshots(
        &conn,
        &SearchRequest {
            query: "restored".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_restored.total_matches, 1);

    // Verify exactly 1 screenshot record exists (no duplicate)
    let total_screenshots: i64 = conn
        .query_row("SELECT COUNT(*) FROM screenshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(total_screenshots, 1);
}

#[test]
fn test_rename_screenshot_inside_watched_folder() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("rename_test");
    fs::create_dir_all(&folder_path).unwrap();

    let old_file = folder_path.join("old_invoice.png");
    let new_file = folder_path.join("renamed_invoice.png");
    {
        let mut f = File::create(&old_file).unwrap();
        f.write_all(b"invoice document bytes").unwrap();
    }

    let conn = setup_test_db();
    let folder_path_str = folder_path.to_string_lossy().to_string();
    conn.execute(
        "INSERT INTO folders (id, path, enabled, recursive) VALUES (1, ?1, 1, 1)",
        [&folder_path_str],
    )
    .unwrap();

    let old_str = old_file.to_string_lossy().to_string();
    let new_str = new_file.to_string_lossy().to_string();

    // 1. Initial screenshot indexed
    let shot_id = screenshots::insert_screenshot(
        &conn,
        1,
        &old_str,
        "old_invoice.png",
        "png",
        1024,
        "2026-09-04T00:00:00Z",
        "hash_inv_123",
    )
    .unwrap();
    screenshots::save_ocr_success(&conn, shot_id, "Invoice #INV-2026-99 Paid in Full", "mock")
        .unwrap();

    // Verify initial search matches
    let res1 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "INV-2026-99".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res1.total_matches, 1);
    assert_eq!(res1.items[0].filename, "old_invoice.png");
    assert_eq!(res1.items[0].path, old_str);

    // 2. Perform rename on disk: old_invoice.png -> renamed_invoice.png
    fs::rename(&old_file, &new_file).unwrap();
    assert!(!old_file.exists());
    assert!(new_file.exists());

    // 3. Trigger atomic rename in database
    let renamed = screenshots::rename_screenshot(&conn, 1, &old_str, &new_str).unwrap();
    assert!(
        renamed,
        "rename_screenshot should return true for registered path"
    );

    // 4. Invariant checks:
    // A. No stale old path in database
    let old_lookup = screenshots::get_screenshot_by_path(&conn, &old_str).unwrap();
    assert!(
        old_lookup.is_none(),
        "Old path must no longer exist in screenshots table"
    );

    // B. No duplicate screenshot record (exactly 1 record, retaining original ID)
    let total_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM screenshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(
        total_count, 1,
        "Exactly 1 screenshot record must exist after rename"
    );

    let detail = screenshots::get_screenshot_by_id(&conn, shot_id)
        .unwrap()
        .expect("Original record ID must be preserved");
    assert_eq!(detail.id, shot_id);
    assert_eq!(detail.path, new_str);
    assert_eq!(detail.filename, "renamed_invoice.png");
    assert_eq!(detail.ocr_status, "SUCCEEDED");
    assert_eq!(
        detail.ocr_text.as_deref(),
        Some("Invoice #INV-2026-99 Paid in Full")
    );

    // C. FTS remains consistent and search reflects new filename with old OCR text
    let res2 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "INV-2026-99".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res2.total_matches, 1);
    assert_eq!(res2.items[0].filename, "renamed_invoice.png");
    assert_eq!(res2.items[0].path, new_str);

    // Also searchable by the new filename token
    let res_fn = search_screenshots(
        &conn,
        &SearchRequest {
            query: "renamed_invoice".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_fn.total_matches, 1);

    // D. Original file content on disk is untouched
    let on_disk_bytes = fs::read(&new_file).unwrap();
    assert_eq!(on_disk_bytes, b"invoice document bytes");
}

#[test]
fn test_pause_resume_accumulates_and_drains_backlog() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("pause_resume_test");
    fs::create_dir_all(&folder_path).unwrap();

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

    let engine = Arc::new(MockOcrEngine::new("processed after resume"));
    let watcher = WatcherManager::new(db.clone());
    let service = IndexingService::new(db.clone(), engine, watcher);

    // 1. Pause indexing BEFORE starting worker
    service.pause();
    assert!(service.is_paused());

    service.start(None);

    // 2. Add 2 screenshot files into the watched directory while paused
    for i in 1..=2 {
        let p = folder_path.join(format!("burst_{i}.png"));
        let mut f = File::create(&p).unwrap();
        f.write_all(format!("image bytes {i}").as_bytes()).unwrap();

        // Enqueue directly through service debouncer/queue mechanism
        let conn_guard = db.conn.lock().unwrap();
        let path_str = p.to_string_lossy().to_string();
        let key = format!("UPSERT:1:{path_str}:hash_{i}");
        jobs::enqueue_job(&conn_guard, 1, &path_str, JOB_TYPE_UPSERT, &key).unwrap();
    }

    // Wait 500ms while paused
    thread::sleep(Duration::from_millis(500));

    // 3. While paused: jobs must remain in index_jobs as PENDING, worker must claim 0 jobs
    {
        let conn_guard = db.conn.lock().unwrap();
        let stats = jobs::get_job_stats(&conn_guard).unwrap();
        assert_eq!(
            stats.pending, 2,
            "Backlog must accumulate in PENDING while paused"
        );
        assert_eq!(
            stats.processing, 0,
            "Worker must not claim jobs while paused"
        );
        assert_eq!(
            stats.succeeded, 0,
            "No jobs should be completed while paused"
        );
    }

    // 4. Resume indexing
    service.resume();
    assert!(!service.is_paused());

    // Wait up to 3 seconds for worker to drain the backlog
    let mut drained = false;
    for _ in 0..30 {
        thread::sleep(Duration::from_millis(100));
        let conn_guard = db.conn.lock().unwrap();
        let stats = jobs::get_job_stats(&conn_guard).unwrap();
        if stats.succeeded == 2 && stats.pending == 0 {
            drained = true;
            break;
        }
    }

    assert!(
        drained,
        "Worker must automatically drain accumulated backlog after resume"
    );

    // Verify search works for both items
    {
        let conn_guard = db.conn.lock().unwrap();
        let res = search_screenshots(
            &conn_guard,
            &SearchRequest {
                query: "processed".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res.total_matches, 2);
    }

    service.shutdown();
}

#[test]
fn test_watcher_and_startup_reconciliation_race_safety() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("race_safety_test");
    fs::create_dir_all(&folder_path).unwrap();

    let test_file = folder_path.join("concurrent_drop.png");
    {
        let mut f = File::create(&test_file).unwrap();
        f.write_all(b"race safety image content").unwrap();
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

    let engine = Arc::new(MockOcrEngine::new("token_race_safe"));
    let watcher = WatcherManager::new(db.clone());
    let service = IndexingService::new(db.clone(), engine, watcher);

    // Simulate concurrent discovery:
    // Thread A: Watcher debouncer enqueues the file
    // Thread B: Startup reconciliation scans the folder and enqueues the file
    let file_str = test_file.to_string_lossy().to_string();
    let key = format!("UPSERT:1:{file_str}:hash_shared");

    let db_a = db.clone();
    let file_a = file_str.clone();
    let key_a = key.clone();
    let handle_a = thread::spawn(move || {
        let conn_a = db_a.conn.lock().unwrap();
        jobs::enqueue_job(&conn_a, 1, &file_a, JOB_TYPE_UPSERT, &key_a)
    });

    let db_b = db.clone();
    let file_b = file_str.clone();
    let key_b = key.clone();
    let handle_b = thread::spawn(move || {
        let conn_b = db_b.conn.lock().unwrap();
        jobs::enqueue_job(&conn_b, 1, &file_b, JOB_TYPE_UPSERT, &key_b)
    });

    let res_a = handle_a.join().unwrap().unwrap();
    let res_b = handle_b.join().unwrap().unwrap();

    // Exactly one thread inserted a new job; the other was cleanly deduplicated
    let inserts = [res_a, res_b].iter().filter(|r| r.is_some()).count();
    assert_eq!(
        inserts, 1,
        "Exactly one thread should enqueue; concurrent duplicate must be rejected"
    );

    // Run worker for the job
    let conn_guard = db.conn.lock().unwrap();
    let job = jobs::claim_next_job(&conn_guard, 60).unwrap().unwrap();
    let mock_eng = MockOcrEngine::new("token_race_safe");
    let shot_id =
        crate::indexing::worker::run_indexing_worker_loop_step(&conn_guard, &mock_eng, &job)
            .unwrap();
    jobs::complete_job(&conn_guard, job.id, shot_id).unwrap();

    // Verify search has exactly 1 match
    let res = search_screenshots(
        &conn_guard,
        &SearchRequest {
            query: "token_race_safe".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res.total_matches, 1);

    // Verify screenshots table has exactly 1 row
    let count: i64 = conn_guard
        .query_row("SELECT COUNT(*) FROM screenshots", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);

    service.shutdown();
}

#[test]
fn test_burst_copy_fifty_plus_images() {
    let dir = tempdir().unwrap();
    let folder_path = dir.path().join("burst_folder");
    fs::create_dir_all(&folder_path).unwrap();

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

    let engine = Arc::new(MockOcrEngine::new("burst_token_found"));
    let watcher = WatcherManager::new(db.clone());
    let service = IndexingService::new(db.clone(), engine, watcher);

    service.start(None);

    // Burst copy 55 screenshots into the watched directory in a tight loop
    const BURST_COUNT: usize = 55;
    for i in 1..=BURST_COUNT {
        let p = folder_path.join(format!("burst_shot_{i:03}.png"));
        let mut f = File::create(&p).unwrap();
        f.write_all(format!("burst image content {i}").as_bytes())
            .unwrap();

        // Enqueue directly via queue to simulate watcher burst completion
        let conn_guard = db.conn.lock().unwrap();
        let path_str = p.to_string_lossy().to_string();
        let key = format!("UPSERT:1:{path_str}:hash_{i}");
        jobs::enqueue_job(&conn_guard, 1, &path_str, JOB_TYPE_UPSERT, &key).unwrap();
    }

    // Wait for the single-flight worker to drain all 55 jobs
    let mut all_completed = false;
    for _ in 0..100 {
        thread::sleep(Duration::from_millis(50));
        let conn_guard = db.conn.lock().unwrap();
        let stats = jobs::get_job_stats(&conn_guard).unwrap();
        if stats.succeeded == BURST_COUNT && stats.pending == 0 {
            all_completed = true;
            break;
        }
    }

    assert!(
        all_completed,
        "Worker must complete all 55 burst jobs without dropping or stalling"
    );

    // Verify search matches all 55 screenshots
    {
        let conn_guard = db.conn.lock().unwrap();
        let res = search_screenshots(
            &conn_guard,
            &SearchRequest {
                query: "burst_token".into(),
                limit: Some(100),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(res.total_matches, BURST_COUNT);
    }

    service.shutdown();
}
