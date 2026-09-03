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

/// Normalizes a path string for consistent storage, comparison, and cross-platform safety.
///
/// Rules:
/// 1. Converts forward slashes to standard Windows backslashes.
/// 2. Converts Windows UNC extended paths (`\\?\UNC\server\share` -> `\\server\share`).
/// 3. Strips standard Windows extended verbatim prefix (`\\?\C:\...` -> `C:\...`).
/// 4. Ensures Windows drive letter is capitalized (`c:\...` -> `C:\...`).
/// 5. Strips trailing path separators unless the path represents a drive root (e.g. `C:\`).
pub fn normalize_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let clean = raw.replace('/', "\\");

    // Handle UNC prefix \\?\UNC\
    let without_verbatim = if let Some(unc) = clean.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", unc)
    } else if let Some(s) = clean.strip_prefix(r"\\?\") {
        s.to_string()
    } else {
        clean
    };

    // Capitalize drive letter on Windows (e.g., c:\ -> C:\)
    let capitalized = if without_verbatim.len() >= 2 && without_verbatim.as_bytes()[1] == b':' {
        let first = without_verbatim
            .chars()
            .next()
            .unwrap()
            .to_ascii_uppercase();
        format!("{}{}", first, &without_verbatim[1..])
    } else {
        without_verbatim
    };

    // Strip trailing backslashes, but preserve drive root (e.g. "C:\" -> 3 chars)
    if capitalized.len() > 3 && capitalized.ends_with('\\') {
        capitalized.trim_end_matches('\\').to_string()
    } else {
        capitalized
    }
}

/// Resolves a path on disk to its canonical representation (resolving symlinks,
/// relative segments `..` and `.`, and filesystem casing), then normalizes it.
/// If canonicalize fails (e.g., path does not exist yet), falls back to standard normalization.
pub fn canonicalize_and_normalize(path: &Path) -> String {
    match fs::canonicalize(path) {
        Ok(canonical) => normalize_path(&canonical),
        Err(_) => normalize_path(path),
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
