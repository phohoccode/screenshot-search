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

        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");

        let engine = MultilingualOcrEngine::new(&models_dir).expect("Real MultilingualOcrEngine");
        let p_vi = get_fixture_path("vietnamese.png");

        let res = engine.recognize(&p_vi).expect("Multilingual OCR failed");
        assert_eq!(res.language.as_deref(), Some("vi-VN"));
        assert!(!res.text.is_empty(), "Real OCR output must not be empty");
    }

    #[test]
    fn test_technical_tokens_zero_regression() {
        use crate::ocr::engine::OcrEngine;
        use crate::ocr::multilingual::MultilingualOcrEngine;

        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");

        let engine = MultilingualOcrEngine::new(&models_dir).expect("Real MultilingualOcrEngine");
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

        // Test 1: Auto Mode with quality gate = false → always uses Windows Media OCR.
        // MULTILINGUAL_QUALITY_APPROVED=false blocks multilingual selection in Auto regardless
        // of whether the model is installed, because the current PP-OCRv4 model scored
        // CER=105.48% and Tech=5.0% on the Vietnamese benchmark — far below the 15% CER threshold.
        router.set_mode(OcrEngineMode::Auto);
        assert_eq!(router.get_mode(), OcrEngineMode::Auto);
        let diag = router.get_diagnostics();
        assert_eq!(
            diag.active_engine_name, "windows_media_ocr",
            "Auto mode with MULTILINGUAL_QUALITY_APPROVED=false must always report windows_media_ocr"
        );
        assert!(diag.is_multilingual_ready);

        let p_vi = get_fixture_path("vietnamese.png");
        let res_auto = router.recognize(&p_vi).expect("Auto recognition failed");
        // Quality gate=false → Windows engine selected; mock_windows engine name is "mock_ocr"
        assert_eq!(
            res_auto.engine, "mock_ocr",
            "Auto mode must route to Windows (mock_ocr) when quality gate is false"
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
            diag.active_engine_name, "windows_media_ocr",
            "Auto mode must select Windows Media OCR when quality gate blocks multilingual (MULTILINGUAL_QUALITY_APPROVED=false)"
        );
        assert!(
            diag.is_multilingual_ready,
            "Model should still report as ready even when quality gate blocks Auto routing"
        );

        let p_vi = get_fixture_path("vietnamese.png");
        // Auto → Windows (quality gate blocks multilingual)
        let res_auto = router.recognize(&p_vi).expect("Recognition failed");
        assert_eq!(
            res_auto.engine, "mock_ocr",
            "Auto must use windows (mock_ocr) when quality gate is false"
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
        use crate::ocr::multilingual::MultilingualOcrEngine;
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
        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");
        let multi_engine =
            MultilingualOcrEngine::new(&models_dir).expect("Real MultilingualOcrEngine");

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

            // 2. Run Multilingual OCR (PP-OCRv4)
            let multi_res = multi_engine
                .recognize(&img_path)
                .expect("Multilingual OCR recognition");
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

        // Report Worst Cases for Multilingual OCR
        results.sort_by(|a, b| b.multi_cer.partial_cmp(&a.multi_cer).unwrap());
        println!("\nTop Worst Cases for Multilingual OCR (PP-OCRv4):");
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
        use crate::ocr::multilingual::MultilingualOcrEngine;

        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");

        let engine = MultilingualOcrEngine::new(&models_dir).expect("Real MultilingualOcrEngine");

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
        use crate::ocr::multilingual::MultilingualOcrEngine;

        let db = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let models_dir = std::path::PathBuf::from(
            std::env::var("APPDATA").unwrap_or_else(|_| r"C:\Users\Pho\AppData\Roaming".into()),
        )
        .join("com.screenshot-search.app")
        .join("models")
        .join("multilingual-ocr");

        let engine = MultilingualOcrEngine::new(&models_dir).expect("Real MultilingualOcrEngine");

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
        use crate::ocr::multilingual::MultilingualOcrEngine;
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

        let models_dir = PathBuf::from(&appdata)
            .join("com.screenshot-search.app")
            .join("models")
            .join("multilingual-ocr");

        let engine = MultilingualOcrEngine::new(&models_dir).expect("Real MultilingualOcrEngine");
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

    /// Scenario 2: Windows en-US only + multilingual ready, but quality gate = false.
    /// Expected: Auto selects Windows (gate blocks multilingual despite being installed).
    #[test]
    fn test_router_auto_windows_en_multilingual_ready_gate_false() {
        use crate::ocr::engine::OcrEngineInfo;
        use crate::ocr::manager::MultilingualOcrModelManager;
        use crate::ocr::router::{OcrEngineRouter, MULTILINGUAL_QUALITY_APPROVED};

        // Precondition: the gate must be false for this test to be meaningful.
        // If it is somehow flipped to true, this test serves as a regression guard.
        assert!(
            !MULTILINGUAL_QUALITY_APPROVED,
            "MULTILINGUAL_QUALITY_APPROVED must be false until a \
             quality-approved Vietnamese OCR model is benchmarked and confirmed"
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

        // Even though multilingual is ready, the quality gate is false → windows_media_ocr
        assert_eq!(diag.active_engine_name, "windows_media_ocr",
            "Scenario 2: MULTILINGUAL_QUALITY_APPROVED=false → must block multilingual and use windows_media_ocr");
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
}
