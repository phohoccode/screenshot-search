# Screenshot Search — Database Context

## 1. Database Choice

Use SQLite because the application is local-first and single-device in the initial architecture.

SQLite is the main persistence layer.

SQLite FTS5 is used for Phase 1 full-text search.

Do not introduce PostgreSQL, MySQL, Redis, or an external vector database without a concrete architectural need.

---

## 2. Data Model Goals

The database must support:

- watched folders
- screenshot metadata
- OCR results
- indexing state
- FTS search
- failure/retry state
- future embeddings
- rebuildable derived data

---

## 3. Suggested Tables

The exact schema may evolve.

### folders

```text
id
path
enabled
recursive
created_at
updated_at
last_scanned_at
```

Constraints:

- normalized path should be unique
- disabled folders should not receive new watcher jobs
- disabling a folder must not automatically delete original files

### screenshots

```text
id
folder_id
path
filename
extension
file_size
modified_at_fs
content_hash
width
height
ocr_text
ocr_status
ocr_engine
ocr_engine_version (Migration v5)
ocr_language (Migration v5)
ocr_pipeline_version (Migration v5, default 'v1')
indexed_at
created_at
updated_at
```

Possible OCR statuses:

```text
PENDING
PROCESSING
SUCCEEDED
FAILED
SKIPPED
```

Constraints / invariants:

- a current canonical path should map to at most one active screenshot record
- successful OCR state must reflect a completed OCR operation
- changing content must invalidate derived search/embedding data
- deleting the original file must remove or mark unavailable the corresponding searchable record

### index_jobs (Implemented in Phase 2 — Migration v3)

Durable, crash-resilient local job queue for filesystem changes and OCR indexing:

```sql
CREATE TABLE IF NOT EXISTS index_jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    folder_id INTEGER REFERENCES folders(id) ON DELETE CASCADE,
    screenshot_id INTEGER REFERENCES screenshots(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    job_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    dedupe_key TEXT NOT NULL UNIQUE,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    available_at TEXT NOT NULL DEFAULT (datetime('now')),
    lease_until TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_index_jobs_status_available 
    ON index_jobs(status, available_at);
CREATE INDEX IF NOT EXISTS idx_index_jobs_folder_id 
    ON index_jobs(folder_id);
CREATE INDEX IF NOT EXISTS idx_index_jobs_screenshot_id 
    ON index_jobs(screenshot_id);
```

Job types implemented:
- `UPSERT_SCREENSHOT`: Discovered/modified screenshots to hash, inspect, and OCR.
- `DELETE_SCREENSHOT`: Verified deleted files to remove from screenshots and FTS5.

Atomic single-statement claim query (zero race condition):
```sql
UPDATE index_jobs
SET status = 'PROCESSING',
    lease_until = datetime('now', '+' || ?1 || ' seconds'),
    attempts = attempts + 1,
    updated_at = datetime('now')
WHERE id = (
    SELECT id FROM index_jobs
    WHERE status = 'PENDING'
      AND available_at <= datetime('now')
    ORDER BY available_at ASC, id ASC
    LIMIT 1
)
RETURNING id, folder_id, screenshot_id, path, job_type, status, dedupe_key, attempts, max_attempts, available_at, lease_until, last_error_code, last_error_message, created_at, updated_at, completed_at;
```

### screenshot_embeddings

Implemented in **Migration v4** for local semantic text search:

```sql
CREATE TABLE IF NOT EXISTS screenshot_embeddings (
    screenshot_id INTEGER PRIMARY KEY,
    model_id TEXT NOT NULL,
    model_version TEXT NOT NULL,
    embedding BLOB NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY(screenshot_id) REFERENCES screenshots(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_screenshot_embeddings_model 
    ON screenshot_embeddings(model_id, model_version);
```

Properties:
- **Primary Key:** `screenshot_id` with `ON DELETE CASCADE` guarantees atomic deletion when screenshot records are removed.
- **Vector format:** Little-endian binary `f32` floats (384 dimensions $\times$ 4 bytes = 1,536 bytes per vector).
- **Model Versioning:** `model_id` and `model_version` isolate embeddings so models can be upgraded or rebuilt cleanly without mixing stale vectors.

### OCR Pipeline Versioning (Migration v5)

Implemented in **Migration v5** for local OCR engine versioning, multilingual fallback tracking, and safe re-processing:

```sql
ALTER TABLE screenshots ADD COLUMN ocr_engine_version TEXT;
ALTER TABLE screenshots ADD COLUMN ocr_language TEXT;
ALTER TABLE screenshots ADD COLUMN ocr_pipeline_version TEXT DEFAULT 'v1';
CREATE INDEX IF NOT EXISTS idx_screenshots_ocr_pipeline_version 
    ON screenshots(ocr_pipeline_version);
```

Properties:
- **`ocr_engine_version`:** Identifies specific engine build (`winrt_v1`, `ppocr_v4`).
- **`ocr_language`:** Records detected or used language tag (`en-US`, `vi-VN`, etc.).
- **`ocr_pipeline_version`:** Composite identifier (e.g. `windows_media_ocr:winrt_v1` or `multilingual_ocr:ppocr_v4`) used by `get_re_ocr_eligible_count` to detect screenshots indexed with older/lower quality pipelines.

---

## 4. FTS5 Virtual Table & Search Index

Implemented in **Migration v2** as a standalone SQLite FTS5 table with explicit synchronization:

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS screenshots_fts USING fts5(
    filename,
    ocr_search_text,
    tokenize = 'unicode61 remove_diacritics 2'
);
```

### Design Rationale:
- **`rowid` mapping:** Each FTS entry's implicit 64-bit `rowid` directly equals `screenshots.id`. This provides $O(1)$ joins (`JOIN screenshots s ON s.id = screenshots_fts.rowid`) without unindexed column overhead.
- **Standalone with Explicit Sync vs External Content:** Avoids brittle SQLite shadow table triggers. The source of truth remains `screenshots`; `screenshots_fts` is purely derived.
- **Tokenizer (`unicode61 remove_diacritics 2`):** Enables accent-insensitive matching across Latin scripts while treating punctuation as natural word boundaries.

### Synchronization Invariants:
1. **OCR Success:** When `save_ocr_success(id, ocr_text)` commits, `normalize_search_text(ocr_text)` is computed and synced via:
   `INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text) SELECT id, filename, ?2 FROM screenshots WHERE id = ?1;`
2. **File Modified:** When `update_screenshot(id, ...)` resets `ocr_status = 'PENDING'`, stale FTS entries are immediately purged via:
   `DELETE FROM screenshots_fts WHERE rowid = ?1;`
3. **File Deleted:** When `delete_screenshot(id)` is called, the FTS record is immediately purged:
   `DELETE FROM screenshots_fts WHERE rowid = ?1;`
4. **Folder Deleted:** When `delete_folder(folder_id)` is invoked, associated FTS rows are cleaned up atomically:
   `DELETE FROM screenshots_fts WHERE rowid IN (SELECT id FROM screenshots WHERE folder_id = ?1);`
5. **Rebuild & Health Diagnostic:**
   - `rebuild_search_index(conn)`: Clears and fully re-populates `screenshots_fts` from all `SUCCEEDED` screenshots.
   - `check_search_index_health(conn)`: Asserts `fts_count == succeeded_count`.

---

## 5. Transactions

Use transactions for logically related database changes.

Examples:

- successful OCR result + searchable state
- deletion of screenshot + FTS cleanup
- content change + invalidation of derived records

Avoid holding SQLite write transactions open while doing:

- OCR
- filesystem reads
- image decoding
- thumbnail generation
- AI inference

Correct pattern:

```text
Read/compute externally
        |
        v
short DB transaction
        |
        v
commit
```

---

## 6. Migrations

Database migrations are append-only once released.

Rules:

- do not manually mutate user databases outside the migration mechanism
- migrations must be safe for existing installations
- destructive migrations require an explicit migration plan
- schema changes affecting FTS must include synchronization/rebuild behavior
- backup/rebuild strategy should be documented before major schema redesign

---

## 7. Derived vs Source Data

Original screenshot files are external source data.

The following should be considered rebuildable/derived:

- thumbnails
- OCR search index
- FTS tables
- embeddings
- auto tags

Where practical, derived data should be recoverable from original screenshots.

---

## 8. Deletion Semantics

If a screenshot disappears from disk:

- it must stop appearing in normal search results
- corresponding FTS data must be removed or disabled
- thumbnail may be cleaned up
- embeddings may be removed
- original user files must never be deleted by index cleanup

If a watched folder is removed from settings, behavior must be explicitly defined:

Option A:
- remove its index records

Option B:
- keep existing index records but stop watching

Do not implement ambiguous behavior.

---

## 9. Sensitive Data

The database can contain OCR text with sensitive information.

Therefore:

- never upload the SQLite database automatically
- never include OCR text in diagnostics by default
- think carefully before implementing backups
- future cloud sync must be explicitly opt-in and privacy-reviewed

---

## 10. Database Performance

Important indexes may include:

```text
screenshots.path
screenshots.folder_id
screenshots.content_hash
screenshots.modified_at_fs
screenshots.indexed_at
index_jobs.status
index_jobs.available_at
```

Do not add indexes blindly.

Measure query patterns first.

---

## 11. Database Source of Truth

The actual migration files and schema implementation are the source of truth.

Update this document when data model invariants or major schema structure change.
