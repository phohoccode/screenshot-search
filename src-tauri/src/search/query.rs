use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::errors::AppError;
use crate::search::normalize::normalize_search_query;

/// Search request parameters sent from frontend or internal callers.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct SearchRequest {
    pub query: String,
    pub folder_id: Option<i64>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// A single matched screenshot result item.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultItem {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub filename: String,
    pub modified_at_fs: String,
    pub content_hash: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub match_snippet: Option<String>,
    pub score: f64,
}

/// Paginated search result page.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResultPage {
    pub items: Vec<SearchResultItem>,
    pub total_matches: usize,
    pub has_more: bool,
}

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 100;

/// Constructs a safe, sanitized FTS5 MATCH expression from a user query.
///
/// Returns `None` if the query contains no searchable tokens.
pub fn build_safe_fts_query(raw_query: &str) -> Option<String> {
    let normalized = normalize_search_query(raw_query);
    let raw_tokens: Vec<&str> = normalized.split_whitespace().collect();

    let mut valid_tokens = Vec::new();
    for token in raw_tokens {
        // Strip any residual FTS control symbols
        let sanitized: String = token
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '.')
            .collect();

        if !sanitized.is_empty() {
            // Quote the token and append prefix wildcard '*' for responsive partial matching
            valid_tokens.push(format!("\"{}\"*", sanitized));
        }
    }

    if valid_tokens.is_empty() {
        None
    } else {
        Some(valid_tokens.join(" AND "))
    }
}

/// Executes a full-text search against the SQLite FTS5 index.
///
/// Features:
/// - Safe token sanitization (prevents FTS syntax errors)
/// - BM25 ranking (with 5.0x boost for filename matches over OCR text)
/// - Match snippet generation with delimiter markers `[[match]]...[[/match]]`
/// - Empty query behavior: returns most recent OCR-ready screenshots
/// - Safe pagination with clamped limit and offset
pub fn search_screenshots(
    conn: &Connection,
    req: &SearchRequest,
) -> Result<SearchResultPage, AppError> {
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let offset = req.offset.unwrap_or(0);

    let safe_fts = build_safe_fts_query(&req.query);

    match safe_fts {
        None => {
            // Empty query behavior: return recent screenshots where OCR has succeeded
            let count_sql = match req.folder_id {
                Some(_) => {
                    "SELECT COUNT(*) FROM screenshots 
                     WHERE ocr_status = 'SUCCEEDED' AND folder_id = ?1"
                }
                None => {
                    "SELECT COUNT(*) FROM screenshots 
                     WHERE ocr_status = 'SUCCEEDED'"
                }
            };

            let total_matches: i64 = match req.folder_id {
                Some(f_id) => conn
                    .query_row(count_sql, params![f_id], |row| row.get(0))
                    .unwrap_or(0),
                None => conn.query_row(count_sql, [], |row| row.get(0)).unwrap_or(0),
            };

            let query_sql = match req.folder_id {
                Some(_) => {
                    "SELECT s.id, s.folder_id, s.path, s.filename, s.modified_at_fs, 
                            s.content_hash, s.width, s.height, NULL, 0.0
                     FROM screenshots s
                     WHERE s.ocr_status = 'SUCCEEDED' AND s.folder_id = ?1
                     ORDER BY s.modified_at_fs DESC, s.id DESC
                     LIMIT ?2 OFFSET ?3"
                }
                None => {
                    "SELECT s.id, s.folder_id, s.path, s.filename, s.modified_at_fs, 
                            s.content_hash, s.width, s.height, NULL, 0.0
                     FROM screenshots s
                     WHERE s.ocr_status = 'SUCCEEDED'
                     ORDER BY s.modified_at_fs DESC, s.id DESC
                     LIMIT ?1 OFFSET ?2"
                }
            };

            let mut stmt = conn.prepare(query_sql).map_err(|e| {
                AppError::database(format!("Failed to prepare empty search query: {e}"))
            })?;

            let map_row = |row: &rusqlite::Row| {
                Ok(SearchResultItem {
                    id: row.get(0)?,
                    folder_id: row.get(1)?,
                    path: row.get(2)?,
                    filename: row.get(3)?,
                    modified_at_fs: row.get(4)?,
                    content_hash: row.get(5)?,
                    width: row.get(6)?,
                    height: row.get(7)?,
                    match_snippet: row.get(8)?,
                    score: row.get(9)?,
                })
            };

            let rows = match req.folder_id {
                Some(f_id) => stmt.query_map(params![f_id, limit as i64, offset as i64], map_row),
                None => stmt.query_map(params![limit as i64, offset as i64], map_row),
            }
            .map_err(|e| {
                AppError::database(format!("Failed to execute empty search query: {e}"))
            })?;

            let mut items = Vec::new();
            for row in rows {
                items.push(
                    row.map_err(|e| {
                        AppError::database(format!("Failed to read search item: {e}"))
                    })?,
                );
            }

            let total = total_matches as usize;
            let has_more = offset + items.len() < total;

            Ok(SearchResultPage {
                items,
                total_matches: total,
                has_more,
            })
        }
        Some(match_expr) => {
            // FTS5 MATCH query with BM25 ranking and snippet extraction
            let count_sql = match req.folder_id {
                Some(_) => {
                    "SELECT COUNT(*) 
                     FROM screenshots_fts f
                     JOIN screenshots s ON s.id = f.rowid
                     WHERE screenshots_fts MATCH ?1 
                       AND s.folder_id = ?2 
                       AND s.ocr_status = 'SUCCEEDED'"
                }
                None => {
                    "SELECT COUNT(*) 
                     FROM screenshots_fts f
                     JOIN screenshots s ON s.id = f.rowid
                     WHERE screenshots_fts MATCH ?1 
                       AND s.ocr_status = 'SUCCEEDED'"
                }
            };

            let total_matches: i64 = match req.folder_id {
                Some(f_id) => conn
                    .query_row(count_sql, params![match_expr, f_id], |row| row.get(0))
                    .unwrap_or(0),
                None => conn
                    .query_row(count_sql, params![match_expr], |row| row.get(0))
                    .unwrap_or(0),
            };

            let query_sql = match req.folder_id {
                Some(_) => {
                    "SELECT 
                        s.id, 
                        s.folder_id, 
                        s.path, 
                        s.filename, 
                        s.modified_at_fs, 
                        s.content_hash,
                        s.width, 
                        s.height,
                        COALESCE(
                            NULLIF(snippet(screenshots_fts, 1, '[[match]]', '[[/match]]', '...', 20), ''),
                            snippet(screenshots_fts, 0, '[[match]]', '[[/match]]', '...', 10)
                        ) AS match_snippet,
                        bm25(screenshots_fts, 5.0, 1.0) AS score
                     FROM screenshots_fts
                     JOIN screenshots s ON s.id = screenshots_fts.rowid
                     WHERE screenshots_fts MATCH ?1
                       AND s.folder_id = ?2
                       AND s.ocr_status = 'SUCCEEDED'
                     ORDER BY score ASC, s.modified_at_fs DESC
                     LIMIT ?3 OFFSET ?4"
                }
                None => {
                    "SELECT 
                        s.id, 
                        s.folder_id, 
                        s.path, 
                        s.filename, 
                        s.modified_at_fs, 
                        s.content_hash,
                        s.width, 
                        s.height,
                        COALESCE(
                            NULLIF(snippet(screenshots_fts, 1, '[[match]]', '[[/match]]', '...', 20), ''),
                            snippet(screenshots_fts, 0, '[[match]]', '[[/match]]', '...', 10)
                        ) AS match_snippet,
                        bm25(screenshots_fts, 5.0, 1.0) AS score
                     FROM screenshots_fts
                     JOIN screenshots s ON s.id = screenshots_fts.rowid
                     WHERE screenshots_fts MATCH ?1
                       AND s.ocr_status = 'SUCCEEDED'
                     ORDER BY score ASC, s.modified_at_fs DESC
                     LIMIT ?2 OFFSET ?3"
                }
            };

            let mut stmt = conn.prepare(query_sql).map_err(|e| {
                AppError::database(format!("Failed to prepare FTS search query: {e}"))
            })?;

            let map_row = |row: &rusqlite::Row| {
                let raw_score: f64 = row.get(9)?;
                // BM25 returns negative values where lower = more relevant. Invert to positive for DTO.
                let normalized_score = -raw_score;

                Ok(SearchResultItem {
                    id: row.get(0)?,
                    folder_id: row.get(1)?,
                    path: row.get(2)?,
                    filename: row.get(3)?,
                    modified_at_fs: row.get(4)?,
                    content_hash: row.get(5)?,
                    width: row.get(6)?,
                    height: row.get(7)?,
                    match_snippet: row.get(8)?,
                    score: normalized_score,
                })
            };

            let rows_result = match req.folder_id {
                Some(f_id) => stmt.query_map(
                    params![match_expr, f_id, limit as i64, offset as i64],
                    map_row,
                ),
                None => stmt.query_map(params![match_expr, limit as i64, offset as i64], map_row),
            };

            let rows = match rows_result {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("FTS query execution failed for expression '{match_expr}': {e}");
                    return Ok(SearchResultPage {
                        items: Vec::new(),
                        total_matches: 0,
                        has_more: false,
                    });
                }
            };

            let mut items = Vec::new();
            for row in rows {
                if let Ok(item) = row {
                    items.push(item);
                }
            }

            let total = total_matches as usize;
            let has_more = offset + items.len() < total;

            Ok(SearchResultPage {
                items,
                total_matches: total,
                has_more,
            })
        }
    }
}
