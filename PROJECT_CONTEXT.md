# Screenshot Search — Project Context

## 1. Project Overview

**Project name:** Screenshot Search

Screenshot Search is a local-first desktop application that allows users to search screenshots by the content inside the images instead of relying on filenames.

The product should behave like a lightweight "Google Search for screenshots on your computer."

Primary use cases:

- Search screenshots by OCR text.
- Find screenshots without remembering filenames.
- Search code errors, UI text, URLs, messages, documents, dashboards, and terminal output captured in screenshots.
- Open the original screenshot or reveal it in the operating system file explorer.
- Later support natural-language and visual semantic search.

Example:

User searches:

> prisma transaction timeout

The app can return a screenshot containing:

> PrismaClientKnownRequestError  
> Transaction API error  
> Transaction already closed  
> timeout 5000ms

Future semantic search may also allow queries such as:

> lỗi database tôi gặp hôm qua

even when those exact words do not exist in the screenshot.

---

## 2. Product Principles

The project follows these principles:

1. **Local-first**
   - Screenshots should stay on the user's device.
   - Core search must work without an internet connection.

2. **Privacy-first**
   - OCR should run locally.
   - AI inference should run locally by default.
   - Screenshot contents and OCR text must not be sent to external services without explicit user opt-in.

3. **Fast**
   - Search should feel instant.
   - Large screenshot collections must not freeze the UI.
   - Full-resolution images should not be loaded unnecessarily.

4. **Incremental**
   - Existing screenshots are indexed once.
   - New screenshots are indexed automatically.
   - Unchanged files must not be OCR'd repeatedly.

5. **AI is an enhancement, not a dependency**
   - Phase 1 must work without cloud AI.
   - Keyword search should be stable before semantic search is introduced.

6. **Windows-first**
   - Initial production target is Windows.
   - Architecture should avoid unnecessary Windows-only coupling where possible so macOS/Linux support can be added later.

---

## 3. Initial Technology Stack

### Frontend

- React
- TypeScript
- Vite
- Tailwind CSS
- shadcn-style component architecture
- Radix-style accessible primitives where appropriate
- lucide-react as the preferred icon system

Frontend/UI work must follow `docs/UI_DESIGN_SYSTEM.md`.

### Desktop Runtime

- Tauri 2

### Native/Core Layer

- Rust

Responsibilities include:

- filesystem access
- folder scanning
- file watching
- hashing
- OCR orchestration
- thumbnail generation
- search orchestration
- SQLite access
- background indexing

### Database

- SQLite
- SQLite FTS5

### OCR

Initial OCR implementation must remain replaceable.

Possible implementations:

- Windows native OCR for Windows-only builds
- PaddleOCR / ONNX for cross-platform local OCR
- Tesseract only if useful for prototyping

The final OCR engine must be selected based on measured accuracy, performance, binary size, and deployment complexity.

### Local AI — Future Phase

- ONNX Runtime or equivalent local inference runtime
- multilingual text embedding model
- optional visual/image embedding model

Cloud LLM APIs are not required for the core product.

---

## 4. High-Level Architecture

```text
Screenshot Folder
       |
       v
Scanner / Watcher
       |
       v
File Validation
       |
       v
Hash / Change Detection
       |
       +------ unchanged ------> Skip
       |
       v
Index Queue
       |
       +----------------+
       |                |
       v                v
      OCR          Thumbnail
       |                |
       v                |
Text Normalize          |
       |                |
       +-------+--------+
               |
               v
             SQLite
       +----------------+
       | metadata       |
       | OCR text       |
       | FTS5 index     |
       +----------------+
               |
               v
             Search
               |
               v
              UI
```

Future semantic pipeline:

```text
OCR Text
   |
   v
Text Embedding
   |
   v
Vector Index

Screenshot
   |
   v
Visual Encoder
   |
   v
Image Embedding
```

---

## 5. Core Features

### Phase 1 — Core MVP

Required:

- choose screenshot folders
- initial folder scan
- discover supported image files
- OCR screenshots
- store metadata
- SQLite persistence
- FTS5 keyword search
- search result grid
- thumbnail display
- screenshot preview
- open original file
- reveal file in Explorer
- index progress
- basic error handling

### Phase 2 — Automatic Indexing

- filesystem watcher
- automatic indexing of new screenshots
- update detection
- deletion handling
- background queue
- pause/resume/cancel indexing
- thumbnail cache
- duplicate detection
- filters

### Phase 3 — Semantic Search

- local text embeddings
- vector storage
- natural-language search
- hybrid ranking
- semantic result explanations if useful

### Phase 4 — Visual Search

- image embeddings
- image-to-text or text-to-image retrieval
- auto classification
- optional auto tags

### Phase 5 — Productization

- onboarding
- settings
- auto update
- crash reporting that does not leak screenshot contents
- license / subscription if needed
- model download manager
- import/export index
- optional cloud features only with explicit opt-in

---

## 6. Supported Files

Initial supported formats:

- PNG
- JPG
- JPEG
- WEBP

Possible later additions:

- HEIC
- AVIF
- GIF first frame

Do not expand supported formats without considering:

- decoder availability
- security
- performance
- test coverage

---

## 7. Important Invariants

### Indexing

- A file must not be OCR'd again if its relevant content hash has not changed.
- Deleted files must not remain visible in search results.
- Failed OCR jobs must not mark a screenshot as successfully indexed.
- Indexing must be resumable.
- Large imports must not block the UI thread.

### Search

- Phase 1 search source of truth is SQLite FTS5.
- AI semantic search must not replace exact keyword search.
- Exact error codes, filenames, and technical identifiers must rank highly.

### Privacy

- Screenshot contents must not be uploaded by default.
- OCR text must not be sent to telemetry.
- Logs must not contain full OCR output.
- Secrets detected in screenshots must not be persisted outside the local database unless explicitly designed.

### Filesystem

- Never delete or modify the user's original screenshot unless the user explicitly requests such a feature.
- Use the original path as a reference only.
- Internal thumbnails and indexes may be deleted/rebuilt safely.

---

## 8. Suggested Repository Structure

```text
screenshot-search/
|
├── PROJECT_CONTEXT.md
├── CURRENT_STATUS.md
├── ARCHITECTURE.md
|
├── docs/
│   ├── DATABASE.md
│   ├── SEARCH_CONTEXT.md
│   ├── AI_CONTEXT.md
│   ├── SECURITY_PRIVACY.md
│   ├── UI_DESIGN_SYSTEM.md
│   ├── CODING_RULES.md
│   └── DECISIONS.md
|
├── src/
│   ├── app/
│   ├── components/
│   ├── features/
│   ├── hooks/
│   ├── lib/
│   ├── pages/
│   └── types/
|
├── src-tauri/
│   ├── src/
│   │   ├── commands/
│   │   ├── db/
│   │   ├── indexing/
│   │   ├── ocr/
│   │   ├── search/
│   │   ├── thumbnails/
│   │   └── watcher/
│   └── migrations/
|
└── README.md
```

The real repository structure is the source of truth. Keep this section updated when the structure materially changes.

---

## 9. Development Rules

Before implementing a feature:

1. Read `PROJECT_CONTEXT.md`.
2. Read `CURRENT_STATUS.md`.
3. Read `docs/CODING_RULES.md`.
4. For any frontend/UI task, read `docs/UI_DESIGN_SYSTEM.md`.
5. Read only the other domain-specific context relevant to the task.
6. Inspect the actual source code.
7. Inspect `git status` and relevant `git diff`.
8. Do not rely only on context documentation when code disagrees with it.

Context files explain intent and invariants.

**Actual code + migrations + tests remain the implementation source of truth.**

---

## 10. Context Update Policy

Update this file only when there is a meaningful change to:

- product scope
- architecture
- technology stack
- major invariants
- repository organization
- privacy model
- core data flow

Do not use this file as a changelog.

Use `CURRENT_STATUS.md` for active progress.
Use `docs/DECISIONS.md` for architectural decisions.
