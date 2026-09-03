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

## 2. Phase 1 — Keyword Search

Technology:

- SQLite FTS5
- BM25 ranking

Searchable text:

- filename
- OCR text

Optional future searchable fields:

- folder name
- manually assigned tags
- AI-generated tags
- AI-generated caption

---

## 3. Query Handling

Do not over-transform technical queries.

For example:

```text
P2028
ERR_CONNECTION_REFUSED
npm ERESOLVE
```

must remain searchable as technical tokens where FTS tokenization permits.

Query normalization may include:

- trim
- collapse whitespace
- safe handling of quotes/operators
- Unicode normalization

Avoid aggressive stemming or synonym rewriting until measured.

---

## 4. Ranking

Phase 1 baseline:

```text
FTS BM25
```

Potential metadata boosts:

- filename exact match
- exact technical identifier match
- recency

Future conceptual hybrid score:

```text
final_score =
    keyword_weight * keyword_score
  + semantic_weight * semantic_score
  + filename_weight * filename_score
  + recency_weight * recency_score
```

Weights must be tuned using test queries.

Do not hardcode arbitrary weights and treat them as final.

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
