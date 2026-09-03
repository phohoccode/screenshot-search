pub mod debouncer;
pub mod event;

#[cfg(test)]
pub mod watcher_tests;

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use self::debouncer::start_debouncer_thread;
use self::event::{normalize_notify_event, NormalizedFsEvent};
use crate::db::connection::Database;
use crate::errors::AppError;

/// Entry representing an active watcher for a specific registered folder.
struct ActiveFolderWatcher {
    #[allow(dead_code)]
    folder_id: i64,
    path: String,
    watcher: RecommendedWatcher,
}

/// Central manager orchestrating filesystem watchers across all enabled folders.
pub struct WatcherManager {
    watchers: Mutex<HashMap<i64, ActiveFolderWatcher>>,
    event_tx: Sender<NormalizedFsEvent>,
    stop_flag: Arc<AtomicBool>,
    debouncer_thread: Mutex<Option<JoinHandle<()>>>,
}

impl WatcherManager {
    /// Creates a new WatcherManager and spawns the background debouncing thread.
    pub fn new(db: Database) -> Arc<Self> {
        let (event_tx, event_rx) = channel::<NormalizedFsEvent>();
        let stop_flag = Arc::new(AtomicBool::new(false));

        let debouncer_thread = start_debouncer_thread(event_rx, db, stop_flag.clone());

        Arc::new(Self {
            watchers: Mutex::new(HashMap::new()),
            event_tx,
            stop_flag,
            debouncer_thread: Mutex::new(Some(debouncer_thread)),
        })
    }

    /// Registers a folder with the OS filesystem watcher.
    /// Uses `RecursiveMode::Recursive` if recursive is true.
    /// Any raw events are normalized and sent to the debouncer channel.
    pub fn watch_folder(
        &self,
        folder_id: i64,
        path_str: &str,
        recursive: bool,
    ) -> Result<(), AppError> {
        let path = Path::new(path_str);
        if !path.exists() {
            log::warn!("Cannot watch non-existent folder: {path_str}");
            return Err(AppError::folder_not_found(format!(
                "Folder does not exist on disk: {path_str}"
            )));
        }

        let mut watchers_guard = self
            .watchers
            .lock()
            .map_err(|e| AppError::unknown(format!("Failed to acquire watcher lock: {e}")))?;

        // If already watching this folder, unwatch first to refresh configuration
        if let Some(mut existing) = watchers_guard.remove(&folder_id) {
            let _ = existing.watcher.unwatch(Path::new(&existing.path));
        }

        let event_tx_clone = self.event_tx.clone();
        let target_folder_id = folder_id;

        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| match res {
                Ok(event) => {
                    let normalized_events = normalize_notify_event(target_folder_id, &event);
                    for ne in normalized_events {
                        let _ = event_tx_clone.send(ne);
                    }
                }
                Err(err) => {
                    log::warn!("Filesystem watcher error on folder {target_folder_id}: {err}");
                }
            },
            Config::default(),
        )
        .map_err(|e| AppError::unknown(format!("Failed to initialize OS watcher: {e}")))?;

        let mode = if recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        watcher
            .watch(path, mode)
            .map_err(|e| AppError::unknown(format!("Failed to watch folder {path_str}: {e}")))?;

        watchers_guard.insert(
            folder_id,
            ActiveFolderWatcher {
                folder_id,
                path: path_str.to_string(),
                watcher,
            },
        );

        log::info!("Registered filesystem watcher for folder {folder_id}: {path_str} (recursive={recursive})");
        Ok(())
    }

    /// Unregisters a folder from active filesystem watching.
    pub fn unwatch_folder(&self, folder_id: i64) -> Result<(), AppError> {
        let mut watchers_guard = self
            .watchers
            .lock()
            .map_err(|e| AppError::unknown(format!("Failed to acquire watcher lock: {e}")))?;

        if let Some(mut active) = watchers_guard.remove(&folder_id) {
            let _ = active.watcher.unwatch(Path::new(&active.path));
            log::info!(
                "Unregistered filesystem watcher for folder {folder_id}: {}",
                active.path
            );
        }

        Ok(())
    }

    /// Checks if a folder currently has an active watcher registered.
    pub fn is_watching(&self, folder_id: i64) -> bool {
        if let Ok(guard) = self.watchers.lock() {
            guard.contains_key(&folder_id)
        } else {
            false
        }
    }

    /// Gracefully stops all active watchers and terminates the debouncer thread.
    pub fn shutdown(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);

        if let Ok(mut guard) = self.watchers.lock() {
            for (_, mut active) in guard.drain() {
                let _ = active.watcher.unwatch(Path::new(&active.path));
            }
        }

        if let Ok(mut debouncer_guard) = self.debouncer_thread.lock() {
            if let Some(handle) = debouncer_guard.take() {
                let _ = handle.join();
            }
        }

        log::info!("Filesystem WatcherManager shutdown completed cleanly");
    }
}

impl Drop for WatcherManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
