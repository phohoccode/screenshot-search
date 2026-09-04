# Screenshot Search — Current Status

> Keep this file short and current. Update it at the end of each meaningful implementation phase.

## Last Updated

2026-09-04

## Current Phase

**Phase 3.5E — Real-World Vietnamese OCR Quality Audit & Optimization**

**Status:** COMPLETED — REAL-WORLD VIETNAMESE OCR QUALITY APPROVED

The Phase 3.5D hybrid architecture remains intact: DBNet detects text lines, Windows Media OCR probes every crop, a deterministic classifier protects technical text, VietOCR handles natural Vietnamese, and results are merged in reading order and normalized to NFC.

Phase 3.5E fixes a real-world routing failure without dictionaries, filename mappings, expected-text substitution, cloud OCR, or another model:

- Windows `en-US` can emit Vietnamese text with structurally invalid Latin combining marks such as diaeresis or ring-above substitutions. The classifier now treats this general Unicode corruption pattern as Natural only after strong technical syntax checks have run.
- Paths, URLs, code/error identifiers, and other strong technical structures still take precedence. A 22-phrase uppercase technical holdout produced 0 Natural classifications.
- The production hybrid pipeline version is now `hybrid_windows_vietocr:hybrid_v2`. Re-OCR eligibility, statistics, persisted OCR metadata, and the legacy batch path all use the actual engine/version instead of the stale `multilingual_ocr:ppocr_v4` label.
- Durable Hybrid re-OCR jobs now wait and retry while the model is loading after app startup instead of silently completing through the temporary Windows fallback.
- Screenshot (11) was reprocessed through the durable production queue. SQLite stores the new Hybrid result and FTS5 was refreshed atomically; all 15 local screenshots have current `hybrid_v2` metadata and FTS health is 15/15.

## Quality Summary

- Unchanged 30-fixture Vietnamese benchmark, final run: **7.00% CER**, **21.44% WER**, **4.06% median CER** (Phase 3.5D baseline: 10.07% CER; repeated Windows OCR runs varied from 6.35% to 7.00% CER).
- Screenshot (11): the critical Vietnamese headings and body text are materially readable; remaining errors are limited to small brand/icon/button/date glyph artifacts.
- Classifier independent holdout: **100% technical recall**, **98.04% technical precision**, **98% natural recall**, **100% natural precision**.
- Uppercase technical prose safety: **22/22** stayed Technical or Uncertain; **0/22** routed to Natural.
- Technical OCR regression: unchanged at **82.69% exact / 90.38% search-normalized** on the 52-token set and **62.75% exact / 83.33% search-normalized** on the independent 102-token set. Phase 3.5D's known technical OCR quality gate remains historical and was not re-scoped.
- Memory: **0 MB incremental model memory**; the existing measured OCR stack remains approximately **172.73 MB**.

## Validation Summary

- `cargo fmt --check --manifest-path ./src-tauri/Cargo.toml` → **PASS**
- `cargo check --manifest-path ./src-tauri/Cargo.toml` → **PASS**
- `cargo test --manifest-path ./src-tauri/Cargo.toml` → **PASS** (**134 passed**, 0 failed)
- `npm run typecheck` → **PASS**
- `npm run build` → **PASS**
- `npm run tauri dev` → **PASS** (desktop process, schema v5, Hybrid model, watcher, and worker all started successfully)
- `npm run lint` → **N/A** (no lint script)

## Blockers

None for Phase 3.5E. Phase 4 has not been started.

## Working Tree Policy

No commit or push was made for Phase 3.5E. Private screenshots, local crops, model files, database files, and benchmark output remain untracked/ignored and must not be committed.
