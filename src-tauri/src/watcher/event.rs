use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind};
use std::path::Path;

use crate::filesystem::metadata::normalize_path;
use crate::filesystem::scanner::is_supported_extension;

/// Normalized filesystem event representing a high-level logical change to a screenshot file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedFsEvent {
    Upsert {
        folder_id: i64,
        path: String,
    },
    Remove {
        folder_id: i64,
        path: String,
    },
    Rename {
        folder_id: i64,
        from_path: String,
        to_path: String,
    },
}

/// Checks if a file path points to a temporary, hidden, or partial download file.
pub fn is_temporary_file(path: &Path) -> bool {
    let filename = match path.file_name().and_then(|f| f.to_str()) {
        Some(name) => name,
        None => return true,
    };

    // Hidden or system files
    if filename.starts_with('.') || filename.starts_with("~$") {
        return true;
    }

    // Common temporary, lock, or partial extensions
    let lower = filename.to_lowercase();
    if lower.ends_with(".tmp")
        || lower.ends_with(".crdownload")
        || lower.ends_with(".partial")
        || lower.ends_with(".part")
        || lower.ends_with(".swp")
        || lower.ends_with(".lock")
    {
        return true;
    }

    false
}

/// Checks if a path represents a supported screenshot image file and is not temporary.
pub fn is_supported_screenshot_path(path: &Path) -> bool {
    if is_temporary_file(path) {
        return false;
    }

    let ext = match path.extension().and_then(|e| e.to_str()) {
        Some(e) => e,
        None => return false,
    };

    is_supported_extension(ext)
}

/// Normalizes a raw `notify::Event` emitted by the OS filesystem watcher for a given watched folder.
pub fn normalize_notify_event(folder_id: i64, event: &Event) -> Vec<NormalizedFsEvent> {
    let mut normalized = Vec::new();

    match event.kind {
        EventKind::Create(_) => {
            for path in &event.paths {
                if is_supported_screenshot_path(path) {
                    normalized.push(NormalizedFsEvent::Upsert {
                        folder_id,
                        path: normalize_path(path),
                    });
                }
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
            if event.paths.len() >= 2 {
                let from = &event.paths[0];
                let to = &event.paths[1];

                let from_valid = is_supported_screenshot_path(from);
                let to_valid = is_supported_screenshot_path(to);

                if from_valid && to_valid {
                    normalized.push(NormalizedFsEvent::Rename {
                        folder_id,
                        from_path: normalize_path(from),
                        to_path: normalize_path(to),
                    });
                } else if from_valid && !to_valid {
                    // Renamed to an unsupported or temporary name -> treat as remove
                    normalized.push(NormalizedFsEvent::Remove {
                        folder_id,
                        path: normalize_path(from),
                    });
                } else if !from_valid && to_valid {
                    // Renamed from a temp name to a valid image name -> treat as new file upsert
                    normalized.push(NormalizedFsEvent::Upsert {
                        folder_id,
                        path: normalize_path(to),
                    });
                }
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) => {
            for path in &event.paths {
                if is_supported_screenshot_path(path) {
                    normalized.push(NormalizedFsEvent::Remove {
                        folder_id,
                        path: normalize_path(path),
                    });
                }
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::To)) => {
            for path in &event.paths {
                if is_supported_screenshot_path(path) {
                    normalized.push(NormalizedFsEvent::Upsert {
                        folder_id,
                        path: normalize_path(path),
                    });
                }
            }
        }
        EventKind::Modify(_) => {
            for path in &event.paths {
                if is_supported_screenshot_path(path) {
                    normalized.push(NormalizedFsEvent::Upsert {
                        folder_id,
                        path: normalize_path(path),
                    });
                }
            }
        }
        EventKind::Remove(_) => {
            for path in &event.paths {
                if is_supported_screenshot_path(path) {
                    normalized.push(NormalizedFsEvent::Remove {
                        folder_id,
                        path: normalize_path(path),
                    });
                }
            }
        }
        _ => {}
    }

    normalized
}
