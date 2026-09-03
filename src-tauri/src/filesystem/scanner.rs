use std::path::Path;
use walkdir::WalkDir;

use super::metadata::DiscoveredFileMetadata;

/// Allowed image extensions (case-insensitive).
pub const SUPPORTED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Checks if an extension (already lowercased) is a supported image format.
pub fn is_supported_extension(ext: &str) -> bool {
    let lower = ext.to_ascii_lowercase();
    SUPPORTED_EXTENSIONS.iter().any(|&supported| supported == lower)
}

/// Recursively scans a root directory for supported image files.
/// 
/// Returns:
/// - `Vec<DiscoveredFileMetadata>`: successfully read image metadata
/// - `usize`: count of files/directories that encountered read/permission errors
pub fn scan_directory(
    root: &Path,
    recursive: bool,
) -> Result<(Vec<DiscoveredFileMetadata>, usize), std::io::Error> {
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

    let mut discovered = Vec::new();
    let mut failed_count = 0;

    let mut walker = WalkDir::new(root).follow_links(false);
    if !recursive {
        walker = walker.max_depth(1);
    }

    for entry_result in walker {
        let entry = match entry_result {
            Ok(e) => e,
            Err(err) => {
                log::warn!("Scanner encountered inaccessible entry: {err}");
                failed_count += 1;
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
                discovered.push(meta);
            }
            Err(err) => {
                log::warn!(
                    "Failed to read metadata for file {}: {err}",
                    path.display()
                );
                failed_count += 1;
            }
        }
    }

    Ok((discovered, failed_count))
}
