# Screenshot Search — Current Status

> Keep this file short and current.  
> Update it at the end of each meaningful implementation phase.

## Last Updated

2026-09-03

## Current Phase

**Phase 1C — Local OCR Pipeline**  
**Status:** COMPLETED

Source code, OCR abstraction (`OcrEngine`), native Windows Media OCR WinRT implementation, adaptive 3-tier image strategy (downscale + bounded in-memory tiling), text normalization, SQLite persistence, background worker orchestration, startup crash recovery, 32 comprehensive tests (including real image fixtures), and the Indexing UI are fully implemented, verified, and validated on the Windows host.

---

## Validation Summary

- `cargo fmt --check` → **PASS** (0 formatting diffs across `src-tauri/`)
- `cargo check --manifest-path ./src-tauri/Cargo.toml` → **PASS** (Clean compilation, 0 errors, 0 warnings)
- `cargo test --manifest-path ./src-tauri/Cargo.toml` → **PASS** (32 passed; 0 failed; finished in 0.27s)
- `npm run typecheck` → **PASS** (0 TypeScript errors in `src/`)
- `npm run build` → **PASS** (Vite v6 production bundle built successfully in 3.00s)
- `npm run tauri dev` → **PASS** (Tauri 2 dev server and native window launched cleanly)
- **Real Windows WinRT OCR** → **PASS** (Empirically verified against English, Vietnamese fallback, mixed technical, code/terminal, 4K, tall, and wide fixtures)

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

None (Phase 1C completed and verified; Phase 1D not started).

---

## Blockers

None.

---

## Next Recommended Work

### Phase 1D — SQLite FTS5 Keyword Search (NOT STARTED)

1. Create SQLite FTS5 virtual table for screenshots (`fts_screenshots`).
2. Implement triggers or synchronization layer from `screenshots(id, filename, ocr_text)` to FTS5.
3. Implement `MATCH` query with BM25 ranking and query term normalization.
4. Build interactive Search UI with search input, debounce, and thumbnail grid.
5. Add screenshot preview modal with highlighted text snippets.
6. Add "Open Original" and "Reveal in Explorer" actions.

---

## Handoff Template

```text
Last completed task:
- Phase 1C — Local OCR Pipeline final acceptance & runtime validation complete (32 tests PASS, real WinRT OCR PASS, tauri dev PASS).

Files changed:
- src-tauri/Cargo.toml (Added windows features: Win32_System_Com, Foundation_Collections, Globalization)
- src-tauri/src/db/connection.rs (Arc<Mutex<Connection>> for thread-safe worker access)
- src-tauri/src/db/screenshots.rs (Unified query_map closure type)
- src-tauri/src/ocr/normalize.rs (Owned String allocation in normalize_ocr_text)
- src-tauri/src/ocr/windows.rs (Adaptive 3-tier tiling strategy, BitmapTransform::SetBounds, conservative deduplication)
- src-tauri/src/commands/ocr.rs (COM MTA CoInitializeEx/CoUninitialize on worker thread)
- src-tauri/src/filesystem/scanner_tests.rs (Added missing ocr_succeeded_count field)
- src-tauri/src/ocr/ocr_tests.rs (Added real fixtures, tiling math, benchmarks, diagnostics)
- src-tauri/tests/fixtures/ (Generated real PNG test fixtures)
- CURRENT_STATUS.md (Updated to Phase 1C — COMPLETED)

Tests run:
- cargo fmt --check (PASS)
- cargo check (PASS - 1.14s)
- cargo test (PASS - 32 passed, 0 failed, 0.27s)
- npm run typecheck (PASS)
- npm run build (PASS - 3.00s)
- npm run tauri dev (PASS - launched successfully)

Current blocker:
- None

Exact next task:
- Phase 1D — SQLite FTS5 Keyword Search (when instructed by user)
```
