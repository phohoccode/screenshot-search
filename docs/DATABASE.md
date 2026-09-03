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

### index_jobs

Potential future persistent queue:

```text
id
screenshot_id
job_type
status
attempts
last_error_code
last_error_message
available_at
lease_until
created_at
updated_at
completed_at
```

Possible job types:

```text
OCR
THUMBNAIL
TEXT_EMBEDDING
VISUAL_EMBEDDING
```

Possible statuses:

```text
PENDING
PROCESSING
SUCCEEDED
FAILED
CANCELLED
```

### screenshot_embeddings

Future phase:

```text
screenshot_id
model_id
model_version
embedding_type
vector
created_at
```

Possible embedding types:

```text
OCR_TEXT
VISUAL
CAPTION
```

---

## 4. FTS5

Suggested logical FTS fields:

```text
screenshot_id
filename
ocr_text
```

The implementation may use:

- external-content FTS table
- contentless FTS table
- normal FTS table with explicit synchronization

Choose one design intentionally and document it.

### Search behavior

FTS should support:

- exact words
- technical error codes
- phrases
- prefixes where useful
- filename search
- OCR text search

Use BM25 or equivalent FTS ranking as the Phase 1 ranking baseline.

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
