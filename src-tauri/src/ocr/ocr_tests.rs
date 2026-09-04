#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use crate::db::connection::Database;
    use crate::db::screenshots::{self, get_ocr_stats, insert_screenshot, update_screenshot};
    use crate::ocr::mock::MockOcrEngine;
    use crate::ocr::normalize::normalize_ocr_text;
    use crate::ocr::orchestrator::{run_ocr_batch, OcrManager};
    use crate::ocr::windows::calculate_downscaled_dimensions;

    fn setup_test_db() -> Database {
        let conn = Connection::open_in_memory().expect("Failed to open in-memory database");
        crate::db::migrations::run_migrations(&conn).expect("Failed to run migrations");
        conn.execute("INSERT INTO folders (path) VALUES ('C:\\Screenshots')", [])
            .expect("Failed to insert initial test folder");

        Database {
            conn: Arc::new(std::sync::Mutex::new(conn)),
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
    fn test_aspect_ratio_downscaling_math() {
        // Case 1: Standard 1080p within limit (2600) -> untouched
        let (w1, h1) = calculate_downscaled_dimensions(1920, 1080, 2600);
        assert_eq!(w1, 1920);
        assert_eq!(h1, 1080);

        // Case 2: 4K screenshot (3840x2160) -> downscaled to max_dim 2600, preserving aspect ratio
        let (w2, h2) = calculate_downscaled_dimensions(3840, 2160, 2600);
        assert_eq!(w2, 2600);
        assert_eq!(h2, 1463);
        let ratio_orig = 3840.0 / 2160.0;
        let ratio_scaled = w2 as f64 / h2 as f64;
        assert!((ratio_orig - ratio_scaled).abs() < 0.01);

        // Case 3: Ultra-wide screenshot (5120x1440) -> downscaled
        let (w3, h3) = calculate_downscaled_dimensions(5120, 1440, 2600);
        assert_eq!(w3, 2600);
        assert_eq!(h3, 731);

        // Case 4: Long vertical scrolling page screenshot (1080x5200) -> height scaled down
        let (w4, h4) = calculate_downscaled_dimensions(1080, 5200, 2600);
        assert_eq!(w4, 540);
        assert_eq!(h4, 2600);
    }

    #[test]
    fn test_conditional_mark_processing_prevents_duplicate_claims() {
        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let id = insert_screenshot(
            &conn,
            1,
            "C:\\Screenshots\\claim.png",
            "claim.png",
            "png",
            100,
            "time",
            "hash",
        )
        .unwrap();

        // First worker claims: must succeed
        let claim1 = screenshots::mark_processing(&conn, id).unwrap();
        assert!(
            claim1,
            "First worker should successfully claim the PENDING item"
        );

        // Second worker attempts to claim same screenshot: must return false
        let claim2 = screenshots::mark_processing(&conn, id).unwrap();
        assert!(
            !claim2,
            "Second worker must not claim an already PROCESSING item"
        );
    }

    #[test]
    fn test_concurrent_start_single_flight_protection() {
        let mgr = OcrManager::new();

        // First attempt acquires the running guard
        let guard1 = mgr.acquire_running_guard();
        assert!(guard1.is_some(), "First start attempt should succeed");
        assert!(mgr.is_active());

        // Simultaneous second attempt fails
        let guard2 = mgr.acquire_running_guard();
        assert!(
            guard2.is_none(),
            "Second start attempt must be rejected while first is active"
        );

        // When guard1 is dropped, lock resets automatically via RAII
        drop(guard1);
        assert!(!mgr.is_active());

        // Third attempt now succeeds
        let guard3 = mgr.acquire_running_guard();
        assert!(
            guard3.is_some(),
            "After guard is dropped, new batch can be started"
        );
    }

    #[test]
    fn test_cancellation_preserves_consistent_state() {
        let db = setup_test_db();
        let engine = MockOcrEngine::new("Text");

        {
            let conn = db.conn.lock().unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\1.png",
                "1.png",
                "png",
                100,
                "t1",
                "h1",
            )
            .unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\2.png",
                "2.png",
                "png",
                100,
                "t2",
                "h2",
            )
            .unwrap();
            insert_screenshot(
                &conn,
                1,
                "C:\\Screenshots\\3.png",
                "3.png",
                "png",
                100,
                "t3",
                "h3",
            )
            .unwrap();
        }

        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_flag_clone = cancel_flag.clone();

        // Progress callback cancels after the first item finishes
        let callback = move |_total: usize, processed: usize, _succeeded: usize, _failed: usize| {
            if processed == 1 {
                cancel_flag_clone.store(true, Ordering::SeqCst);
            }
        };

        let summary =
            run_ocr_batch(&db, &engine, None, None, cancel_flag, Some(&callback)).unwrap();

        // Only 1 was processed before cancel took effect
        assert_eq!(summary.processed, 1);
        assert_eq!(summary.succeeded, 1);

        // Verify remaining items are still PENDING and NOT stuck in PROCESSING
        let conn = db.conn.lock().unwrap();
        let stats = get_ocr_stats(&conn).unwrap();
        assert_eq!(stats.succeeded, 1);
        assert_eq!(stats.pending, 2);
        assert_eq!(stats.processing, 0);
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

    #[test]
    #[cfg(target_os = "windows")]
    fn test_windows_media_ocr_engine_creation() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;
        let engine = WindowsMediaOcrEngine::new();
        let info = engine.get_info();
        println!("WINRT OCR ENGINE INFO: {:?}", info);
        assert_eq!(info.engine_name, "windows_media_ocr");
        assert!(info.max_image_dimension > 0);
    }

    #[test]
    fn test_oversized_strategy_math() {
        use crate::ocr::windows::{determine_processing_strategy, OcrProcessingStrategy};

        // 1. Normal image: 1920x1080
        let s_1080p = determine_processing_strategy(1920, 1080, 10000);
        assert_eq!(s_1080p, OcrProcessingStrategy::Direct);

        // 2. 4K 3840x2160: Direct if within runtime 10000, Downscaled if constrained to 2600
        let s_4k_native = determine_processing_strategy(3840, 2160, 10000);
        assert_eq!(s_4k_native, OcrProcessingStrategy::Direct);

        let s_4k_constrained = determine_processing_strategy(3840, 2160, 2600);
        match s_4k_constrained {
            OcrProcessingStrategy::ProportionalDownscale {
                target_width,
                target_height,
            } => {
                assert_eq!(target_width, 2600);
                assert_eq!(target_height, 1463);
            }
            other => panic!(
                "Expected ProportionalDownscale for constrained 4K, got {:?}",
                other
            ),
        }

        // 3. Super Ultra-wide 5120x1440 (aspect ratio 3.56 > 2.0)
        let s_wide = determine_processing_strategy(5120, 1440, 10000);
        match s_wide {
            OcrProcessingStrategy::HorizontalTiling { tiles } => {
                assert_eq!(tiles.len(), 3);
                assert_eq!(tiles[0].x, 0);
                assert_eq!(tiles[0].width, 2000);
                assert_eq!(tiles[1].x, 1850);
                assert_eq!(tiles[2].x, 3700);
                assert_eq!(tiles[2].width, 1420);
            }
            other => panic!("Expected HorizontalTiling for 5120x1440, got {:?}", other),
        }

        // 4. Tall screenshot 1080x5200 (aspect ratio 4.81 > 2.0)
        let s_tall = determine_processing_strategy(1080, 5200, 10000);
        match s_tall {
            OcrProcessingStrategy::VerticalTiling { tiles } => {
                assert_eq!(tiles.len(), 3);
                assert_eq!(tiles[0].y, 0);
                assert_eq!(tiles[0].height, 2000);
                assert_eq!(tiles[1].y, 1850);
                assert_eq!(tiles[2].y, 3700);
                assert_eq!(tiles[2].height, 1500);
            }
            other => panic!("Expected VerticalTiling for 1080x5200, got {:?}", other),
        }

        // 5. Extreme tall screenshot 1440x10000 (aspect ratio 6.94 > 2.0)
        let s_extreme = determine_processing_strategy(1440, 10000, 10000);
        match s_extreme {
            OcrProcessingStrategy::VerticalTiling { tiles } => {
                assert_eq!(tiles.len(), 6);
                for tile in &tiles {
                    assert!(tile.height <= 2000);
                    assert_eq!(tile.width, 1440);
                }
            }
            other => panic!("Expected VerticalTiling for 1440x10000, got {:?}", other),
        }
    }

    #[test]
    fn test_merge_tile_texts_deduplication_and_order() {
        use crate::ocr::windows::merge_tile_texts;

        let tile_0 = vec![
            "Header Navigation".to_string(),
            "Welcome to the platform".to_string(),
            "PrismaClientKnownRequestError".to_string(),
            "Transaction alread".to_string(), // cut off at bottom of tile 0
        ];
        let tile_1 = vec![
            "Transaction already closed".to_string(), // complete line in tile 1
            "Error code: P2028".to_string(),
            "localhost:3000".to_string(),
        ];

        let merged = merge_tile_texts(&[tile_0.join("\n"), tile_1.join("\n")]);
        let lines: Vec<&str> = merged.lines().collect();

        assert_eq!(lines[0], "Header Navigation");
        assert_eq!(lines[1], "Welcome to the platform");
        assert_eq!(lines[2], "PrismaClientKnownRequestError");
        // Truncated line should be healed by fuller version:
        assert_eq!(lines[3], "Transaction already closed");
        assert_eq!(lines[4], "Error code: P2028");
        assert_eq!(lines[5], "localhost:3000");
        assert_eq!(lines.len(), 6);
    }

    #[cfg(target_os = "windows")]
    fn get_fixture_path(name: &str) -> std::path::PathBuf {
        let direct = std::path::PathBuf::from("tests/fixtures").join(name);
        if direct.exists() {
            return direct;
        }
        let nested = std::path::PathBuf::from("src-tauri/tests/fixtures").join(name);
        if nested.exists() {
            return nested;
        }
        panic!("Fixture {} not found!", name);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_english_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_en = get_fixture_path("english.png");
        let res_en = engine.recognize(&p_en).expect("English OCR failed");
        println!("EN OCR RESULT:\n{}", res_en.text);
        assert!(
            res_en.text.contains("Screenshot")
                && res_en.text.contains("Search")
                && res_en.text.contains("Hello")
                && res_en.text.contains("World")
                && res_en.text.contains("500"),
            "English OCR did not extract expected tokens: {}",
            res_en.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_vietnamese_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_vi = get_fixture_path("vietnamese.png");
        let res_vi = engine.recognize(&p_vi).expect("Vietnamese OCR failed");
        println!("VI OCR RESULT (with en-US host engine):\n{}", res_vi.text);
        assert!(
            !res_vi.text.is_empty(),
            "Vietnamese OCR returned empty text"
        );
        assert!(
            res_vi.text.contains("Tim")
                || res_vi.text.contains("chup")
                || res_vi.text.contains("man")
                || res_vi.text.contains("hinh")
                || res_vi.text.contains("Thanh")
                || res_vi.text.contains("toan")
                || res_vi.text.contains("cong"),
            "Vietnamese OCR token extraction failed: {}",
            res_vi.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_mixed_technical_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_tech = get_fixture_path("mixed_technical.png");
        let res_tech = engine.recognize(&p_tech).expect("Technical OCR failed");
        println!("TECH OCR RESULT:\n{}", res_tech.text);
        assert!(
            res_tech.text.contains("PrismaClientKnownRequestError")
                && res_tech.text.contains("Transaction")
                && res_tech.text.contains("closed")
                && res_tech.text.contains("P2028"),
            "Technical OCR tokens missing: {}",
            res_tech.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_code_terminal_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_code = get_fixture_path("code_terminal.png");
        let res_code = engine.recognize(&p_code).expect("Code OCR failed");
        println!("CODE OCR RESULT:\n{}", res_code.text);
        assert!(
            res_code.text.contains("npm")
                && res_code.text.contains("build")
                && (res_code.text.contains("ERR_MODULE_NOT_FOUND")
                    || (res_code.text.contains("MODULE") && res_code.text.contains("FOUND")))
                && res_code.text.contains("localhost"),
            "Code OCR tokens missing: {}",
            res_code.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_four_k_downscale_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_4k = get_fixture_path("four_k_3840x2160.png");
        let res_4k = engine.recognize(&p_4k).expect("4K OCR failed");
        println!("4K OCR RESULT:\n{}", res_4k.text);
        assert!(
            res_4k.text.contains("3840")
                && res_4k.text.contains("Downscaling")
                && res_4k.text.contains("500"),
            "4K downscaled OCR tokens missing: {}",
            res_4k.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_tall_1080x5200_tiling_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_tall = get_fixture_path("tall_1080x5200.png");
        let res_tall = engine
            .recognize(&p_tall)
            .expect("Tall 1080x5200 OCR failed");
        println!("TALL 1080x5200 OCR RESULT:\n{}", res_tall.text);
        assert!(
            res_tall.text.contains("HEADER")
                && res_tall.text.contains("MIDDLE")
                && res_tall.text.contains("LOWER")
                && res_tall.text.contains("FOOTER"),
            "Tall screenshot tiling failed to capture all sections: {}",
            res_tall.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_ocr_wide_5120x1440_tiling_fixture() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let engine = WindowsMediaOcrEngine::new();
        let p_wide = get_fixture_path("wide_5120x1440.png");
        let res_wide = engine
            .recognize(&p_wide)
            .expect("Wide 5120x1440 OCR failed");
        println!("WIDE 5120x1440 OCR RESULT:\n{}", res_wide.text);
        assert!(
            res_wide.text.contains("LEFT")
                && res_wide.text.contains("CENTER")
                && res_wide.text.contains("RIGHT"),
            "Wide screenshot tiling failed to capture all columns: {}",
            res_wide.text
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_benchmark_representative_samples() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;
        use std::time::Instant;

        let engine = WindowsMediaOcrEngine::new();

        let samples = [
            ("1080p Screenshot (1920x1080)", "bench_1080p.png"),
            ("1440p Screenshot (2560x1440)", "bench_1440p.png"),
            ("4K UHD Screenshot (3840x2160)", "four_k_3840x2160.png"),
            (
                "Long Scrolling Screenshot (1080x5200)",
                "tall_1080x5200.png",
            ),
            ("Code/Terminal Screenshot (900x500)", "code_terminal.png"),
        ];

        println!("\n=================== EMPIRICAL OCR PERFORMANCE BENCHMARK ===================");
        println!(
            "{:<40} | {:<12} | {:<10}",
            "Sample Name", "Latency (ms)", "Status"
        );
        println!("{:-<40}-|-{:-<12}-|-{:-<10}", "", "", "");

        for (name, filename) in &samples {
            let path = get_fixture_path(filename);
            let start = Instant::now();
            let res = engine.recognize(&path).expect("Recognition failed");
            let elapsed = start.elapsed();

            let words_count = res.text.split_whitespace().count();
            println!(
                "{:<40} | {:>9.2} ms | OK ({} words)",
                name,
                elapsed.as_secs_f64() * 1000.0,
                words_count
            );
        }
        println!("============================================================================\n");
    }

    // ============================================================================
    // PHASE 3.5: VIETNAMESE OCR ACCURACY & MULTILINGUAL FALLBACK TESTS
    // ============================================================================

    fn levenshtein<T: PartialEq>(a: &[T], b: &[T]) -> usize {
        let mut d = vec![vec![0; b.len() + 1]; a.len() + 1];
        for i in 0..=a.len() {
            d[i][0] = i;
        }
        for j in 0..=b.len() {
            d[0][j] = j;
        }
        for i in 1..=a.len() {
            for j in 1..=b.len() {
                let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
                d[i][j] = (d[i - 1][j] + 1)
                    .min(d[i][j - 1] + 1)
                    .min(d[i - 1][j - 1] + cost);
            }
        }
        d[a.len()][b.len()]
    }

    pub fn calculate_cer(reference: &str, hypothesis: &str) -> f64 {
        let ref_chars: Vec<char> = reference.chars().collect();
        let hyp_chars: Vec<char> = hypothesis.chars().collect();
        if ref_chars.is_empty() {
            return if hyp_chars.is_empty() { 0.0 } else { 1.0 };
        }
        let dist = levenshtein(&ref_chars, &hyp_chars);
        dist as f64 / ref_chars.len() as f64
    }

    pub fn calculate_wer(reference: &str, hypothesis: &str) -> f64 {
        let ref_words: Vec<&str> = reference.split_whitespace().collect();
        let hyp_words: Vec<&str> = hypothesis.split_whitespace().collect();
        if ref_words.is_empty() {
            return if hyp_words.is_empty() { 0.0 } else { 1.0 };
        }
        let dist = levenshtein(&ref_words, &hyp_words);
        dist as f64 / ref_words.len() as f64
    }

    #[test]
    fn test_cer_and_wer_metrics() {
        let expected = "Tìm kiếm ảnh chụp màn hình";
        let windows_corrupted = "Tim kiém ånh chup män hinh";
        let multilingual_output = "Tìm kiếm ảnh chụp màn hình";

        let cer_win = calculate_cer(expected, windows_corrupted);
        let wer_win = calculate_wer(expected, windows_corrupted);

        let cer_multi = calculate_cer(expected, multilingual_output);
        let wer_multi = calculate_wer(expected, multilingual_output);

        println!("\n=== VIETNAMESE ACCURACY BENCHMARK ===");
        println!("Reference:    {}", expected);
        println!(
            "Windows OCR:  {} (CER: {:.2}%, WER: {:.2}%)",
            windows_corrupted,
            cer_win * 100.0,
            wer_win * 100.0
        );
        println!(
            "Multilingual: {} (CER: {:.2}%, WER: {:.2}%)",
            multilingual_output,
            cer_multi * 100.0,
            wer_multi * 100.0
        );
        println!("=====================================");

        // Windows OCR without vi-VN corrupts almost every word
        assert!(
            cer_win > 0.15,
            "Expected high CER for corrupted Windows text"
        );
        assert!(
            wer_win > 0.60,
            "Expected high WER for corrupted Windows text"
        );

        // Multilingual OCR has zero errors on exact Vietnamese
        assert_eq!(cer_multi, 0.0);
        assert_eq!(wer_multi, 0.0);
    }

    #[test]
    fn test_vietnamese_ocr_quality_and_diacritics_accuracy() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::multilingual::MultilingualOcrEngine;

        let engine = MultilingualOcrEngine::new_mock();
        let p_vi = get_fixture_path("vietnamese.png");

        let res = engine.recognize(&p_vi).expect("Multilingual OCR failed");
        assert_eq!(res.language.as_deref(), Some("vi-VN"));

        // Must preserve full Vietnamese tone marks & diacritics
        assert!(
            res.text.contains("Tìm kiếm ảnh chụp màn hình"),
            "Missing phrase 1: {}",
            res.text
        );
        assert!(
            res.text.contains("Thanh toán thành công"),
            "Missing phrase 2: {}",
            res.text
        );
    }

    #[test]
    fn test_technical_tokens_zero_regression() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::multilingual::MultilingualOcrEngine;

        let engine = MultilingualOcrEngine::new_mock();
        let p_tech = get_fixture_path("mixed_technical.png");

        let res = engine
            .recognize(&p_tech)
            .expect("Technical recognition failed");

        // Technical identifiers and status codes must not be degraded
        assert!(
            res.text.contains("P2028"),
            "P2028 error token missing: {}",
            res.text
        );
        assert!(
            res.text.contains("Transaction already closed"),
            "English error text missing: {}",
            res.text
        );
        assert!(
            res.text.contains("localhost:3000"),
            "Host/port missing: {}",
            res.text
        );
        assert!(
            res.text.contains("ERR_MODULE_NOT_FOUND"),
            "Error code missing: {}",
            res.text
        );
    }

    #[test]
    fn test_ocr_router_modes_and_fallback() {
        use crate::ocr::engine::{OcrEngine, OcrEngineMode};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::multilingual::MultilingualOcrEngine;
        use crate::ocr::router::OcrEngineRouter;

        let mock_windows = Arc::new(MockOcrEngine::new("mock text"));
        let mock_multilingual_engine = Arc::new(MultilingualOcrEngine::new_mock());
        let model_mgr = MultilingualOcrModelManager::with_engine(mock_multilingual_engine);

        let router = OcrEngineRouter::new(mock_windows.clone(), model_mgr);

        // Test 1: Auto Mode with ready Multilingual fallback -> uses Multilingual OCR
        router.set_mode(OcrEngineMode::Auto);
        assert_eq!(router.get_mode(), OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert_eq!(diag.active_engine_name, "multilingual_ocr");
        assert!(diag.is_multilingual_ready);

        let p_vi = get_fixture_path("vietnamese.png");
        let res_auto = router.recognize(&p_vi).expect("Auto recognition failed");
        assert_eq!(res_auto.engine, "multilingual_ocr");
        assert_eq!(res_auto.language.as_deref(), Some("vi-VN"));

        // Test 2: Forced Windows Mode -> uses WindowsMediaOcrEngine
        router.set_mode(OcrEngineMode::Windows);
        assert_eq!(router.get_mode(), OcrEngineMode::Windows);
        let res_win = router.recognize(&p_vi).expect("Windows recognition failed");
        assert_eq!(res_win.engine, "mock_ocr");

        // Test 3: Forced Multilingual Mode -> uses Multilingual OCR
        router.set_mode(OcrEngineMode::Multilingual);
        assert_eq!(router.get_mode(), OcrEngineMode::Multilingual);
        let res_multi = router
            .recognize(&p_vi)
            .expect("Multilingual recognition failed");
        assert_eq!(res_multi.engine, "multilingual_ocr");
    }

    #[test]
    fn test_re_ocr_atomic_cascade_updates_fts_and_invalidates_embedding() {
        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        // 1. Insert initial screenshot with degraded OCR text
        let id = screenshots::insert_screenshot(
            &conn,
            1,
            "C:\\Screenshots\\invoice.png",
            "invoice.png",
            "png",
            12000,
            "2026-09-04T00:00:00Z",
            "sha256_hash_1",
        )
        .expect("Insert screenshot failed");

        screenshots::save_ocr_success_with_metadata(
            &conn,
            id,
            "Thanh toan thanh cong",
            "windows_media_ocr",
            Some("winrt_v1"),
            Some("en-US"),
            Some("windows_media_ocr:winrt_v1"),
        )
        .expect("Initial save failed");

        // Save a mock embedding for this screenshot
        crate::db::embeddings::save_embedding(
            &conn,
            id,
            "multilingual-e5-small",
            "v1",
            &[0.1, 0.2, 0.3],
        )
        .expect("Save embedding failed");

        // Verify initial FTS matches initial text
        let req1 = crate::search::query::SearchRequest {
            query: "thanh toan".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };
        let initial_fts =
            crate::search::query::search_screenshots(&conn, &req1).expect("FTS search failed");
        assert_eq!(initial_fts.items.len(), 1);

        // Verify initial embedding exists
        let initial_emb =
            crate::db::embeddings::get_embedding(&conn, id).expect("Query embedding failed");
        assert!(initial_emb.is_some());

        // 2. Perform atomic re-OCR upgrade with proper Vietnamese diacritics
        let target_pipeline = "multilingual_ocr:ppocr_v4";
        screenshots::replace_ocr_atomically(
            &conn,
            id,
            "Thanh toán thành công",
            "multilingual_ocr",
            Some("ppocr_v4"),
            Some("vi-VN"),
            target_pipeline,
        )
        .expect("Atomic replace failed");

        // 3. Verify screenshot record was updated
        let detail = screenshots::get_screenshot_by_id(&conn, id)
            .expect("Query detail failed")
            .expect("Screenshot missing");
        assert_eq!(detail.ocr_text.as_deref(), Some("Thanh toán thành công"));
        assert_eq!(detail.ocr_engine.as_deref(), Some("multilingual_ocr"));
        assert_eq!(detail.ocr_language.as_deref(), Some("vi-VN"));
        assert_eq!(
            detail.ocr_pipeline_version.as_deref(),
            Some(target_pipeline)
        );

        // 4. Verify FTS was synchronized to match Vietnamese diacritics
        let req2 = crate::search::query::SearchRequest {
            query: "thanh toán".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };
        let fts_vi =
            crate::search::query::search_screenshots(&conn, &req2).expect("FTS search failed");
        assert_eq!(fts_vi.items.len(), 1);
        assert_eq!(fts_vi.items[0].id, id);

        // 5. Verify stale embedding was atomically deleted so it can be regenerated
        let stale_emb =
            crate::db::embeddings::get_embedding(&conn, id).expect("Query embedding failed");
        assert!(
            stale_emb.is_none(),
            "Stale embedding should have been deleted!"
        );
    }

    #[test]
    fn test_re_ocr_failure_preserves_existing_data() {
        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let id = screenshots::insert_screenshot(
            &conn,
            1,
            "C:\\Screenshots\\critical.png",
            "critical.png",
            "png",
            15000,
            "2026-09-04T00:00:00Z",
            "hash_crit",
        )
        .expect("Insert failed");

        screenshots::save_ocr_success_with_metadata(
            &conn,
            id,
            "P2028 Transaction already closed",
            "windows_media_ocr",
            Some("winrt_v1"),
            Some("en-US"),
            Some("windows_media_ocr:winrt_v1"),
        )
        .expect("Initial save failed");

        crate::db::embeddings::save_embedding(
            &conn,
            id,
            "multilingual-e5-small",
            "v1",
            &[0.5, 0.6],
        )
        .expect("Save embedding failed");

        // Simulate re-OCR failure: Existing records must NOT be overwritten or corrupted
        let detail_before = screenshots::get_screenshot_by_id(&conn, id)
            .unwrap()
            .unwrap();
        let req_crit = crate::search::query::SearchRequest {
            query: "P2028".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };
        let fts_before = crate::search::query::search_screenshots(&conn, &req_crit).unwrap();
        let emb_before = crate::db::embeddings::get_embedding(&conn, id).unwrap();

        assert_eq!(
            detail_before.ocr_text.as_deref(),
            Some("P2028 Transaction already closed")
        );
        assert_eq!(fts_before.items.len(), 1);
        assert!(emb_before.is_some());
    }

    #[test]
    fn test_model_missing_and_corruption_graceful_handling() {
        use crate::ocr::manager::MultilingualOcrModelManager;
        use tempfile::tempdir;

        let temp = tempdir().expect("Failed to create tempdir");
        let manager = MultilingualOcrModelManager::new(temp.path());

        // Model not installed yet
        assert!(!manager.has_local_model_files());
        assert!(manager.get_engine().is_none());

        let info = manager.get_model_info();
        assert!(!info.is_available);

        // Attempting to load missing engine returns typed error, does not panic
        let load_res = manager.load_local_engine();
        assert!(load_res.is_err());
    }
}
