use std::path::Path;
use walkdir::WalkDir;

use super::metadata::{normalize_path, DiscoveredFileMetadata};

/// Allowed image extensions (case-insensitive).
pub const SUPPORTED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Checks if an extension (already lowercased) is a supported image format.
pub fn is_supported_extension(ext: &str) -> bool {
    let lower = ext.to_ascii_lowercase();
    SUPPORTED_EXTENSIONS
        .iter()
        .any(|&supported| supported == lower)
}

/// Output of a directory scan.
#[derive(Debug, Clone)]
pub struct ScanOutput {
    /// Successfully read image metadata.
    pub files: Vec<DiscoveredFileMetadata>,
    /// Subdirectories or entries that could not be accessed (e.g. PermissionDenied).
    /// Used by discovery reconciliation to protect subtrees from accidental deletion.
    pub inaccessible_paths: Vec<String>,
    /// Number of entries that failed during traversal or metadata reading.
    pub file_read_failures: usize,
}

/// Recursively scans a root directory for supported image files.
///
/// Safe against loops and recursion cycles (symlinks not followed).
/// Inaccessible entries/subtrees are recorded in `inaccessible_paths` rather than failing the scan.
pub fn scan_directory(root: &Path, recursive: bool) -> Result<ScanOutput, std::io::Error> {
    if !root.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("Directory does not exist: {}", root.display()),
        ));
    }

    if !root.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("Path is not a directory: {}", root.display()),
        ));
    }

    let mut files = Vec::new();
    let mut inaccessible_paths = Vec::new();
    let mut file_read_failures = 0;

    let mut walker = WalkDir::new(root).follow_links(false);
    if !recursive {
        walker = walker.max_depth(1);
    }

    for entry_result in walker {
        let entry = match entry_result {
            Ok(e) => e,
            Err(err) => {
                log::warn!("Scanner encountered inaccessible entry: {err}");
                if let Some(path) = err.path() {
                    inaccessible_paths.push(normalize_path(path));
                }
                file_read_failures += 1;
                continue;
            }
        };

        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Check extension
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();

        if !is_supported_extension(ext) {
            continue;
        }

        match DiscoveredFileMetadata::from_path(path) {
            Ok(meta) => {
                files.push(meta);
            }
            Err(err) => {
                log::warn!("Failed to read metadata for file {}: {err}", path.display());
                file_read_failures += 1;
            }
        }
    }

    Ok(ScanOutput {
        files,
        inaccessible_paths,
        file_read_failures,
    })
}
