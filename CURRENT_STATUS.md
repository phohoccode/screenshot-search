# Screenshot Search — Current Status

> Keep this file short and current.  
> Update it at the end of each meaningful implementation phase.

## Last Updated

2026-09-03

## Current Phase

**Phase 1A — Project Bootstrap**  
**Status:** Implementation complete, environment validation blocked

Source code and configuration for Phase 1A are fully implemented. Frontend build validation is complete. Rust and Tauri compilation/runtime validation is currently blocked by a host environment prerequisite (missing MSVC `link.exe`).

---

## Validation Summary

- `npm run typecheck` → **PASS** (0 TypeScript errors in `src/`)
- `npm run build` → **PASS** (Vite v6 production bundle built successfully)
- `cargo check` → **BLOCKED** (Missing MSVC C++ Build Tools: `link.exe` not found for `stable-x86_64-pc-windows-msvc`)
- `npm run tauri dev` → **BLOCKED** (Blocked by Rust compilation prerequisite)

---

## Completed

- Product concept defined.
- Local-first architecture selected.
- Initial stack configured:
  - Tauri 2
  - React 19
  - TypeScript
  - Vite 6
  - Tailwind CSS v4
  - Rust
  - SQLite (rusqlite bundled)
  - SQLite FTS5 (foundation schema ready)
- Privacy-first constraints defined.
- Minimal SaaS / shadcn-style UI direction defined and documented in `docs/UI_DESIGN_SYSTEM.md`.
- Shared UI primitives implemented: `button`, `input`, `dialog`, `alert-dialog`, `progress`, `dropdown-menu`, `tooltip`, `skeleton`.
- Application shell and sidebar navigation implemented with dark/light theme support.
- Rust core layer module boundaries defined (`commands`, `db`, `errors`).
- SQLite integration with WAL mode and pragmas implemented.
- Database migration runner and initial v1 schema implemented (`folders`, `screenshots`).
- Shared typed error handling (`AppError`, `ErrorCode`, `CommandResult`) established.
- Frontend build validated (`@types/node` installed, `npm run build` passing).

---

## In Progress

None (Phase 1A implementation complete; waiting for MSVC build tools environment setup to run Tauri/Rust validation).

---

## Environment Blocker & Resolution

- **Target:** `stable-x86_64-pc-windows-msvc`
- **Missing component:** Microsoft C++ Build Tools linker (`link.exe`).
- **Required user action:** Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) and select the **Desktop development with C++** workload.

---

## Next Recommended Work

### Phase 1B — Folder and File Discovery

1. Folder picker.
2. Persist watched folders.
3. Scan images recursively or according to selected configuration.
4. Validate supported extensions.
5. Read file metadata.
6. Compute stable file identity/hash.
7. Insert discovered screenshots into SQLite.

### Phase 1C — OCR

1. Select initial OCR implementation.
2. Build OCR service abstraction.
3. Execute OCR outside the UI thread.
4. Normalize OCR text.
5. Persist successful OCR result.
6. Persist failure information separately.
7. Add retry-safe behavior.

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
- Phase 1A implementation and frontend build validation

Files changed:
- package.json
- package-lock.json
- CURRENT_STATUS.md

Database migrations:
- v1 (folders, screenshots table schema defined)

Tests run:
- npm run typecheck (PASS)
- npm run build (PASS)
- cargo check (BLOCKED - missing link.exe)
- npm run tauri dev (BLOCKED - missing link.exe)

Known failures:
- MSVC linker link.exe not found on host machine

Current blocker:
- Need Microsoft Visual Studio C++ Build Tools installed with "Desktop development with C++"

Exact next task:
- Install MSVC C++ Build Tools, verify cargo check / tauri dev, then proceed to Phase 1B (Folder and File Discovery)
```
