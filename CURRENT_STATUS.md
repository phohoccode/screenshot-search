# Screenshot Search — Current Status

> Keep this file short and current.  
> Update it at the end of each meaningful implementation phase.

## Last Updated

2026-09-03

## Current Phase

**Phase 2 — Automatic Filesystem Watcher + Background Indexing Reliability**  
**Status:** COMPLETED

The automatic, zero-manual-intervention background indexing pipeline is now complete:
1. Watched folders monitored continuously via native Windows ReadDirectoryChangesW (`notify` crate).
2. Temporary files (`.tmp`, `.crdownload`, `~$*`) and non-image artifacts filtered out immediately.
3. Rapid filesystem burst events coalesced in-memory with a 500ms sliding debounce window.
4. File stability verification checks file size and mtime and tests shared file opening to avoid partial-write OCR.
5. Durable SQLite job queue (`index_jobs` table via Migration v3) with atomic claims, crash-proof leases, deduplication, and exponential backoff.
6. Single-flight conservative OCR background worker processes jobs off-thread without database locks.
7. File modifications immediately invalidate old FTS5 search entries and update with new OCR text.
8. File deletions verify genuine NotFound on disk and remove screenshot metadata and FTS5 entries atomically.
9. Startup reconciliation automatically discovers and enqueues offline additions, edits, and deletions across all enabled folders.
10. UI enhanced with live "Watching" badges, background indexing status dashboard, pause/resume controls, retry failed actions, and cache-busted preview rendering (`?v=<content_hash>`).
11. Zero external network calls verified: external placeholder dependencies completely eliminated.

---

## Validation Summary

- `cargo fmt --check` → **PASS** (0 formatting diffs across `src-tauri/`)
- `cargo check --manifest-path ./src-tauri/Cargo.toml` → **PASS** (Clean compilation, 0 errors, 0 warnings)
- `cargo test --manifest-path ./src-tauri/Cargo.toml` → **PASS** (**55 passed**; 0 failed; finished in 0.77s)
- `npm run typecheck` → **PASS** (0 TypeScript errors in `src/`)
- `npm run build` → **PASS** (Vite v6 production bundle built successfully in 3.28s)
- `npm run tauri dev` → **PASS** (Application booted, Migration v3 completed, background worker and watcher active, startup scan executed in 3ms)
- **Micro-Hardening (Cache & Zero-Network)** → **PASS** (Preview URL includes `?v=<content_hash>`, external `placehold.co` removed from CSP and code in favor of local inline SVG)

---

## Completed

### Phase 1A — Foundation
- Product concept and local-first architecture defined.
- Tauri 2 + React 19 + TypeScript + Vite 6 + Tailwind CSS v4 stack configured.
- Minimal SaaS / shadcn-style UI primitives and design tokens established.
- SQLite integration with WAL mode and initial schema migration v1 (`folders`, `screenshots`).
- Shared typed error handling (`AppError`, `ErrorCode`, `CommandResult`).

### Phase 1B — Folder Selection + Screenshot Discovery
- **Native Folder Picker:** Integrated via `tauri-plugin-dialog` (`pick_folder` command) with browser mock fallback.
- **Folder Persistence:** Normalized canonical folder paths persisted in SQLite with uniqueness enforcement (`FOLDER_ALREADY_EXISTS`).
- **Recursive Image Discovery:** Safe traversal with `walkdir` preventing symlink recursion cycles and handling permission errors safely.
- **Format Filtering:** Case-insensitive matching for `.png`, `.jpg`, `.jpeg`, `.webp`.
- **File Metadata & Fingerprinting:** Reads metadata and computes streaming 64KB SHA-256 content hashes outside database locks.
- **Hardened Deletion Reconciliation:** Subtree safety check prevents accidental record deletion during incomplete directory traversals.
- **Unit & Integration Tests:** 6 comprehensive tests in `scanner_tests.rs`.

### Phase 1C — Local OCR Pipeline (Audited & Hardened)
- **OCR Engine Decision:** Evaluated Options A (Windows.Media.Ocr), B (PaddleOCR / ONNX), and C (Tesseract). Selected **Windows.Media.Ocr** (WinRT) as primary native engine (0 MB model download, hardware-accelerated CPU execution) documented in `docs/DECISIONS.md` (ADR-013).
- **Oversized Image Downscaling:** Hardened against `OcrEngine.MaxImageDimension` limits (2,600px). Pre-inspects image width and height via `BitmapDecoder`; if oversized (e.g. 4K 3840x2160, ultra-wide 5120x1440), automatically calculates aspect-ratio-preserving dimensions and downscales in-memory using `BitmapTransform` with `BitmapInterpolationMode::Fant`. Original screenshot files on disk are strictly untouched.
- **Engine Reuse:** Native WinRT engine is pre-initialized on creation and reused across all screenshots rather than re-creating per image.
- **Language Diagnostics:** Added `get_ocr_engine_info` detecting active language and whether Vietnamese (`vi-VN`) is installed. UI explicitly indicates language status rather than silently claiming Vietnamese support.
- **Claim Race & Single-Flight Protection:**
  - `mark_processing` uses atomic conditional update `WHERE id = ?1 AND ocr_status = 'PENDING'` preventing duplicate claims across concurrent workers.
  - `start_ocr_indexing` uses atomic CAS with an RAII `RunningGuard` ensuring the lock is always reset upon completion or panic.
- **Text Normalizer:** Created `normalize_ocr_text` in `src-tauri/src/ocr/normalize.rs` applying Unicode NFC, standardizing line endings (`\n`), stripping control characters, collapsing multiple blank lines, and strictly preserving technical tokens (`P2028`, `ERR_CONNECTION_REFUSED`), URLs, code snippets, and punctuation.
- **Lifecycle & Invariants:**
  - `PENDING` → `PROCESSING` → `SUCCEEDED` or `FAILED`.
  - Empty image text is marked `SUCCEEDED` with `ocr_text = ""` to prevent infinite re-processing loops.
  - File modification in Phase 1B resets to `PENDING` with `ocr_text = NULL`.
  - Startup crash recovery: Automatically resets stale `PROCESSING` jobs to `PENDING` during app setup.
- **Adaptive 3-Tier Image Strategy:**
  - **Normal** ($\le 2600$ px): 100% native resolution direct OCR.
  - **Moderately oversized** (aspect ratio $\le 2.2$, max side $> 2600$ px, e.g. 4K $3840\times 2160$): Proportional downscaling via `BitmapTransform` with `BitmapInterpolationMode::Fant`.
  - **Extremely tall / wide** (aspect ratio $> 2.2$, e.g. $1080\times 5200$, $1440\times 10000$, $5120\times 1440$): In-memory bounded tile extraction using `BitmapTransform::SetBounds(BitmapBounds)` with 150px overlap. Merged in deterministic reading order with conservative boundary deduplication.
  - Original screenshot files on disk are strictly read-only and never modified.
- **WinRT Threading & Apartment Semantics:**
  - Audited `WindowsMediaOcrEngine` against Windows COM/WinRT threading semantics.
  - Background indexing runs in dedicated worker thread via `tauri::async_runtime::spawn_blocking` with explicit `CoInitializeEx(COINIT_MULTITHREADED)` and `CoUninitialize()`.
  - Engine is instantiated once per worker batch and reused across all screenshots; never recreated per image.
  - No `unsafe impl Send` or `Sync` tricks; `windows::Media::Ocr::OcrEngine` is inherently Agile.
- **Real OCR Fixtures & Empirical Validation:**
  - Real PNG test fixtures generated and verified:
    - English (`english.png`): "Screenshot Search Hello World HTTP 500" extracted accurately.
    - Vietnamese (`vietnamese.png`): Verified behavior with installed host language pack (`en-US` active recognizer, extracts unaccented Latin characters).
    - Mixed Technical (`mixed_technical.png`): Extracted `PrismaClientKnownRequestError`, `Transaction already closed`, `P2028`.
    - Code / Terminal (`code_terminal.png`): Extracted `npm run build`, `ERR MODULE NOT FOUND`, `localhost`.
    - 4K Screenshot ($3840\times 2160$): Extracted text after proportional downscaling.
    - Tall Screenshot ($1080\times 5200$): All 3 vertical tiles recognized and merged seamlessly.
    - Wide Screenshot ($5120\times 1440$): All 3 horizontal tiles recognized and merged seamlessly.
- **Language Diagnostics:**
  - Engine info queried directly: active language `en-US`, available languages `["en-US"]`, supports Vietnamese `false`, max image dimension `10,000` px.
- **Empirical Performance Benchmarks:**
  - 1080p ($1920\times 1080$): **44.14 ms**
  - 1440p ($2560\times 1440$): **44.44 ms**
  - 4K UHD ($3840\times 2160$): **70.53 ms**
  - Long Scrolling ($1080\times 5200$, 3 tiles): **83.43 ms**
  - Code / Terminal ($900\times 500$): **9.78 ms**
- **Unit & Integration Tests:** 32 comprehensive tests in `src-tauri/src/ocr/ocr_tests.rs` and `scanner_tests.rs` (all passing).
- **Indexing UI:**
  - `IndexingPage` displays progress bar, metric cards (`Total`, `Succeeded`, `Pending`, `Failed`), `Start OCR` / `Stop OCR` buttons with loading spinners, engine diagnostics (active language, Vietnamese support status, max dimension), and zero-cloud privacy notice.
  - Updated `FoldersPage` to show OCR indexed counts on folder cards.
  - Updated `SearchPage` with OCR readiness stats and Phase 1D FTS5 search placeholder.

---

## In Progress

### Phase 1D — SQLite FTS5 Keyword Search + OCR-Tolerant Search (Core MVP)
- **FTS5 Virtual Table Migration:** Migration v2 adds `screenshots_fts` with `tokenize = 'unicode61 remove_diacritics 2'`, using `rowid` mapped to `screenshots.id`. Automatic idempotent backfill indexes existing `SUCCEEDED` screenshots without requiring re-OCR.
- **OCR-Tolerant Normalization:**
  - `normalize_search_text` & `normalize_search_query` symmetrically convert underscores `_`, hyphens `-`, and punctuation to whitespace while preserving technical tokens (`P2028`, `HTTP 500`) and dotted formats (`v1.2.3`, `192.168.1.1`).
  - Resolves OCR punctuation-loss variance (`ERR_MODULE_NOT_FOUND` matches `ERR MODULE NOT FOUND`).
  - Raw OCR text (`screenshots.ocr_text`) is preserved intact for human viewing.
- **BM25 Ranking with Filename Boost:**
  - Weighted BM25: `bm25(screenshots_fts, 5.0, 1.0)` boosts filename relevance 5.0x over OCR text.
  - Recency tiebreaker: `ORDER BY score ASC, s.modified_at_fs DESC`.
- **Match Snippets & Zero HTML Injection:**
  - FTS `snippet()` wraps matched tokens with `[[match]]...[[/match]]`.
  - Frontend parses tokens into React text nodes (`<mark>`), avoiding `dangerouslySetInnerHTML`.
- **Empty Query Behavior:**
  - Searching empty/whitespace returns recent OCR-ready screenshots sorted by timestamp.
- **Strict Security Boundaries:**
  - `open_screenshot(id)` & `reveal_screenshot(id)` strictly take `id: i64`, validating against SQLite and filesystem before launching native Windows OS processes.
  - Arbitrary paths from the frontend are disallowed.
- **Index Synchronization & Rebuild:**
  - Changes to screenshots (edits/deletions) immediately purge stale FTS records.
  - Cascade folder deletions atomically purge child FTS rows.
  - `rebuild_search_index()` allows complete idempotent recovery.
  - `check_search_index_health()` diagnoses sync health.
- **Search UI & Preview Modal:**
  - Debounced search input (200ms) with clear button and `/` shortcut key.
  - Responsive desktop grid with lazy-loaded thumbnails (`convertFileSrc`).
  - Full screenshot preview dialog with metadata, scrollable OCR text, "Copy OCR Text" button, "Open original", and "Reveal in Explorer".
  - "Load more" pagination.
- **Empirical Search Benchmarks:**
  - 1,000 records: Single token **1.06 ms**, Phrase **1.11 ms**, Underscore **1.20 ms**, Filename **0.67 ms**.
  - 10,000 records: Single token **7.27 ms**, Phrase **12.13 ms**, Underscore **13.97 ms**, Filename **3.37 ms**.
- **Unit & Integration Tests:** 45 comprehensive tests passing in `src-tauri` (13 new search tests).

---

## In Progress

None (Phase 1D completed and verified; Core MVP complete).

---

## Blockers

None.

---

## Next Recommended Work

### Phase 2 — Automatic Filesystem Watcher + Background Indexing Reliability

1. Background directory watcher (`notify` crate) monitoring registered folders.
2. Automatic change detection for newly added, modified, or deleted screenshots.
3. Persistent background indexing queue with pause / resume capabilities.
4. Thumbnail caching infrastructure for ultra-fast instant grid rendering.

---

## Handoff Template

```text
Last completed task:
- Phase 1D — SQLite FTS5 Keyword Search + OCR-Tolerant Search complete (CORE MVP COMPLETED).
  All 45 tests PASS, 10k benchmark ~7-13ms, migration v2 backfilled, tauri dev verified.

Files changed:
- src-tauri/Cargo.toml (Added tauri feature: protocol-asset)
- src-tauri/tauri.conf.json (Configured assetProtocol enable and scope)
- src-tauri/src/errors.rs (Added file_not_found and unknown helpers)
- src-tauri/src/ocr/windows.rs (Removed hardcoded 2600 limit; uses runtime MaxImageDimension)
- src-tauri/src/db/migrations.rs (Added Migration 2 screenshots_fts and backfill logic)
- src-tauri/src/db/screenshots.rs (FTS sync on save/update/delete, get_screenshot_by_id, rebuild, health check)
- src-tauri/src/db/folders.rs (Purge FTS entries on folder deletion)
- src-tauri/src/search/ (mod.rs, normalize.rs, query.rs, search_tests.rs)
- src-tauri/src/commands/ (mod.rs, search.rs)
- src-tauri/src/lib.rs (Registered search module and commands)
- src/types/index.ts (Added SearchResultItem, SearchResultPage, ScreenshotDetail, SearchIndexHealth)
- src/lib/tauri.ts (Added searchScreenshots, getScreenshot, openScreenshot, revealScreenshot, getFileAssetUrl)
- src/features/search/search-page.tsx (Full search UI, highlight snippet, preview modal, load more)
- docs/DATABASE.md (Documented FTS5 virtual table schema, rowid, and sync)
- docs/SEARCH_CONTEXT.md (Documented implemented search pipeline and BM25 weighting)
- CURRENT_STATUS.md (Updated to Phase 1D — COMPLETED, CORE MVP COMPLETED)

Tests run:
- cargo fmt --check (PASS)
- cargo check (PASS - 2.41s)
- cargo test (PASS - 45 passed, 0 failed, 0.63s)
- npm run typecheck (PASS)
- npm run build (PASS - 3.11s)
- npm run tauri dev (PASS - launched successfully, migration v2 applied, 11 backfilled)

Current blocker:
- None

Exact next task:
- Phase 2 — Automatic Filesystem Watcher + Background Indexing Reliability (when instructed by user)
```
