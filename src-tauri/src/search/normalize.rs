use unicode_normalization::UnicodeNormalization;

/// Normalizes OCR text specifically for search indexing.
///
/// This transformation produces an OCR-tolerant searchable representation:
/// 1. Applies Unicode NFC normalization.
/// 2. Converts characters to lowercase.
/// 3. Normalizes separators: converts underscores (`_`), hyphens (`-`), colons (`:`),
///    slashes (`/`, `\`), and punctuation into whitespace. This bridges OCR variance
///    where symbols like `_` in `ERR_MODULE_NOT_FOUND` are recognized as spaces `ERR MODULE NOT FOUND`.
/// 4. Preserves technical tokens, alphanumeric IDs (`P2028`), and dotted formats (`v1.2.3`, `192.168.1.1`).
/// 5. Collapses multiple whitespace characters into a single space and trims.
pub fn normalize_search_text(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // 1. Unicode NFC & lowercase
    let nfc: String = raw.nfc().collect();
    let lower = nfc.to_lowercase();

    let mut result = String::with_capacity(lower.len());
    let chars: Vec<char> = lower.chars().collect();
    let len = chars.len();

    for i in 0..len {
        let c = chars[i];
        if c.is_alphanumeric() {
            result.push(c);
        } else if c == '.' {
            // Preserve dot if directly surrounded by alphanumeric characters (e.g. 192.168.1.1 or v1.2.3)
            let prev_alnum = i > 0 && chars[i - 1].is_alphanumeric();
            let next_alnum = i + 1 < len && chars[i + 1].is_alphanumeric();
            if prev_alnum && next_alnum {
                result.push('.');
            } else {
                result.push(' ');
            }
        } else {
            // Separators: underscores, hyphens, colons, brackets, quotes, etc. -> whitespace
            result.push(' ');
        }
    }

    // Collapse multiple whitespace and trim
    result.split_whitespace().collect::<Vec<&str>>().join(" ")
}

/// Normalizes user search queries with the same rules so query tokens align with index tokens.
pub fn normalize_search_query(query: &str) -> String {
    normalize_search_text(query)
}
