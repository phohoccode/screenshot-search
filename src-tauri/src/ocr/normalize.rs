use unicode_normalization::UnicodeNormalization;

/// Normalizes raw OCR text before persisting to SQLite.
///
/// Steps applied:
/// 1. Unicode NFC normalization.
/// 2. Line ending normalization to standard `\n`.
/// 3. Filters non-printable control characters (excluding `\t`).
/// 4. Trims trailing whitespace on each line.
/// 5. Collapses excessive consecutive empty lines (at most 1 blank line for paragraph separation).
/// 6. Trims leading and trailing whitespace of the overall text.
///
/// Strictly preserves:
/// - Technical identifiers (e.g. `P2028`, `ERR_CONNECTION_REFUSED`, `npm ERESOLVE`)
/// - URLs (e.g. `https://example.com/api/v1?test=123`)
/// - Code syntax, punctuation (`{}[]();:="'.`), and JSON formatting
/// - Email addresses and file paths
pub fn normalize_ocr_text(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }

    // 1. Unicode NFC normalization
    let nfc_text: String = raw.nfc().collect();

    // 2. Normalize CRLF / CR to standard LF
    let standardized = nfc_text.replace("\r\n", "\n").replace('\r', "\n");

    let mut result_lines = Vec::new();
    let mut consecutive_empty_lines = 0;

    for line in standardized.lines() {
        // 3. Remove non-printable control characters, but allow tab
        let cleaned_line: String = line
            .chars()
            .filter(|&c| c == '\t' || (!c.is_control() && c != '\0'))
            .collect();

        // 4. Trim trailing line whitespace
        let trimmed_line = cleaned_line.trim_end();

        if trimmed_line.is_empty() {
            consecutive_empty_lines += 1;
            // 5. Allow at most 1 empty line in a row
            if consecutive_empty_lines <= 1 && !result_lines.is_empty() {
                result_lines.push(String::new());
            }
        } else {
            consecutive_empty_lines = 0;
            result_lines.push(trimmed_line.to_string());
        }
    }

    // 6. Overall trim
    result_lines.join("\n").trim().to_string()
}
