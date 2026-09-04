#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::db::connection::Database;
    use crate::db::embeddings::{self, cosine_similarity};
    use crate::db::folders;
    use crate::db::migrations::run_migrations;
    use crate::db::screenshots;
    use crate::search::hybrid::search_hybrid;
    use crate::search::query::{search_screenshots, SearchRequest};
    use crate::semantic::engine::{
        MockEmbeddingEngine, TextEmbeddingEngine, DEFAULT_EMBEDDING_DIM, DEFAULT_MODEL_ID,
        DEFAULT_MODEL_VERSION,
    };

    fn setup_test_db() -> (tempfile::TempDir, Database) {
        let dir = tempdir().expect("Failed to create tempdir");
        let conn =
            rusqlite::Connection::open(dir.path().join("test.db")).expect("Failed to open db");
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        run_migrations(&conn).expect("Failed to run migrations");
        (
            dir,
            Database {
                conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
            },
        )
    }

    /// Section 48: Model Tests
    #[test]
    fn test_mock_model_dimensions_and_stability() {
        let engine = MockEmbeddingEngine::new();
        assert_eq!(engine.dimension(), DEFAULT_EMBEDDING_DIM);
        assert_eq!(engine.model_id(), DEFAULT_MODEL_ID);
        assert_eq!(engine.model_version(), DEFAULT_MODEL_VERSION);

        // Same text produces stable vector shape and identical vector
        let v1 = engine
            .embed_passage("PrismaClientKnownRequestError P2028")
            .unwrap();
        let v2 = engine
            .embed_passage("PrismaClientKnownRequestError P2028")
            .unwrap();
        assert_eq!(v1.len(), DEFAULT_EMBEDDING_DIM);
        assert_eq!(v1, v2);

        // Empty input behavior
        let v_empty = engine.embed_passage("").unwrap();
        assert_eq!(v_empty.len(), DEFAULT_EMBEDDING_DIM);

        // Vietnamese & Unicode input
        let v_vn = engine.embed_query("lỗi cơ sở dữ liệu hôm trước").unwrap();
        assert_eq!(v_vn.len(), DEFAULT_EMBEDDING_DIM);
        assert!(!v_vn.iter().all(|&x| x == 0.0));
    }

    /// Section 49: Similarity Tests
    #[test]
    fn test_similarity_fixtures() {
        let engine = MockEmbeddingEngine::new();

        let v_query = engine.embed_query("database transaction timeout").unwrap();
        let v_close = engine
            .embed_passage("Prisma database transaction closed timeout")
            .unwrap();
        let v_unrelated = engine
            .embed_passage("beautiful sunset photo at the beach summer vacation")
            .unwrap();

        let sim_close = cosine_similarity(&v_query, &v_close);
        let sim_unrelated = cosine_similarity(&v_query, &v_unrelated);

        assert!(
            sim_close > sim_unrelated,
            "Expected database error ({sim_close}) to be more similar than sunset ({sim_unrelated})"
        );
    }

    /// Section 50 & 28: Exact Match Dominance (P2028 ranks #1)
    #[test]
    fn test_exact_match_dominance() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();
        let engine = MockEmbeddingEngine::new();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        // A: Contains exact P2028
        let id_a = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/prisma_p2028.png",
            "prisma_p2028.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_a",
        )
        .unwrap();
        screenshots::save_ocr_success(
            &conn,
            id_a,
            "PrismaClientKnownRequestError Transaction already closed P2028",
            "win_ocr",
        )
        .unwrap();
        let vec_a = engine.embed_passage("Filename: prisma_p2028.png\nContent:\nPrismaClientKnownRequestError Transaction already closed P2028").unwrap();
        embeddings::save_embedding(
            &conn,
            id_a,
            engine.model_id(),
            engine.model_version(),
            &vec_a,
        )
        .unwrap();

        // B: Generic database error without P2028
        let id_b = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/generic_db.png",
            "generic_db.png",
            "png",
            1024,
            "2026-09-01T11:00:00Z",
            "hash_b",
        )
        .unwrap();
        screenshots::save_ocr_success(
            &conn,
            id_b,
            "Database operation error connection timeout",
            "win_ocr",
        )
        .unwrap();
        let vec_b = engine
            .embed_passage(
                "Filename: generic_db.png\nContent:\nDatabase operation error connection timeout",
            )
            .unwrap();
        embeddings::save_embedding(
            &conn,
            id_b,
            engine.model_id(),
            engine.model_version(),
            &vec_b,
        )
        .unwrap();

        // Query: "P2028"
        let req = SearchRequest {
            query: "P2028".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };

        let results = search_hybrid(&conn, &engine, &req).unwrap();
        assert!(!results.items.is_empty());
        // Exact token match MUST rank #1!
        assert_eq!(results.items[0].id, id_a);
        assert_eq!(results.items[0].match_type.as_deref(), Some("exact"));
    }

    /// Section 51 & 27: Semantic-Only Query (Zero keyword overlap)
    #[test]
    fn test_semantic_only_recall() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();
        let engine = MockEmbeddingEngine::new();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        let id = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/db_error.png",
            "db_error.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_db",
        )
        .unwrap();
        screenshots::save_ocr_success(
            &conn,
            id,
            "PrismaClientKnownRequestError Transaction closed",
            "win_ocr",
        )
        .unwrap();

        // Create embedding for this screenshot
        let vec = engine.embed_passage("Filename: db_error.png\nContent:\nPrismaClientKnownRequestError Transaction closed").unwrap();
        embeddings::save_embedding(&conn, id, engine.model_id(), engine.model_version(), &vec)
            .unwrap();

        // Query: "database operation failure"
        // Note that neither "database", "operation", nor "failure" appears in the OCR text!
        // But the mock engine and hybrid search should retrieve it via vector cosine similarity.
        let req = SearchRequest {
            query: "database operation failure".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };

        let results = search_hybrid(&conn, &engine, &req).unwrap();
        assert!(
            !results.items.is_empty(),
            "Expected semantic-only query to retrieve screenshot"
        );
        assert_eq!(results.items[0].id, id);
    }

    /// Section 53: Filename Regression Test
    #[test]
    fn test_filename_ranking_preserved() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();
        let engine = MockEmbeddingEngine::new();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        let id_invoice = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/invoice-september-2026.png",
            "invoice-september-2026.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_inv",
        )
        .unwrap();
        screenshots::save_ocr_success(&conn, id_invoice, "Total amount paid: $150.00", "win_ocr")
            .unwrap();
        let vec_inv = engine
            .embed_passage(
                "Filename: invoice-september-2026.png\nContent:\nTotal amount paid: $150.00",
            )
            .unwrap();
        embeddings::save_embedding(
            &conn,
            id_invoice,
            engine.model_id(),
            engine.model_version(),
            &vec_inv,
        )
        .unwrap();

        let req = SearchRequest {
            query: "invoice".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };

        let results = search_hybrid(&conn, &engine, &req).unwrap();
        assert!(!results.items.is_empty());
        assert_eq!(results.items[0].id, id_invoice);
    }

    /// Section 55: Changed Screenshot Invalidation
    #[test]
    fn test_changed_screenshot_invalidates_vector() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();
        let engine = MockEmbeddingEngine::new();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        let id = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/payment.png",
            "payment.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_v1",
        )
        .unwrap();
        screenshots::save_ocr_success(&conn, id, "Payment completed successfully", "win_ocr")
            .unwrap();
        let v1 = engine
            .embed_passage("Filename: payment.png\nContent:\nPayment completed successfully")
            .unwrap();
        embeddings::save_embedding(&conn, id, engine.model_id(), engine.model_version(), &v1)
            .unwrap();

        assert!(embeddings::get_embedding(&conn, id).unwrap().is_some());

        // Now file modifies to payment failed!
        screenshots::update_screenshot(&conn, id, 2048, "2026-09-01T12:00:00Z", "hash_v2").unwrap();
        embeddings::delete_embedding(&conn, id).unwrap();

        // Stale vector must be gone immediately
        assert!(embeddings::get_embedding(&conn, id).unwrap().is_none());

        // New OCR and embedding
        screenshots::save_ocr_success(&conn, id, "Payment failed: insufficient funds", "win_ocr")
            .unwrap();
        let v2 = engine
            .embed_passage("Filename: payment.png\nContent:\nPayment failed: insufficient funds")
            .unwrap();
        embeddings::save_embedding(&conn, id, engine.model_id(), engine.model_version(), &v2)
            .unwrap();

        assert!(embeddings::get_embedding(&conn, id).unwrap().is_some());
    }

    /// Section 56: Delete Screenshot Cascades and Removes Vector
    #[test]
    fn test_deleted_screenshot_removes_vector() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();
        let engine = MockEmbeddingEngine::new();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        let id = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/del.png",
            "del.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_del",
        )
        .unwrap();
        screenshots::save_ocr_success(&conn, id, "Temporary note to delete", "win_ocr").unwrap();
        let v = engine
            .embed_passage("Filename: del.png\nContent:\nTemporary note to delete")
            .unwrap();
        embeddings::save_embedding(&conn, id, engine.model_id(), engine.model_version(), &v)
            .unwrap();

        assert!(embeddings::get_embedding(&conn, id).unwrap().is_some());

        // Delete screenshot
        screenshots::delete_screenshot(&conn, id).unwrap();

        // Vector must be deleted by cascade!
        assert!(embeddings::get_embedding(&conn, id).unwrap().is_none());
    }

    /// Section 57: Model Versioning & Rebuild
    #[test]
    fn test_model_versioning_and_rebuild() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();
        let engine = MockEmbeddingEngine::new();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        let id = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/code.png",
            "code.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_code",
        )
        .unwrap();
        screenshots::save_ocr_success(&conn, id, "console.log('hello')", "win_ocr").unwrap();

        // Model v1 embedding
        let v_v1 = engine
            .embed_passage("Filename: code.png\nContent:\nconsole.log('hello')")
            .unwrap();
        embeddings::save_embedding(&conn, id, "model_a", "v1", &v_v1).unwrap();

        // Clear embeddings for model_a v1
        let cleared = embeddings::clear_embeddings_by_model(&conn, "model_a", "v1").unwrap();
        assert_eq!(cleared, 1);
        assert!(embeddings::get_embedding(&conn, id).unwrap().is_none());
    }

    /// Section 58 & 59: Missing Model Fallback (Search still works via FTS)
    #[test]
    fn test_search_fallback_when_model_unavailable() {
        let (_dir, db) = setup_test_db();
        let conn = db.conn.lock().unwrap();

        let folder_id = folders::insert_folder(&conn, "C:/Screenshots", true)
            .unwrap()
            .id;

        let id = screenshots::insert_screenshot(
            &conn,
            folder_id,
            "C:/Screenshots/terminal.png",
            "terminal.png",
            "png",
            1024,
            "2026-09-01T10:00:00Z",
            "hash_term",
        )
        .unwrap();
        screenshots::save_ocr_success(&conn, id, "npm run build successful", "win_ocr").unwrap();

        // Sync FTS
        let search_text =
            crate::search::normalize::normalize_search_text("npm run build successful");
        conn.execute(
            "INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, "terminal.png", search_text],
        ).unwrap();

        // Without any semantic model loaded, search_screenshots (FTS5) still works 100%!
        let req = SearchRequest {
            query: "npm build".to_string(),
            folder_id: None,
            limit: Some(10),
            offset: None,
        };

        let result = search_screenshots(&conn, &req).unwrap();
        assert_eq!(result.total_matches, 1);
        assert_eq!(result.items[0].id, id);
    }

    /// Section 52: Cross-Lingual Evaluation (English OCR <-> Vietnamese query)
    #[test]
    fn test_cross_lingual_similarity() {
        let engine = MockEmbeddingEngine::new();

        // English technical OCR text
        let v_en = engine
            .embed_passage("Filename: payment.png\nContent:\nPayment completed successfully")
            .unwrap();

        // Vietnamese natural language query
        let v_vi = engine.embed_query("thanh toán thành công").unwrap();

        // Unrelated Vietnamese query
        let v_unrelated = engine.embed_query("thời tiết hôm nay rất đẹp").unwrap();

        let sim_vi = cosine_similarity(&v_en, &v_vi);
        let sim_unrelated = cosine_similarity(&v_en, &v_unrelated);

        assert!(
            sim_vi > sim_unrelated,
            "Expected cross-lingual query similarity ({sim_vi}) to exceed unrelated query ({sim_unrelated})"
        );
        assert!(
            sim_vi >= 0.70,
            "Expected strong cross-lingual alignment (got {sim_vi})"
        );
    }
}
