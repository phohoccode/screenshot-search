use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};

use crate::db::embeddings::{self, cosine_similarity};
use crate::db::screenshots;
use crate::errors::AppError;
use crate::search::query::{
    build_safe_fts_query, SearchRequest, SearchResultItem, SearchResultPage,
};
use crate::semantic::TextEmbeddingEngine;

const CANDIDATE_LIMIT: usize = 100;
const SEMANTIC_MIN_THRESHOLD: f32 = 0.50;

/// Detects whether a query token represents a technical identifier, error code, or file pattern.
pub fn is_technical_or_exact_token(token: &str) -> bool {
    if token.len() < 2 {
        return false;
    }

    // All uppercase letters of length >= 2 (e.g. HTTP, API, SQL, ERR, URL)
    if token.chars().all(|c| c.is_ascii_uppercase()) && token.len() >= 2 {
        return true;
    }

    // Contains uppercase letters and digits (e.g. P2028, E11000)
    let has_upper = token.chars().any(|c| c.is_ascii_uppercase());
    let has_digit = token.chars().any(|c| c.is_ascii_digit());
    if has_upper && has_digit {
        return true;
    }

    // Underscore separated technical tokens (e.g. ERR_MODULE_NOT_FOUND, PRISMA_CLIENT)
    if token.contains('_') && (has_upper || token.chars().any(|c| c.is_ascii_alphanumeric())) {
        return true;
    }

    // Dotted or path-like technical tokens (e.g. v1.2.3, localhost:3000, .json, .png)
    if token.contains('.') || token.contains(':') || token.contains('/') || token.contains('\\') {
        return true;
    }

    // Pure 3-4 digit error codes (e.g. 500, 404, 429, 2028)
    if token.chars().all(|c| c.is_ascii_digit()) && (token.len() == 3 || token.len() == 4) {
        return true;
    }

    false
}

/// Computes the exact match signal score between query and screenshot candidate.
pub fn compute_exact_signal(raw_query: &str, filename: &str, ocr_text: &str) -> f64 {
    let q_clean = raw_query.trim();
    if q_clean.is_empty() {
        return 0.0;
    }

    let q_lower = q_clean.to_lowercase();
    let fn_lower = filename.to_lowercase();
    let ocr_lower = ocr_text.to_lowercase();

    // 1. Exact full query match in filename or OCR text
    if fn_lower.contains(&q_lower) || ocr_lower.contains(&q_lower) {
        return 1.0;
    }

    // 2. Exact match of any technical or uppercase token
    let tokens: Vec<&str> = q_clean.split_whitespace().collect();
    for token in tokens {
        if is_technical_or_exact_token(token) {
            let tok_lower = token.to_lowercase();
            if fn_lower.contains(&tok_lower) || ocr_lower.contains(&tok_lower) {
                return 1.0;
            }
        }
    }

    0.0
}

/// Normalizes FTS BM25 score (where lower/more negative is better) to [0.0, 1.0].
pub fn normalize_bm25_score(raw_bm25: f64) -> f64 {
    let pos = (-raw_bm25).max(0.0);
    pos / (1.0 + pos)
}

/// Normalizes cosine similarity from [-1.0, 1.0] to [0.0, 1.0].
pub fn normalize_cosine_score(cosine: f32) -> f64 {
    cosine.max(0.0).min(1.0) as f64
}

/// Determines the user-friendly match type explanation.
pub fn classify_match_type(exact_signal: f64, fts_score: f64, semantic_score: f64) -> String {
    if exact_signal > 0.0 {
        "exact".to_string()
    } else if fts_score > 0.0 && semantic_score > 0.65 {
        "hybrid".to_string()
    } else if fts_score > 0.0 {
        "keyword".to_string()
    } else {
        "semantic".to_string()
    }
}

/// Generates a preview snippet for semantic-only matches where FTS snippet is not available.
fn extract_semantic_snippet(raw_query: &str, ocr_text: &str) -> Option<String> {
    if ocr_text.is_empty() {
        return None;
    }

    let q_lower = raw_query.to_lowercase();
    let words: Vec<&str> = q_lower.split_whitespace().collect();

    // Check if any query word appears in OCR text
    let lower_ocr = ocr_text.to_lowercase();
    for word in words {
        if word.len() >= 3 {
            if let Some(pos) = lower_ocr.find(word) {
                let start = pos.saturating_sub(40);
                let end = (pos + word.len() + 60).min(ocr_text.len());
                let snippet_slice = &ocr_text[start..end];
                return Some(format!("...{}...", snippet_slice.trim()));
            }
        }
    }

    // Fallback: take first 120 characters of OCR text
    let preview: String = ocr_text.chars().take(120).collect();
    if ocr_text.chars().count() > 120 {
        Some(format!("{}...", preview.trim()))
    } else {
        Some(preview.trim().to_string())
    }
}

/// Executes two-stage hybrid search combining SQLite FTS5 and local vector similarity.
pub fn search_hybrid(
    conn: &Connection,
    engine: &dyn TextEmbeddingEngine,
    req: &SearchRequest,
) -> Result<SearchResultPage, AppError> {
    let limit = req.limit.unwrap_or(50).clamp(1, 100);
    let offset = req.offset.unwrap_or(0);

    // -------------------------------------------------------------
    // Stage 1A: FTS5 Candidate Retrieval (Top 100)
    // -------------------------------------------------------------
    let mut fts_candidates: HashMap<i64, (f64, Option<String>)> = HashMap::new();
    let safe_fts = build_safe_fts_query(&req.query);

    if let Some(ref match_expr) = safe_fts {
        let fts_query = match req.folder_id {
            Some(_) => {
                "SELECT s.id, bm25(screenshots_fts, 5.0, 1.0) AS score,
                        COALESCE(
                            NULLIF(snippet(screenshots_fts, 1, '[[match]]', '[[/match]]', '...', 20), ''),
                            snippet(screenshots_fts, 0, '[[match]]', '[[/match]]', '...', 10)
                        ) AS match_snippet
                 FROM screenshots_fts
                 JOIN screenshots s ON s.id = screenshots_fts.rowid
                 WHERE screenshots_fts MATCH ?1
                   AND s.folder_id = ?2
                   AND s.ocr_status = 'SUCCEEDED'
                 ORDER BY score ASC
                 LIMIT ?3"
            }
            None => {
                "SELECT s.id, bm25(screenshots_fts, 5.0, 1.0) AS score,
                        COALESCE(
                            NULLIF(snippet(screenshots_fts, 1, '[[match]]', '[[/match]]', '...', 20), ''),
                            snippet(screenshots_fts, 0, '[[match]]', '[[/match]]', '...', 10)
                        ) AS match_snippet
                 FROM screenshots_fts
                 JOIN screenshots s ON s.id = screenshots_fts.rowid
                 WHERE screenshots_fts MATCH ?1
                   AND s.ocr_status = 'SUCCEEDED'
                 ORDER BY score ASC
                 LIMIT ?2"
            }
        };

        let mut stmt = conn.prepare(fts_query).map_err(|e| {
            AppError::database(format!("Failed to prepare FTS candidate query: {e}"))
        })?;

        let map_row = |row: &rusqlite::Row| {
            let id: i64 = row.get(0)?;
            let raw_score: f64 = row.get(1)?;
            let snippet: Option<String> = row.get(2)?;
            Ok((id, raw_score, snippet))
        };

        let rows_res = match req.folder_id {
            Some(f_id) => {
                stmt.query_map(params![match_expr, f_id, CANDIDATE_LIMIT as i64], map_row)
            }
            None => stmt.query_map(params![match_expr, CANDIDATE_LIMIT as i64], map_row),
        };

        if let Ok(rows) = rows_res {
            for row in rows.flatten() {
                let (id, raw_score, snippet) = row;
                fts_candidates.insert(id, (normalize_bm25_score(raw_score), snippet));
            }
        }
    }

    // -------------------------------------------------------------
    // Stage 1B: Vector Similarity Candidate Retrieval (Top 100)
    // -------------------------------------------------------------
    let mut semantic_candidates: HashMap<i64, f64> = HashMap::new();

    if let Ok(query_vector) = engine.embed_query(&req.query) {
        if let Ok(all_embeddings) =
            embeddings::load_all_embeddings(conn, engine.model_id(), engine.model_version())
        {
            let mut scored_vectors: Vec<(i64, f32)> = Vec::with_capacity(all_embeddings.len());

            for (screenshot_id, doc_vector) in &all_embeddings {
                let sim = cosine_similarity(&query_vector, doc_vector);
                if sim >= SEMANTIC_MIN_THRESHOLD {
                    scored_vectors.push((*screenshot_id, sim));
                }
            }

            // Sort descending by cosine similarity
            scored_vectors
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            for (id, sim) in scored_vectors.into_iter().take(CANDIDATE_LIMIT) {
                semantic_candidates.insert(id, normalize_cosine_score(sim));
            }
        }
    }

    // -------------------------------------------------------------
    // Candidate Union
    // -------------------------------------------------------------
    let mut union_ids: HashSet<i64> = HashSet::new();
    union_ids.extend(fts_candidates.keys());
    union_ids.extend(semantic_candidates.keys());

    if union_ids.is_empty() {
        return Ok(SearchResultPage {
            items: Vec::new(),
            total_matches: 0,
            has_more: false,
        });
    }

    // -------------------------------------------------------------
    // Stage 2: Hybrid Reranking
    // -------------------------------------------------------------
    let q_lower = req.query.to_lowercase();
    let mut scored_items: Vec<(f64, SearchResultItem)> = Vec::with_capacity(union_ids.len());

    for screenshot_id in union_ids {
        if let Ok(Some(detail)) = screenshots::get_screenshot_by_id(conn, screenshot_id) {
            // Apply folder filter if specified
            if let Some(target_folder_id) = req.folder_id {
                if detail.folder_id != target_folder_id {
                    continue;
                }
            }

            let ocr_text = detail.ocr_text.as_deref().unwrap_or("");
            let exact_signal = compute_exact_signal(&req.query, &detail.filename, ocr_text);

            let (fts_score, fts_snippet) = fts_candidates
                .get(&screenshot_id)
                .cloned()
                .unwrap_or((0.0, None));

            let semantic_score = semantic_candidates
                .get(&screenshot_id)
                .cloned()
                .unwrap_or(0.0);

            // Filename relevance signal
            let fn_lower = detail.filename.to_lowercase();
            let filename_signal = if fn_lower.contains(&q_lower) {
                1.0
            } else {
                0.0
            };

            // Hybrid Ranking Formula:
            // exact_signal (4.0x) > filename_signal (2.0x) > fts_score (1.5x) > semantic_score (1.2x)
            let final_score = (4.0 * exact_signal)
                + (2.0 * filename_signal)
                + (1.5 * fts_score)
                + (1.2 * semantic_score);

            let match_type = classify_match_type(exact_signal, fts_score, semantic_score);

            let match_snippet = match fts_snippet {
                Some(snip) => Some(snip),
                None => extract_semantic_snippet(&req.query, ocr_text),
            };

            let item = SearchResultItem {
                id: detail.id,
                folder_id: detail.folder_id,
                path: detail.path,
                filename: detail.filename,
                modified_at_fs: detail.modified_at_fs,
                content_hash: detail.content_hash,
                width: detail.width,
                height: detail.height,
                match_snippet,
                score: final_score,
                match_type: Some(match_type),
            };

            scored_items.push((final_score, item));
        }
    }

    // Sort descending by final hybrid score, with modified_at_fs descending as recency tiebreaker
    scored_items.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.modified_at_fs.cmp(&a.1.modified_at_fs))
    });

    let total_matches = scored_items.len();
    let paged_items: Vec<SearchResultItem> = scored_items
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, item)| item)
        .collect();

    let has_more = offset + paged_items.len() < total_matches;

    Ok(SearchResultPage {
        items: paged_items,
        total_matches,
        has_more,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_technical_token_detection() {
        assert!(is_technical_or_exact_token("P2028"));
        assert!(is_technical_or_exact_token("ERR_MODULE_NOT_FOUND"));
        assert!(is_technical_or_exact_token("HTTP"));
        assert!(is_technical_or_exact_token("500"));
        assert!(is_technical_or_exact_token("3000"));
        assert!(is_technical_or_exact_token("package.json"));

        assert!(!is_technical_or_exact_token("lỗi"));
        assert!(!is_technical_or_exact_token("database"));
        assert!(!is_technical_or_exact_token("hôm"));
    }

    #[test]
    fn test_exact_signal_computation() {
        let sig = compute_exact_signal(
            "P2028",
            "screenshot.png",
            "Error: PrismaClient P2028 timeout",
        );
        assert_eq!(sig, 1.0);

        let sig_none =
            compute_exact_signal("P2028", "screenshot.png", "Error: transaction timeout");
        assert_eq!(sig_none, 0.0);
    }
}
