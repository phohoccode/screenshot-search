/// Formats filename and OCR text into a structured document representation for semantic embedding.
///
/// Format:
/// Filename: <filename>
/// Content:
/// <trimmed_ocr_text>
///
/// Bounded to max_chars (default 2,000 chars) to comfortably fit within the model's 512 token context window.
pub fn format_semantic_document(
    filename: &str,
    ocr_text: &str,
    max_chars: Option<usize>,
) -> String {
    let limit = max_chars.unwrap_or(2000);
    let trimmed_ocr = ocr_text.trim();

    let bounded_ocr = if trimmed_ocr.chars().count() > limit {
        trimmed_ocr.chars().take(limit).collect::<String>()
    } else {
        trimmed_ocr.to_string()
    };

    if bounded_ocr.is_empty() {
        format!("Filename: {filename}")
    } else {
        format!("Filename: {filename}\nContent:\n{bounded_ocr}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_semantic_document() {
        let doc = format_semantic_document("error.png", "P2028 timeout", None);
        assert_eq!(doc, "Filename: error.png\nContent:\nP2028 timeout");

        let doc_empty = format_semantic_document("empty.png", "", None);
        assert_eq!(doc_empty, "Filename: empty.png");

        let long_text = "a".repeat(3000);
        let doc_bounded = format_semantic_document("long.png", &long_text, Some(50));
        assert!(doc_bounded.contains("Content:\n"));
        assert!(doc_bounded.len() < 100);
    }
}
