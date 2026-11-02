use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};

/// RFC 5987 でエスケープが不要な文字群 (attr-char):
/// ALPHA / DIGIT / "!" / "#" / "$" / "&" / "+" / "-" / "." / "^" / "_" / "`" / "|" / "~"
/// NON_ALPHANUMERIC からこれらの許可記号を除外してエンコードセットを作成する。
const RFC5987_SPECIAL_CHARS: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

/// 画像許可リストにある MIME タイプかどうか判定する
pub fn is_inline_allowed_mime(content_type: &str) -> bool {
    let lower = content_type.to_ascii_lowercase();
    let mime = lower.split(';').next().unwrap_or("").trim();
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

/// RFC 6266 / RFC 5987 に準拠した Content-Disposition ヘッダー値を生成する
pub fn format_content_disposition(disposition_type: &str, file_name: &str) -> String {
    // 制御文字 (\r, \n, \0 等) や引用符・バックスラッシュを置換・除去した ASCII フォールバック
    let mut ascii_name = String::with_capacity(file_name.len());
    for ch in file_name.chars() {
        if ch.is_ascii_graphic() && ch != '"' && ch != '\\' && ch != ';' {
            ascii_name.push(ch);
        } else if ch == ' ' {
            ascii_name.push(' ');
        } else {
            ascii_name.push('_');
        }
    }
    let ascii_name = ascii_name.trim();
    let safe_ascii = if ascii_name.is_empty() {
        "file"
    } else {
        ascii_name
    };

    let encoded_name = utf8_percent_encode(file_name, RFC5987_SPECIAL_CHARS).to_string();

    format!("{disposition_type}; filename=\"{safe_ascii}\"; filename*=UTF-8''{encoded_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_content_disposition_ascii() {
        let cd = format_content_disposition("attachment", "document.pdf");
        assert_eq!(
            cd,
            "attachment; filename=\"document.pdf\"; filename*=UTF-8''document.pdf"
        );
    }

    #[test]
    fn test_format_content_disposition_japanese() {
        let cd = format_content_disposition("inline", "スクリーンショット 2026.png");
        assert!(cd.starts_with("inline; filename=\"_________ 2026.png\"; filename*=UTF-8''"));
        assert!(cd.contains("%E3%82%B9%E3%82%AF%E3%83%AA%E3%83%BC%E3%83%B3"));
    }

    #[test]
    fn test_format_content_disposition_quotes_and_control_chars() {
        let cd = format_content_disposition("attachment", "bad\"file\nname\\test.png");
        assert_eq!(
            cd,
            "attachment; filename=\"bad_file_name_test.png\"; filename*=UTF-8''bad%22file%0Aname%5Ctest.png"
        );
    }

    #[test]
    fn test_is_inline_allowed_mime() {
        assert!(is_inline_allowed_mime("image/png"));
        assert!(is_inline_allowed_mime("IMAGE/JPEG"));
        assert!(is_inline_allowed_mime("image/webp; charset=utf-8"));
        assert!(is_inline_allowed_mime("image/gif"));

        // XSS リスクのある形式やドキュメントは拒否 (false)
        assert!(!is_inline_allowed_mime("image/svg+xml"));
        assert!(!is_inline_allowed_mime("text/html"));
        assert!(!is_inline_allowed_mime("application/pdf"));
        assert!(!is_inline_allowed_mime("application/javascript"));
    }
}
