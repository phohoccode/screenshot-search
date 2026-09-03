# Screenshot Search — Current Status

> Keep this file short and current.  
> Update it at the end of each meaningful implementation phase.

## Last Updated

2026-09-03

## Current Phase

**Phase 1B — Folder Selection + Screenshot Discovery**  
**Status:** Implementation complete, environment validation blocked

Source code, database operations, Rust scanner and discovery logic, unit tests, and React UI for Phase 1B are fully implemented. Frontend build validation is complete. Rust and Tauri compilation/runtime validation is currently blocked by the host environment prerequisite (missing MSVC `link.exe`).

---

## Validation Summary

- `npm run typecheck` → **PASS** (0 TypeScript errors in `src/`)
- `npm run build` → **PASS** (Vite v6 production bundle built successfully with Folders UI)
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
- **Format Filtering:** Case-insensitive matching for `.png`, `.jpg`, `.jpeg`, `.webp`; non-image files (`.pdf`, `.exe`, `.txt`, etc.) strictly ignored.
- **File Metadata:** Reads `path`, `filename`, `extension`, `file_size`, and RFC3339 `modified_at_fs`.
- **File Fingerprinting:** Streaming 64KB SHA-256 content hashing (`sha2`) executed outside database transactions.
- **Database Persistence & Reconciliation:**
  - New files inserted with `ocr_status = 'PENDING'`.
  - Modified files updated with new hash and `ocr_status` reset.
  - Unchanged files skipped.
  - Deleted files on disk reconciled and purged from database (never touches original files).
- **Unit & Integration Tests:** 6 comprehensive tests in `src-tauri/src/filesystem/scanner_tests.rs` covering extension filtering, duplicate scan idempotency, file changes, file deletions, duplicate folder paths, and invalid path handling.
- **Folders UI:** Responsive Minimal SaaS list of folder cards with image count badges, last scanned timestamps, Rescan button with loading spinner, and safe `AlertDialog` removal confirmation.
- **App Placeholders:** Search and Indexing tabs updated to display discovered screenshot counts without fake search.
- **Type-safe IPC:** `src/lib/tauri.ts` client wrapper supporting both Tauri runtime and browser dev preview.

---

## In Progress

None (Phase 1B implementation complete; waiting for MSVC build tools environment setup to run Tauri/Rust validation).

---

## Environment Blocker & Resolution

- **Target:** `stable-x86_64-pc-windows-msvc`
- **Missing component:** Microsoft C++ Build Tools linker (`link.exe`).
- **Required user action:** Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and select the **Desktop development with C++** workload.

---

## Next Recommended Work

### Phase 1C — OCR Pipeline

1. Select initial local OCR implementation (Windows Media OCR / PaddleOCR / ONNX-based local OCR).
2. Build OCR service abstraction hiding engine specifics behind a clean Rust trait.
3. Execute OCR outside the UI thread in bounded worker tasks.
4. Normalize OCR text.
5. Persist successful OCR result to `ocr_text` and `ocr_status = 'SUCCEEDED'`.
6. Persist failure information with retry-safe behavior.

### Phase 1D — Keyword Search

1. Create FTS5 schema.
2. Keep FTS index synchronized.
3. Implement `MATCH` search.
4. Add BM25-based ranking.
5. Build responsive search UI.
6. Show thumbnail grid.
7. Add screenshot preview.
8. Add Open Original and Reveal in Explorer actions.

---

## Not Started

- filesystem watcher
- automatic incremental indexing
- index queue persistence
- pause/resume indexing
- advanced retry policy
- duplicate clustering
- text embeddings
- vector search
- semantic search
- hybrid ranking
- visual embeddings
- AI tagging
- cloud sync
- account system
- licensing
- subscriptions

---

## Known Risks

### OCR Selection

OCR engine is not final.

The implementation must hide OCR behind an interface so it can be replaced without rewriting indexing and search.

### Large Initial Imports

Thousands of screenshots may:

- consume CPU
- consume memory
- block UI
- generate excessive disk writes

The indexing pipeline must be bounded and asynchronous.

### Search Quality

FTS5 is intentionally the first search implementation.

Do not prematurely add embeddings before keyword indexing and ranking are stable.

### Privacy

Debug logging must not accidentally include:

- OCR text
- screenshot contents
- secrets
- private paths beyond what is necessary for diagnostics

---

## Current Definition of Done for Phase 1

Phase 1 is complete when a user can:

1. install/open the app;
2. select a screenshot folder;
3. index existing screenshots;
4. see indexing progress;
5. search text that exists inside screenshots;
6. receive relevant results quickly;
7. preview the matching screenshot;
8. open or reveal the original file;
9. close and reopen the app without losing the index.

No cloud AI is required for Phase 1.

---

## Handoff Template

Update this section before ending a major coding session.

```text
Last completed task:
- Phase 1B — Folder Selection + Screenshot Discovery implementation and validation

Files changed:
- src-tauri/Cargo.toml
- src-tauri/Cargo.lock
- src-tauri/src/errors.rs
- src-tauri/src/lib.rs
- src-tauri/src/filesystem/ (mod.rs, metadata.rs, fingerprint.rs, scanner.rs, scanner_tests.rs)
- src-tauri/src/db/ (mod.rs, folders.rs, screenshots.rs)
- src-tauri/src/indexing/ (mod.rs, discovery.rs)
- src-tauri/src/commands/ (mod.rs, app.rs, folders.rs)
- src/types/index.ts
- src/lib/tauri.ts
- src/features/folders/folders-page.tsx
- src/features/search/search-page.tsx
- src/features/indexing/indexing-page.tsx
- CURRENT_STATUS.md

Database migrations:
- v1 (folders, screenshots table schema verified sufficient, no migration needed)

Tests run:
- npm run typecheck (PASS)
- npm run build (PASS)
- cargo check (BLOCKED - missing MSVC link.exe)
- cargo test (BLOCKED - missing MSVC link.exe)
- npm run tauri dev (BLOCKED - missing MSVC link.exe)

Known failures:
- MSVC linker link.exe not found on host machine

Current blocker:
- Need Microsoft Visual Studio C++ Build Tools installed with "Desktop development with C++"

Exact next task:
- Install MSVC C++ Build Tools, run cargo test / cargo check / tauri dev, then proceed to Phase 1C (OCR Pipeline)
```
