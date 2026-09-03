use chrono::{DateTime, Utc};
use std::fs;
use std::path::Path;

/// Normalized metadata for an image file discovered on the filesystem.
#[derive(Debug, Clone)]
pub struct DiscoveredFileMetadata {
    pub path: String,
    pub filename: String,
    pub extension: String,
    pub file_size: u64,
    pub modified_at_fs: String,
}

/// Normalizes a path string for consistent storage and comparison.
/// Replaces forward slashes with platform standard, strips Windows \\?\ verbatim prefix,
/// and capitalizes Windows drive letters.
pub fn normalize_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let clean = raw.replace('/', "\\");

    let stripped = if let Some(s) = clean.strip_prefix(r"\\?\") {
        s.to_string()
    } else {
        clean
    };

    // Capitalize drive letter on Windows (e.g., c:\ -> C:\)
    if stripped.len() >= 2 && stripped.as_bytes()[1] == b':' {
        let first = stripped.chars().next().unwrap().to_ascii_uppercase();
        format!("{}{}", first, &stripped[1..])
    } else {
        stripped
    }
}

impl DiscoveredFileMetadata {
    /// Reads file metadata from the filesystem.
    pub fn from_path(path: &Path) -> Result<Self, std::io::Error> {
        let meta = fs::metadata(path)?;
        let file_size = meta.len();

        let modified_at_fs = meta
            .modified()
            .map(|systime| {
                let dt: DateTime<Utc> = systime.into();
                dt.to_rfc3339()
            })
            .unwrap_or_else(|_| Utc::now().to_rfc3339());

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let normalized_path = normalize_path(path);

        Ok(Self {
            path: normalized_path,
            filename,
            extension,
            file_size,
            modified_at_fs,
        })
    }
}
