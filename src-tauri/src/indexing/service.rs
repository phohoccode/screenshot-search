use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use crate::db::connection::Database;
use crate::db::folders;
use crate::db::jobs::{self, IndexJobStats, JOB_TYPE_UPSERT};
use crate::errors::AppError;
use crate::indexing::discovery::execute_discovery_scan;
use crate::indexing::worker::run_indexing_worker_loop;
use crate::ocr::engine::OcrEngine;
use crate::watcher::WatcherManager;

/// High-level status of the automatic background indexing service.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexingServiceStatus {
    pub is_running: bool,
    pub is_paused: bool,
    pub active_watchers_count: usize,
    pub stats: IndexJobStats,
}

/// Central service coordinating filesystem watching, startup reconciliation, and durable background indexing.
pub struct IndexingService {
    db: Database,
    engine: Arc<dyn OcrEngine>,
    watcher: Arc<WatcherManager>,
    is_paused: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    worker_thread: Mutex<Option<JoinHandle<()>>>,
}

impl IndexingService {
    /// Initializes the IndexingService and its components.
    pub fn new(
        db: Database,
        engine: Arc<dyn OcrEngine>,
        watcher: Arc<WatcherManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            engine,
            watcher,
            is_paused: Arc::new(AtomicBool::new(false)),
            stop_flag: Arc::new(AtomicBool::new(false)),
            worker_thread: Mutex::new(None),
        })
    }

    /// Starts the background worker, registers watchers for all enabled folders,
    /// and performs initial startup reconciliation for any changes that occurred while offline.
    pub fn start(self: &Arc<Self>, on_job_completed: Option<Arc<dyn Fn(i64, &str) + Send + Sync>>) {
        let db_clone = self.db.clone();
        let engine_clone = self.engine.clone();
        let is_paused_clone = self.is_paused.clone();
        let stop_flag_clone = self.stop_flag.clone();

        // 1. Spawn background worker thread
        let handle = thread::Builder::new()
            .name("indexing-worker".into())
            .spawn(move || {
                run_indexing_worker_loop(
                    db_clone,
                    engine_clone,
                    is_paused_clone,
                    stop_flag_clone,
                    on_job_completed,
                );
            })
            .expect("Failed to spawn indexing-worker thread");

        if let Ok(mut guard) = self.worker_thread.lock() {
            *guard = Some(handle);
        }

        // 2. Register watchers for all enabled folders from SQLite
        self.register_all_enabled_watchers();

        // 3. Run background startup reconciliation (asynchronous to not block UI startup)
        let self_clone = self.clone();
        thread::Builder::new()
            .name("startup-reconciliation".into())
            .spawn(move || {
                self_clone.run_startup_reconciliation();
            })
            .expect("Failed to spawn startup-reconciliation thread");

        log::info!("IndexingService started successfully");
    }

    /// Registers filesystem watchers for all enabled folders found in the database.
    pub fn register_all_enabled_watchers(&self) {
        let enabled_folders = {
            match self.db.conn.lock() {
                Ok(conn) => folders::list_folders(&conn).unwrap_or_default(),
                Err(e) => {
                    log::warn!("Failed to lock DB while registering watchers: {e}");
                    return;
                }
            }
        };

        for folder in enabled_folders {
            if folder.enabled {
                if let Err(e) = self
                    .watcher
                    .watch_folder(folder.id, &folder.path, folder.recursive)
                {
                    log::warn!(
                        "Failed to watch folder {} ({}): {e}",
                        folder.id,
                        folder.path
                    );
                }
            }
        }
    }

    /// Reconciles filesystem state with the database on startup:
    /// Discovers any screenshots that were added, modified, or deleted while the application was closed.
    pub fn run_startup_reconciliation(&self) {
        log::info!("Starting background startup reconciliation across registered folders");

        let folders_list = match self.db.conn.lock() {
            Ok(conn) => folders::list_folders(&conn).unwrap_or_default(),
            Err(e) => {
                log::warn!("Startup reconciliation aborted, DB lock error: {e}");
                return;
            }
        };

        for folder in folders_list {
            if self.stop_flag.load(Ordering::SeqCst) {
                break;
            }

            if !folder.enabled {
                continue;
            }

            // Execute discovery scan (which reconciles new, updated, and deleted screenshots)
            let scan_result = {
                match self.db.conn.lock() {
                    Ok(conn) => execute_discovery_scan(&conn, &folder),
                    Err(e) => {
                        log::warn!("Startup scan failed for folder {}: {e}", folder.id);
                        continue;
                    }
                }
            };

            match scan_result {
                Ok(summary) => {
                    log::info!(
                        "Startup scan folder {} ({}): added={}, updated={}, unchanged={}, removed={}",
                        folder.id,
                        folder.path,
                        summary.added,
                        summary.updated,
                        summary.unchanged,
                        summary.removed
                    );

                    // For any screenshots in this folder that are PENDING OCR, enqueue them into index_jobs
                    if let Ok(conn) = self.db.conn.lock() {
                        let stmt = conn.prepare(
                            "SELECT id, path, content_hash FROM screenshots 
                             WHERE folder_id = ?1 AND ocr_status = 'PENDING'",
                        );
                        if let Ok(mut prepared) = stmt {
                            let rows = prepared.query_map([folder.id], |row| {
                                Ok((
                                    row.get::<_, i64>(0)?,
                                    row.get::<_, String>(1)?,
                                    row.get::<_, Option<String>>(2)?,
                                ))
                            });
                            if let Ok(mapped) = rows {
                                for item in mapped.flatten() {
                                    let (_, path, content_hash) = item;
                                    let dedupe_key = format!(
                                        "UPSERT:{}:{}:{}",
                                        folder.id,
                                        path,
                                        content_hash.unwrap_or_default()
                                    );
                                    let _ = jobs::enqueue_job(
                                        &conn,
                                        folder.id,
                                        &path,
                                        JOB_TYPE_UPSERT,
                                        &dedupe_key,
                                    );
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Reconciliation error on folder {}: {e}", folder.id);
                }
            }
        }

        log::info!("Startup reconciliation completed");
    }

    /// Pauses background indexing. The watcher continues capturing events into the durable queue,
    /// but the worker will not claim new jobs until resumed.
    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::SeqCst);
        log::info!("Background indexing paused by user");
    }

    /// Resumes background indexing.
    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::SeqCst);
        log::info!("Background indexing resumed by user");
    }

    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }

    /// Resets all FAILED jobs back to PENDING for re-indexing.
    pub fn retry_failed(&self) -> Result<usize, AppError> {
        let conn = self
            .db
            .conn
            .lock()
            .map_err(|e| AppError::database(format!("Failed to lock DB: {e}")))?;
        jobs::retry_all_failed_jobs(&conn)
    }

    /// Retrieves current diagnostics and status for the UI.
    pub fn get_status(&self) -> Result<IndexingServiceStatus, AppError> {
        let stats = {
            let conn = self
                .db
                .conn
                .lock()
                .map_err(|e| AppError::database(format!("Failed to lock DB: {e}")))?;
            jobs::get_job_stats(&conn)?
        };

        let active_watchers_count = {
            let conn = self
                .db
                .conn
                .lock()
                .map_err(|e| AppError::database(format!("Failed to lock DB: {e}")))?;
            let folders_list = folders::list_folders(&conn).unwrap_or_default();
            folders_list
                .iter()
                .filter(|f| self.watcher.is_watching(f.id))
                .count()
        };

        Ok(IndexingServiceStatus {
            is_running: !self.stop_flag.load(Ordering::SeqCst),
            is_paused: self.is_paused.load(Ordering::SeqCst),
            active_watchers_count,
            stats,
        })
    }

    /// Provides direct access to the WatcherManager for folder lifecycle registration.
    pub fn watcher(&self) -> &Arc<WatcherManager> {
        &self.watcher
    }

    /// Gracefully shuts down the IndexingService and its threads.
    pub fn shutdown(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.watcher.shutdown();

        if let Ok(mut guard) = self.worker_thread.lock() {
            if let Some(handle) = guard.take() {
                let _ = handle.join();
            }
        }

        log::info!("IndexingService shutdown cleanly");
    }
}

impl Drop for IndexingService {
    fn drop(&mut self) {
        self.shutdown();
    }
}
