use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

/// Classification of a detected text line's content for hybrid OCR routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineContentType {
    /// Line contains code, URLs, paths, error codes, identifiers, or technical syntax.
    /// MUST be recognized with Windows Media OCR (literal transcription).
    Technical,
    /// Line contains standard natural language prose, UI labels, or documentation.
    /// Safe to route to VietOCR for enhanced Vietnamese diacritic accuracy.
    Natural,
    /// Ambiguous, short, or mixed content.
    /// Fail-safe invariant: KEEP Windows Media OCR.
    Uncertain,
}

/// Deterministic, zero-cloud, lightweight line content classifier.
/// Evaluates the Windows OCR probe text to identify if a line is safe for
/// autoregressive recognition or dangerous (which requires literal CTC transcription).
pub struct LineContentClassifier;

impl LineContentClassifier {
    /// Classifies a text line into Technical, Natural, or Uncertain.
    /// Follows the fail-safe rule: When in doubt, prefer Technical or Uncertain.
    pub fn classify(text: &str) -> LineContentType {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return LineContentType::Uncertain;
        }

        // 1. Strong Technical Disqualifiers (immediate Technical classification)
        if Self::has_strong_technical_pattern(trimmed) {
            return LineContentType::Technical;
        }

        // Windows en-US OCR commonly represents Vietnamese tone/shape marks as
        // Latin letters carrying non-Vietnamese combining marks (for example,
        // diaeresis or ring-above). This is a structural Unicode corruption
        // signal, not a phrase or heading dictionary. Strong technical syntax
        // above always takes precedence, and all-uppercase text is excluded so
        // plain technical status headings retain the conservative path.
        if Self::has_windows_ocr_diacritic_corruption_signal(trimmed) {
            return LineContentType::Natural;
        }

        // 2. Compute weighted technical and natural scores
        let (tech_score, natural_score) = Self::compute_scores(trimmed);

        // 3. Decision threshold with conservative safety bias
        if tech_score >= 30 {
            LineContentType::Technical
        } else if tech_score >= 15 {
            LineContentType::Uncertain
        } else if natural_score >= 20 && tech_score < 10 {
            LineContentType::Natural
        } else {
            LineContentType::Uncertain
        }
    }

    /// Checks for immediate, unequivocal technical signals.
    fn has_strong_technical_pattern(text: &str) -> bool {
        let lower = text.to_lowercase();

        // URLs & Protocols
        if lower.contains("http://")
            || lower.contains("https://")
            || lower.contains("www.")
            || lower.contains("://")
            || lower.contains("localhost:")
            || lower.contains("127.0.0.1")
            || lower.contains("192.168.")
        {
            return true;
        }

        // File system paths
        if Self::is_windows_path(text) || Self::is_unix_path(text) {
            return true;
        }

        // File extensions (e.g. package.json, main.rs, schema.prisma)
        if Self::has_file_extension(&lower) {
            return true;
        }

        // Error codes and exception markers
        if lower.contains("err_")
            || lower.contains("econnrefused")
            || lower.contains("eacces")
            || lower.contains("eperm")
            || lower.contains("eaddrinuse")
            || lower.contains("prismaclientknownrequesterror")
            || lower.contains("nullpointerexception")
            || lower.contains("exception")
            || lower.contains("panic")
            || lower.contains("error:")
            || lower.contains("fatal:")
            || lower.contains("stacktrace")
            || lower.contains("internal server error")
            || lower.contains("transaction already closed")
            || lower.contains("http 500")
            || lower.contains("http 404")
            || lower.contains("http 502")
            || lower.contains("http 503")
            || lower.starts_with("at ")
        {
            return true;
        }

        // P-error codes (e.g. P2028, P3009, P1001)
        if Self::has_p_error_code(text) {
            return true;
        }

        // Command line & tool syntax
        if lower.contains("npm run")
            || lower.contains("npx ")
            || lower.contains("cargo ")
            || lower.contains("git commit")
            || lower.contains("git push")
            || lower.contains("git status")
            || lower.contains("git checkout")
            || lower.contains("git tag")
            || lower.contains("commit ")
            || lower.contains("sha256:")
            || lower.contains("container_id=")
            || lower.contains("container id:")
        {
            return true;
        }

        // Token-level technical patterns
        for word in text.split_whitespace() {
            // Scoped packages e.g. @swc/helpers, @prisma/client, @tauri-apps/api
            if word.starts_with('@') && word.contains('/') {
                return true;
            }
            // Key=value assignment syntax (e.g. requestId=8fc20a13, status=502, pid=14092)
            if word.contains('=') && word.len() >= 5 {
                let parts: Vec<&str> = word.split('=').collect();
                if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                    let key = parts[0]
                        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
                    let val = parts[1]
                        .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
                    if !key.is_empty()
                        && key
                            .chars()
                            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
                        && val
                            .chars()
                            .any(|c| c.is_ascii_digit() || c.is_ascii_hexdigit())
                    {
                        return true;
                    }
                }
            }
            // Hex memory address or literal (0x80004005, 0x1a)
            if (word.starts_with("0x") || word.starts_with("0X")) && word.len() >= 4 {
                return true;
            }
            // Vendor error codes e.g. ORA-12154, VCB-982341, SIGKILL
            if (word.starts_with("ORA-") || word.starts_with("SIG")) && word.len() >= 5 {
                return true;
            }
        }

        // Shell prompts
        if text.starts_with("PS ") || text.starts_with("$ ") || text.starts_with("> ") {
            return true;
        }

        // SQL keywords in uppercase code style
        if (text.contains("SELECT ") && text.contains("FROM "))
            || (text.contains("INSERT INTO ") && text.contains("VALUES"))
            || (text.contains("UPDATE ") && text.contains("SET "))
            || (text.contains("DELETE FROM ") && text.contains("WHERE "))
        {
            return true;
        }

        // Programming operators and syntax
        if text.contains("::")
            || text.contains("->")
            || text.contains("=>")
            || text.contains("&&")
            || text.contains("||")
            || text.contains("==")
            || text.contains("!=")
            || text.contains("/*")
            || text.contains("*/")
            || text.contains("//")
            || text.contains("fn ")
            || text.contains("pub fn")
            || text.contains("let mut")
            || text.contains("const ")
        {
            return true;
        }

        false
    }

    fn has_windows_ocr_diacritic_corruption_signal(text: &str) -> bool {
        let words = text.split_whitespace().count();
        let letter_count = text.chars().filter(|c| c.is_alphabetic()).count();
        if words < 2 || letter_count < 6 {
            return false;
        }

        let has_lowercase = text.chars().any(|c| c.is_lowercase());
        if !has_lowercase {
            return false;
        }

        text.chars().any(Self::has_non_vietnamese_latin_mark)
    }

    fn has_non_vietnamese_latin_mark(c: char) -> bool {
        if c.is_ascii() || !c.is_alphabetic() {
            return false;
        }

        let decomposed: Vec<char> = c.to_string().nfd().collect();
        if decomposed.len() < 2 || !decomposed[0].is_ascii_alphabetic() {
            return false;
        }

        decomposed[1..].iter().any(|mark| {
            !matches!(
                *mark,
                '\u{0300}' // grave
                    | '\u{0301}' // acute
                    | '\u{0302}' // circumflex
                    | '\u{0303}' // tilde
                    | '\u{0306}' // breve
                    | '\u{0309}' // hook above
                    | '\u{031b}' // horn
                    | '\u{0323}' // dot below
            )
        })
    }

    /// Evaluates weighted scores based on token density, character types, and identifiers.
    fn compute_scores(text: &str) -> (i32, i32) {
        let mut tech_score = 0;
        let mut natural_score = 0;

        let total_chars = text.chars().count().max(1);
        let mut symbol_count = 0;
        let mut digit_count = 0;
        let mut letter_count = 0;

        for c in text.chars() {
            if c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphabetic()) {
                letter_count += 1;
            } else if c.is_ascii_digit() {
                digit_count += 1;
            } else if !c.is_whitespace() {
                symbol_count += 1;
            }
        }

        let letter_ratio = (letter_count as f32) / (total_chars as f32);
        let symbol_ratio = (symbol_count as f32) / (total_chars as f32);
        let digit_ratio = (digit_count as f32) / (total_chars as f32);

        if letter_ratio > 0.80 {
            natural_score += 15;
        }

        // Symbol density penalties/signals
        if symbol_ratio > 0.15 {
            tech_score += 25;
        } else if symbol_ratio > 0.08 {
            tech_score += 10;
        } else {
            natural_score += 10;
        }

        // Digit density signals
        if digit_ratio > 0.30 {
            tech_score += 20;
        } else if digit_ratio > 0.15 {
            tech_score += 10;
        }

        let words: Vec<&str> = text.split_whitespace().collect();
        if words.len() >= 3 {
            natural_score += 15;
        } else if words.len() >= 2 {
            natural_score += 5;
        }

        // Check each word for identifier patterns (snake_case, camelCase, PascalCase, hex)
        for word in &words {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');

            // snake_case (contains underscore between letters)
            if clean.contains('_') && clean.len() > 3 {
                tech_score += 25;
            }

            // UPPER_CASE constant
            if clean.len() >= 4
                && clean
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                && clean.chars().any(|c| c.is_ascii_uppercase())
            {
                tech_score += 15;
            }

            // camelCase / PascalCase transitions (e.g. PrismaClient, localhost)
            let chars: Vec<char> = clean.chars().collect();
            let mut uppercase_transitions = 0;
            for i in 1..chars.len() {
                if chars[i - 1].is_lowercase() && chars[i].is_uppercase() {
                    uppercase_transitions += 1;
                }
            }
            if uppercase_transitions >= 2 {
                tech_score += 30;
            } else if uppercase_transitions == 1 && clean.len() >= 6 {
                tech_score += 15;
            }

            // Hex string or commit hash (e.g. 9f8a7b6c5d4e, c79401a0, a84fe91)
            if clean.len() >= 7
                && clean.chars().all(|c| c.is_ascii_hexdigit())
                && clean.chars().any(|c| c.is_ascii_digit())
                && clean.chars().any(|c| c.is_ascii_alphabetic())
            {
                tech_score += 25;
            }

            // Hyphenated package/tool identifier (e.g. bcrypt-pbkdf, onnxruntime-node, libusb-1.0)
            if clean.contains('-') && clean.len() >= 8 {
                let parts: Vec<&str> = clean.split('-').collect();
                if parts.len() >= 2
                    && parts
                        .iter()
                        .all(|p| p.len() >= 2 && p.chars().all(|c| c.is_alphanumeric() || c == '.'))
                {
                    tech_score += 20;
                }
            }

            // Common port numbers (e.g. 8080, 3000, 6379, 5432)
            if clean == "8080"
                || clean == "3000"
                || clean == "6379"
                || clean == "5432"
                || clean == "8000"
                || clean == "8443"
            {
                tech_score += 15;
            }

            // High punctuation inside word (e.g. `obj.prop`, `arr[0]`, `res->val`)
            if word.contains("->")
                || word.contains("::")
                || clean.contains('.')
                || clean.contains('/')
                || clean.contains('\\')
            {
                tech_score += 10;
            }
        }

        (tech_score, natural_score)
    }

    fn is_windows_path(text: &str) -> bool {
        // Match "C:\", "E:\", or UNC "\\server\share"
        if text.len() >= 3 {
            let chars: Vec<char> = text.chars().take(3).collect();
            if chars.len() == 3
                && chars[0].is_ascii_alphabetic()
                && chars[1] == ':'
                && (chars[2] == '\\' || chars[2] == '/')
            {
                return true;
            }
        }

        // Windows OCR can drop the first path segment or insert whitespace
        // after a drive marker while retaining later separators. Keep these
        // partially corrupted paths on the literal technical recognizer.
        let starts_with_drive = {
            let mut chars = text.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some(drive), Some(':')) if drive.is_ascii_alphabetic()
            )
        };
        let separator_count = text.chars().filter(|c| *c == '\\' || *c == '/').count();

        (starts_with_drive && separator_count >= 1)
            || separator_count >= 2
            || text.contains(r"\Users\")
            || text.contains(r"\Project\")
            || text.contains(r"\AppData\")
            || text.contains(r"\Program Files\")
    }

    fn is_unix_path(text: &str) -> bool {
        if text.starts_with("./")
            || text.starts_with("../")
            || text.contains("/usr/")
            || text.contains("/home/")
            || text.contains("/etc/")
            || text.contains("/var/")
            || text.contains("/node_modules/")
            || text.contains("/api/")
            || text.starts_with("GET /")
            || text.starts_with("POST /")
            || text.starts_with("PUT /")
            || text.starts_with("DELETE /")
        {
            return true;
        }

        // Multi-segment Unix paths in words (e.g. /sys/fs/cgroup, /opt/homebrew/bin/node)
        for word in text.split_whitespace() {
            let clean = word.trim_matches(|c: char| {
                !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-'
            });
            if clean.starts_with('/') && clean[1..].contains('/') && clean.len() >= 5 {
                return true;
            }
        }
        false
    }

    fn has_file_extension(lower: &str) -> bool {
        let extensions = [
            ".json", ".toml", ".rs", ".ts", ".tsx", ".js", ".py", ".prisma", ".dll", ".exe",
            ".onnx", ".png", ".jpg", ".jpeg", ".lock", ".node", ".css", ".html", ".md", ".sh",
            ".ps1", ".yml", ".yaml", ".sqlite", ".db", ".so", ".sql", ".rlib", ".log",
        ];
        for ext in extensions {
            if lower.contains(ext) {
                // Ensure it's not just a stray dot, but part of a filename token
                return true;
            }
        }
        false
    }

    fn has_p_error_code(text: &str) -> bool {
        // Match e.g. "P2028", "P3009", "P1001", "P2002"
        for word in text.split_whitespace() {
            let clean = word.trim_matches(|c: char| !c.is_alphanumeric());
            let chars: Vec<char> = clean.chars().collect();
            if chars.len() == 5
                && (chars[0] == 'P' || chars[0] == 'p')
                && chars[1..].iter().all(|c| c.is_ascii_digit())
            {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_technical_urls_and_ports() {
        assert_eq!(
            LineContentClassifier::classify("http://localhost:3000/api/v1/search"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("https://github.com/pbcquoc/vietocr"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("tại cổng localhost:3000/api/v1/search"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("127.0.0.1:8080"),
            LineContentType::Technical
        );
    }

    #[test]
    fn test_technical_paths() {
        assert_eq!(
            LineContentClassifier::classify("C:\\Users\\Pho\\AppData\\Local"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("E:\\Project\\screenshot-search"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("./features/search"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("../package.json"),
            LineContentType::Technical
        );
    }

    #[test]
    fn test_technical_code_and_errors() {
        assert_eq!(
            LineContentClassifier::classify(
                "PrismaClientKnownRequestError: Transaction already closed."
            ),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Mã lỗi kỹ thuật: P2028 trong tiến trình xử lý cơ sở dữ liệu."
            ),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("Lỗi nghiêm trọng: ERR_MODULE_NOT_FOUND"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("CONTAINER ID: 9f8a7b6c5d4e - Cổng mạng 8080"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("const duongDanCSDL = 'database.sqlite';"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify(
                "SELECT ten_tep, noi_dung_ocr FROM danh_sach_anh WHERE trang_thai = 'HOAN_THANH';"
            ),
            LineContentType::Technical
        );
    }

    #[test]
    fn test_technical_commands_and_packages() {
        assert_eq!(
            LineContentClassifier::classify("git commit -m 'feat: cập nhật bộ lọc tìm kiếm FTS5'"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("npm run build"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("cargo check"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("@tauri-apps/cli"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("query_engine-windows.dll.node"),
            LineContentType::Technical
        );
    }

    #[test]
    fn test_natural_vietnamese_sentences() {
        assert_eq!(
            LineContentClassifier::classify("Lưu thay đổi Hủy bỏ Tiếp tục Quay lại"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Trang chủ Ảnh chụp màn hình Cài đặt hệ thống Tài khoản người dùng"
            ),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("Xác nhận xóa dữ liệu"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Bạn có chắc chắn muốn xóa ảnh này không? Thao tác này không thể hoàn tác."
            ),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Thủ đô Hà Nội bước vào mùa thu với tiết trời se lạnh."
            ),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Kiến trúc hệ thống tìm kiếm cục bộ đảm bảo quyền riêng tư tuyệt đối."
            ),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("Tìm kiếm ảnh chụp màn hình bằng trí tuệ nhân tạo"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Thực đơn món ngon mỗi ngày: Phở bò tái nạm, Bún chả nướng Hà Nội"
            ),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify(
                "Chào mừng bạn đến với ứng dụng tìm kiếm ảnh chụp màn hình cục bộ thông minh nhất."
            ),
            LineContentType::Natural
        );
    }

    #[test]
    fn test_natural_english_sentences() {
        assert_eq!(
            LineContentClassifier::classify("Payment completed successfully"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("Unable to connect to server"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("Please try again later"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("System maintenance in progress"),
            LineContentType::Natural
        );
    }

    #[test]
    fn test_mixed_technical_vietnamese_is_safe() {
        // In mixed lines, technical tokens MUST classify as Technical or Uncertain
        // to ensure they remain on Windows Media OCR and are never sent to VietOCR.
        assert_ne!(
            LineContentClassifier::classify("Lỗi giao dịch P2028"),
            LineContentType::Natural
        );
        assert_ne!(
            LineContentClassifier::classify("Không thể kết nối localhost:3000"),
            LineContentType::Natural
        );
        assert_ne!(
            LineContentClassifier::classify("Không tìm thấy ERR_MODULE_NOT_FOUND"),
            LineContentType::Natural
        );
    }

    #[test]
    fn test_structural_windows_vietnamese_corruption_signal() {
        assert_eq!(
            LineContentClassifier::classify("DANG NÄNGCÄp"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("THONG BÄo HÉ THÖNG"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("THdl GIAN KIEN HOÄN TÄT"),
            LineContentType::Natural
        );
        assert_eq!(
            LineContentClassifier::classify("lüc 23:47 22 thång 8, 2026"),
            LineContentType::Natural
        );

        // Technical syntax wins before the corruption signal.
        assert_eq!(
            LineContentClassifier::classify(r"C:\Äpp\build.exe"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify(r"C: nguröi düng>\pictures\screenshots"),
            LineContentType::Technical
        );
        assert_eq!(
            LineContentClassifier::classify("BUILD FÄILED"),
            LineContentType::Uncertain
        );
    }

    #[test]
    fn test_uppercase_technical_prose_remains_conservative() {
        let lines = [
            "BUILD FAILED",
            "ACCESS DENIED",
            "DATABASE ERROR",
            "CONNECTION RESET",
            "INVALID TOKEN",
            "SERVER ERROR",
            "REQUEST TIMEOUT",
            "MIGRATION FAILED",
            "INTERNAL SERVER ERROR",
            "TRANSACTION ABORTED",
            "NETWORK UNREACHABLE",
            "PERMISSION DENIED",
            "RESOURCE EXHAUSTED",
            "SERVICE UNAVAILABLE",
            "MEMORY OVERFLOW",
            "DEADLOCK DETECTED",
            "COMPILATION FAILED",
            "AUTHENTICATION FAILED",
            "DEADLOCK",
            "OVERFLOW",
            "SEGFAULT",
            "UNREACHABLE",
        ];

        for line in lines {
            assert_ne!(
                LineContentClassifier::classify(line),
                LineContentType::Natural,
                "technical status line must not route to VietOCR: {line}"
            );
        }
    }
}
