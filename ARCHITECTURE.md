# Screenshot Search — Architecture

## 1. Architecture Goals

The architecture must support:

- local-first operation
- privacy-sensitive screenshot processing
- fast search
- incremental indexing
- large screenshot collections
- replaceable OCR implementation
- future local AI inference
- minimal background resource usage
- safe recovery from interrupted indexing

---

## 2. Layer Overview

```text
React UI
   |
   v
Tauri IPC Commands / Events
   |
   v
Application Services
   |
   +-------------------+-------------------+
   |                   |                   |
   v                   v                   v
Indexing            Search             Settings
   |
   +---------+---------+---------+
   |         |         |         |
   v         v         v         v
Scanner    Watcher    OCR    Thumbnail
   |
   v
Persistence
   |
   v
SQLite + FTS5
```

Phase 3.5 OCR Engine Router Architecture:

```text
Screenshot File
   │
   ▼
OcrEngineRouter (Auto / Windows / Multilingual)
   │
   ├── [Auto / Windows] ──────► WindowsMediaOcrEngine (WinRT native)
   │                             └── Fast native OCR (prioritized if vi-VN supported)
   │
   └── [Auto / Multilingual] ──► MultilingualOcrEngine (Local PP-OCRv4 ONNX)
                                 └── High-accuracy Vietnamese & diacritics (~16 MB)
   │
   ▼
Normalized OCR Output
   │
   ├──► SQLite `screenshots` (ocr_text, ocr_engine, ocr_pipeline_version)
   ├──► SQLite FTS5 `screenshots_fts` (atomic sync)
   └──► Stale Vector Invalidation ──► `GENERATE_TEXT_EMBEDDING` (Phase 3 refresh)
```

Phase 3 Hybrid Search Architecture:

```text
Query
  |
  +-----------------------------------+
  |                                   |
  v                                   v
SQLite FTS5 (Top 100)        Local Embedding Model
(BM25 normalized)             `multilingual-e5-small`
                                      |
                                      v
                             In-Process Cosine Scan
                             SQLite BLOB Vectors (Top 100)
                                      |
  +-----------------------------------+
  |
  v
Union Candidate Set
  |
  v
Hybrid Ranker
  ├── Exact Technical Token Guard (4.0x dominance)
  ├── Filename Matching (2.0x boost)
  ├── Normalized FTS Score (1.5x)
  ├── Normalized Semantic Score (1.2x)
  └── Recency Tiebreak
  |
  v
Paginated Ranked Results (Top 50)
```

---

## 3. Frontend Responsibilities

The frontend should be responsible for:

- rendering UI according to `docs/UI_DESIGN_SYSTEM.md`
- reusing shared shadcn-style primitives before creating custom controls
- accepting user input
- displaying indexing state
- search query state
- result rendering
- screenshot preview
- settings UI
- user-triggered actions

The frontend should NOT:

- invent feature-specific visual systems that conflict with `docs/UI_DESIGN_SYSTEM.md`
- duplicate existing shared UI primitives without justification
- recursively scan folders itself
- perform expensive OCR
- hash large files
- directly manipulate SQLite
- implement filesystem watchers
- run heavy AI inference on the UI thread

---

## 4. Tauri/Rust Responsibilities

Rust is the trusted local core.

Suggested domains:

```text
src-tauri/src/
├── commands/
├── db/
├── indexing/
├── ocr/
├── search/
├── thumbnails/
├── watcher/
├── settings/
├── filesystem/
└── errors/
```

### commands

Thin IPC boundary between frontend and native core.

Commands should:

- validate input
- invoke application services
- map errors to stable frontend-safe errors

Commands should not contain large amounts of business logic.

### db

Responsibilities:

- connection initialization
- migrations
- repositories
- transactions
- FTS synchronization
- database health checks

### indexing

Responsibilities:

- create indexing jobs
- coordinate scanner/OCR/thumbnail work
- enforce concurrency limits
- track progress
- retry recoverable failures
- skip unchanged files

### ocr

Expose an OCR abstraction.

Conceptual interface:

```text
OCRService
  recognize(image_path) -> OCRResult
```

`OCRResult` should support at least:

- full normalized text
- raw text if needed internally
- optional text blocks
- optional bounding boxes
- detected language if available
- engine metadata/version

Indexing code must not depend directly on one OCR vendor.

### search

Responsibilities:

- normalize query
- FTS query
- ranking
- filters
- result hydration
- future semantic search
- future hybrid ranking

### thumbnails

Responsibilities:

- generate efficient preview images
- deterministic thumbnail paths
- invalidate thumbnails when source changes
- avoid loading original 4K images into search grid

### watcher

Responsibilities:

- subscribe to selected folders
- debounce noisy file events
- detect create/update/delete
- schedule indexing
- avoid performing OCR directly inside watcher callback

---

## 5. Indexing Pipeline

### Initial Scan

```text
User selects folder
       |
       v
Persist Folder
       |
       v
Recursive Scan
       |
       v
Supported File?
   |         |
   no        yes
   |          |
 skip         v
         Read Metadata
              |
              v
          File Identity
              |
              v
      Existing unchanged?
        |           |
       yes          no
        |            |
      skip           v
                 Queue Job
                     |
                     v
              Generate Thumb
                     |
                     v
                    OCR
                     |
                     v
              Normalize Text
                     |
                     v
                  Commit DB
                     |
                     v
                 Update FTS
```

The exact ordering of thumbnail and OCR may be parallelized later.

---

## 6. File Identity

Do not use filename alone.

A screenshot may be:

- renamed
- moved
- replaced
- edited

Possible identity inputs:

- canonical path
- size
- modification time
- hash

Recommended approach:

- use path for current location
- use content hash or a robust file fingerprint to determine whether content changed
- avoid hashing every large file repeatedly when metadata proves it is unchanged

The implementation may introduce a fast fingerprint and only calculate full hash when needed.

---

## 7. Background Work

All heavy operations must be outside the UI thread.

Examples:

- recursive scanning
- hashing
- OCR
- thumbnail generation
- embedding generation

Use bounded concurrency.

Do not spawn unbounded work for thousands of images.

Desired behavior:

```text
8,000 screenshots
       |
       v
bounded queue
       |
       +--> worker
       +--> worker
       +--> worker
       |
       v
steady progress
```

Concurrency should be configurable internally and later tunable based on CPU capabilities.

---

## 8. Progress Model

The backend should expose enough state for UI such as:

```text
status
discovered_count
queued_count
processing_count
indexed_count
failed_count
skipped_count
total_count
```

Potential statuses:

```text
IDLE
SCANNING
INDEXING
PAUSED
COMPLETED
CANCELLED
FAILED
```

Do not rely solely on frontend state for indexing truth.

---

## 9. Search Architecture

Phase 1:

```text
Query
  |
  v
Normalize
  |
  v
SQLite FTS5
  |
  v
BM25 ranking
  |
  v
Metadata filters
  |
  v
Hydrate results
  |
  v
UI
```

Future hybrid:

```text
Query
  |
  +-------------------+
  |                   |
  v                   v
Keyword            Embedding
  |                   |
  v                   v
FTS Score        Semantic Score
  |                   |
  +---------+---------+
            |
            v
       Hybrid Ranker
            |
            v
          Results
```

---

## 10. Thumbnail Architecture

Thumbnail cache should live in application-owned storage.

Example:

```text
AppData/
└── ScreenshotSearch/
    ├── database.sqlite
    ├── thumbnails/
    ├── models/
    └── logs/
```

Do not modify original screenshots.

Thumbnail filenames should derive from a stable identifier such as screenshot ID or content hash.

---

## 11. Failure Handling

Failures should be categorized.

Examples:

```text
FILE_NOT_FOUND
FILE_PERMISSION_DENIED
UNSUPPORTED_IMAGE
IMAGE_DECODE_FAILED
OCR_FAILED
DATABASE_FAILED
THUMBNAIL_FAILED
WATCHER_FAILED
MODEL_LOAD_FAILED
```

Do not collapse all failures into generic strings.

Recoverable failures should be retryable.

Permanent failures should not loop forever.

---

## 12. Shutdown and Recovery

The app may close while indexing.

On restart:

- recover unfinished jobs
- verify source files still exist
- avoid duplicate OCR
- continue safely
- do not assume in-memory queue state survived

Initial MVP may use a simpler recoverable model, but architecture should allow persistent queue state later.

---

## 13. Future AI Boundaries

AI must remain behind dedicated services.

Conceptual interfaces:

```text
TextEmbeddingService
VisualEmbeddingService
SemanticSearchService
```

Do not call AI models directly from React components.

Do not mix embedding model implementation with general database repositories.

---

## 14. Architecture Source of Truth

When this document disagrees with actual code:

1. inspect the implementation;
2. determine whether the code or document is outdated;
3. do not silently assume this document is correct;
4. update the document after architectural changes are intentionally accepted.
