# Screenshot Search — Search Context

## 1. Search Product Goal

Search should help users find screenshots based on what they remember, not on how the file was named.

Examples:

```text
prisma timeout
```

```text
meeting password
```

```text
ảnh terminal bị lỗi database
```

The first implementation supports keyword-based retrieval.

Natural-language semantic retrieval is a later phase.

---

## 2. Phase 1 — Keyword Search (Implemented in Phase 1D)

Technology:
- SQLite FTS5 (`screenshots_fts`)
- BM25 ranking (`bm25(screenshots_fts, 5.0, 1.0)`)
- Custom OCR-tolerant normalization pipeline

Searchable text:
- `filename`: weighted 5.0x for exact/prefix filename relevance.
- `ocr_search_text`: normalized OCR content weighted 1.0x.

---

## 3. Query & Text Normalization Pipeline

To reconcile real-world OCR variance (where punctuation like `_` in `ERR_MODULE_NOT_FOUND` may be dropped as whitespace `ERR MODULE NOT FOUND`), the system generates two representations:
- **Raw OCR text (`screenshots.ocr_text`):** Stored intact for human inspection, snippet preview, and text copying.
- **Search representation (`screenshots_fts.ocr_search_text`):**
  - Unicode NFC normalization
  - Lowercasing
  - Normalizes underscores `_`, hyphens `-`, colons `:`, slashes `/`, and noise punctuation to single spaces
  - Preserves technical identifiers (`P2028`, `HTTP 500`) and dotted formats (`v1.2.3`, `192.168.1.1`)
  - Collapses multiple whitespace into single space

The same rules are applied symmetrically to user queries via `normalize_search_query()`, ensuring that `ERR_MODULE_NOT_FOUND`, `ERR-MODULE-NOT-FOUND`, and `err module not found` all match identical index tokens.

---

## 4. Ranking & Snippets

Implemented ranking:
```sql
ORDER BY bm25(screenshots_fts, 5.0, 1.0) ASC, s.modified_at_fs DESC
```

- Filename matches receive a 5.0x relevance boost over body text.
- Recency tiebreaker: among identical relevance scores, newer screenshots rank first.
- Safe Snippets: FTS5 `snippet()` wraps matched tokens with `[[match]]...[[/match]]`. The frontend parses these markers into React text nodes, preventing any HTML injection vulnerability.

## 5. Phase 3 — Hybrid Search (Implemented)

Technology:
- Two-stage candidate retrieval: Union of top 100 SQLite FTS5 candidates + top 100 semantic vector candidates (in-process cosine scan).
- Score normalization:
  - FTS5 BM25 ($bm25 \le 0$): Normalized via reciprocal rank: $pos = -bm25$, $\text{norm} = \frac{pos}{1.0 + pos} \in [0.0, 1.0]$.
  - Semantic Cosine ($[-1.0, 1.0]$): Normalized via min-max: $\text{norm} = \frac{\text{cosine} + 1.0}{2.0} \in [0.0, 1.0]$.
- Exact Technical Token Guard:
  Detects alphanumeric error codes, HTTP codes, uppercase abbreviations, and identifiers (`P2028`, `ERR_MODULE_NOT_FOUND`, `HTTP 500`). Grants a massive `4.0x` exact signal boost when an exact token is matched, guaranteeing technical tokens dominate rank #1.
- Hybrid Ranking Formula:
  $$\text{FinalScore} = 4.0 \cdot \text{exact\_signal} + 2.0 \cdot \text{filename\_signal} + 1.5 \cdot \text{fts\_score} + 1.2 \cdot \text{semantic\_score} + \text{recency\_tiebreak}$$
- Transparent Fallback:
  If the semantic model is not downloaded or unavailable, search falls back seamlessly to SQLite FTS5 without error.

---

## 5. Exact Match Importance

Semantic search must never make exact technical lookup worse.

Example:

User searches:

```text
P2028
```

A screenshot containing exactly `P2028` should normally rank above semantically related database errors that do not contain the code.

Similar examples:

```text
ERR_MODULE_NOT_FOUND
HTTP 429
CVV
invoice_2026
```

Hybrid search must preserve this behavior.

---

## 6. Filters

Potential filters:

- date range
- folder
- file extension
- OCR status
- screenshot dimensions
- source application if later detected
- tag

Filters should be applied efficiently and should not require loading all results into memory.

---

## 7. Search Result Model

A search result may contain:

```text
screenshot_id
path
filename
thumbnail_path
created/modified time
match snippet
match source
rank score
```

Future result metadata may include:

```text
semantic score
keyword score
visual score
matched tags
```

Frontend should not need to know internal SQL details.

---

## 8. Snippets / Highlighting

When possible, show a compact text snippet around the matching OCR term.

Do not show the entire OCR text in result cards.

Possible behavior:

```text
... Transaction API error: Transaction already closed ...
```

Highlight relevant matched terms in UI.

Avoid exposing overly large sensitive text unnecessarily.

---

## 9. Semantic Search — Future Phase

Pipeline:

```text
OCR text
   |
   v
Normalization
   |
   v
Embedding model
   |
   v
Vector
   |
   v
Vector index
```

Query:

```text
natural language query
        |
        v
same embedding model
        |
        v
similarity search
```

Requirements:

- local inference by default
- multilingual model preferred
- reasonable CPU performance
- model version stored with embeddings
- embeddings regenerated when model version changes

---

## 10. Visual Search — Future Phase

Visual search should support concepts not necessarily visible as OCR text.

Examples:

```text
dashboard có biểu đồ
```

```text
ảnh vscode đang mở terminal
```

```text
màn hình thanh toán QR
```

Pipeline:

```text
Screenshot
   |
   v
Visual embedding model
   |
   v
Image vector
```

Query text must be compatible with the visual embedding space.

---

## 11. Evaluation

Maintain a small search evaluation set.

Example:

```text
Query: prisma timeout
Expected screenshot IDs:
- ...

Query: lỗi transaction database
Expected screenshot IDs:
- ...

Query: P2028
Expected screenshot IDs:
- ...
```

Track:

- top-1 relevance
- top-5 recall
- query latency
- index latency
- false positives

Do not judge search quality only by intuition.

---

## 12. Performance Targets

Initial targets should be measured, not assumed.

Suggested goals:

- query results appear interactively
- pagination/limit prevents rendering huge result sets
- search should not read original full-resolution images
- UI should remain responsive during indexing

---

## 13. Scope Guard

**Do not implement semantic search until FTS keyword search is stable, tested, and useful.**

AI is Phase 3, not a requirement for the initial MVP.
