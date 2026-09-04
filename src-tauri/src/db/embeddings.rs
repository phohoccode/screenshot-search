use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::errors::AppError;

/// Aggregated metrics for semantic embedding coverage.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingStats {
    pub total_succeeded: usize,
    pub embedded_count: usize,
    pub pending_count: usize,
    pub active_model_id: String,
    pub active_model_version: String,
}

/// Converts a slice of 32-bit floats into a raw byte vector in little-endian format.
pub fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for &val in vector {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Converts a raw byte slice in little-endian format back into a vector of 32-bit floats.
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    let num_floats = blob.len() / 4;
    let mut vector = Vec::with_capacity(num_floats);
    for chunk in blob.chunks_exact(4) {
        let arr: [u8; 4] = [chunk[0], chunk[1], chunk[2], chunk[3]];
        vector.push(f32::from_le_bytes(arr));
    }
    vector
}

/// Computes the cosine similarity between two float vectors.
/// Clamped to [-1.0, 1.0]. Returns 0.0 for zero-norm or mismatched vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for (&x, &y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-12 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0)
    }
}

/// Inserts or replaces the embedding vector for a screenshot.
pub fn save_embedding(
    conn: &Connection,
    screenshot_id: i64,
    model_id: &str,
    model_version: &str,
    vector: &[f32],
) -> Result<(), AppError> {
    let blob = vector_to_blob(vector);
    conn.execute(
        "INSERT INTO screenshot_embeddings (screenshot_id, model_id, model_version, embedding, created_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))
         ON CONFLICT(screenshot_id) DO UPDATE SET
             model_id = excluded.model_id,
             model_version = excluded.model_version,
             embedding = excluded.embedding,
             created_at = datetime('now')",
        params![screenshot_id, model_id, model_version, blob],
    )
    .map_err(|e| AppError::database(format!("Failed to save embedding for screenshot {screenshot_id}: {e}")))?;

    Ok(())
}

/// Retrieves the embedding vector for a single screenshot by ID.
pub fn get_embedding(conn: &Connection, screenshot_id: i64) -> Result<Option<Vec<f32>>, AppError> {
    let blob: Option<Vec<u8>> = conn
        .query_row(
            "SELECT embedding FROM screenshot_embeddings WHERE screenshot_id = ?1",
            params![screenshot_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| {
            AppError::database(format!(
                "Failed to load embedding for screenshot {screenshot_id}: {e}"
            ))
        })?;

    Ok(blob.map(|b| blob_to_vector(&b)))
}

/// Deletes the embedding vector for a single screenshot.
pub fn delete_embedding(conn: &Connection, screenshot_id: i64) -> Result<bool, AppError> {
    let affected = conn
        .execute(
            "DELETE FROM screenshot_embeddings WHERE screenshot_id = ?1",
            params![screenshot_id],
        )
        .map_err(|e| {
            AppError::database(format!(
                "Failed to delete embedding for screenshot {screenshot_id}: {e}"
            ))
        })?;

    Ok(affected > 0)
}

/// Loads all embeddings associated with the specified model ID and version for in-memory scanning.
pub fn load_all_embeddings(
    conn: &Connection,
    model_id: &str,
    model_version: &str,
) -> Result<Vec<(i64, Vec<f32>)>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT screenshot_id, embedding 
             FROM screenshot_embeddings 
             WHERE model_id = ?1 AND model_version = ?2",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare load_all_embeddings: {e}")))?;

    let rows = stmt
        .query_map(params![model_id, model_version], |row| {
            let id: i64 = row.get(0)?;
            let blob: Vec<u8> = row.get(1)?;
            Ok((id, blob))
        })
        .map_err(|e| AppError::database(format!("Failed to query embeddings: {e}")))?;

    let mut result = Vec::new();
    for item in rows {
        let (id, blob) =
            item.map_err(|e| AppError::database(format!("Failed to read embedding row: {e}")))?;
        result.push((id, blob_to_vector(&blob)));
    }

    Ok(result)
}

/// Deletes all embeddings for a specific model version (used during model migration or rebuild).
pub fn clear_embeddings_by_model(
    conn: &Connection,
    model_id: &str,
    model_version: &str,
) -> Result<usize, AppError> {
    let count = conn
        .execute(
            "DELETE FROM screenshot_embeddings WHERE model_id = ?1 AND model_version = ?2",
            params![model_id, model_version],
        )
        .map_err(|e| {
            AppError::database(format!(
                "Failed to clear embeddings for model {model_id} {model_version}: {e}"
            ))
        })?;

    Ok(count)
}

/// Retrieves coverage metrics for the specified embedding model.
pub fn get_embedding_stats(
    conn: &Connection,
    model_id: &str,
    model_version: &str,
) -> Result<EmbeddingStats, AppError> {
    let total_succeeded: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM screenshots WHERE ocr_status = 'SUCCEEDED'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let embedded_count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM screenshot_embeddings 
             WHERE model_id = ?1 AND model_version = ?2",
            params![model_id, model_version],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let pending_count = total_succeeded.saturating_sub(embedded_count);

    Ok(EmbeddingStats {
        total_succeeded,
        embedded_count,
        pending_count,
        active_model_id: model_id.to_string(),
        active_model_version: model_version.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vector_blob_roundtrip() {
        let original = vec![0.123f32, -45.67f32, 0.0f32, 1000.0f32];
        let blob = vector_to_blob(&original);
        assert_eq!(blob.len(), original.len() * 4);
        let restored = blob_to_vector(&blob);
        assert_eq!(original, restored);
    }

    #[test]
    fn test_cosine_similarity() {
        let v1 = vec![1.0, 0.0, 0.0];
        let v2 = vec![1.0, 0.0, 0.0];
        let v3 = vec![0.0, 1.0, 0.0];
        let v4 = vec![-1.0, 0.0, 0.0];

        assert!((cosine_similarity(&v1, &v2) - 1.0).abs() < 1e-5);
        assert!(cosine_similarity(&v1, &v3).abs() < 1e-5);
        assert!((cosine_similarity(&v1, &v4) - (-1.0)).abs() < 1e-5);
    }
}
