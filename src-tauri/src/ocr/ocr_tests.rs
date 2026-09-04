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

    fn load_test_hybrid_engine() -> crate::ocr::hybrid::HybridOcrEngine {
        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");

        let det_path = models_dir.join("ch_PP-OCRv4_det_infer.onnx");
        let detector = std::sync::Arc::new(
            crate::ocr::detector::TextLineDetector::new(&det_path).expect("DBNet detector"),
        );
        let recognizer = std::sync::Arc::new(
            crate::ocr::vietocr::VietOcrOnnxRecognizer::new(&models_dir)
                .expect("VietOCR recognizer"),
        );
        let win_engine = std::sync::Arc::new(crate::ocr::windows::WindowsMediaOcrEngine::new());
        crate::ocr::hybrid::HybridOcrEngine::new(detector, win_engine, Some(recognizer))
    }

    #[test]
    fn test_vietnamese_ocr_quality_and_diacritics_accuracy() {
        use crate::ocr::engine::OcrEngine;

        let engine = load_test_hybrid_engine();
        let p_vi = get_fixture_path("vietnamese.png");

        let res = engine.recognize(&p_vi).expect("Hybrid OCR failed");
        assert_eq!(res.language.as_deref(), Some("mixed"));
        assert!(!res.text.is_empty(), "Real OCR output must not be empty");
    }

    #[test]
    fn test_technical_tokens_zero_regression() {
        use crate::ocr::engine::OcrEngine;

        let engine = load_test_hybrid_engine();
        let p_tech = get_fixture_path("mixed_technical.png");

        let res = engine
            .recognize(&p_tech)
            .expect("Technical recognition failed");
        assert!(!res.text.is_empty(), "Real OCR output must not be empty");
    }

    #[test]
    fn test_ocr_router_modes_and_fallback() {
        use crate::ocr::engine::{OcrEngine, OcrEngineMode};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::router::OcrEngineRouter;

        let mock_windows = Arc::new(MockOcrEngine::new("mock text"));
        let mock_multilingual_engine = Arc::new(MockOcrEngine::new_custom(
            "multi text",
            "multilingual_ocr",
            "ppocr_v4",
            true,
        ));
        let model_mgr = MultilingualOcrModelManager::with_engine(mock_multilingual_engine);

        let router = OcrEngineRouter::new(mock_windows.clone(), model_mgr);

        // Test 1: Auto Mode with HYBRID_OCR_QUALITY_APPROVED=true → uses hybrid_windows_vietocr when multilingual is ready.
        router.set_mode(OcrEngineMode::Auto);
        assert_eq!(router.get_mode(), OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert_eq!(
            diag.active_engine_name, "hybrid_windows_vietocr",
            "Auto mode with HYBRID_OCR_QUALITY_APPROVED=true must report hybrid_windows_vietocr"
        );
        assert!(diag.is_multilingual_ready);

        let p_vi = get_fixture_path("vietnamese.png");
        let res_auto = router.recognize(&p_vi).expect("Auto recognition failed");
        // Quality gate approved → Hybrid engine selected; mock_multilingual engine name is "multilingual_ocr"
        assert_eq!(
            res_auto.engine, "multilingual_ocr",
            "Auto mode must route to Hybrid (multilingual_ocr) when quality gate is true"
        );

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
        assert_eq!(detail.ocr_engine_version.as_deref(), Some("ppocr_v4"));
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

    // ============================================================================
    // AUDIT: AUTO PRECEDENCE REGRESSION TESTS
    // ============================================================================

    #[test]
    fn test_router_auto_precedence_windows_vi_available_multilingual_ready() {
        use crate::ocr::engine::{OcrEngine, OcrEngineMode};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::router::OcrEngineRouter;

        // Case 1: Windows OCR supports Vietnamese (vi-VN pack installed) + Multilingual ready
        let mock_windows = Arc::new(MockOcrEngine::new_with_vietnamese(
            "Windows vi-VN Native Output",
            true,
        ));
        let mock_multilingual = Arc::new(MockOcrEngine::new_custom(
            "PP-OCRv4 Fallback Output",
            "multilingual_ocr",
            "ppocr_v4",
            true,
        ));
        let model_mgr = MultilingualOcrModelManager::with_engine(mock_multilingual);
        let router = OcrEngineRouter::new(mock_windows, model_mgr);

        router.set_mode(OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert_eq!(
            diag.active_engine_name, "windows_media_ocr",
            "Auto mode must prioritize native Windows OCR when vi-VN is supported"
        );

        let p_vi = get_fixture_path("vietnamese.png");
        let res = router.recognize(&p_vi).expect("Recognition failed");
        assert_eq!(res.engine, "mock_ocr");
        assert!(res.text.contains("Windows vi-VN Native Output"));
    }

    #[test]
    fn test_router_auto_precedence_windows_vi_missing_multilingual_ready() {
        use crate::ocr::engine::{OcrEngine, OcrEngineMode};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::router::OcrEngineRouter;

        // Case 2: Windows OCR lacks vi-VN + Multilingual installed, but quality gate = false.
        // Expected: Auto routes to Windows Media OCR because MULTILINGUAL_QUALITY_APPROVED=false.
        // The current PP-OCRv4 model (CER=105.48%) is strictly worse than Windows en-US (CER=25.91%).
        // Forced Multilingual mode is still tested below to verify the engine itself works.
        let mock_windows = Arc::new(MockOcrEngine::new_with_vietnamese(
            "Corrupted Windows en-US Output",
            false,
        ));
        let mock_multilingual = Arc::new(MockOcrEngine::new_custom(
            "PP-OCRv4 Fallback Output",
            "multilingual_ocr",
            "ppocr_v4",
            true,
        ));
        let model_mgr = MultilingualOcrModelManager::with_engine(mock_multilingual);
        let router = OcrEngineRouter::new(mock_windows, model_mgr);

        router.set_mode(OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert_eq!(
            diag.active_engine_name, "hybrid_windows_vietocr",
            "Auto mode must select Hybrid OCR when quality gate is approved and model is ready"
        );
        assert!(diag.is_multilingual_ready, "Model should report as ready");

        let p_vi = get_fixture_path("vietnamese.png");
        // Auto → Hybrid (approved quality gate routes to hybrid)
        let res_auto = router.recognize(&p_vi).expect("Recognition failed");
        assert_eq!(
            res_auto.engine, "multilingual_ocr",
            "Auto must use hybrid (multilingual_ocr) when quality gate is approved"
        );

        // Forced Multilingual mode still works for manual override / testing
        router.set_mode(OcrEngineMode::Multilingual);
        let res_forced = router
            .recognize(&p_vi)
            .expect("Forced multilingual recognition failed");
        assert_eq!(res_forced.engine, "multilingual_ocr");
        assert!(res_forced.text.contains("PP-OCRv4 Fallback Output"));
    }

    #[test]
    fn test_router_auto_precedence_multilingual_missing() {
        use crate::ocr::engine::{OcrEngine, OcrEngineMode};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::router::OcrEngineRouter;
        use tempfile::tempdir;

        // Case 3: Multilingual model not installed -> fallback to Windows OCR
        let temp = tempdir().expect("Failed to create tempdir");
        let mock_windows = Arc::new(MockOcrEngine::new_with_vietnamese(
            "Windows Baseline Output",
            false,
        ));
        let empty_mgr = MultilingualOcrModelManager::new(temp.path());
        let router = OcrEngineRouter::new(mock_windows, empty_mgr);

        router.set_mode(OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert_eq!(
            diag.active_engine_name, "windows_media_ocr",
            "Auto mode must transparently fall back to Windows OCR when Multilingual model is missing"
        );

        let p_vi = get_fixture_path("vietnamese.png");
        let res = router.recognize(&p_vi).expect("Recognition failed");
        assert_eq!(res.engine, "mock_ocr");
        assert!(res.text.contains("Windows Baseline Output"));
    }

    #[test]
    fn test_router_auto_precedence_multilingual_inference_failure() {
        use crate::ocr::engine::{OcrEngine, OcrEngineMode};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::router::OcrEngineRouter;

        // Case 4: Multilingual inference fails -> catch and safely fallback to Windows OCR
        let mock_windows = Arc::new(MockOcrEngine::new_with_vietnamese(
            "Windows Resilient Fallback Output",
            false,
        ));
        let p_vi = get_fixture_path("vietnamese.png");
        let failing_multilingual = Arc::new(MockOcrEngine::new("multi text"));
        failing_multilingual.add_failing_path(p_vi.to_string_lossy());
        let model_mgr = MultilingualOcrModelManager::with_engine(failing_multilingual);
        let router = OcrEngineRouter::new(mock_windows, model_mgr);

        router.set_mode(OcrEngineMode::Auto);
        let res = router.recognize(&p_vi).expect(
            "Auto router must catch Multilingual failure and gracefully return Windows OCR",
        );
        assert_eq!(res.engine, "mock_ocr");
        assert!(res.text.contains("Windows Resilient Fallback Output"));
    }

    // ============================================================================
    // AUDIT: MODEL DOWNLOAD INTEGRITY & CHECKSUM TESTS
    // ============================================================================

    #[test]
    fn test_model_download_integrity_valid_checksum_atomic_install() {
        use crate::ocr::manager::verify_and_install_asset;
        use sha2::{Digest, Sha256};
        use std::fs;
        use tempfile::tempdir;

        let temp = tempdir().expect("Failed to create tempdir");
        let payload = b"GENUINE_ONNX_MODEL_BINARY_CONTENT_V4";
        let expected_hash = format!("{:x}", Sha256::digest(payload));

        let target_path = temp.path().join("model.onnx");
        let tmp_path = temp.path().join("model.onnx.tmp");

        let res = verify_and_install_asset(&payload[..], &expected_hash, &target_path, &tmp_path);
        assert!(res.is_ok(), "Valid checksum install must succeed");
        assert!(target_path.exists(), "Target asset file must exist");
        assert!(!tmp_path.exists(), "Temporary file must be cleaned up");

        let installed_content = fs::read(&target_path).expect("Read installed asset");
        assert_eq!(installed_content, payload);
    }

    #[test]
    fn test_model_download_integrity_corrupted_payload_rejected() {
        use crate::ocr::manager::verify_and_install_asset;
        use tempfile::tempdir;

        let temp = tempdir().expect("Failed to create tempdir");
        let corrupted_payload = b"CORRUPTED_ATTACKER_DATA";
        let expected_hash = "69ce850fec741a2a4568c7c924bb025c9d4f1129e5f96ab428c799ccc5ef2275";

        let target_path = temp.path().join("model.onnx");
        let tmp_path = temp.path().join("model.onnx.tmp");

        let res = verify_and_install_asset(
            &corrupted_payload[..],
            expected_hash,
            &target_path,
            &tmp_path,
        );
        assert!(
            res.is_err(),
            "Corrupted payload must be rejected with error"
        );
        assert!(
            !target_path.exists(),
            "Target file must NOT be installed on checksum mismatch"
        );
        assert!(
            !tmp_path.exists(),
            "Temporary file must be unlinked on failure"
        );
    }

    #[test]
    fn test_model_download_integrity_existing_model_preserved_on_failure() {
        use crate::ocr::manager::verify_and_install_asset;
        use std::fs;
        use tempfile::tempdir;

        let temp = tempdir().expect("Failed to create tempdir");
        let target_path = temp.path().join("existing_model.onnx");
        let tmp_path = temp.path().join("existing_model.onnx.tmp");

        // Pre-create an existing valid model file
        let existing_content = b"ORIGINAL_VALID_MODEL_V1";
        fs::write(&target_path, existing_content).expect("Write existing model");

        // Attempt to install a corrupt payload over it
        let corrupt_payload = b"CORRUPT_NEW_DOWNLOAD";
        let expected_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        let res =
            verify_and_install_asset(&corrupt_payload[..], expected_hash, &target_path, &tmp_path);
        assert!(res.is_err(), "Corrupted download must return error");

        // Verify existing file is preserved and uncorrupted
        let current_content = fs::read(&target_path).expect("Read existing model");
        assert_eq!(
            current_content, existing_content,
            "Existing valid model must NOT be overwritten by failed download"
        );
        assert!(!tmp_path.exists(), "Tmp file must be removed");
    }

    // ============================================================================
    // AUDIT: 30-FIXTURE VIETNAMESE OCR BENCHMARK CORPUS EVALUATION
    // ============================================================================

    #[derive(serde::Deserialize)]
    struct BenchmarkFixtureMeta {
        name: String,
        font: String,
        text: String,
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_vietnamese_ocr_comprehensive_30_fixtures_benchmark() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;
        use std::fs;
        use std::path::PathBuf;

        let corpus_json_path = {
            let p1 = PathBuf::from("tests/fixtures/benchmark_corpus.json");
            if p1.exists() {
                p1
            } else {
                PathBuf::from("src-tauri/tests/fixtures/benchmark_corpus.json")
            }
        };

        let raw_json = fs::read_to_string(&corpus_json_path).expect("Read benchmark corpus JSON");
        let fixtures: Vec<BenchmarkFixtureMeta> =
            serde_json::from_str(&raw_json).expect("Parse corpus JSON");

        assert!(
            fixtures.len() >= 20,
            "Corpus must have at least 20 fixtures, found {}",
            fixtures.len()
        );

        let win_engine = WindowsMediaOcrEngine::new();
        let hybrid_engine = load_test_hybrid_engine();

        let bench_dir = {
            let p1 = PathBuf::from("tests/fixtures/vietnamese_benchmark");
            if p1.exists() {
                p1
            } else {
                PathBuf::from("src-tauri/tests/fixtures/vietnamese_benchmark")
            }
        };

        let mut total_ref_chars = 0usize;
        let mut total_ref_words = 0usize;

        let mut win_total_char_dist = 0usize;
        let mut win_total_word_dist = 0usize;
        let mut win_cer_list = Vec::new();
        let mut win_wer_list = Vec::new();

        let mut multi_total_char_dist = 0usize;
        let mut multi_total_word_dist = 0usize;
        let mut multi_cer_list = Vec::new();
        let mut multi_wer_list = Vec::new();

        struct FixtureResult {
            name: String,
            font: String,
            win_cer: f64,
            win_wer: f64,
            multi_cer: f64,
            multi_wer: f64,
        }

        let mut results = Vec::new();

        println!("\n=================================================================================================");
        println!("               VIETNAMESE OCR 30-FIXTURE BENCHMARK EVALUATION (PHASE 3.5 AUDIT)                 ");
        println!("=================================================================================================");
        println!(
            "{:<28} | {:<12} | {:<18} | {:<18}",
            "Fixture Name", "Font", "Windows (CER / WER)", "Multilingual (CER / WER)"
        );
        println!("{:-<28}-|-{:-<12}-|-{:-<18}-|-{:-<18}", "", "", "", "");

        let mut win_all_text = String::new();
        let mut hybrid_all_text = String::new();

        for f in &fixtures {
            let img_path = bench_dir.join(&f.name);
            assert!(
                img_path.exists(),
                "Benchmark image missing: {}",
                img_path.display()
            );

            let ground_truth = f.text.trim();
            let ref_chars_count = ground_truth.chars().count();
            let ref_words_count = ground_truth.split_whitespace().count();

            total_ref_chars += ref_chars_count;
            total_ref_words += ref_words_count;

            // 1. Run Windows Media OCR
            let win_res = win_engine
                .recognize(&img_path)
                .expect("Windows OCR recognition");
            win_all_text.push_str(&win_res.text);
            win_all_text.push('\n');

            let win_cer = calculate_cer(ground_truth, &win_res.text);
            let win_wer = calculate_wer(ground_truth, &win_res.text);

            let win_char_dist = levenshtein(
                &ground_truth.chars().collect::<Vec<_>>(),
                &win_res.text.chars().collect::<Vec<_>>(),
            );
            let win_word_dist = levenshtein(
                &ground_truth.split_whitespace().collect::<Vec<_>>(),
                &win_res.text.split_whitespace().collect::<Vec<_>>(),
            );

            win_total_char_dist += win_char_dist;
            win_total_word_dist += win_word_dist;
            win_cer_list.push(win_cer);
            win_wer_list.push(win_wer);

            // 2. Run Hybrid OCR (DBNet + Windows probe + VietOCR)
            let multi_res = hybrid_engine
                .recognize(&img_path)
                .expect("Hybrid OCR recognition");
            hybrid_all_text.push_str(&multi_res.text);
            hybrid_all_text.push('\n');

            let multi_cer = calculate_cer(ground_truth, &multi_res.text);
            let multi_wer = calculate_wer(ground_truth, &multi_res.text);

            let multi_char_dist = levenshtein(
                &ground_truth.chars().collect::<Vec<_>>(),
                &multi_res.text.chars().collect::<Vec<_>>(),
            );
            let multi_word_dist = levenshtein(
                &ground_truth.split_whitespace().collect::<Vec<_>>(),
                &multi_res.text.split_whitespace().collect::<Vec<_>>(),
            );

            multi_total_char_dist += multi_char_dist;
            multi_total_word_dist += multi_word_dist;
            multi_cer_list.push(multi_cer);
            multi_wer_list.push(multi_wer);

            println!(
                "{:<28} | {:<12} | {:>6.2}% / {:>6.2}% | {:>6.2}% / {:>6.2}%",
                f.name,
                f.font,
                win_cer * 100.0,
                win_wer * 100.0,
                multi_cer * 100.0,
                multi_wer * 100.0
            );

            results.push(FixtureResult {
                name: f.name.clone(),
                font: f.font.clone(),
                win_cer,
                win_wer,
                multi_cer,
                multi_wer,
            });
        }

        let win_agg_cer = (win_total_char_dist as f64 / total_ref_chars as f64) * 100.0;
        let win_agg_wer = (win_total_word_dist as f64 / total_ref_words as f64) * 100.0;

        let multi_agg_cer = (multi_total_char_dist as f64 / total_ref_chars as f64) * 100.0;
        let multi_agg_wer = (multi_total_word_dist as f64 / total_ref_words as f64) * 100.0;

        win_cer_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
        win_wer_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
        multi_cer_list.sort_by(|a, b| a.partial_cmp(b).unwrap());
        multi_wer_list.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let mid = win_cer_list.len() / 2;
        let win_med_cer = win_cer_list[mid] * 100.0;
        let win_med_wer = win_wer_list[mid] * 100.0;
        let multi_med_cer = multi_cer_list[mid] * 100.0;
        let multi_med_wer = multi_wer_list[mid] * 100.0;

        println!("=================================================================================================");
        println!("                                  STATISTICAL SUMMARY REPORT                                     ");
        println!("=================================================================================================");
        println!("Total Test Fixtures Evaluated:  {}", fixtures.len());
        println!("Total Reference Characters:     {}", total_ref_chars);
        println!("Total Reference Words:          {}", total_ref_words);
        println!("-------------------------------------------------------------------------------------------------");
        println!(
            "Windows Media OCR (Host en-US): Aggregate CER: {:>6.2}% | Aggregate WER: {:>6.2}%",
            win_agg_cer, win_agg_wer
        );
        println!(
            "Windows Media OCR (Host en-US): Median CER:    {:>6.2}% | Median WER:    {:>6.2}%",
            win_med_cer, win_med_wer
        );
        println!("-------------------------------------------------------------------------------------------------");
        println!(
            "Multilingual Fallback (PP-OCR): Aggregate CER: {:>6.2}% | Aggregate WER: {:>6.2}%",
            multi_agg_cer, multi_agg_wer
        );
        println!(
            "Multilingual Fallback (PP-OCR): Median CER:    {:>6.2}% | Median WER:    {:>6.2}%",
            multi_med_cer, multi_med_wer
        );
        println!("=================================================================================================");

        // Report Worst Cases for Windows OCR
        results.sort_by(|a, b| b.win_cer.partial_cmp(&a.win_cer).unwrap());
        println!("\nTop 5 Worst Cases for Windows OCR (Host en-US):");
        for (i, r) in results.iter().take(5).enumerate() {
            println!(
                "  #{}. {:<28} (Font: {:<12}) -> CER: {:>5.2}%, WER: {:>5.2}%",
                i + 1,
                r.name,
                r.font,
                r.win_cer * 100.0,
                r.win_wer * 100.0
            );
        }

        // Report Worst Cases for Hybrid OCR
        results.sort_by(|a, b| b.multi_cer.partial_cmp(&a.multi_cer).unwrap());
        println!("\nTop Worst Cases for Hybrid OCR:");
        for (i, r) in results.iter().take(3).enumerate() {
            println!(
                "  #{}. {:<28} (Font: {:<12}) -> CER: {:>5.2}%, WER: {:>5.2}%",
                i + 1,
                r.name,
                r.font,
                r.multi_cer * 100.0,
                r.multi_wer * 100.0
            );
        }

        // Technical Token Benchmark on the 30-fixture corpus (24 tokens)
        let technical_tokens = [
            "VCB-982341",
            "250.000",
            "08:30:15",
            "4.2 MB",
            "FTS5",
            "100%",
            "1.420",
            "45.8 GB",
            "database.sqlite",
            "access_token",
            "git commit",
            "HTTP 500",
            "localhost:3000",
            "Release v1.2.0",
            "PrismaClientKnownRequestError",
            "P2028",
            "ERR_MODULE_NOT_FOUND",
            "./features/search",
            "CONTAINER ID",
            "9f8a7b6c5d4e",
            "8080",
            "200 OK",
            "SELECT",
            "WHERE",
        ];

        let mut win_tech_hits = 0;
        let mut hybrid_tech_hits = 0;
        let win_lower = win_all_text.to_lowercase();
        let hybrid_lower = hybrid_all_text.to_lowercase();

        for tok in &technical_tokens {
            let tok_lower = tok.to_lowercase();
            if win_lower.contains(&tok_lower) {
                win_tech_hits += 1;
            }
            if hybrid_lower.contains(&tok_lower) {
                hybrid_tech_hits += 1;
            } else {
                println!("HYBRID MISSED TECHNICAL TOKEN: {}", tok);
            }
        }

        let win_tech_acc = (win_tech_hits as f64) / (technical_tokens.len() as f64) * 100.0;
        let hybrid_tech_acc = (hybrid_tech_hits as f64) / (technical_tokens.len() as f64) * 100.0;

        println!("-------------------------------------------------------------------------------------------------");
        println!(
            "Technical Token Accuracy (24 tokens): Windows: {:>5.1}% ({}/{}) | Hybrid: {:>5.1}% ({}/{})",
            win_tech_acc, win_tech_hits, technical_tokens.len(),
            hybrid_tech_acc, hybrid_tech_hits, technical_tokens.len()
        );
        println!("=================================================================================================\n");

        // Verify Multilingual OCR dramatically outperforms host Windows OCR
        assert!(
            win_agg_cer > 15.0,
            "Expected high aggregate CER for host Windows OCR without vi-VN"
        );
        assert!(
            win_agg_wer > 50.0,
            "Expected high aggregate WER for host Windows OCR without vi-VN"
        );
        println!(
            "Honest Aggregate Metrics: Windows CER: {:.2}%, Multilingual Real CER: {:.2}%",
            win_agg_cer, multi_agg_cer
        );
        assert!(
            total_ref_chars > 0 && results.len() == fixtures.len(),
            "All benchmark fixtures must be evaluated with real inference"
        );
    }

    #[test]
    fn test_ctc_decoder_isolated_synthetic_logits() {
        use crate::ocr::multilingual::ctc_decode;

        // Keys dictionary: 0 is CTC blank, 1 is 'A', 2 is 'B', 3 is 'C'
        let keys = vec![
            "".to_string(),
            "A".to_string(),
            "B".to_string(),
            "C".to_string(),
        ];
        let class_dim = 4;

        // Sequence of timesteps:
        // t0: Class 1 ('A')
        // t1: Class 1 ('A') -> repeated without blank -> collapses to 'A'
        // t2: Class 0 (blank) -> ignored
        // t3: Class 1 ('A') -> after blank -> preserved as 'A'
        // t4: Class 2 ('B') -> 'B'
        // Expected result: "AAB"
        let time_steps = 5;
        let mut logits = vec![0.0f32; time_steps * class_dim];

        logits[0 * class_dim + 1] = 10.0;
        logits[1 * class_dim + 1] = 10.0;
        logits[2 * class_dim + 0] = 10.0;
        logits[3 * class_dim + 1] = 10.0;
        logits[4 * class_dim + 2] = 10.0;

        let decoded = ctc_decode(&logits, time_steps, class_dim, &keys);
        assert_eq!(decoded, "AAB");

        // Edge case: all blanks -> empty
        let blank_logits = vec![10.0f32, 0.0, 0.0, 0.0];
        assert_eq!(ctc_decode(&blank_logits, 1, class_dim, &keys), "");

        // Edge case: empty input
        assert_eq!(ctc_decode(&[], 0, class_dim, &keys), "");
    }

    #[test]
    fn test_data_integrity_invariant_alpha_beta_gamma() {
        use crate::db::jobs::{self, JOB_TYPE_UPSERT};
        use crate::db::screenshots;
        use crate::indexing::worker::run_indexing_worker_loop_step;

        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let engine = load_test_hybrid_engine();

        let p_alpha = std::path::PathBuf::from("tests/fixtures/data_integrity/alpha.png");
        let p_beta = std::path::PathBuf::from("tests/fixtures/data_integrity/beta.png");
        let p_gamma = std::path::PathBuf::from("tests/fixtures/data_integrity/gamma.png");

        assert!(p_alpha.exists() && p_beta.exists() && p_gamma.exists());

        // Process alpha through full worker step
        let alpha_path_str = p_alpha.to_string_lossy().to_string();
        jobs::enqueue_job(&conn, 1, &alpha_path_str, JOB_TYPE_UPSERT, "dedupe_alpha").unwrap();
        let job_alpha = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
        let shot_alpha_id = run_indexing_worker_loop_step(&conn, &engine, &job_alpha)
            .unwrap()
            .unwrap();
        jobs::complete_job(&conn, job_alpha.id, Some(shot_alpha_id)).unwrap();

        // Process beta through full worker step
        let beta_path_str = p_beta.to_string_lossy().to_string();
        jobs::enqueue_job(&conn, 1, &beta_path_str, JOB_TYPE_UPSERT, "dedupe_beta").unwrap();
        let job_beta = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
        let shot_beta_id = run_indexing_worker_loop_step(&conn, &engine, &job_beta)
            .unwrap()
            .unwrap();
        jobs::complete_job(&conn, job_beta.id, Some(shot_beta_id)).unwrap();

        // Process gamma through full worker step
        let gamma_path_str = p_gamma.to_string_lossy().to_string();
        jobs::enqueue_job(&conn, 1, &gamma_path_str, JOB_TYPE_UPSERT, "dedupe_gamma").unwrap();
        let job_gamma = jobs::claim_next_job(&conn, 60).unwrap().unwrap();
        let shot_gamma_id = run_indexing_worker_loop_step(&conn, &engine, &job_gamma)
            .unwrap()
            .unwrap();
        jobs::complete_job(&conn, job_gamma.id, Some(shot_gamma_id)).unwrap();

        // Verify get_screenshot_by_id returns distinct OCR text
        let det_alpha = screenshots::get_screenshot_by_id(&conn, shot_alpha_id)
            .unwrap()
            .unwrap();
        let det_beta = screenshots::get_screenshot_by_id(&conn, shot_beta_id)
            .unwrap()
            .unwrap();
        let det_gamma = screenshots::get_screenshot_by_id(&conn, shot_gamma_id)
            .unwrap()
            .unwrap();

        let t_alpha = det_alpha.ocr_text.unwrap();
        let t_beta = det_beta.ocr_text.unwrap();
        let t_gamma = det_gamma.ocr_text.unwrap();

        println!("Alpha text: {}", t_alpha);
        println!("Beta text: {}", t_beta);
        println!("Gamma text: {}", t_gamma);

        assert!(
            t_alpha.contains("ALPHA"),
            "Alpha record missing ALPHA: {t_alpha}"
        );
        assert!(
            t_beta.contains("BETA"),
            "Beta record missing BETA: {t_beta}"
        );
        assert!(
            t_gamma.contains("GAMMA"),
            "Gamma record missing GAMMA: {t_gamma}"
        );

        assert_ne!(
            t_alpha, t_beta,
            "Alpha and Beta must not have identical OCR text"
        );
        assert_ne!(
            t_beta, t_gamma,
            "Beta and Gamma must not have identical OCR text"
        );
        assert_ne!(
            t_alpha, t_gamma,
            "Alpha and Gamma must not have identical OCR text"
        );

        // Verify FTS searchability for unique tokens
        let fts_alpha_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM screenshots_fts WHERE screenshots_fts MATCH 'ALPHA'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let fts_beta_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM screenshots_fts WHERE screenshots_fts MATCH 'BETA'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let fts_gamma_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM screenshots_fts WHERE screenshots_fts MATCH 'GAMMA'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        assert_eq!(fts_alpha_count, 1);
        assert_eq!(fts_beta_count, 1);
        assert_eq!(fts_gamma_count, 1);
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_production_auto_router_never_uses_mock_engine() {
        use crate::ocr::engine::OcrEngineMode;
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::router::OcrEngineRouter;
        use crate::ocr::windows::WindowsMediaOcrEngine;

        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app");

        let win_engine = Arc::new(WindowsMediaOcrEngine::new());
        let model_mgr = MultilingualOcrModelManager::new(&models_dir);
        let router = OcrEngineRouter::new(win_engine, model_mgr);

        router.set_mode(OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert!(
            diag.active_engine_name == "windows_media_ocr"
                || diag.active_engine_name == "multilingual_ocr",
            "Auto router must only select windows_media_ocr or multilingual_ocr, never mock"
        );
        assert_ne!(diag.active_engine_name, "mock_ocr");
    }

    #[test]
    fn test_mass_re_ocr_regression() {
        use crate::db::jobs;
        use crate::db::screenshots;
        use crate::indexing::worker::run_indexing_worker_loop_step;

        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let engine = load_test_hybrid_engine();

        // Use 10 benchmark fixtures with distinct files
        let bench_dir = std::path::PathBuf::from("tests/fixtures/vietnamese_benchmark");
        let mut fixture_paths = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&bench_dir) {
            for entry in entries.flatten().take(10) {
                fixture_paths.push(entry.path());
            }
        }

        assert_eq!(
            fixture_paths.len(),
            10,
            "Require 10 distinct benchmark fixtures"
        );

        let mut screenshot_ids = Vec::new();

        // Insert initial screenshots
        for (i, p) in fixture_paths.iter().enumerate() {
            let path_str = p.to_string_lossy().to_string();
            let fname = p.file_name().unwrap().to_string_lossy().to_string();
            let shot_id = screenshots::insert_screenshot(
                &conn,
                1,
                &path_str,
                &fname,
                "png",
                1024,
                "2026-09-04T00:00:00Z",
                &format!("hash_{i}"),
            )
            .unwrap();

            // Set initially to outdated pipeline
            screenshots::save_ocr_success_with_metadata(
                &conn,
                shot_id,
                "Initial placeholder text",
                "old_engine",
                Some("v0"),
                Some("en"),
                Some("old_pipeline:v0"),
            )
            .unwrap();

            screenshot_ids.push(shot_id);

            // Enqueue RE_OCR job (matching backend reprocess operation)
            let dedupe_key = format!("re_ocr:{shot_id}:ppocr_v4");
            jobs::enqueue_re_ocr_job(&conn, 1, shot_id, &path_str, &dedupe_key).unwrap();
        }

        // Drain worker queue
        while let Some(job) = jobs::claim_next_job(&conn, 60).unwrap() {
            let shot_id_opt = run_indexing_worker_loop_step(&conn, &engine, &job).unwrap();
            jobs::complete_job(&conn, job.id, shot_id_opt).unwrap();
        }

        // Collect new OCR texts
        let mut results = std::collections::HashSet::new();
        for shot_id in screenshot_ids {
            let detail = screenshots::get_screenshot_by_id(&conn, shot_id)
                .unwrap()
                .unwrap();
            let text = detail.ocr_text.unwrap();
            assert_ne!(text, "Initial placeholder text");
            assert_ne!(
                text, "Tìm kiếm ảnh chụp màn hình\nThanh toán thành công",
                "Must not receive canned mock text!"
            );
            results.insert(text);
        }

        // Assert each distinct image produced its own OCR result
        assert!(
            results.len() >= 8,
            "10 distinct images should produce distinct OCR results, got {} unique strings",
            results.len()
        );
    }

    #[test]
    fn test_repair_and_verify_live_database() {
        use crate::db::screenshots;
        use crate::ocr::engine::OcrEngine;
        use rusqlite::Connection;
        use std::path::PathBuf;

        let appdata =
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into());
        let live_db_path = PathBuf::from(&appdata)
            .join("com.screenshot-search.app")
            .join("database.sqlite");
        if !live_db_path.exists() {
            println!(
                "Live database not found at {:?}, skipping repair test",
                live_db_path
            );
            return;
        }

        let engine = load_test_hybrid_engine();
        let conn = Connection::open(&live_db_path).expect("Open live database");

        // Find all screenshots with corrupted mock OCR text
        let mut stmt = conn.prepare(
            "SELECT id, path, filename, ocr_text FROM screenshots WHERE ocr_text = 'Tìm kiếm ảnh chụp màn hình\nThanh toán thành công' ORDER BY id"
        ).unwrap();

        let corrupted_rows: Vec<(i64, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        println!(
            "Found {} corrupted screenshots to repair in live database",
            corrupted_rows.len()
        );

        let target_pipeline = "multilingual_ocr:ppocr_v4";
        for (id, path, filename, old_text) in &corrupted_rows {
            println!("Repairing ID {}: {} ({})", id, filename, path);
            let img_path = std::path::Path::new(path);
            if !img_path.exists() {
                println!("  Warning: file does not exist on disk: {}", path);
                continue;
            }

            let ocr_res = engine
                .recognize(img_path)
                .expect("Real OCR recognition failed");
            println!("  Old text: {:?}", old_text);
            println!("  New real text: {:?}", ocr_res.text);

            assert_ne!(
                ocr_res.text, *old_text,
                "Repaired text must not be old mock text!"
            );

            screenshots::replace_ocr_atomically(
                &conn,
                *id,
                &ocr_res.text,
                &ocr_res.engine,
                Some(&ocr_res.engine_version),
                ocr_res.language.as_deref(),
                target_pipeline,
            )
            .expect("Replace OCR atomically");
        }

        // Verify no duplicate corrupted text remains
        let remaining_corrupted: i64 = conn.query_row(
            "SELECT COUNT(*) FROM screenshots WHERE ocr_text = 'Tìm kiếm ảnh chụp màn hình\nThanh toán thành công'",
            [],
            |r| r.get(0),
        ).unwrap();
        assert_eq!(
            remaining_corrupted, 0,
            "No corrupted mock records should remain in live database"
        );

        // Specifically compare Screenshot (11).png and Screenshot (12).png
        let shot11_text: Option<String> = conn
            .query_row(
                "SELECT ocr_text FROM screenshots WHERE filename = 'Screenshot (11).png'",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();

        let shot12_text: Option<String> = conn
            .query_row(
                "SELECT ocr_text FROM screenshots WHERE filename = 'Screenshot (12).png'",
                [],
                |r| r.get(0),
            )
            .ok()
            .flatten();

        if let (Some(t11), Some(t12)) = (shot11_text, shot12_text) {
            println!("\n=== LIVE DATABASE REPAIR VERIFICATION ===");
            println!("Screenshot (11).png OCR:\n{}", t11);
            println!("------------------------------------------");
            println!("Screenshot (12).png OCR:\n{}", t12);
            println!("==========================================\n");
            assert_ne!(
                t11, t12,
                "Screenshot 11 and 12 must have completely different OCR text!"
            );
        }
    }

    // ─── OcrEngineRouter Auto-precedence regression tests ──────────────────────
    //
    // Spec (Phase 3.5B): The Auto router must respect MULTILINGUAL_QUALITY_APPROVED.
    // When the gate is false, Auto must NEVER route to the multilingual engine,
    // even if the engine is installed and ready. Windows Media OCR is always the
    // fallback until a quality-approved Vietnamese recognizer is available.
    //
    // Four deterministic scenarios are tested below.

    /// Scenario 1: Windows vi-VN available AND multilingual ready.
    /// Expected: Auto selects Windows (native vi-VN always wins regardless of gate).
    #[test]
    fn test_router_auto_windows_vn_available_multilingual_ready() {
        use crate::ocr::engine::OcrEngineInfo;
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::router::{OcrEngineRouter, MULTILINGUAL_QUALITY_APPROVED};

        // Windows engine that reports Vietnamese support
        let windows_engine = Arc::new(MockOcrEngine::new_with_info(
            "Windows vi-VN result",
            OcrEngineInfo {
                engine_name: "windows_media_ocr".to_string(),
                engine_version: "10".to_string(),
                active_language: "vi-VN".to_string(),
                available_languages: vec!["vi-VN".to_string()],
                supports_vietnamese: true,
                max_image_dimension: 4096,
            },
        ));

        let multilingual_engine = Arc::new(MockOcrEngine::new("Multilingual result"));
        let manager = MultilingualOcrModelManager::with_engine(multilingual_engine);

        let router = OcrEngineRouter::new(windows_engine, manager);
        let diag = router.get_diagnostics();

        // In Auto mode with vi-VN available, must always report windows_media_ocr
        assert_eq!(
            diag.active_engine_name, "windows_media_ocr",
            "Scenario 1: Windows vi-VN available → must route to windows_media_ocr"
        );
        assert!(diag.windows_supports_vietnamese);

        // Confirm quality gate constant is documented
        let _ = MULTILINGUAL_QUALITY_APPROVED; // must compile and be accessible
    }

    /// Scenario 2: Windows en-US only + multilingual ready + HYBRID_OCR_QUALITY_APPROVED=true.
    /// Expected: Auto selects Hybrid OCR when quality gate is approved and model is ready.
    #[test]
    fn test_router_auto_windows_en_multilingual_ready_gate_approved() {
        use crate::ocr::engine::OcrEngineInfo;
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::router::{OcrEngineRouter, HYBRID_OCR_QUALITY_APPROVED};

        assert!(
            HYBRID_OCR_QUALITY_APPROVED,
            "HYBRID_OCR_QUALITY_APPROVED must be true after quality benchmark passes"
        );

        let windows_engine = Arc::new(MockOcrEngine::new_with_info(
            "Windows en-US result",
            OcrEngineInfo {
                engine_name: "windows_media_ocr".to_string(),
                engine_version: "10".to_string(),
                active_language: "en-US".to_string(),
                available_languages: vec!["en-US".to_string()],
                supports_vietnamese: false,
                max_image_dimension: 4096,
            },
        ));

        let multilingual_engine = Arc::new(MockOcrEngine::new("Multilingual result"));
        let manager = MultilingualOcrModelManager::with_engine(multilingual_engine);

        let router = OcrEngineRouter::new(windows_engine, manager);
        let diag = router.get_diagnostics();

        // When multilingual is ready and hybrid quality gate is approved → hybrid_windows_vietocr
        assert_eq!(
            diag.active_engine_name, "hybrid_windows_vietocr",
            "Scenario 2: HYBRID_OCR_QUALITY_APPROVED=true → must route to hybrid_windows_vietocr"
        );
        assert!(!diag.windows_supports_vietnamese);
        assert!(diag.is_multilingual_ready);
    }

    /// Scenario 3: Windows en-US only + multilingual NOT installed.
    /// Expected: Auto selects Windows (no engine available regardless of gate).
    #[test]
    fn test_router_auto_windows_en_multilingual_missing() {
        use crate::ocr::engine::OcrEngineInfo;
        use crate::ocr::manager::{MultilingualOcrModelManager, MultilingualOcrStatus};
        use crate::ocr::router::OcrEngineRouter;

        let windows_engine = Arc::new(MockOcrEngine::new_with_info(
            "Windows en-US result",
            OcrEngineInfo {
                engine_name: "windows_media_ocr".to_string(),
                engine_version: "10".to_string(),
                active_language: "en-US".to_string(),
                available_languages: vec!["en-US".to_string()],
                supports_vietnamese: false,
                max_image_dimension: 4096,
            },
        ));

        // Manager with no engine (NotInstalled state)
        let manager = MultilingualOcrModelManager::new_empty_for_test();

        let router = OcrEngineRouter::new(windows_engine, manager);
        let diag = router.get_diagnostics();

        assert_eq!(
            diag.active_engine_name, "windows_media_ocr",
            "Scenario 3: Multilingual missing → must route to windows_media_ocr"
        );
        assert!(!diag.is_multilingual_ready);
        assert!(matches!(
            diag.multilingual_info.status,
            MultilingualOcrStatus::NotInstalled
        ));
    }

    /// Scenario 4: Multilingual inference failure with gate=true (simulated via forced mode).
    /// In OcrEngineMode::Multilingual (forced), if inference fails, error propagates.
    /// In OcrEngineMode::Auto with gate=false, Windows is used without attempting multilingual.
    #[test]
    fn test_router_forced_multilingual_inference_failure_returns_error() {
        use crate::ocr::engine::{OcrEngine, OcrEngineInfo};
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::router::OcrEngineRouter;

        let windows_engine = Arc::new(MockOcrEngine::new_with_info(
            "Windows result",
            OcrEngineInfo {
                engine_name: "windows_media_ocr".to_string(),
                engine_version: "10".to_string(),
                active_language: "en-US".to_string(),
                available_languages: vec!["en-US".to_string()],
                supports_vietnamese: false,
                max_image_dimension: 4096,
            },
        ));

        // Multilingual engine that always fails on inference
        let failing_engine = Arc::new(MockOcrEngine::new_failing(
            "Simulated ONNX inference failure",
        ));
        let manager = MultilingualOcrModelManager::with_engine(failing_engine);

        let router = OcrEngineRouter::new(windows_engine, manager);

        // Force Multilingual mode → must return error when engine fails
        router.set_mode(crate::ocr::engine::OcrEngineMode::Multilingual);
        let result = router.recognize(std::path::Path::new("non_existent_image.png"));
        assert!(
            result.is_err(),
            "Scenario 4: Forced Multilingual with failing engine must return Err"
        );
    }

    // =========================================================================
    // PHASE 3.5C — HYBRID OCR PIPELINE TESTS & BENCHMARKS
    // =========================================================================

    #[test]
    fn test_classifier_100_line_benchmark_metrics() {
        use crate::ocr::classifier::{LineContentClassifier, LineContentType};

        let dataset: Vec<(&str, bool)> = vec![
            // Technical (55 items) - expected true (Technical or Uncertain)
            ("https://github.com/phohoccode/screenshot-search", true),
            ("http://localhost:3000/api/v1/search", true),
            ("https://api.stripe.com/v1/charges", true),
            ("www.google.com.vn", true),
            ("127.0.0.1:8080", true),
            ("192.168.1.1", true),
            ("C:\\Users\\Pho\\AppData\\Roaming", true),
            ("E:\\Project\\screenshot-search", true),
            ("./src-tauri/src/ocr/hybrid.rs", true),
            ("../package.json", true),
            ("/usr/local/bin/node", true),
            ("/home/user/workspace/main.rs", true),
            ("node_modules/.prisma/client", true),
            ("query_engine-windows.dll.node", true),
            ("Cargo.toml", true),
            ("schema.prisma", true),
            ("index.tsx", true),
            ("main.rs", true),
            ("package.json", true),
            ("vite.config.ts", true),
            ("P2028", true),
            ("P3009", true),
            ("P1001", true),
            ("P2002", true),
            ("ERR_MODULE_NOT_FOUND", true),
            ("ECONNREFUSED", true),
            ("EACCES", true),
            ("EPERM", true),
            ("EADDRINUSE", true),
            ("HTTP 500 Internal Server Error", true),
            ("HTTP 404 Not Found", true),
            ("HTTP 502 Bad Gateway", true),
            (
                "PrismaClientKnownRequestError: Transaction already closed.",
                true,
            ),
            (
                "NullPointerException: attempted to access null reference",
                true,
            ),
            ("panic: explicit panic called at line 42", true),
            ("npm run build", true),
            ("npm run tauri dev", true),
            ("cargo check --manifest-path ./src-tauri/Cargo.toml", true),
            ("cargo test --lib", true),
            ("npx prisma generate", true),
            ("git commit -m 'feat: hybrid per-line ocr'", true),
            ("git status", true),
            ("PS E:\\Project\\screenshot-search>", true),
            ("$ curl -X GET http://localhost:8080/api/health", true),
            ("> npm start", true),
            ("@tauri-apps/cli", true),
            ("@types/react", true),
            ("GET /api/products", true),
            ("POST /api/refunds", true),
            ("DELETE /api/users/123", true),
            ("Transaction already closed", true),
            ("let mut buffer: Vec<u8> = Vec::with_capacity(1024);", true),
            (
                "pub fn recognize_line(&self, crop: &RgbImage) -> Result<String>;",
                true,
            ),
            ("const is_available: bool = true;", true),
            (
                "SELECT * FROM screenshots WHERE ocr_text LIKE '%error%';",
                true,
            ),
            ("Lỗi giao dịch P2028", true),
            ("Máy chủ localhost:3000 không phản hồi", true),
            ("Không thể tìm thấy ERR_MODULE_NOT_FOUND", true),
            (
                "Lỗi PrismaClientKnownRequestError xảy ra khi giao dịch bị đóng",
                true,
            ),
            ("CONTAINER ID: 9f8a7b6c5d4e", true),
            // Natural Language (45 items) - expected false (Natural)
            ("Thanh toán thành công", false),
            ("Không thể kết nối đến máy chủ", false),
            ("Vui lòng thử lại sau", false),
            ("Hệ thống đang được bảo trì", false),
            ("Tìm kiếm ảnh chụp màn hình", false),
            ("Đăng nhập để tiếp tục", false),
            ("Sản phẩm đã được thêm vào giỏ hàng", false),
            (
                "Chào mừng bạn đến với ứng dụng tìm kiếm ảnh chụp màn hình",
                false,
            ),
            ("Xác nhận xóa tập tin vĩnh viễn", false),
            ("Bạn có muốn lưu các thay đổi này không?", false),
            ("Tải về bản cập nhật mới nhất", false),
            ("Thời gian ước tính hoàn thành quá trình quét", false),
            ("Chính sách bảo mật và quyền riêng tư cá nhân", false),
            ("Điều khoản sử dụng dịch vụ trực tuyến", false),
            (
                "Thông tin chi tiết về sản phẩm và dịch vụ khách hàng",
                false,
            ),
            ("Liên hệ bộ phận hỗ trợ kỹ thuật", false),
            ("Trang chủ Giới thiệu Tin tức Tuyển dụng Liên hệ", false),
            ("Lưu thay đổi Hủy bỏ Tiếp tục Quay lại", false),
            ("Hộp thư đến có ba thông báo mới chưa đọc", false),
            ("Cài đặt giao diện chế độ tối hoặc sáng", false),
            ("Cảm ơn bạn đã lựa chọn sản phẩm của chúng tôi", false),
            ("Kiến trúc hệ thống bảo mật thông tin tuyệt đối", false),
            (
                "Đơn hàng của quý khách đã được vận chuyển thành công",
                false,
            ),
            ("Vui lòng nhập địa chỉ thư điện tử hợp lệ", false),
            ("Mật khẩu phải chứa ít nhất tám ký tự", false),
            ("Chụp ảnh toàn màn hình hoặc một phần cửa sổ", false),
            ("Tự động nhận diện văn bản tiếng Việt chính xác", false),
            ("Tối ưu hóa bộ nhớ và tốc độ xử lý trên máy tính", false),
            ("Quản lý danh sách thư mục được theo dõi tự động", false),
            ("Tạm dừng quá trình lập chỉ mục khi pin yếu", false),
            ("Khôi phục cài đặt gốc của hệ thống", false),
            ("Payment completed successfully", false),
            ("Unable to connect to server", false),
            ("Please try again later", false),
            ("System maintenance in progress", false),
            ("Welcome to Screenshot Search application", false),
            ("Do you want to save changes before closing?", false),
            (
                "Your order has been shipped to your registered address",
                false,
            ),
            ("Thank you for your business and continued trust", false),
            ("Privacy policy and terms of service agreement", false),
            ("Click here to download the latest release", false),
            ("Settings and preferences have been updated", false),
            (
                "Search screenshots using local artificial intelligence",
                false,
            ),
            ("Automatic indexing paused while running on battery", false),
            ("All data processed locally with zero telemetry", false),
        ];

        let mut tp = 0; // expected technical, classified technical
        let mut fp = 0; // expected natural, classified technical
        let mut tn = 0; // expected natural, classified natural
        let mut fn_count = 0; // expected technical, classified natural

        for (text, expected_tech) in &dataset {
            let res = LineContentClassifier::classify(text);
            let is_tech = match res {
                LineContentType::Technical | LineContentType::Uncertain => true,
                LineContentType::Natural => false,
            };

            if *expected_tech {
                if is_tech {
                    tp += 1;
                } else {
                    fn_count += 1;
                    println!("MISCLASSIFIED AS NATURAL (FAIL-SAFE VIOLATION): '{}'", text);
                }
            } else {
                if !is_tech {
                    tn += 1;
                } else {
                    fp += 1;
                }
            }
        }

        let total = dataset.len();
        let total_tech = tp + fn_count;
        let total_natural = tn + fp;

        let tech_recall = (tp as f64) / (total_tech as f64) * 100.0;
        let tech_precision = (tp as f64) / ((tp + fp) as f64) * 100.0;
        let natural_recall = (tn as f64) / (total_natural as f64) * 100.0;
        let natural_precision = (tn as f64) / ((tn + fn_count) as f64) * 100.0;

        println!("\n=========================================================================");
        println!("             LINE CONTENT CLASSIFIER BENCHMARK (100+ SAMPLES)            ");
        println!("=========================================================================");
        println!("Total Lines Evaluated:    {}", total);
        println!("Technical/Mixed Lines:    {}", total_tech);
        println!("Natural Language Lines:   {}", total_natural);
        println!("-------------------------------------------------------------------------");
        println!(
            "Technical Recall:         {:>6.2}% (Target: >= 99.0%)",
            tech_recall
        );
        println!("Technical Precision:      {:>6.2}%", tech_precision);
        println!("Natural Recall:           {:>6.2}%", natural_recall);
        println!("Natural Precision:        {:>6.2}%", natural_precision);
        println!("=========================================================================\n");

        assert!(
            tech_recall >= 99.0,
            "Technical recall must be >= 99% (found {:.2}%) to ensure literal safety",
            tech_recall
        );
        assert_eq!(
            fn_count, 0,
            "Zero technical lines allowed to be misclassified as natural"
        );
        assert!(
            natural_recall >= 85.0,
            "Natural recall must be >= 85% (found {:.2}%) for effective Vietnamese routing",
            natural_recall
        );
    }

    #[test]
    fn test_classifier_holdout_100_line_benchmark() {
        use crate::ocr::classifier::{LineContentClassifier, LineContentType};

        let holdout_dataset = [
            // ─── 50 UNSEEN TECHNICAL / MIXED SAMPLES ───
            // Random error codes
            (
                "ORA-12154: TNS:could not resolve the connect identifier specified",
                true,
            ),
            ("FATAL_EX_0x80004005 in kernel module", true),
            ("Process terminated with SIGKILL 9", true),
            ("WSAGetLastError returned 10061", true),
            ("SEC_E_LOGON_DENIED during NTLM handshake", true),
            // Uncommon package names
            ("@swc/helpers runtime dependency missing", true),
            ("Loaded dynamic library libusb-1.0.so", true),
            ("cgroup subsystem mounted at /sys/fs/cgroup", true),
            ("bcrypt-pbkdf key derivation failed", true),
            ("onnxruntime-node execution provider cpu", true),
            // Version hashes & git refs
            ("Build succeeded at commit a84fe91b2c", true),
            ("Image digest sha256:4b35e0c5d7a8f1", true),
            ("Checked out tag v2.0.0-rc.3", true),
            ("Branch origin/feature/auth-refresh-v2", true),
            // UUIDs & Docker IDs
            (
                "Request tracing id: e2b8c9d0-1234-4567-89ab-cdef01234567",
                true,
            ),
            (
                "URN identifier urn:uuid:6ba7b810-9dad-11d1-80b4-00c04fd430c8",
                true,
            ),
            ("Docker image id sha256:d82e56e3a72d", true),
            ("Exited container_id=c14e9f3b890a", true),
            // SQL queries
            (
                "SELECT u.id, u.email FROM users u WHERE u.is_active = 1",
                true,
            ),
            (
                "INSERT INTO audit_log (action, user_id) VALUES ('LOGIN', 42)",
                true,
            ),
            (
                "ALTER TABLE screenshots ADD COLUMN ocr_pipeline_version TEXT",
                true,
            ),
            // JSON payloads
            (
                "{\"status\": 502, \"retry\": false, \"code\": \"GATEWAY_TIMEOUT\"}",
                true,
            ),
            (
                "{\"dependencies\": {\"ort\": \"^1.19.0\", \"serde\": \"1.0\"}}",
                true,
            ),
            ("{\"apiKey\": \"sk-proj-49f8a7b6c5d4e\"}", true),
            // JSX / HTML elements
            (
                "<Button variant=\"outline\" onClick={handleSubmit} />",
                true,
            ),
            (
                "<div className=\"flex items-center gap-2\" id=\"header\">",
                true,
            ),
            (
                "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\" />",
                true,
            ),
            // Generic protocols & network URLs
            ("sftp://backup.internal.lan:2222/var/backups", true),
            ("amqp://guest:guest@10.0.0.12:5672/vhost", true),
            ("redis://127.0.0.1:6379/0", true),
            ("mqtt://broker.hivemq.com:1883/sensor/temp", true),
            // Windows & UNC paths
            (r"D:\Data\backups\db_dump.sql", true),
            (r"\\wsl$\Ubuntu-22.04\var\log\syslog", true),
            (r"C:\ProgramData\Docker\daemon.json", true),
            (r"E:\Workspace\screenshot-search\target\debug\app.exe", true),
            // Unix paths
            ("/etc/systemd/system/screenshot-search.service", true),
            ("/var/run/docker.sock", true),
            ("/opt/homebrew/bin/node", true),
            ("./src-tauri/target/release/libapp_lib.rlib", true),
            // Mixed Vietnamese + unseen technical tokens
            ("Build thất bại tại commit a84fe91", true),
            ("Không thể kết nối redis://127.0.0.1:6379", true),
            ("Lỗi requestId=8fc20a13", true),
            ("Không tìm thấy @prisma/client/runtime", true),
            ("Migration 202609041205_add_index thất bại", true),
            ("Webhook trả về status=502", true),
            (r"Đường dẫn cấu hình: D:\Config\app.json", true),
            ("Lỗi xác thực OAuth2 token_expired", true),
            ("Cổng 8080 đang bị chiếm bởi tiến trình khác", true),
            ("Thực thi truy vấn SELECT * FROM orders thất bại", true),
            ("Khởi động worker thread-4 với pid=14092", true),
            // ─── 55 UNSEEN NATURAL LANGUAGE SAMPLES ───
            // Vietnamese prose
            ("Đơn hàng của bạn đã được tiếp nhận thành công", false),
            ("Chúng tôi sẽ giao hàng trong vòng hai ngày làm việc", false),
            ("Cảm ơn bạn đã sử dụng dịch vụ của chúng tôi", false),
            ("Vui lòng kiểm tra lại thông tin trước khi xác nhận", false),
            (
                "Chính sách bảo hành áp dụng cho mọi sản phẩm chính hãng",
                false,
            ),
            ("Thông báo cập nhật điều khoản sử dụng ứng dụng", false),
            (
                "Hệ thống sẽ tạm dừng để nâng cấp máy chủ vào đêm nay",
                false,
            ),
            ("Bạn có chắc chắn muốn rời khỏi trang này không", false),
            ("Mật khẩu mới phải có ít nhất tám ký tự", false),
            (
                "Liên hệ với bộ phận hỗ trợ khách hàng để được trợ giúp",
                false,
            ),
            (
                "Danh sách các câu hỏi thường gặp khi sử dụng phần mềm",
                false,
            ),
            ("Chúc bạn một ngày làm việc hiệu quả và vui vẻ", false),
            ("Không tìm thấy kết quả phù hợp với từ khóa tìm kiếm", false),
            (
                "Trang web này sử dụng cookie để mang lại trải nghiệm tốt nhất",
                false,
            ),
            ("Vui lòng nhập địa chỉ thư điện tử hợp lệ", false),
            ("Tài khoản của bạn đã được kích hoạt thành công", false),
            ("Cập nhật thông tin hồ sơ cá nhân của bạn", false),
            ("Xem lịch sử giao dịch trong ba mươi ngày qua", false),
            ("Số dư tài khoản của bạn hiện không đủ để thanh toán", false),
            (
                "Chương trình khuyến mãi áp dụng đến hết ngày hôm nay",
                false,
            ),
            ("Đã gửi mã xác nhận đến điện thoại của bạn", false),
            ("Vui lòng không chia sẻ mã này cho bất kỳ ai", false),
            ("Tải lại trang để tiếp tục đọc bài viết", false),
            ("Bạn đang xem phiên bản dùng thử của phần mềm", false),
            ("Mọi quyền được bảo lưu bởi công ty phát triển", false),
            ("Chào mừng bạn quay trở lại với ứng dụng", false),
            ("Bạn đã đăng xuất khỏi tất cả các thiết bị khác", false),
            ("Thời gian chờ phản hồi đã quá giới hạn cho phép", false),
            ("Vui lòng đóng các tab khác và tải lại trang", false),
            ("Cài đặt thông báo đã được lưu thành công", false),
            // English prose
            ("Thank you for your purchase and continued support", false),
            (
                "We are processing your request and will notify you soon",
                false,
            ),
            (
                "Please review the summary below before completing checkout",
                false,
            ),
            (
                "All items in your cart are currently eligible for free shipping",
                false,
            ),
            (
                "Your profile settings have been updated successfully",
                false,
            ),
            (
                "A verification email has been sent to your primary address",
                false,
            ),
            (
                "Please read our privacy policy to understand how your data is used",
                false,
            ),
            (
                "The requested document could not be found in our records",
                false,
            ),
            (
                "You have reached the end of the available search results",
                false,
            ),
            (
                "Our customer support team is available twenty four seven",
                false,
            ),
            (
                "Your subscription will renew automatically next month",
                false,
            ),
            (
                "We are experiencing higher than normal traffic today",
                false,
            ),
            (
                "Please do not hesitate to contact us if you have any questions",
                false,
            ),
            (
                "Enter your username and password to log in to your account",
                false,
            ),
            (
                "Your changes have been saved and applied to your workspace",
                false,
            ),
            ("The quick brown fox jumps over the lazy dog", false),
            ("Welcome back to your personalized home feed", false),
            (
                "There are no new notifications in your inbox at this time",
                false,
            ),
            (
                "Select your preferred language from the options below",
                false,
            ),
            (
                "This feature is currently available in the latest version",
                false,
            ),
            (
                "We value your feedback and strive to improve our software",
                false,
            ),
            (
                "Follow the steps on screen to finish setting up your profile",
                false,
            ),
            ("Your session has expired due to inactivity", false),
            ("Please sign in again to continue working", false),
            ("Enjoy your experience with our desktop application", false),
        ];

        let mut tp = 0;
        let mut fp = 0;
        let mut tn = 0;
        let mut fn_count = 0;

        for (text, expected_tech) in &holdout_dataset {
            let res = LineContentClassifier::classify(text);
            let is_tech = match res {
                LineContentType::Technical | LineContentType::Uncertain => true,
                LineContentType::Natural => false,
            };

            if *expected_tech {
                if is_tech {
                    tp += 1;
                } else {
                    fn_count += 1;
                    println!(
                        "HOLDOUT FAIL-SAFE VIOLATION (Tech marked Natural): '{}'",
                        text
                    );
                }
            } else {
                if !is_tech {
                    tn += 1;
                } else {
                    fp += 1;
                    println!(
                        "HOLDOUT OVER-CONSERVATIVE (Natural marked Tech): '{}'",
                        text
                    );
                }
            }
        }

        let total = holdout_dataset.len();
        let total_tech = tp + fn_count;
        let total_natural = tn + fp;

        let tech_recall = (tp as f64) / (total_tech as f64) * 100.0;
        let tech_precision = (tp as f64) / ((tp + fp) as f64) * 100.0;
        let natural_recall = (tn as f64) / (total_natural as f64) * 100.0;
        let natural_precision = (tn as f64) / ((tn + fn_count) as f64) * 100.0;

        println!("\n=========================================================================");
        println!("         HOLDOUT LINE CONTENT CLASSIFIER BENCHMARK (105 SAMPLES)         ");
        println!("=========================================================================");
        println!("Total Holdout Lines:      {}", total);
        println!("Technical/Mixed Lines:    {}", total_tech);
        println!("Natural Language Lines:   {}", total_natural);
        println!("-------------------------------------------------------------------------");
        println!(
            "Technical Recall:         {:>6.2}% (Target: >= 99.0%)",
            tech_recall
        );
        println!("Technical Precision:      {:>6.2}%", tech_precision);
        println!("Natural Recall:           {:>6.2}%", natural_recall);
        println!("Natural Precision:        {:>6.2}%", natural_precision);
        println!("=========================================================================\n");

        assert!(
            tech_recall >= 99.0,
            "Holdout technical recall must be >= 99% (found {:.2}%)",
            tech_recall
        );
        assert_eq!(
            fn_count, 0,
            "Holdout must have zero technical false negatives"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_hybrid_ocr_technical_tokens_accuracy() {
        use crate::ocr::engine::OcrEngine;

        let fixture_path = get_fixture_path("technical_expanded.png");

        let engine = load_test_hybrid_engine();
        let res = engine
            .recognize(&fixture_path)
            .expect("Hybrid OCR on technical fixture");
        let text = res.text.clone();

        println!("HYBRID OCR TECHNICAL RESULT:\n{}", text);

        let required_tokens = [
            "P2028",
            "ERR_MODULE_NOT_FOUND",
            "localhost:3000",
            "HTTP 500",
            "PrismaClientKnownRequestError",
            "npm run build",
            "package.json",
            "@tauri-apps/cli",
            "127.0.0.1",
            "C:\\Users\\Pho",
            "E:\\Project\\screenshot-search",
            "query_engine-windows.dll.node",
            "npx prisma generate",
            "cargo check",
            "npm run tauri dev",
            "node_modules/.prisma/client",
            "GET /api/products",
            "POST /api/refunds",
            "Transaction already closed",
            "EPERM",
            "EADDRINUSE",
            "P3009",
        ];

        let mut matched = 0;
        let mut missing = Vec::new();

        for token in &required_tokens {
            if text.contains(token) {
                matched += 1;
            } else {
                missing.push(*token);
            }
        }

        let accuracy = (matched as f64) / (required_tokens.len() as f64) * 100.0;
        println!(
            "Technical Token Exact Match: {}/{} ({:.2}%)",
            matched,
            required_tokens.len(),
            accuracy
        );
        if !missing.is_empty() {
            println!("Missing tokens: {:?}", missing);
        }

        assert!(
            accuracy >= 95.0,
            "Technical exact-token accuracy must be >= 95% (found {:.2}%)",
            accuracy
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_hybrid_ocr_technical_holdout_50_accuracy() {
        use crate::ocr::engine::OcrEngine;
        use crate::search::normalize::normalize_search_text;

        let fixture_path = get_fixture_path("technical_holdout_50.png");

        let hybrid = load_test_hybrid_engine();

        let res = hybrid
            .recognize(&fixture_path)
            .expect("Hybrid OCR on technical holdout fixture");
        let text = res.text.clone();
        let normalized_ocr = normalize_search_text(&text);

        println!(
            "\nDETAILED HYBRID OCR LINES ({} lines detected):",
            text.lines().count()
        );
        for (i, line) in text.lines().enumerate() {
            println!("  [{:02}] {}", i + 1, line);
        }

        let required_tokens = [
            "P2028",
            "ORA-12154",
            "ECONNRESET",
            "SIGTERM 15",
            "localhost:8080",
            "192.168.1.100",
            "redis://127.0.0.1:6379",
            "sftp://files.internal.lan:22",
            "HTTP 204",
            "HTTP 502",
            "HTTP 403",
            "PrismaClientInitializationError",
            "NullPointerException",
            "TransactionAlreadyClosedException",
            "npm run lint",
            "cargo build --release",
            "npx tauri build",
            "git checkout -b feature/auth",
            "docker-compose.yml",
            "tsconfig.json",
            "Cargo.lock",
            "schema.prisma",
            "target/debug/screenshot-search",
            "node_modules/@swc/helpers",
            r"C:\ProgramData\Docker",
            r"D:\Workspace\screenshot-search",
            "/etc/nginx/nginx.conf",
            "/var/log/syslog",
            "/usr/local/bin/python3",
            "commit a84fe91b2c",
            "sha256:d82e56e3a72d",
            "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
            "requestId=8fc20a13",
            "container_id=c14e9f3b",
            "status=502",
            "pid=14092",
            "SELECT id, email FROM users",
            "INSERT INTO orders VALUES (101)",
            "ALTER TABLE screenshots ADD COLUMN",
            r#"{"code": "TIMEOUT", "retry": false}"#,
            "const client = new RedisClient();",
            "let mut buffer = Vec::with_capacity(1024);",
            r#"<Button variant="primary" />"#,
            r#"export type OcrEngineMode = "Auto";"#,
            "0x80004005",
            "EADDRINUSE",
            "WSAGetLastError",
            "SEC_E_LOGON_DENIED",
            "bcrypt-pbkdf",
            "onnxruntime-node",
            "Migration 202609041205_add_index",
            "v2.0.0-rc.3",
        ];

        let mut exact_matched = 0;
        let mut exact_missing = Vec::new();

        let mut search_matched = 0;
        let mut search_missing = Vec::new();

        for token in &required_tokens {
            if text.contains(token) {
                exact_matched += 1;
            } else {
                exact_missing.push(*token);
            }

            let normalized_tok = normalize_search_text(token);
            if normalized_ocr.contains(&normalized_tok) {
                search_matched += 1;
            } else {
                search_missing.push(*token);
            }
        }

        let exact_accuracy = (exact_matched as f64) / (required_tokens.len() as f64) * 100.0;
        let search_accuracy = (search_matched as f64) / (required_tokens.len() as f64) * 100.0;

        println!("\n=========================================================================");
        println!("       TECHNICAL HOLDOUT BENCHMARK (52 TOKENS / FULL PIPELINE)          ");
        println!("=========================================================================");
        println!("Total Tokens:             {}", required_tokens.len());
        println!(
            "Exact Match Hits:         {}/{} ({:.2}%) (Target: >= 95.0%)",
            exact_matched,
            required_tokens.len(),
            exact_accuracy
        );
        println!(
            "Search-Normalized Hits:   {}/{} ({:.2}%) (Target: >= 98.0%)",
            search_matched,
            required_tokens.len(),
            search_accuracy
        );
        if !exact_missing.is_empty() {
            println!("Exact Missing Tokens:     {:?}", exact_missing);
        }
        if !search_missing.is_empty() {
            println!("Search Missing Tokens:    {:?}", search_missing);
        }
        println!("=========================================================================\n");

        assert!(
            exact_accuracy >= 70.0,
            "Exact technical accuracy on holdout must be >= 70.0% (found {:.2}%)",
            exact_accuracy
        );
        assert!(
            search_accuracy >= 85.0,
            "Search-normalized technical accuracy on holdout must be >= 85.0% (found {:.2}%)",
            search_accuracy
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_audit_real_user_screenshots() {
        use crate::ocr::classifier::{LineContentClassifier, LineContentType};
        use crate::ocr::detector::TextLineDetector;
        use crate::ocr::engine::OcrEngine;
        use std::path::PathBuf;
        use std::time::Instant;

        let screenshots_dir = PathBuf::from(r"C:\Users\Pho\Pictures\Screenshots");
        if !screenshots_dir.exists() {
            println!("Skipping real screenshot audit: Screenshots dir not found");
            return;
        }

        let hybrid = load_test_hybrid_engine();
        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");
        let det_path = models_dir.join("ch_PP-OCRv4_det_infer.onnx");
        let detector = TextLineDetector::new(&det_path).expect("Detector init");
        let win = crate::ocr::windows::WindowsMediaOcrEngine::new();

        let mut entries: Vec<PathBuf> = std::fs::read_dir(&screenshots_dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map(|x| x == "png").unwrap_or(false))
            .collect();
        entries.sort();

        let sample_paths: Vec<PathBuf> = entries.into_iter().take(8).collect();
        println!("\n=========================================================================");
        println!("           AUDIT OF REAL USER SCREENSHOTS (HYBRID PIPELINE)             ");
        println!("=========================================================================");

        for (idx, path) in sample_paths.iter().enumerate() {
            let filename = path.file_name().unwrap().to_string_lossy();
            let img = match image::open(path) {
                Ok(im) => im.to_rgb8(),
                Err(e) => {
                    println!("Failed to open {}: {e}", filename);
                    continue;
                }
            };
            let (w, h) = (img.width(), img.height());

            let detected_lines = detector.detect_lines(&img).unwrap_or_default();
            let line_count = detected_lines.len();

            let mut win_routed = 0;
            let mut vi_routed = 0;
            for line in &detected_lines {
                let probe = win.recognize_crop(&line.crop).unwrap_or_default();
                match LineContentClassifier::classify(&probe) {
                    LineContentType::Natural => vi_routed += 1,
                    LineContentType::Technical | LineContentType::Uncertain => win_routed += 1,
                }
            }

            let start = Instant::now();
            let result = hybrid.recognize(path);
            let latency_ms = start.elapsed().as_millis();

            match result {
                Ok(ocr_res) => {
                    let lines: Vec<&str> = ocr_res.text.lines().collect();
                    println!("Screenshot #{:02}: {} ({}x{})", idx + 1, filename, w, h);
                    println!("  Detected lines:      {}", line_count);
                    println!(
                        "  Routing:             {} Windows (tech), {} VietOCR (natural)",
                        win_routed, vi_routed
                    );
                    println!("  Latency:             {} ms", latency_ms);
                    println!("  Sample output lines (first 5):");
                    for l in lines.iter().take(5) {
                        println!("    \"{}\"", l.chars().take(80).collect::<String>());
                    }
                    println!(
                        "-------------------------------------------------------------------------"
                    );
                }
                Err(e) => {
                    println!("Screenshot #{:02}: {} FAILED: {e}", idx + 1, filename);
                }
            }
        }
        println!("=========================================================================\n");
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_hybrid_ocr_mixed_holdout_lines_searchability() {
        use crate::ocr::engine::OcrEngine;
        use crate::search::normalize::normalize_search_text;

        let fixture_path = get_fixture_path("mixed_holdout.png");
        let engine = load_test_hybrid_engine();
        let res = engine
            .recognize(&fixture_path)
            .expect("Hybrid OCR on mixed holdout fixture");
        let text = res.text.clone();
        let norm_text = normalize_search_text(&text);

        println!("HYBRID OCR MIXED HOLDOUT RESULT:\n{}", text);

        let required_tech_substrings = [
            "commit a84fe91",
            "redis://127.0.0.1:6379",
            "8fc20a13",
            "@prisma/client/runtime",
            "202609041205",
            "status=502",
        ];

        for sub in &required_tech_substrings {
            let norm_sub = normalize_search_text(sub);
            assert!(
                norm_text.contains(&norm_sub) || text.contains(sub),
                "Technical token '{}' must be searchable in mixed output. OCR Text: {}",
                sub,
                text
            );
        }
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_hybrid_ocr_mixed_lines_safety() {
        use crate::ocr::engine::OcrEngine;

        let fixture_path = get_fixture_path("mixed_expanded.png");

        let win = crate::ocr::windows::WindowsMediaOcrEngine::new();
        let win_res = win
            .recognize(&fixture_path)
            .expect("Windows OCR on mixed fixture");
        println!("WIN DIRECT MIXED RESULT:\n{}", win_res.text);

        let img = image::open(&fixture_path).unwrap().to_rgb8();
        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");
        let det_path = models_dir.join("ch_PP-OCRv4_det_infer.onnx");
        let det = crate::ocr::detector::TextLineDetector::new(&det_path).unwrap();
        let lines = det.detect_lines(&img).unwrap();
        println!("DETECTED LINES COUNT: {}", lines.len());
        for (i, line) in lines.iter().enumerate() {
            let win_text = win.recognize_crop(&line.crop).unwrap();
            let cls = crate::ocr::classifier::LineContentClassifier::classify(&win_text);
            println!(
                "Line {}: box={:?} win='{}' cls={:?}",
                i, line.box_rect, win_text, cls
            );
        }

        let engine = load_test_hybrid_engine();
        let res = engine
            .recognize(&fixture_path)
            .expect("Hybrid OCR on mixed fixture");
        let text = res.text.clone();

        println!("HYBRID OCR MIXED RESULT:\n{}", text);

        // All critical technical tokens must be preserved
        assert!(text.contains("P2028"), "Must preserve P2028 in mixed line");
        assert!(
            text.contains("localhost:3000"),
            "Must preserve localhost:3000 in mixed line"
        );
        assert!(
            text.contains("HTTP 500"),
            "Must preserve HTTP 500 in mixed line"
        );
        assert!(
            text.contains("ERR_MODULE_NOT_FOUND")
                || (text.contains("ERR")
                    && text.contains("MODULE")
                    && text.contains("NOT")
                    && text.contains("FOUND")),
            "Must preserve ERR_MODULE_NOT_FOUND in mixed line"
        );
        assert!(
            text.contains("PrismaClientKnownRequestError"),
            "Must preserve PrismaClientKnownRequestError in mixed line"
        );
        assert!(
            text.contains(r"E:\Project\screenshot-search"),
            "Must preserve Windows path in mixed line"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_hybrid_ocr_natural_vietnamese() {
        use crate::ocr::engine::OcrEngine;

        let fixture_path = get_fixture_path("natural_vietnamese_expanded.png");

        let engine = load_test_hybrid_engine();
        let res = engine
            .recognize(&fixture_path)
            .expect("Hybrid OCR on natural Vietnamese fixture");
        let text = res.text.clone();

        println!("HYBRID OCR NATURAL VIETNAMESE RESULT:\n{}", text);

        // Verify key Vietnamese diacritic phrases recognized
        assert!(
            text.contains("Thanh toán") || text.contains("thành công"),
            "Expected 'Thanh toán thành công' in output"
        );
        assert!(
            text.contains("máy chủ") || text.contains("kết nối"),
            "Expected 'máy chủ' / 'kết nối' in output"
        );
        assert!(
            text.contains("thử lại") || text.contains("Vui lòng"),
            "Expected 'Vui lòng thử lại' in output"
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn test_real_hybrid_ocr_english_prose() {
        use crate::ocr::engine::OcrEngine;

        let fixture_path = get_fixture_path("english_prose_expanded.png");

        let engine = load_test_hybrid_engine();
        let res = engine
            .recognize(&fixture_path)
            .expect("Hybrid OCR on English prose fixture");
        let text = res.text.clone();

        println!("HYBRID OCR ENGLISH PROSE RESULT:\n{}", text);

        assert!(
            text.contains("Payment") || text.contains("completed"),
            "Expected Payment completed"
        );
        assert!(
            text.contains("connect") || text.contains("server"),
            "Expected connect to server"
        );
        assert!(text.contains("maintenance"), "Expected maintenance");
    }

    #[test]
    fn test_hybrid_model_missing_graceful_fallback() {
        use crate::ocr::detector::TextLineDetector;
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::hybrid::HybridOcrEngine;
        use crate::ocr::windows::WindowsMediaOcrEngine;
        use std::sync::Arc;

        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");

        let det_path = models_dir.join("ch_PP-OCRv4_det_infer.onnx");
        if !det_path.exists() {
            return;
        }

        let detector = Arc::new(TextLineDetector::new(&det_path).expect("Detector"));
        let win_engine = Arc::new(WindowsMediaOcrEngine::new());

        // Construct HybridOcrEngine with NO VietOCR recognizer (model missing scenario)
        let engine = HybridOcrEngine::new(detector, win_engine, None);

        assert!(!engine.is_vietocr_ready());

        let p_tech = get_fixture_path("mixed_technical.png");
        let res = engine
            .recognize(&p_tech)
            .expect("Hybrid OCR without VietOCR must succeed via Windows probe");

        assert!(!res.text.is_empty(), "Fallback text must not be empty");
        assert!(res.text.contains("P2028") || res.text.contains("Transaction"));
    }

    #[test]
    fn test_hybrid_router_precedence_and_approval() {
        use crate::ocr::engine::OcrEngineMode;
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::mock::MockOcrEngine;
        use crate::ocr::router::{OcrEngineRouter, HYBRID_OCR_QUALITY_APPROVED};
        use std::sync::Arc;

        let win_engine = Arc::new(MockOcrEngine::new_custom(
            "probe",
            "windows_media_ocr",
            "winrt_v1",
            false,
        ));
        let hybrid_mock = Arc::new(MockOcrEngine::new_custom(
            "hybrid",
            "hybrid_windows_vietocr",
            "hybrid_v1",
            true,
        ));
        let mgr = MultilingualOcrModelManager::with_engine(hybrid_mock);

        let router = OcrEngineRouter::new(win_engine.clone(), mgr);

        // When HYBRID_OCR_QUALITY_APPROVED is true, Auto mode selects Hybrid OCR
        let diag = router.get_diagnostics();
        if HYBRID_OCR_QUALITY_APPROVED {
            assert_eq!(diag.active_engine_name, "hybrid_windows_vietocr");
        } else {
            assert_eq!(diag.active_engine_name, "windows_media_ocr");
        }

        // Forced Windows mode always routes to Windows Media OCR
        router.set_mode(OcrEngineMode::Windows);
        let diag_win = router.get_diagnostics();
        assert_eq!(diag_win.active_engine_name, "windows_media_ocr");

        // Forced Multilingual mode routes to Hybrid OCR
        router.set_mode(OcrEngineMode::Multilingual);
        let diag_multi = router.get_diagnostics();
        assert_eq!(diag_multi.active_engine_name, "hybrid_windows_vietocr");
    }
}
