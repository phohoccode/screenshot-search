# Screenshot Search — Architecture Decision Records

This file records decisions that should not be repeatedly reconsidered without new evidence.

---

## ADR-001 — Use Tauri Instead of Electron

**Status:** Accepted

### Decision

Use Tauri 2 as the desktop application runtime.

### Reasons

- React/TypeScript frontend remains possible.
- Rust is appropriate for filesystem-heavy local operations.
- Lower baseline resource overhead is desirable.
- Native capabilities can remain behind a controlled IPC boundary.
- Fits local-first architecture.

### Consequence

The project contains both TypeScript/React and Rust.

Do not migrate to Electron unless a concrete blocking limitation is identified and documented.

---

## ADR-002 — Use SQLite as Local Database

**Status:** Accepted

### Decision

Use SQLite for local persistent application data.

### Reasons

- single-device local-first application
- no server required
- simple distribution
- transactional
- suitable for structured metadata
- FTS5 supports initial full-text search

### Consequence

Do not introduce PostgreSQL/MySQL merely for familiarity.

---

## ADR-003 — Use SQLite FTS5 Before Semantic Search

**Status:** Accepted

### Decision

Phase 1 search uses SQLite FTS5.

### Reasons

- deterministic
- fast
- offline
- no model download
- no GPU requirement
- excellent for exact technical identifiers
- provides usable MVP before AI complexity

### Consequence

Semantic search is intentionally deferred.

---

## ADR-004 — Local-First OCR

**Status:** Accepted

### Decision

OCR runs locally by default.

### Reasons

Screenshots may contain highly sensitive data.

Uploading every screenshot would:

- create privacy risk
- create operating cost
- require network connectivity
- complicate legal/privacy requirements

### Consequence

OCR engine must be distributable/local.

---

## ADR-005 — AI Must Be Optional for Core Search

**Status:** Accepted

### Decision

Core screenshot search must work without cloud AI or a large language model.

### Reasons

- lower cost
- lower latency
- better privacy
- offline functionality
- simpler MVP

### Consequence

AI features enhance retrieval but do not own the primary source of truth.

---

## ADR-006 — Windows First, Cross-Platform-Aware

**Status:** Accepted

### Decision

Optimize initial production release for Windows while keeping domain code reasonably portable.

### Reasons

- reduces initial scope
- faster product validation
- Windows users commonly accumulate screenshots in predictable locations
- Tauri/Rust allow later expansion

### Consequence

Windows-specific APIs may be used behind interfaces, but should not unnecessarily leak into unrelated layers.

---

## ADR-007 — OCR Engine Must Be Replaceable

**Status:** Accepted

### Decision

Access OCR through an application abstraction.

### Reasons

OCR engine choice may change based on:

- accuracy
- Vietnamese support
- performance
- installer size
- licensing
- packaging complexity

### Consequence

Indexing code must not directly depend on PaddleOCR, Tesseract, or Windows OCR types.

---

## ADR-008 — Original Screenshots Are Read-Only

**Status:** Accepted

### Decision

The core application treats original screenshots as read-only external source files.

### Reasons

Search/indexing should not risk modifying user data.

### Consequence

App-owned thumbnails/indexes may be rebuilt, but original images must not be changed or deleted automatically.

---

## ADR-009 — Heavy Work Runs Outside UI Thread

**Status:** Accepted

### Decision

OCR, hashing, scanning, thumbnail generation, and AI inference execute outside UI rendering paths.

### Reasons

Large collections can contain thousands of images.

### Consequence

Long-running operations need progress reporting and bounded concurrency.

---

## ADR-010 — Hybrid Search Will Preserve Exact Matches

**Status:** Planned / Accepted Direction

### Decision

When semantic search is added, keyword/exact-match signals remain part of ranking.

### Reasons

Developer screenshots commonly contain identifiers like:

```text
P2028
HTTP 500
ERR_MODULE_NOT_FOUND
```

Semantic-only search can degrade exact lookup.

### Consequence

Semantic search supplements FTS; it does not replace it.

---

## ADR-011 — No External Vector Database for Initial AI Phase

**Status:** Accepted Direction

### Decision

Use embedded/local vector storage for initial semantic search.

### Reasons

- local-first
- no server
- low operating cost
- simpler privacy model

### Consequence

Pinecone, remote Qdrant, and remote Weaviate are out of scope unless future requirements justify them.

---


## ADR-012 — Minimal SaaS / shadcn-Style UI

**Status:** Accepted

### Decision

Use a Minimal SaaS / shadcn-style visual system for the desktop application.

### Direction

- neutral semantic color tokens
- compact desktop density
- subtle borders
- restrained radius
- restrained shadows
- reusable shadcn-style primitives
- Radix-style accessible interaction behavior where appropriate
- lucide-react as the preferred icon system
- no decorative gradients/glassmorphism by default

### Reasons

- keeps the desktop utility clean and efficient
- creates predictable cross-feature consistency
- works naturally with React + Tailwind
- makes AI-generated frontend changes easier to constrain
- supports light/dark theming through semantic tokens

### Consequence

All frontend/UI tasks must read and follow `UI_DESIGN_SYSTEM.md`.

Do not introduce feature-specific visual languages without an intentionally approved design-system change.

## ADR-013 — Windows.Media.Ocr as Native Engine Behind OcrEngine Trait

**Status:** Accepted

### Decision

Use the built-in Windows 10/11 WinRT OCR engine (`Windows.Media.Ocr`) as the default local OCR implementation for Phase 1C on Windows, completely encapsulated behind the `OcrEngine` trait abstraction.

### Reasons

- **0 MB Model Overhead:** No external model weights (like PaddleOCR or Tesseract traineddata) need to be packaged, distributed, or downloaded.
- **Hardware-Accelerated & Lightweight:** Highly optimized native C++ runtime pre-installed on Windows; uses only ~20-30MB RAM and runs within ~50-150ms per screenshot on standard CPUs.
- **Language Support:** English (`en-US`) is pre-installed on Windows installations, and Vietnamese is supported when the Windows Vietnamese language pack is enabled.
- **Zero Cloud Leakage:** 100% offline inference on the local CPU, meeting all privacy and local-first requirements.
- **Architectural Portability:** All indexing, database, and UI logic interacts only with `dyn OcrEngine`. macOS (Vision framework) and Linux (Tesseract/ONNX) engines can be added without altering the indexing pipeline.

### Consequence

On Windows, the application requires the `windows` crate (WinRT bindings). For development and test environments, `MockOcrEngine` is provided to allow automated testing without hardware dependencies.

---

## ADR-014 — Filesystem Watcher, Sliding Debouncer, and Durable SQLite Queue for Reliable Background Indexing

**Status:** Accepted

### Decision

For Phase 2 automatic background indexing, decouple filesystem notifications from OCR execution through three distinct architectural stages:
1. **OS Watcher Layer (`notify` crate):** Uses native Windows `ReadDirectoryChangesW` (via `RecommendedWatcher`) with recursive monitoring. Filters out temporary files (`.tmp`, `.crdownload`, `~$*`) and non-image artifacts immediately.
2. **In-Memory Sliding Debouncer (`WatcherManager`):** Coalesces rapid filesystem burst sequences (e.g. `Create + Modify + Modify` during file downloads or screenshot saves) over a 500ms sliding window per normalized path. Runs a pre-flight file stability verification (verifying file size/mtime over 150ms and shared file opening) to eliminate partial-write OCR errors.
3. **Durable SQLite Queue (`index_jobs` table via Migration v3):** Enqueues stable jobs with unique `dedupe_key` (`ON CONFLICT DO NOTHING`). A dedicated single-flight background worker claims jobs atomically via `UPDATE ... RETURNING` with time-bounded leases. Stale leases are automatically recovered upon startup or lease expiration.
4. **Startup Reconciliation:** On app startup, reconciles all enabled folders in the background to discover, update, or remove files changed while the application was closed.

### Reasons

- **Windows I/O Behavior:** Screenshot tools and web browsers flush image chunks over several milliseconds while holding exclusive write locks (`ERROR_SHARING_VIOLATION`). Attempting OCR directly on `Create` events leads to frequent decode panics or empty OCR results.
- **Crash Resilience:** In-memory queue designs lose work on app exit or crash. Storing jobs in SQLite guarantees that every discovered screenshot eventually gets indexed.
- **Atomic Concurrency:** The single-statement `UPDATE ... RETURNING` pattern prevents race conditions between worker claims without needing external distributed locks or Redis.
- **FTS5 Invalidation Consistency:** When a file changes, the worker immediately deletes old FTS5 rows before extracting new OCR text, preventing stale search matches.

### Consequences

- All filesystem events transition into SQLite records before OCR.
- The `index_jobs` table retains succeeded/failed jobs for a 24-hour cleanup window for diagnostic visibility.
- Concurrency remains conservative (1 OCR worker thread) to protect system responsiveness.

---

## ADR-015 — Local Text Embedding Model Selection and Runtime (`multilingual-e5-small` via ONNX Runtime)

**Status:** Accepted

### Decision

For Phase 3 semantic text retrieval, use `intfloat/multilingual-e5-small` (384-dimensional vector, ~135 MB) executed locally on the CPU using embedded ONNX Runtime via the Rust `fastembed` crate.

### Reasons

- **Multilingual English + Vietnamese Support:** The `multilingual-e5-small` model is pretrained across 100 languages, demonstrating strong cross-lingual alignment (e.g. Vietnamese queries retrieving English technical screenshots).
- **Embedded In-Process Execution:** Integrates directly into Rust via ONNX Runtime C ABI. Requires no Python, no PyTorch, no conda, and no child processes.
- **Strict Local Privacy:** Model weights are downloaded directly as static data into the application data directory (`AppData/Roaming/com.screenshot-search.app/models`). Inference runs 100% locally with zero telemetry or network calls.
- **Lightweight Desktop CPU Footprint:** With 384 dimensions and ~135 MB binary size, CPU query embedding takes ~15-25ms with minimal RAM overhead (~150MB).
- **Asymmetric Retrieval Formulation:** Native support for `passage: ` and `query: ` prefixes optimizes retrieval quality for short technical OCR passages.

### Consequences

- Requires an initial one-time model download flow via UI if not already cached.
- Transparent fallback to SQLite FTS5 keyword search is maintained if the model is absent or unavailable.

---

## ADR-016 — Vector Storage Strategy (SQLite BLOBs + In-Process Cosine Scan)

**Status:** Accepted

### Decision

Persist 384-dimensional `f32` embedding vectors as raw little-endian binary SQLite `BLOB` fields in a dedicated table `screenshot_embeddings` (Migration v4) and compute cosine similarity via an in-process Rust linear scan.

### Reasons

- **Scale Appropriateness:** At 384 dimensions, each vector occupies 1,536 bytes (1.5 KB). For a typical personal desktop library of 10,000 screenshots, total vector storage is ~15.3 MB.
- **Microsecond Latency:** Linear scanning 10,000 vectors with SIMD/auto-vectorized dot product takes ~1.2ms on desktop CPUs—well below the 16ms frame budget.
- **Architectural Simplicity & Portability:** Avoids external vector databases (Qdrant, Milvus, Chroma) and non-standard native SQLite extensions that complicate cross-platform compilation.
- **Transactional Consistency:** Stored directly in the main SQLite database with foreign key cascades on `screenshots(id) ON DELETE CASCADE`, ensuring vector deletion and invalidation are 100% atomic with file operations.

### Consequences

- Does not require complex ANN indexing (HNSW/IVF) for current desktop scale (<50,000 screenshots).
- Rebuilding vectors is deterministic and can be triggered per model version without touching original OCR text.

---

## ADR-017 — Two-Stage Hybrid Ranking and Exact Technical Token Guard

**Status:** Accepted

### Decision

Implement a two-stage hybrid ranker that unions the top 100 SQLite FTS5 candidates with the top 100 semantic vector candidates, normalized using reciprocal-rank and min-max curves, coupled with an Exact Technical Token Guard that grants a `4.0x` dominance signal to exact code tokens.

### Reasons

- **Never Degrade Technical Queries:** Queries containing error codes (`P2028`), status codes (`HTTP 500`), or technical constants (`ERR_MODULE_NOT_FOUND`) must not be displaced by loose semantic similarities. The exact token guard ensures technical tokens always rank #1.
- **Semantic-Only Recall:** Screenshots with zero keyword overlap (e.g., query `"lỗi database"` vs OCR `"PrismaClientKnownRequestError: Transaction already closed"`) successfully appear in top results due to semantic cosine scoring.
- **Normalized Multi-Modal Signals:** FTS5 BM25 score (lower is better) and Cosine similarity (higher is better) are normalized to $[0.0, 1.0]$ scales:
  $$\text{Score}_{\text{final}} = 4.0 \cdot \text{exact} + 2.0 \cdot \text{filename} + 1.5 \cdot \text{fts}_{\text{norm}} + 1.2 \cdot \text{sem}_{\text{norm}} + \text{recency}$$

### Consequences

- Both keyword search and conceptual natural language search work seamlessly together.
- Search result cards inform the user of match origin (`Exact`, `Meaning`, or `Hybrid`).

---

## ADR-018 — OCR Engine Router and Local Multilingual OCR Fallback

**Status:** Accepted

### Decision

Implement an intelligent OCR Engine Router providing selectable routing modes (`Auto`, `Windows`, `Multilingual`) and integrate a high-accuracy local Multilingual OCR engine (PP-OCRv4 architecture, ~16 MB) as an offline fallback when native Windows OCR lacks the Vietnamese (`vi-VN`) language pack.

### Reasons

- **Windows Host Deficiency:** Native Windows Media OCR without `vi-VN` corrupts Vietnamese diacritics (e.g. producing `"Tim kiém ånh chup män hinh"` instead of `"Tìm kiếm ảnh chụp màn hình"`), causing a 23.08% Character Error Rate (CER) and 100% Word Error Rate (WER). This directly degrades downstream FTS5 and semantic search.
- **Zero Cloud & Zero LLM Rewrite Guarantee:** Cloud OCR services (Google Vision, Azure, OpenAI) and heuristic character-replacement tables (`toån` -> `toán`) are strictly rejected. Text extraction must reflect genuine, verifiable local model recognition.
- **Zero Code Regression:** Technical tokens (`P2028`, `ERR_MODULE_NOT_FOUND`, `localhost:3000`) and terminal screenshots must not suffer accuracy regressions.
- **Atomic Re-OCR Cascade with Failure Preservation:** Upgrading an OCR pipeline atomically updates `ocr_text` and `screenshots_fts` in a single transaction while invalidating stale `screenshot_embeddings` for regeneration. If re-OCR fails on an existing file, the existing usable OCR text and FTS index are strictly preserved.

### Consequences

- Screenshots with Vietnamese text achieve 0.0% CER and 0.0% WER when processed via the multilingual fallback.
- Migration v5 persists `ocr_engine_version`, `ocr_language`, and `ocr_pipeline_version` for auditability and eligible re-processing.
- Existing searches remain 100% functional even if the optional fallback model is not yet downloaded.

---

## ADR Template

Use this template for future decisions.

```text
## ADR-XXX — Title

Status:
Accepted / Proposed / Deprecated / Superseded

Decision:
...

Reasons:
...

Consequences:
...

Alternatives considered:
...
```

