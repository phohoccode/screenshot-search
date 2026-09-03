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
