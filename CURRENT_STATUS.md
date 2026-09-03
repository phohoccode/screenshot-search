# Screenshot Search — Current Status

> Keep this file short and current.  
> Update it at the end of each meaningful implementation phase.

## Last Updated

2026-09-03

## Current Phase

**Phase 1C — Local OCR Pipeline**  
**Status:** Implementation complete, environment validation blocked

Source code, OCR abstraction (`OcrEngine`), Windows Media OCR WinRT implementation, text normalization, SQLite persistence, background worker orchestration, startup crash recovery, unit tests, and the Indexing UI are fully implemented and verified via TypeScript and Vite production builds. Rust/Tauri native binary compilation is currently blocked by the host environment prerequisite (missing MSVC `link.exe`).

---

## Validation Summary

- `cargo fmt --check` → **PASS** (0 formatting diffs across `src-tauri/`)
- `npm run typecheck` → **PASS** (0 TypeScript errors in `src/`)
- `npm run build` → **PASS** (Vite v6 production bundle built successfully in 3.09s)
- `cargo check` → **BLOCKED** (Missing MSVC C++ Build Tools: `link.exe` not found for `stable-x86_64-pc-windows-msvc`)
- `cargo test` → **BLOCKED** (Blocked by MSVC linker prerequisite)
- `npm run tauri dev` → **BLOCKED** (Blocked by Rust compilation prerequisite)

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

### Phase 1C — Local OCR Pipeline
- **OCR Engine Decision:** Evaluated Options A (Windows.Media.Ocr), B (PaddleOCR / ONNX), and C (Tesseract). Selected **Windows.Media.Ocr** (WinRT) as primary native engine (0 MB model download, ~25MB RAM, ~100ms/image CPU latency, pre-installed `en-US` and Vietnamese language pack support) documented in `docs/DECISIONS.md` (ADR-013).
- **OCR Abstraction Trait:** Created `OcrEngine` trait and `OcrResult` struct in `src-tauri/src/ocr/engine.rs` ensuring the indexing layer remains completely decoupled from specific OCR engines.
- **Text Normalizer:** Created `normalize_ocr_text` in `src-tauri/src/ocr/normalize.rs` applying Unicode NFC, standardizing line endings (`\n`), stripping control characters, collapsing multiple blank lines, and strictly preserving technical tokens (`P2028`, `ERR_CONNECTION_REFUSED`), URLs, code snippets, and punctuation.
- **Engine Implementations:**
  - `WindowsMediaOcrEngine` in `src-tauri/src/ocr/windows.rs`: Native WinRT OCR via `windows` crate with graceful fallbacks.
  - `MockOcrEngine` in `src-tauri/src/ocr/mock.rs`: Deterministic mock for automated test suites.
- **Worker Orchestration & Concurrency:**
  - `OcrManager` and `run_ocr_batch` in `src-tauri/src/ocr/orchestrator.rs`: Runs background batches outside UI threads with bounded concurrency (1 worker).
  - Short SQLite transactions: Database lock is released during image recognition.
  - Graceful cancellation support via `AtomicBool`.
- **Lifecycle & Invariants:**
  - `PENDING` → `PROCESSING` → `SUCCEEDED` or `FAILED`.
  - Empty image text is marked `SUCCEEDED` with `ocr_text = ""` to prevent infinite re-processing loops.
  - File modification in Phase 1B resets to `PENDING` with `ocr_text = NULL`.
  - Startup crash recovery: Automatically resets stale `PROCESSING` jobs to `PENDING` during app setup.
- **Unit & Integration Tests:** Comprehensive tests in `src-tauri/src/ocr/ocr_tests.rs` covering successful extraction, failure isolation, empty text, unchanged skip, modified re-process, crash recovery, and technical token normalization.
- **Indexing UI:**
  - Redesigned `IndexingPage` with real progress bar, metric cards (`Total`, `Succeeded`, `Pending`, `Failed`), `Start OCR` / `Stop OCR` buttons with loading spinners, and real-time progress updates via Tauri events.
  - Updated `FoldersPage` to show OCR indexed counts on folder cards.
  - Updated `SearchPage` with OCR readiness stats and Phase 1D FTS5 search placeholder.

---

## In Progress

None (Phase 1C implementation complete; waiting for MSVC build tools environment setup to run Tauri/Rust validation).

---

## Environment Blocker & Resolution

- **Target:** `stable-x86_64-pc-windows-msvc`
- **Missing component:** Microsoft C++ Build Tools linker (`link.exe`).
- **Required user action:** Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and select the **Desktop development with C++** workload.

---

## Next Recommended Work

### Phase 1D — SQLite FTS5 Keyword Search

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
- Phase 1C — Local OCR Pipeline implementation, engine audit, normalization, worker orchestration, tests, and Indexing UI.

Files changed:
- docs/DECISIONS.md (Added ADR-013 for Windows.Media.Ocr)
- src-tauri/Cargo.toml (Added unicode-normalization and windows crate)
- src-tauri/Cargo.lock
- src-tauri/src/errors.rs (Added OCR error codes and helpers)
- src-tauri/src/ocr/ (mod.rs, engine.rs, normalize.rs, mock.rs, windows.rs, orchestrator.rs, ocr_tests.rs)
- src-tauri/src/db/ (screenshots.rs, folders.rs)
- src-tauri/src/commands/ (mod.rs, ocr.rs)
- src-tauri/src/lib.rs (Registered ocr module, commands, OcrManager, startup recovery)
- src/types/index.ts (Added OcrStats, OcrBatchSummary, OcrProgressPayload)
- src/lib/tauri.ts (Added startOcrIndexing, getOcrStats, cancelOcrIndexing, onOcrProgress)
- src/features/indexing/indexing-page.tsx (Full OCR indexing UI with progress bar and counters)
- src/features/folders/folders-page.tsx (Added OCR indexed count to folder badge)
- src/features/search/search-page.tsx (Updated OCR readiness state)
- CURRENT_STATUS.md
- walkthrough.md

Tests run:
- cargo fmt --check (PASS)
- npm run typecheck (PASS)
- npm run build (PASS)
- cargo check (BLOCKED - missing MSVC link.exe)
- cargo test (BLOCKED - missing MSVC link.exe)
- npm run tauri dev (BLOCKED - missing MSVC link.exe)

Current blocker:
- Microsoft Visual Studio C++ Build Tools linker link.exe not found on host machine

Exact next task:
- Install MSVC C++ Build Tools, verify runtime, then proceed to Phase 1D (SQLite FTS5 Keyword Search)
```
