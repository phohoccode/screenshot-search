use rusqlite::Connection;
use std::time::Instant;

use crate::db::migrations::run_migrations;
use crate::db::screenshots::{
    check_search_index_health, delete_screenshot, insert_screenshot, rebuild_search_index,
    save_ocr_success, update_screenshot,
};
use crate::search::normalize::{normalize_search_query, normalize_search_text};
use crate::search::query::{search_screenshots, SearchRequest};

fn setup_test_db() -> Connection {
    let conn = Connection::open_in_memory().expect("Failed to create in-memory database");
    run_migrations(&conn).expect("Failed to run migrations");
    conn
}

#[test]
fn test_search_normalization_technical_tokens() {
    // 1. Underscores to whitespace
    assert_eq!(
        normalize_search_text("ERR_MODULE_NOT_FOUND"),
        "err module not found"
    );

    // 2. Hyphens to whitespace
    assert_eq!(
        normalize_search_text("ERR-MODULE-NOT-FOUND"),
        "err module not found"
    );

    // 3. Raw spaces
    assert_eq!(
        normalize_search_text("ERR MODULE NOT FOUND"),
        "err module not found"
    );

    // 4. Exact technical token preservation
    assert_eq!(normalize_search_text("P2028"), "p2028");
    assert_eq!(normalize_search_text("HTTP 500"), "http 500");

    // 5. Port / URL separator normalization
    assert_eq!(normalize_search_text("localhost:3000"), "localhost 3000");

    // 6. Whitespace collapsing
    assert_eq!(
        normalize_search_text("Transaction    already   closed"),
        "transaction already closed"
    );

    // 7. Dotted semver & IP address preservation
    assert_eq!(normalize_search_text("v1.2.3"), "v1.2.3");
    assert_eq!(normalize_search_text("192.168.1.1"), "192.168.1.1");

    // 8. Filename normalization
    assert_eq!(
        normalize_search_text("invoice-september-2026.png"),
        "invoice september 2026.png"
    );

    // 9. Query normalization symmetry
    assert_eq!(
        normalize_search_query("ERR_MODULE_NOT_FOUND"),
        normalize_search_text("ERR MODULE NOT FOUND")
    );
}

#[test]
fn test_fts_exact_technical_token_matching() {
    let conn = setup_test_db();

    // Insert folder
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    // Insert screenshot
    let id = insert_screenshot(
        &conn,
        1,
        "C:\\Screenshots\\error.png",
        "error.png",
        "png",
        1024,
        "2026-09-03T10:00:00Z",
        "hash1",
    )
    .unwrap();

    // Save OCR with technical token
    save_ocr_success(
        &conn,
        id,
        "PrismaClientKnownRequestError: Transaction already closed (P2028)",
        "mock",
    )
    .unwrap();

    // Search exact code
    let req = SearchRequest {
        query: "P2028".to_string(),
        ..Default::default()
    };
    let res = search_screenshots(&conn, &req).unwrap();

    assert_eq!(res.total_matches, 1);
    assert_eq!(res.items[0].id, id);
    assert!(res.items[0]
        .match_snippet
        .as_ref()
        .unwrap()
        .contains("[[match]]p2028[[/match]]"));
}

#[test]
fn test_fts_ocr_punctuation_loss_matching() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    let id = insert_screenshot(
        &conn,
        1,
        "C:\\Screenshots\\terminal.png",
        "terminal.png",
        "png",
        2048,
        "2026-09-03T10:00:00Z",
        "hash2",
    )
    .unwrap();

    // OCR extracted text with space instead of underscore
    save_ocr_success(
        &conn,
        id,
        "npm run build\nERR MODULE NOT FOUND\nlocalhost : 3000",
        "mock",
    )
    .unwrap();

    // 1. Query with underscore matches OCR text with spaces
    let req_under = SearchRequest {
        query: "ERR_MODULE_NOT_FOUND".to_string(),
        ..Default::default()
    };
    let res_under = search_screenshots(&conn, &req_under).unwrap();
    assert_eq!(res_under.total_matches, 1);
    assert_eq!(res_under.items[0].id, id);

    // 2. Query with hyphen matches
    let req_hyphen = SearchRequest {
        query: "ERR-MODULE-NOT-FOUND".to_string(),
        ..Default::default()
    };
    let res_hyphen = search_screenshots(&conn, &req_hyphen).unwrap();
    assert_eq!(res_hyphen.total_matches, 1);

    // 3. Query localhost:3000 matches
    let req_url = SearchRequest {
        query: "localhost:3000".to_string(),
        ..Default::default()
    };
    let res_url = search_screenshots(&conn, &req_url).unwrap();
    assert_eq!(res_url.total_matches, 1);
}

#[test]
fn test_fts_phrase_search() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    let id = insert_screenshot(
        &conn,
        1,
        "C:\\Screenshots\\db.png",
        "db.png",
        "png",
        100,
        "2026-09-03T10:00:00Z",
        "hash3",
    )
    .unwrap();

    save_ocr_success(
        &conn,
        id,
        "Fatal error: transaction already closed during rollback",
        "mock",
    )
    .unwrap();

    let req = SearchRequest {
        query: "transaction already closed".to_string(),
        ..Default::default()
    };
    let res = search_screenshots(&conn, &req).unwrap();
    assert_eq!(res.total_matches, 1);
    assert_eq!(res.items[0].id, id);
}

#[test]
fn test_fts_filename_search() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    // Screenshot with filename but NO OCR text
    let id = insert_screenshot(
        &conn,
        1,
        "C:\\Screenshots\\invoice-september-2026.png",
        "invoice-september-2026.png",
        "png",
        500,
        "2026-09-03T10:00:00Z",
        "hash4",
    )
    .unwrap();

    save_ocr_success(&conn, id, "", "mock").unwrap();

    let req = SearchRequest {
        query: "invoice".to_string(),
        ..Default::default()
    };
    let res = search_screenshots(&conn, &req).unwrap();
    assert_eq!(res.total_matches, 1);
    assert_eq!(res.items[0].filename, "invoice-september-2026.png");
}

#[test]
fn test_fts_empty_query() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    for i in 1..=5 {
        let id = insert_screenshot(
            &conn,
            1,
            &format!("C:\\Screenshots\\img{i}.png"),
            &format!("img{i}.png"),
            "png",
            100,
            &format!("2026-09-03T1{i}:00:00Z"),
            &format!("h{i}"),
        )
        .unwrap();
        save_ocr_success(
            &conn,
            id,
            &format!("Text content for screenshot {i}"),
            "mock",
        )
        .unwrap();
    }

    let req_empty = SearchRequest {
        query: "".to_string(),
        ..Default::default()
    };
    let res_empty = search_screenshots(&conn, &req_empty).unwrap();
    assert_eq!(res_empty.total_matches, 5);
    assert_eq!(res_empty.items.len(), 5);
    // Verified sorted by recent date descending
    assert_eq!(res_empty.items[0].filename, "img5.png");

    let req_spaces = SearchRequest {
        query: "    ".to_string(),
        ..Default::default()
    };
    let res_spaces = search_screenshots(&conn, &req_spaces).unwrap();
    assert_eq!(res_spaces.total_matches, 5);
}

#[test]
fn test_fts_malformed_syntax_safety() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    let bad_queries = [
        "\"",
        "\"\"",
        "*",
        "(",
        ")",
        "AND",
        "OR NOT NEAR",
        "hello \"world",
        "::*^%$#@!",
        "SELECT * FROM screenshots",
    ];

    for bad in bad_queries {
        let req = SearchRequest {
            query: bad.to_string(),
            ..Default::default()
        };
        let res = search_screenshots(&conn, &req);
        assert!(
            res.is_ok(),
            "Query '{bad}' caused a database error: {:?}",
            res.err()
        );
    }
}

#[test]
fn test_fts_pagination() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    for i in 1..=10 {
        let id = insert_screenshot(
            &conn,
            1,
            &format!("C:\\Screenshots\\test{i}.png"),
            &format!("test{i}.png"),
            "png",
            100,
            &format!("2026-09-03T{:02}:00:00Z", i),
            &format!("h{i}"),
        )
        .unwrap();
        save_ocr_success(&conn, id, "common keyword in all screenshots", "mock").unwrap();
    }

    // Page 1: limit 3, offset 0
    let res1 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "common keyword".to_string(),
            limit: Some(3),
            offset: Some(0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res1.total_matches, 10);
    assert_eq!(res1.items.len(), 3);
    assert!(res1.has_more);

    // Page 2: limit 3, offset 3
    let res2 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "common keyword".to_string(),
            limit: Some(3),
            offset: Some(3),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res2.items.len(), 3);
    assert!(res2.has_more);
    assert_ne!(res1.items[0].id, res2.items[0].id);

    // Page 4: limit 3, offset 9
    let res4 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "common keyword".to_string(),
            limit: Some(3),
            offset: Some(9),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res4.items.len(), 1);
    assert!(!res4.has_more);
}

#[test]
fn test_fts_deleted_screenshot_sync() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    let id = insert_screenshot(
        &conn,
        1,
        "C:\\Screenshots\\delete_me.png",
        "delete_me.png",
        "png",
        100,
        "2026-09-03T10:00:00Z",
        "h1",
    )
    .unwrap();

    save_ocr_success(&conn, id, "UniqueTermXYZ will be deleted", "mock").unwrap();

    // Verify found
    let res_before = search_screenshots(
        &conn,
        &SearchRequest {
            query: "UniqueTermXYZ".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_before.total_matches, 1);

    // Delete screenshot
    delete_screenshot(&conn, id).unwrap();

    // Verify no longer found
    let res_after = search_screenshots(
        &conn,
        &SearchRequest {
            query: "UniqueTermXYZ".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_after.total_matches, 0);
}

#[test]
fn test_fts_changed_screenshot_invalidation() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    let id = insert_screenshot(
        &conn,
        1,
        "C:\\Screenshots\\modified.png",
        "modified.png",
        "png",
        100,
        "2026-09-03T10:00:00Z",
        "h1",
    )
    .unwrap();

    save_ocr_success(&conn, id, "InitialErrorP2028", "mock").unwrap();

    // Matches initially
    let res1 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "InitialErrorP2028".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res1.total_matches, 1);

    // File content changed on disk -> update_screenshot resets OCR
    update_screenshot(&conn, id, 150, "2026-09-03T11:00:00Z", "h2").unwrap();

    // Must immediately cease matching old OCR text
    let res2 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "InitialErrorP2028".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res2.total_matches, 0);

    // After re-OCR, new content matches
    save_ocr_success(&conn, id, "NewErrorHTTP500", "mock").unwrap();
    let res3 = search_screenshots(
        &conn,
        &SearchRequest {
            query: "NewErrorHTTP500".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res3.total_matches, 1);
}

#[test]
fn test_rebuild_search_index_idempotency() {
    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'C:\\Screenshots')",
        [],
    )
    .unwrap();

    for i in 1..=5 {
        let id = insert_screenshot(
            &conn,
            1,
            &format!("C:\\Screenshots\\rec{i}.png"),
            &format!("rec{i}.png"),
            "png",
            100,
            "2026-09-03T10:00:00Z",
            &format!("h{i}"),
        )
        .unwrap();
        save_ocr_success(&conn, id, &format!("Searchable record content {i}"), "mock").unwrap();
    }

    let health_before = check_search_index_health(&conn).unwrap();
    assert_eq!(health_before.fts_count, 5);
    assert_eq!(health_before.succeeded_count, 5);
    assert!(health_before.is_healthy);

    // Rebuild index 1st time
    let count1 = rebuild_search_index(&conn).unwrap();
    assert_eq!(count1, 5);

    // Rebuild index 2nd time (idempotent)
    let count2 = rebuild_search_index(&conn).unwrap();
    assert_eq!(count2, 5);

    let health_after = check_search_index_health(&conn).unwrap();
    assert!(health_after.is_healthy);
    assert_eq!(health_after.fts_count, 5);

    let res = search_screenshots(
        &conn,
        &SearchRequest {
            query: "Searchable".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res.total_matches, 5);
}

#[test]
#[cfg(target_os = "windows")]
fn test_full_pipeline_integration_real_ocr_fixtures() {
    use crate::ocr::engine::OcrEngine;
    use crate::ocr::windows::WindowsMediaOcrEngine;
    use std::path::PathBuf;

    let conn = setup_test_db();
    conn.execute(
        "INSERT INTO folders (id, path) VALUES (1, 'tests/fixtures')",
        [],
    )
    .unwrap();

    let engine = WindowsMediaOcrEngine::new();

    let fixtures = [
        ("english.png", "Screenshot Search"),
        ("mixed_technical.png", "PrismaClientKnownRequestError"),
        ("code_terminal.png", "npm run build"),
    ];

    for (filename, _) in &fixtures {
        let mut fixture_path = PathBuf::from("tests/fixtures").join(filename);
        if !fixture_path.exists() {
            fixture_path = PathBuf::from("src-tauri/tests/fixtures").join(filename);
        }
        assert!(fixture_path.exists(), "Fixture {} not found", filename);

        let id = insert_screenshot(
            &conn,
            1,
            &fixture_path.to_string_lossy(),
            filename,
            "png",
            1000,
            "2026-09-03T12:00:00Z",
            &format!("hash_{filename}"),
        )
        .unwrap();

        let ocr_res = engine.recognize(&fixture_path).expect("Real OCR failed");
        save_ocr_success(&conn, id, &ocr_res.text, "windows_media_ocr").unwrap();
    }

    // 1. Search technical code from mixed_technical.png
    let res_code = search_screenshots(
        &conn,
        &SearchRequest {
            query: "P2028".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_code.total_matches, 1);
    assert_eq!(res_code.items[0].filename, "mixed_technical.png");

    // 2. Search phrase from mixed_technical.png
    let res_phrase = search_screenshots(
        &conn,
        &SearchRequest {
            query: "transaction already closed".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_phrase.total_matches, 1);
    assert_eq!(res_phrase.items[0].filename, "mixed_technical.png");

    // 3. Search code terminal with underscore query against OCR spaces
    let res_term = search_screenshots(
        &conn,
        &SearchRequest {
            query: "ERR_MODULE_NOT_FOUND".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_term.total_matches, 1);
    assert_eq!(res_term.items[0].filename, "code_terminal.png");

    // 4. Search English tokens
    let res_en = search_screenshots(
        &conn,
        &SearchRequest {
            query: "Screenshot Search".to_string(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(res_en.total_matches, 1);
    assert_eq!(res_en.items[0].filename, "english.png");
}

#[test]
fn test_search_benchmark_1k_and_10k() {
    let conn = setup_test_db();
    conn.execute("PRAGMA synchronous = OFF", []).unwrap();
    conn.execute("INSERT INTO folders (id, path) VALUES (1, 'C:\\Bench')", [])
        .unwrap();

    let sample_texts = [
        "PrismaClientKnownRequestError: Transaction API error. Transaction already closed P2028",
        "npm ERR! code ERR_MODULE_NOT_FOUND Cannot find package 'vite' imported from index.js",
        "HTTP/1.1 500 Internal Server Error - Database connection refused on localhost:5432",
        "Authentication successful for user admin@example.com token expires in 3600 seconds",
        "Screenshot Search local desktop OCR indexing pipeline completed successfully",
        "TypeError: Cannot read properties of undefined (reading 'map') in React component",
        "Docker container exited with code 137 OOMKilled out of memory on node-4",
        "Payment invoice #2026-09-03 verified total amount $150.00 USD paid via Stripe",
        "SELECT * FROM users WHERE active = 1 ORDER BY created_at DESC LIMIT 50",
        "Git merge conflict in src/app.tsx please resolve before committing to main branch",
    ];

    println!("\n================== EMPIRICAL FTS5 SEARCH BENCHMARK ==================");
    println!(
        "{:<15} | {:<25} | {:<12} | {:<8}",
        "Dataset Size", "Query Type", "Latency", "Matches"
    );
    println!("{:-<15}-|-{:-<25}-|-{:-<12}-|-{:-<8}", "", "", "", "");

    let benchmarks = [(1_000, "1,000 Records"), (10_000, "10,000 Records")];

    let mut current_count = 0;
    for (target_count, label) in benchmarks {
        // Batch insert in a single transaction for speed
        conn.execute("BEGIN TRANSACTION", []).unwrap();
        for i in (current_count + 1)..=target_count {
            let sample = sample_texts[i % sample_texts.len()];
            let filename = format!("screenshot_{i:05}.png");
            let path = format!("C:\\Bench\\{filename}");
            let hash = format!("h_{i}");

            conn.execute(
                "INSERT INTO screenshots (folder_id, path, filename, extension, file_size, modified_at_fs, content_hash, ocr_text, ocr_status, indexed_at)
                 VALUES (1, ?1, ?2, 'png', 1024, '2026-09-03T12:00:00Z', ?3, ?4, 'SUCCEEDED', datetime('now'))",
                rusqlite::params![path, filename, hash, sample],
            ).unwrap();

            let rowid: i64 = conn.last_insert_rowid();
            let search_text = normalize_search_text(sample);
            conn.execute(
                "INSERT INTO screenshots_fts (rowid, filename, ocr_search_text) VALUES (?1, ?2, ?3)",
                rusqlite::params![rowid, filename, search_text],
            ).unwrap();
        }
        conn.execute("COMMIT", []).unwrap();
        current_count = target_count;

        let query_cases = [
            ("Single Token", "P2028"),
            ("Multi-word Phrase", "transaction already closed"),
            ("OCR Underscore Tolerance", "ERR_MODULE_NOT_FOUND"),
            ("Filename Query", "screenshot_00500"),
            ("No-match Query", "NonExistentTermXYZ999"),
        ];

        for (type_name, query) in query_cases {
            let start = Instant::now();
            let res = search_screenshots(
                &conn,
                &SearchRequest {
                    query: query.to_string(),
                    limit: Some(50),
                    offset: Some(0),
                    ..Default::default()
                },
            )
            .unwrap();
            let elapsed = start.elapsed();

            println!(
                "{:<15} | {:<25} | {:>9.2} ms | {:>7}",
                label,
                type_name,
                elapsed.as_secs_f64() * 1000.0,
                res.total_matches
            );
        }
        println!("{:-<15}-|-{:-<25}-|-{:-<12}-|-{:-<8}", "", "", "", "");
    }
    println!("=====================================================================\n");
}
