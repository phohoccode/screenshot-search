# Screenshot Search — Coding Rules

## 1. General Principles

Code should prioritize:

1. correctness
2. privacy
3. maintainability
4. responsiveness
5. observability without leaking sensitive data
6. performance after correctness

Do not prematurely optimize at the cost of architecture clarity.

---

## 2. Before Editing

For every non-trivial task:

1. Read `PROJECT_CONTEXT.md`.
2. Read `CURRENT_STATUS.md`.
3. Read this file.
4. For any frontend/UI task, read `UI_DESIGN_SYSTEM.md`.
5. Read other relevant domain context.
6. Inspect actual code.
7. Inspect relevant tests.
8. Run `git status`.
9. Inspect `git diff` before finishing.

Never modify code based only on context documentation.

---

## 3. TypeScript

Requirements:

- TypeScript strict mode
- avoid `any`
- prefer explicit domain types
- use discriminated unions for state where appropriate
- validate data crossing Tauri IPC boundaries
- avoid unsafe type assertions unless justified

Example:

Prefer:

```ts
type IndexState =
  | { status: "idle" }
  | { status: "indexing"; completed: number; total: number }
  | { status: "failed"; message: string };
```

over loosely coupled booleans.

---

## 4. React

Use:

- functional components
- focused components
- custom hooks for reusable UI state
- domain services outside visual components where appropriate

Avoid:

- huge page components
- business logic hidden in JSX
- UI that conflicts with `UI_DESIGN_SYSTEM.md`
- duplicate custom primitives when a shared shadcn-style component exists
- inconsistent icon libraries
- unnecessary `useEffect`
- duplicated loading/error state patterns
- storing backend source-of-truth state only in component memory

Expensive work must not run in React.


### UI Design

All frontend changes must follow `UI_DESIGN_SYSTEM.md`.

Default visual direction:

- Minimal SaaS
- shadcn-style
- neutral surfaces
- compact desktop density
- subtle borders
- restrained shadows
- one consistent icon system

Do not introduce a new visual language for an isolated feature.

---

## 5. Rust

Rules:

- avoid `unwrap()` and `expect()` on production paths unless an invariant is truly impossible to violate and documented
- use typed/domain errors
- propagate recoverable errors cleanly
- validate filesystem input
- keep Tauri commands thin
- isolate blocking CPU work
- bound concurrency
- avoid global mutable state where possible

Do not panic for ordinary user/file errors.

---

## 6. Tauri IPC

IPC commands should:

- have stable inputs/outputs
- validate frontend data
- return structured errors
- avoid exposing unrestricted filesystem primitives

Frontend should request domain operations such as:

```text
select_folder
search_screenshots
open_screenshot
reveal_screenshot
start_indexing
pause_indexing
```

rather than arbitrary native access.

---

## 7. SQLite

Rules:

- migrations are source of truth
- keep transactions short
- no OCR inside DB transaction
- no AI inference inside DB transaction
- no large filesystem scan inside DB transaction
- always use parameterized queries
- update FTS consistently
- think about restart/recovery

---

## 8. Indexing

Indexing must be:

- idempotent
- incremental
- retry-safe
- cancellable or safely stoppable
- bounded in concurrency

Do not OCR unchanged files.

Do not create one unbounded async task per screenshot.

---

## 9. OCR

OCR engine must be behind an abstraction.

Do not scatter OCR-provider-specific logic through indexing code.

OCR failures must not crash the app.

Normalize output before indexing.

Do not log OCR contents.

---

## 10. Search

Rules:

- preserve exact technical tokens
- keyword search remains available after semantic search is introduced
- paginate/limit results
- do not load full-resolution images for every result
- ranking changes require evaluation against test queries

---

## 11. Images

For search result grids:

- use thumbnails
- lazy load
- avoid decoding original 4K images unnecessarily

Original images should be loaded only for explicit preview/open actions where practical.

---

## 12. Performance

Never block UI on:

- initial folder scan
- OCR
- hashing
- thumbnail generation
- embedding generation
- vector indexing

Add progress events/state for long-running operations.

Measure before tuning concurrency.

---

## 13. Error Handling

Use stable error categories.

Example:

```text
FILE_PERMISSION_DENIED
OCR_FAILED
DATABASE_FAILED
INDEX_JOB_FAILED
MODEL_LOAD_FAILED
```

Frontend messages should be user-friendly.

Internal diagnostics may contain technical details but not sensitive content.

---

## 14. Logging

Use structured logs.

Never log:

- screenshot bytes
- OCR text
- secret values
- auth tokens
- sensitive search queries by default

Avoid unnecessary full filesystem paths.

---

## 15. Security

Before adding a new native capability:

- confirm it is required
- minimize permission scope
- validate inputs
- consider malicious filenames and paths

Do not add arbitrary shell execution unless absolutely required and security-reviewed.

---

## 16. Testing

Core logic should have tests.

Prioritize:

- file identity/change detection
- OCR text normalization
- query normalization
- FTS synchronization
- ranking
- deletion handling
- indexing idempotency
- retry behavior
- migration behavior

Bug fixes should include regression tests where practical.

---

## 17. Refactoring

Do not perform unrelated broad refactors while implementing a focused feature unless necessary.

If a blocker requires refactoring:

- document why
- keep scope controlled
- preserve behavior
- run relevant tests

---

## 18. Dependencies

Before adding a dependency:

- check whether current stack already solves the problem
- prefer maintained libraries
- inspect license
- consider binary size
- consider native build complexity
- consider security
- consider cross-platform impact

Do not add an AI framework just to call one ONNX model.

---

## 19. Context Files

Do not copy large source files into context docs.

Context docs should capture:

- intent
- architecture
- invariants
- decisions
- current status

Actual code remains source of truth.

---

## 20. Completion Checklist

Before reporting a task complete:

- implementation finished
- relevant tests run
- build/typecheck run when practical
- migrations reviewed
- privacy impact reviewed
- `git diff` reviewed
- no accidental unrelated changes
- `CURRENT_STATUS.md` updated when progress materially changed
- architectural docs updated if architecture changed
