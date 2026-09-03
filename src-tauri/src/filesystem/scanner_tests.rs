#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use tempfile::tempdir;

    use crate::db;
    use crate::errors::ErrorCode;
    use crate::filesystem::metadata::{canonicalize_and_normalize, normalize_path};
    use crate::filesystem::scanner::{is_supported_extension, scan_directory};
    use crate::indexing::discovery::execute_discovery_scan;

    fn setup_test_db() -> Connection {
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
             CREATE UNIQUE INDEX idx_screenshots_path ON screenshots(path);",
        )
        .expect("Failed to create tables");
        conn
    }

    #[test]
    fn test_supported_extensions() {
        assert!(is_supported_extension("png"));
        assert!(is_supported_extension("PNG"));
        assert!(is_supported_extension("jpg"));
        assert!(is_supported_extension("JPG"));
        assert!(is_supported_extension("jpeg"));
        assert!(is_supported_extension("JPEG"));
        assert!(is_supported_extension("webp"));
        assert!(is_supported_extension("WEBP"));

        // Non-supported extensions
        assert!(!is_supported_extension("pdf"));
        assert!(!is_supported_extension("txt"));
        assert!(!is_supported_extension("exe"));
        assert!(!is_supported_extension("zip"));
        assert!(!is_supported_extension("svg"));
        assert!(!is_supported_extension("gif"));
    }

    #[test]
    fn test_windows_path_normalization_variants() {
        // Trailing separator stripping (non-root)
        assert_eq!(
            normalize_path(Path::new("C:\\Users\\Pho\\Screenshots\\")),
            "C:\\Users\\Pho\\Screenshots"
        );
        assert_eq!(
            normalize_path(Path::new("C:/Users/Pho/Screenshots/")),
            "C:\\Users\\Pho\\Screenshots"
        );

        // Root drive preservation
        assert_eq!(normalize_path(Path::new("c:\\")), "C:\\");
        assert_eq!(normalize_path(Path::new("C:\\")), "C:\\");

        // Verbatim \\?\ prefix stripping
        assert_eq!(
            normalize_path(Path::new(r"\\?\C:\Users\Pho\Screenshots")),
            "C:\\Users\\Pho\\Screenshots"
        );
        assert_eq!(
            normalize_path(Path::new(r"\\?\c:\Users\Pho\Screenshots\")),
            "C:\\Users\\Pho\\Screenshots"
        );

        // UNC extended path handling
        assert_eq!(
            normalize_path(Path::new(r"\\?\UNC\server\share\folder")),
            r"\\server\share\folder"
        );
    }

    #[test]
    fn test_canonicalize_and_normalize_existing_directory() {
        let dir = tempdir().expect("Failed to create tempdir");
        let raw_path = dir.path().to_str().unwrap();

        let canonical_expected = canonicalize_and_normalize(Path::new(raw_path));

        // Variant with trailing slash
        let with_trailing = format!("{raw_path}\\");
        assert_eq!(
            canonicalize_and_normalize(Path::new(&with_trailing)),
            canonical_expected
        );

        // Variant with forward slashes
        let with_forward = raw_path.replace('\\', "/");
        assert_eq!(
            canonicalize_and_normalize(Path::new(&with_forward)),
            canonical_expected
        );

        // Variant with lowercase drive letter
        let lower = raw_path.to_lowercase();
        assert_eq!(
            canonicalize_and_normalize(Path::new(&lower)),
            canonical_expected
        );
    }

    #[test]
    fn test_scan_directory_and_filter() {
        let dir = tempdir().expect("Failed to create tempdir");
        let dir_path = dir.path();

        // Create sample files
        File::create(dir_path.join("screenshot1.png"))
            .unwrap()
            .write_all(b"image 1 data")
            .unwrap();
        File::create(dir_path.join("photo.JPG"))
            .unwrap()
            .write_all(b"image 2 data")
            .unwrap();
        File::create(dir_path.join("doc.pdf"))
            .unwrap()
            .write_all(b"pdf data")
            .unwrap();
        File::create(dir_path.join("notes.txt"))
            .unwrap()
            .write_all(b"notes data")
            .unwrap();

        let scan_output = scan_directory(dir_path, true).expect("Scan failed");
        assert_eq!(scan_output.file_read_failures, 0);
        assert_eq!(scan_output.files.len(), 2);

        let names: Vec<String> = scan_output.files.into_iter().map(|d| d.filename).collect();
        assert!(names.contains(&"screenshot1.png".to_string()));
        assert!(names.contains(&"photo.JPG".to_string()));
    }

    #[test]
    fn test_duplicate_scan_idempotency() {
        let conn = setup_test_db();
        let dir = tempdir().expect("Failed to create tempdir");
        let dir_path = dir.path();
        let normalized = normalize_path(dir_path);

        let folder =
            db::folders::insert_folder(&conn, &normalized, true).expect("Failed to insert folder");

        File::create(dir_path.join("screen1.png"))
            .unwrap()
            .write_all(b"image content 1")
            .unwrap();
        File::create(dir_path.join("screen2.jpg"))
            .unwrap()
            .write_all(b"image content 2")
            .unwrap();

        // First scan
        let summary1 = execute_discovery_scan(&conn, &folder).expect("Scan 1 failed");
        assert_eq!(summary1.discovered, 2);
        assert_eq!(summary1.added, 2);
        assert_eq!(summary1.unchanged, 0);
        assert_eq!(summary1.updated, 0);
        assert_eq!(summary1.removed, 0);

        let total_count = db::screenshots::count_for_folder(&conn, folder.id).unwrap();
        assert_eq!(total_count, 2);

        // Second scan with no changes
        let summary2 = execute_discovery_scan(&conn, &folder).expect("Scan 2 failed");
        assert_eq!(summary2.discovered, 2);
        assert_eq!(summary2.added, 0);
        assert_eq!(summary2.unchanged, 2);
        assert_eq!(summary2.updated, 0);
        assert_eq!(summary2.removed, 0);

        let total_count_after = db::screenshots::count_for_folder(&conn, folder.id).unwrap();
        assert_eq!(total_count_after, 2);
    }

    #[test]
    fn test_changed_file_detection() {
        let conn = setup_test_db();
        let dir = tempdir().expect("Failed to create tempdir");
        let dir_path = dir.path();
        let normalized = normalize_path(dir_path);

        let folder =
            db::folders::insert_folder(&conn, &normalized, true).expect("Failed to insert folder");

        let file1_path = dir_path.join("screen1.png");
        let file2_path = dir_path.join("screen2.jpg");

        File::create(&file1_path)
            .unwrap()
            .write_all(b"image content 1")
            .unwrap();
        File::create(&file2_path)
            .unwrap()
            .write_all(b"image content 2")
            .unwrap();

        execute_discovery_scan(&conn, &folder).expect("Initial scan failed");

        // Modify file1 content and size
        fs::write(&file1_path, b"updated image content with different size!").unwrap();

        let summary = execute_discovery_scan(&conn, &folder).expect("Rescan failed");
        assert_eq!(summary.discovered, 2);
        assert_eq!(summary.added, 0);
        assert_eq!(summary.updated, 1);
        assert_eq!(summary.unchanged, 1);
        assert_eq!(summary.removed, 0);
    }

    #[test]
    fn test_reconciliation_only_deletes_when_genuinely_not_found() {
        let conn = setup_test_db();
        let dir = tempdir().expect("Failed to create tempdir");
        let dir_path = dir.path();
        let normalized = normalize_path(dir_path);

        let folder =
            db::folders::insert_folder(&conn, &normalized, true).expect("Failed to insert folder");

        let file1_path = dir_path.join("screen1.png");
        let file2_path = dir_path.join("screen2.jpg");

        File::create(&file1_path)
            .unwrap()
            .write_all(b"content 1")
            .unwrap();
        File::create(&file2_path)
            .unwrap()
            .write_all(b"content 2")
            .unwrap();

        execute_discovery_scan(&conn, &folder).expect("Initial scan failed");
        assert_eq!(
            db::screenshots::count_for_folder(&conn, folder.id).unwrap(),
            2
        );

        // Delete file2 genuinely from disk
        fs::remove_file(&file2_path).expect("Failed to delete file2");

        let summary = execute_discovery_scan(&conn, &folder).expect("Rescan failed");
        assert_eq!(summary.discovered, 1);
        assert_eq!(summary.removed, 1);
        assert_eq!(summary.unchanged, 1);

        let count_after = db::screenshots::count_for_folder(&conn, folder.id).unwrap();
        assert_eq!(count_after, 1);
    }

    #[test]
    fn test_reconciliation_safeguard_on_inaccessible_subtree() {
        let conn = setup_test_db();
        let dir = tempdir().expect("Failed to create tempdir");
        let dir_path = dir.path();
        let normalized = normalize_path(dir_path);

        let folder =
            db::folders::insert_folder(&conn, &normalized, true).expect("Failed to insert folder");

        // Subdirectory representing an inaccessible folder branch
        let sub_dir = dir_path.join("restricted_sub");
        fs::create_dir(&sub_dir).unwrap();
        let sub_file = sub_dir.join("secret.png");
        File::create(&sub_file)
            .unwrap()
            .write_all(b"secret image")
            .unwrap();

        execute_discovery_scan(&conn, &folder).expect("Initial scan failed");
        assert_eq!(
            db::screenshots::count_for_folder(&conn, folder.id).unwrap(),
            1
        );

        // Now simulate that the file still exists on disk, but was not returned in discovered_paths
        // (for example, simulated by running a non-recursive scan where sub_dir was not traversed)
        let non_rec_folder = db::folders::FolderRecord {
            id: folder.id,
            path: folder.path.clone(),
            enabled: true,
            recursive: false, // does not traverse restricted_sub
            created_at: folder.created_at.clone(),
            updated_at: folder.updated_at.clone(),
            last_scanned_at: folder.last_scanned_at.clone(),
            screenshot_count: 1,
        };

        let summary =
            execute_discovery_scan(&conn, &non_rec_folder).expect("Non-recursive scan failed");
        // Because secret.png still exists on disk (fs::metadata returns Ok),
        // deletion reconciliation MUST NOT delete the record!
        assert_eq!(summary.removed, 0);
        assert_eq!(
            db::screenshots::count_for_folder(&conn, folder.id).unwrap(),
            1
        );
    }

    #[test]
    fn test_folder_duplicate_rejection_with_variants() {
        let conn = setup_test_db();
        let dir = tempdir().expect("Failed to create tempdir");
        let path_str = dir.path().to_str().unwrap();

        // 1. Initial insert with standard canonical path
        let canonical = canonicalize_and_normalize(Path::new(path_str));
        let res1 = db::folders::insert_folder(&conn, &canonical, true);
        assert!(res1.is_ok());

        // 2. Duplicate check with trailing slash
        let with_trailing = format!("{canonical}\\");
        let canonical2 = canonicalize_and_normalize(Path::new(&with_trailing));
        let res2 = db::folders::insert_folder(&conn, &canonical2, true);
        assert!(res2.is_err());
        assert_eq!(res2.unwrap_err().code, ErrorCode::FolderAlreadyExists);

        // 3. Duplicate check with lowercase casing
        let lower = canonical.to_lowercase();
        let canonical3 = canonicalize_and_normalize(Path::new(&lower));
        let res3 = db::folders::insert_folder(&conn, &canonical3, true);
        assert!(res3.is_err());
        assert_eq!(res3.unwrap_err().code, ErrorCode::FolderAlreadyExists);
    }

    #[test]
    fn test_invalid_path_no_panic() {
        let non_existent = Path::new("Z:\\Definitely\\Does\\Not\\Exist_12345");
        let result = scan_directory(non_existent, true);
        assert!(result.is_err());
    }
}
