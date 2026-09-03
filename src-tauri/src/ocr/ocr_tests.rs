#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use crate::db::connection::Database;
    use crate::db::screenshots::{self, get_ocr_stats, insert_screenshot, update_screenshot};
    use crate::ocr::mock::MockOcrEngine;
    use crate::ocr::normalize::normalize_ocr_text;
    use crate::ocr::orchestrator::run_ocr_batch;

    fn setup_test_db() -> Database {
        let conn = Connection::open_in_memory().expect("Failed to open in-memory database");
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             CREATE TABLE folders (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 path TEXT NOT NULL UNIQUE,
                 enabled INTEGER NOT NULL DEFAULT 1,
                 recursive INTEGER NOT NULL DEFAULT 1,
                 created_at TEXT NOT NULL DEFAULT (datetime('now')),
                 updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                 last_scanned_at TEXT
             );
             CREATE TABLE screenshots (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                 path TEXT NOT NULL,
                 filename TEXT NOT NULL,
                 extension TEXT NOT NULL,
                 file_size INTEGER NOT NULL,
                 modified_at_fs TEXT NOT NULL,
                 content_hash TEXT,
                 width INTEGER,
                 height INTEGER,
                 ocr_text TEXT,
                 ocr_status TEXT NOT NULL DEFAULT 'PENDING',
                 ocr_engine TEXT,
                 indexed_at TEXT,
                 created_at TEXT NOT NULL DEFAULT (datetime('now')),
                 updated_at TEXT NOT NULL DEFAULT (datetime('now'))
             );
             CREATE UNIQUE INDEX idx_screenshots_path ON screenshots(path);
             CREATE INDEX idx_screenshots_ocr_status ON screenshots(ocr_status);
             
             INSERT INTO folders (path) VALUES ('C:\\Screenshots');",
        )
        .expect("Failed to setup test database schema");

        Database {
            conn: std::sync::Mutex::new(conn),
        }
    }

    #[test]
    fn test_text_normalization_preserves_technical_tokens() {
        let raw = "  P2028: Transaction API error\r\n\r\n\r\nERR_CONNECTION_REFUSED at https://example.com/api?id=42  \n\nconst obj = { key: \"value\" };\0\x01\t";
        let normalized = normalize_ocr_text(raw);

        // Technical tokens must be preserved
        assert!(normalized.contains("P2028"));
        assert!(normalized.contains("ERR_CONNECTION_REFUSED"));
        assert!(normalized.contains("https://example.com/api?id=42"));
        assert!(normalized.contains("const obj = { key: \"value\" };"));
        // Control characters must be stripped
        assert!(!normalized.contains('\0'));
        assert!(!normalized.contains('\x01'));
        // CRLF normalized to LF
        assert!(!normalized.contains("\r\n"));
        // Multiple empty lines collapsed (at most one blank line)
        assert!(!normalized.contains("\n\n\n"));
    }

    #[test]
    fn test_successful_ocr_flow() {
        let db = setup_test_db();
        let engine = MockOcrEngine::new("Error: P2028 Transaction failed");

        // Insert pending screenshot
        {
            let conn = db.conn.lock().unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\error.png",
                "error.png",
                "png",
                1024,
                "2026-09-03T10:00:00Z",
                "hash123",
            )
            .unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let summary = run_ocr_batch(&db, &engine, None, None, cancel_flag, None).unwrap();

        assert_eq!(summary.total_candidates, 1);
        assert_eq!(summary.processed, 1);
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 0);

        // Check SQLite state
        let conn = db.conn.lock().unwrap();
        let stats = get_ocr_stats(&conn).unwrap();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.succeeded, 1);
        assert_eq!(stats.pending, 0);

        // Verify stored text
        let ocr_text: String = conn
            .query_row("SELECT ocr_text FROM screenshots WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ocr_text, "Error: P2028 Transaction failed");
    }

    #[test]
    fn test_failure_isolation() {
        let db = setup_test_db();
        let engine = MockOcrEngine::new("Normal text");
        engine.add_failing_path("C:\\Screenshots\\corrupt.png");

        {
            let conn = db.conn.lock().unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\corrupt.png",
                "corrupt.png",
                "png",
                50,
                "time1",
                "h1",
            )
            .unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\valid.png",
                "valid.png",
                "png",
                100,
                "time2",
                "h2",
            )
            .unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let summary = run_ocr_batch(&db, &engine, None, None, cancel_flag, None).unwrap();

        assert_eq!(summary.total_candidates, 2);
        assert_eq!(summary.processed, 2);
        assert_eq!(summary.succeeded, 1);
        assert_eq!(summary.failed, 1);

        let conn = db.conn.lock().unwrap();
        let stats = get_ocr_stats(&conn).unwrap();
        assert_eq!(stats.succeeded, 1);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn test_empty_ocr_text_handling() {
        let db = setup_test_db();
        let engine = MockOcrEngine::new("");
        engine.add_empty_path("C:\\Screenshots\\wallpaper.png");

        {
            let conn = db.conn.lock().unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\wallpaper.png",
                "wallpaper.png",
                "png",
                200,
                "time1",
                "h1",
            )
            .unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let summary = run_ocr_batch(&db, &engine, None, None, cancel_flag, None).unwrap();

        // Must succeed with empty text to avoid re-processing loops
        assert_eq!(summary.succeeded, 1);

        let conn = db.conn.lock().unwrap();
        let ocr_text: String = conn
            .query_row("SELECT ocr_text FROM screenshots WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ocr_text, "");
    }

    #[test]
    fn test_unchanged_succeeded_screenshot_not_reprocessed() {
        let db = setup_test_db();
        let engine = MockOcrEngine::new("Extracted text");

        {
            let conn = db.conn.lock().unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\test.png",
                "test.png",
                "png",
                100,
                "time1",
                "h1",
            )
            .unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        // First batch processes the image
        let summary1 = run_ocr_batch(&db, &engine, None, None, cancel_flag.clone(), None).unwrap();
        assert_eq!(summary1.succeeded, 1);

        // Second batch: No pending items exist
        let summary2 = run_ocr_batch(&db, &engine, None, None, cancel_flag, None).unwrap();
        assert_eq!(summary2.total_candidates, 0);
        assert_eq!(summary2.processed, 0);
    }

    #[test]
    fn test_changed_screenshot_reprocessed() {
        let db = setup_test_db();
        let engine = MockOcrEngine::new("Initial text");

        {
            let conn = db.conn.lock().unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\mutated.png",
                "mutated.png",
                "png",
                100,
                "time1",
                "h1",
            )
            .unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        run_ocr_batch(&db, &engine, None, None, cancel_flag.clone(), None).unwrap();

        // Simulate file modification from Phase 1B (resetting to PENDING)
        {
            let conn = db.conn.lock().unwrap();
            update_screenshot(&conn, 1, 150, "time2", "h2").unwrap();
        }

        // New text in modified file
        engine.set_default_text("Updated new text P2028");
        let summary = run_ocr_batch(&db, &engine, None, None, cancel_flag, None).unwrap();
        assert_eq!(summary.total_candidates, 1);
        assert_eq!(summary.succeeded, 1);

        let conn = db.conn.lock().unwrap();
        let ocr_text: String = conn
            .query_row("SELECT ocr_text FROM screenshots WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(ocr_text, "Updated new text P2028");
    }

    #[test]
    fn test_stale_processing_recovery() {
        let db = setup_test_db();

        // Insert item and leave it in PROCESSING state (simulating abrupt app exit)
        {
            let conn = db.conn.lock().unwrap();
            let id = insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\stale.png",
                "stale.png",
                "png",
                100,
                "time1",
                "h1",
            )
            .unwrap();
            screenshots::mark_processing(&conn, id).unwrap();

            let stats_before = get_ocr_stats(&conn).unwrap();
            assert_eq!(stats_before.processing, 1);
            assert_eq!(stats_before.pending, 0);

            // Trigger recovery
            let recovered = screenshots::recover_stale_processing(&conn).unwrap();
            assert_eq!(recovered, 1);

            let stats_after = get_ocr_stats(&conn).unwrap();
            assert_eq!(stats_after.processing, 0);
            assert_eq!(stats_after.pending, 1);
        }
    }
}
