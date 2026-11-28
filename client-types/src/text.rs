//! サーバーから届いた文字列を端末などに表示する前の無害化。

use std::borrow::Cow;

/// 制御文字の代わりに表示する文字。
pub const REPLACEMENT: char = '\u{FFFD}';
/// タブを展開する空白の数。
pub const TAB_WIDTH: usize = 4;

/// 表示用に制御文字を取り除く。
///
/// `char::is_control()` に当たる文字（C0、DEL、C1）は U+FFFD に置き換える。タブは
/// 空白に展開する。改行（`\n`、`\r`）も置き換えるので、1 行で表示する文字列に使う。
/// 複数行の本文は行に分けてから、行ごとに呼ぶ。制御文字がなければ借用のまま返す。
pub fn sanitize_display(text: &str) -> Cow<'_, str> {
    if !text.chars().any(char::is_control) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.extend(std::iter::repeat_n(' ', TAB_WIDTH)),
            c if c.is_control() => out.push(REPLACEMENT),
            c => out.push(c),
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed() {
        assert!(matches!(
            sanitize_display("こんにちは abc"),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn control_characters_are_replaced() {
        assert_eq!(
            sanitize_display("a\u{1b}[2Jb\u{7}c\u{7f}d\u{9b}e\u{85}"),
            "a\u{FFFD}[2Jb\u{FFFD}c\u{FFFD}d\u{FFFD}e\u{FFFD}"
        );
        assert_eq!(
            sanitize_display("\u{1b}]52;c;AAAA\u{7}"),
            "\u{FFFD}]52;c;AAAA\u{FFFD}"
        );
    }

    #[test]
    fn tab_is_expanded_and_newlines_are_replaced() {
        assert_eq!(sanitize_display("a\tb"), "a    b");
        assert_eq!(sanitize_display("a\r\nb"), "a\u{FFFD}\u{FFFD}b");
    }
}
