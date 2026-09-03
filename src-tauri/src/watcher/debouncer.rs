use std::collections::HashMap;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use super::event::NormalizedFsEvent;
use crate::db::connection::Database;
use crate::db::jobs::{self, JOB_TYPE_DELETE, JOB_TYPE_UPSERT};
use crate::errors::AppError;
use crate::filesystem::fingerprint::compute_sha256;

/// Interval used to debounce incoming rapid events per path.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(500);

/// Stability polling interval when verifying if a file is still being written to.
const STABILITY_POLL_INTERVAL: Duration = Duration::from_millis(150);

/// Max attempts for stability check before deferring with retry.
const MAX_STABILITY_ATTEMPTS: usize = 5;

/// In-memory coalescing state for a single file path.
#[derive(Debug, Clone)]
struct CoalescedEvent {
    folder_id: i64,
    path: String,
    is_remove: bool,
    deadline: Instant,
}

/// Verifies whether an image file has finished writing and is safe to read.
/// Checks that file size and modified timestamp remain constant over a short interval,
/// and that the file can be successfully opened for shared reading without sharing violations.
pub fn check_file_stability(path: &Path) -> Result<bool, AppError> {
    for attempt in 0..MAX_STABILITY_ATTEMPTS {
        let meta1 = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => {
                log::debug!(
                    "Stability check meta1 attempt {attempt} failed for {}: {e}",
                    path.display()
                );
                thread::sleep(STABILITY_POLL_INTERVAL);
                continue;
            }
        };

        let size1 = meta1.len();
        let mtime1 = meta1.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        thread::sleep(STABILITY_POLL_INTERVAL);

        let meta2 = match fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(false),
            Err(e) => {
                log::debug!(
                    "Stability check meta2 attempt {attempt} failed for {}: {e}",
                    path.display()
                );
                continue;
            }
        };

        let size2 = meta2.len();
        let mtime2 = meta2.modified().unwrap_or(SystemTime::UNIX_EPOCH);

        // If size or modification time is still fluctuating, file is actively being written
        if size1 != size2 || mtime1 != mtime2 {
            log::debug!(
                "File {} is still growing ({size1} -> {size2} bytes); waiting",
                path.display()
            );
            continue;
        }

        // Try opening the file to ensure no exclusive locks are held
        match File::open(path) {
            Ok(_) => return Ok(true),
            Err(e) => {
                log::debug!(
                    "File {} locked or busy: {e}; attempt {attempt}",
                    path.display()
                );
                thread::sleep(STABILITY_POLL_INTERVAL);
            }
        }
    }

    // After bounded attempts, still could not verify stable reading
    Ok(false)
}

/// Spawns the debouncing and stability thread which receives normalized events from the watcher,
/// coalesces rapid events for the same file path, checks file stability, and enqueues durable
/// jobs into SQLite `index_jobs`.
pub fn start_debouncer_thread(
    event_rx: Receiver<NormalizedFsEvent>,
    db: Database,
    stop_flag: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("fs-debouncer".into())
        .spawn(move || {
            let mut pending_map: HashMap<String, CoalescedEvent> = HashMap::new();

            while !stop_flag.load(Ordering::SeqCst) {
                // 1. Drain incoming events from channel (non-blocking)
                while let Ok(event) = event_rx.try_recv() {
                    match event {
                        NormalizedFsEvent::Upsert { folder_id, path } => {
                            pending_map.insert(
                                path.clone(),
                                CoalescedEvent {
                                    folder_id,
                                    path,
                                    is_remove: false,
                                    deadline: Instant::now() + DEBOUNCE_WINDOW,
                                },
                            );
                        }
                        NormalizedFsEvent::Remove { folder_id, path } => {
                            pending_map.insert(
                                path.clone(),
                                CoalescedEvent {
                                    folder_id,
                                    path,
                                    is_remove: true,
                                    deadline: Instant::now() + DEBOUNCE_WINDOW,
                                },
                            );
                        }
                        NormalizedFsEvent::Rename {
                            folder_id,
                            from_path,
                            to_path,
                        } => {
                            // Split rename into Remove(from) and Upsert(to)
                            pending_map.insert(
                                from_path.clone(),
                                CoalescedEvent {
                                    folder_id,
                                    path: from_path,
                                    is_remove: true,
                                    deadline: Instant::now() + DEBOUNCE_WINDOW,
                                },
                            );
                            pending_map.insert(
                                to_path.clone(),
                                CoalescedEvent {
                                    folder_id,
                                    path: to_path,
                                    is_remove: false,
                                    deadline: Instant::now() + DEBOUNCE_WINDOW,
                                },
                            );
                        }
                    }
                }

                // 2. Check for events whose debounce deadline has expired
                let now = Instant::now();
                let ready_keys: Vec<String> = pending_map
                    .iter()
                    .filter(|(_, ev)| ev.deadline <= now)
                    .map(|(k, _)| k.clone())
                    .collect();

                for key in ready_keys {
                    if let Some(event) = pending_map.remove(&key) {
                        let path_obj = Path::new(&event.path);

                        if event.is_remove {
                            // File was removed: enqueue DELETE job
                            let dedupe_key = format!("DELETE:{}:{}", event.folder_id, event.path);
                            if let Ok(conn) = db.conn.lock() {
                                if let Err(e) = jobs::enqueue_job(
                                    &conn,
                                    event.folder_id,
                                    &event.path,
                                    JOB_TYPE_DELETE,
                                    &dedupe_key,
                                ) {
                                    log::warn!(
                                        "Failed to enqueue DELETE job for {}: {e}",
                                        event.path
                                    );
                                }
                            }
                        } else {
                            // File was created/modified: perform stability check
                            match check_file_stability(path_obj) {
                                Ok(true) => {
                                    // File is stable. Compute quick fingerprint/hash for dedupe key.
                                    let content_hash = match compute_sha256(path_obj) {
                                        Ok(h) => h,
                                        Err(e) => {
                                            log::warn!(
                                                "Failed to hash stable file {}: {e}",
                                                event.path
                                            );
                                            // Re-schedule with small backoff
                                            pending_map.insert(
                                                event.path.clone(),
                                                CoalescedEvent {
                                                    folder_id: event.folder_id,
                                                    path: event.path,
                                                    is_remove: false,
                                                    deadline: Instant::now()
                                                        + Duration::from_millis(500),
                                                },
                                            );
                                            continue;
                                        }
                                    };

                                    let dedupe_key = format!(
                                        "UPSERT:{}:{}:{}",
                                        event.folder_id, event.path, content_hash
                                    );

                                    if let Ok(conn) = db.conn.lock() {
                                        if let Err(e) = jobs::enqueue_job(
                                            &conn,
                                            event.folder_id,
                                            &event.path,
                                            JOB_TYPE_UPSERT,
                                            &dedupe_key,
                                        ) {
                                            log::warn!(
                                                "Failed to enqueue UPSERT job for {}: {e}",
                                                event.path
                                            );
                                        }
                                    }
                                }
                                Ok(false) => {
                                    // File is still being written or was transiently deleted
                                    if path_obj.exists() {
                                        // Still exists but not stable yet: re-schedule
                                        pending_map.insert(
                                            event.path.clone(),
                                            CoalescedEvent {
                                                folder_id: event.folder_id,
                                                path: event.path,
                                                is_remove: false,
                                                deadline: Instant::now()
                                                    + Duration::from_millis(500),
                                            },
                                        );
                                    }
                                }
                                Err(e) => {
                                    log::warn!("Stability check error on {}: {e}", event.path);
                                }
                            }
                        }
                    }
                }

                // Sleep briefly to avoid busy-waiting
                thread::sleep(Duration::from_millis(50));
            }
        })
        .expect("Failed to spawn fs-debouncer thread")
}
