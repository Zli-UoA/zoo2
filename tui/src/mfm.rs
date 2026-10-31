//! Misskey MFM の簡易レンダラ。ターミナルで表せる装飾 (太字、斜体、取り消し線、コード、
//! 引用、リンク、メンション、ハッシュタグ、絵文字ショートコード) だけをスタイルに変換し、
//! アニメーションなどの `$[fn ...]` は中身の文字だけを表示する。

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

const CODE: Style = Style::new().fg(Color::Yellow);
const LINK: Style = Style::new()
    .fg(Color::Blue)
    .add_modifier(Modifier::UNDERLINED);
const MENTION: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
const HASHTAG: Style = Style::new().fg(Color::Cyan);
const MATH: Style = Style::new().fg(Color::Magenta);
const QUOTE: Style = Style::new().fg(Color::DarkGray);

/// 本文を行ごとのスタイル付きテキストにする。`me` は自分の `user_name` で、自分宛ての
/// メンションを反転表示する。
pub fn render(content: &str, me: Option<&str>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_code = false;
    for raw in content.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(Line::from(Span::styled(line.to_string(), CODE)));
            continue;
        }
        let (prefix, body, base) = match line.strip_prefix('>') {
            Some(rest) => (
                Some(Span::styled("│ ", QUOTE)),
                rest.strip_prefix(' ').unwrap_or(rest),
                QUOTE,
            ),
            None => (None, line, Style::new()),
        };
        let mut spans: Vec<Span<'static>> = prefix.into_iter().collect();
        Inline { me }.parse(body, base, &mut spans);
        lines.push(Line::from(spans));
    }
    lines
}

struct Inline<'a> {
    me: Option<&'a str>,
}

/// 囲み記法。開きと閉じの間を `style` で再帰的に解析する。
const WRAPPERS: &[(&str, &str, Style)] = &[
    ("**", "**", Style::new().add_modifier(Modifier::BOLD)),
    ("~~", "~~", Style::new().add_modifier(Modifier::CROSSED_OUT)),
    ("<b>", "</b>", Style::new().add_modifier(Modifier::BOLD)),
    ("<i>", "</i>", Style::new().add_modifier(Modifier::ITALIC)),
    (
        "<s>",
        "</s>",
        Style::new().add_modifier(Modifier::CROSSED_OUT),
    ),
    (
        "<small>",
        "</small>",
        Style::new().add_modifier(Modifier::DIM),
    ),
    ("<center>", "</center>", Style::new()),
];

impl Inline<'_> {
    fn parse(&self, text: &str, base: Style, out: &mut Vec<Span<'static>>) {
        let mut plain = String::new();
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            let previous = text[..i].chars().next_back();
            if let Some((consumed, spans)) = self.token(rest, previous, base) {
                flush(&mut plain, base, out);
                out.extend(spans);
                i += consumed;
                continue;
            }
            let ch = rest.chars().next().expect("i は文字境界");
            plain.push(ch);
            i += ch.len_utf8();
        }
        flush(&mut plain, base, out);
    }

    /// `rest` の先頭が記法なら、消費したバイト数と変換結果を返す。
    fn token(
        &self,
        rest: &str,
        previous: Option<char>,
        base: Style,
    ) -> Option<(usize, Vec<Span<'static>>)> {
        if let Some(inner) = between(rest, "`", "`").filter(|inner| !inner.contains('\n')) {
            return Some((
                inner.len() + 2,
                vec![Span::styled(inner.to_string(), base.patch(CODE))],
            ));
        }
        for (open, close, style) in WRAPPERS {
            if let Some(inner) = between(rest, open, close).filter(|inner| !inner.is_empty()) {
                let mut spans = Vec::new();
                self.parse(inner, base.patch(*style), &mut spans);
                return Some((open.len() + inner.len() + close.len(), spans));
            }
        }
        if let Some(inner) = between(rest, "<plain>", "</plain>") {
            return Some((
                inner.len() + 15,
                vec![Span::styled(inner.to_string(), base)],
            ));
        }
        if let Some(inner) = between(rest, "\\(", "\\)") {
            return Some((
                inner.len() + 4,
                vec![Span::styled(inner.to_string(), base.patch(MATH))],
            ));
        }
        if rest.starts_with("$[") {
            return self.function(rest, base);
        }
        if rest.starts_with('[') || rest.starts_with("?[") {
            return link(rest, base);
        }
        let at_boundary = previous.is_none_or(is_boundary);
        if at_boundary && (rest.starts_with("https://") || rest.starts_with("http://")) {
            let url = trim_url(rest.split(char::is_whitespace).next().unwrap_or(rest));
            return Some((
                url.len(),
                vec![Span::styled(url.to_string(), base.patch(LINK))],
            ));
        }
        if at_boundary && rest.starts_with('@') {
            return self.mention(rest, base);
        }
        if at_boundary && rest.starts_with('#') {
            let tag = rest[1..]
                .split(|c: char| c.is_whitespace() || "#,.!?()[]{}<>\"'「」『』、。".contains(c))
                .next()
                .unwrap_or("");
            if !tag.is_empty() {
                return Some((
                    tag.len() + 1,
                    vec![Span::styled(format!("#{tag}"), base.patch(HASHTAG))],
                ));
            }
        }
        if rest.starts_with(':') {
            return shortcode(rest, base);
        }
        None
    }

    /// `$[name.args text]`。対応する `]` までを探し、中身の text だけを解析する。
    fn function(&self, rest: &str, base: Style) -> Option<(usize, Vec<Span<'static>>)> {
        let mut depth = 0usize;
        let mut end = None;
        for (index, ch) in rest.char_indices().skip(2) {
            match ch {
                '[' => depth += 1,
                ']' if depth == 0 => {
                    end = Some(index);
                    break;
                }
                ']' => depth -= 1,
                _ => {}
            }
        }
        let end = end?;
        let inner = &rest[2..end];
        let (name, body) = inner.split_once(' ').unwrap_or((inner, ""));
        if name.is_empty() {
            return None;
        }
        if name.split('.').next() == Some("unixtime") {
            return Some((end + 1, unixtime(body, base)));
        }
        let mut spans = Vec::new();
        self.parse(body, base, &mut spans);
        Some((end + 1, spans))
    }

    fn mention(&self, rest: &str, base: Style) -> Option<(usize, Vec<Span<'static>>)> {
        let name_len = rest[1..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || "._-@".contains(c)))
            .unwrap_or(rest.len() - 1);
        // 末尾のドットは文の句点として扱う (frontend の mentions.ts と同じ)。
        let name = rest[1..1 + name_len].trim_end_matches(['.', '@']);
        if name.len() < 2 {
            return None;
        }
        let mine = matches!(name, "channel" | "here")
            || self.me.is_some_and(|me| name.split('@').next() == Some(me));
        let style = if mine {
            base.patch(MENTION).add_modifier(Modifier::REVERSED)
        } else {
            base.patch(MENTION)
        };
        Some((
            name.len() + 1,
            vec![Span::styled(format!("@{name}"), style)],
        ))
    }
}

fn flush(plain: &mut String, style: Style, out: &mut Vec<Span<'static>>) {
    if !plain.is_empty() {
        out.push(Span::styled(std::mem::take(plain), style));
    }
}

fn between<'t>(text: &'t str, open: &str, close: &str) -> Option<&'t str> {
    let rest = text.strip_prefix(open)?;
    let end = rest.find(close)?;
    Some(&rest[..end])
}

fn is_boundary(ch: char) -> bool {
    ch.is_whitespace() || "([{<（「『\"'。、".contains(ch)
}

/// URL の末尾の句読点と、対応しない閉じ括弧を除く。
fn trim_url(url: &str) -> &str {
    let mut url = url;
    loop {
        let Some(last) = url.chars().next_back() else {
            return url;
        };
        let unbalanced = |open: char, close: char| {
            last == close && url.matches(open).count() < url.matches(close).count()
        };
        if ".,!?;:'\"".contains(last) || unbalanced('(', ')') || unbalanced('[', ']') {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            return url;
        }
    }
}

/// `[label](url)` と `?[label](url)`。ターミナルでは URL を開けないので、ラベルの後に URL を添える。
fn link(rest: &str, base: Style) -> Option<(usize, Vec<Span<'static>>)> {
    let offset = usize::from(rest.starts_with('?'));
    let label = between(&rest[offset..], "[", "]")?;
    let after = &rest[offset + label.len() + 2..];
    let url = between(after, "(", ")")?;
    if !(url.starts_with("https://") || url.starts_with("http://")) || label.contains('\n') {
        return None;
    }
    let consumed = offset + label.len() + 2 + url.len() + 2;
    let mut spans = vec![Span::styled(label.to_string(), base.patch(LINK))];
    if label != url {
        spans.push(Span::styled(
            format!(" <{url}>"),
            base.add_modifier(Modifier::DIM),
        ));
    }
    Some((consumed, spans))
}

fn shortcode(rest: &str, base: Style) -> Option<(usize, Vec<Span<'static>>)> {
    let name = between(rest, ":", ":")?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
    {
        return None;
    }
    let emoji = emojis::get_by_shortcode(name)?;
    Some((
        name.len() + 2,
        vec![Span::styled(emoji.as_str().to_string(), base)],
    ))
}

fn unixtime(body: &str, base: Style) -> Vec<Span<'static>> {
    let text = body
        .trim()
        .parse::<i64>()
        .ok()
        .and_then(|seconds| jiff::Timestamp::from_second(seconds).ok())
        .map(|timestamp| {
            timestamp
                .to_zoned(jiff::tz::TimeZone::system())
                .strftime("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| body.to_string());
    vec![Span::styled(text, base)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spans(content: &str) -> Vec<(String, Style)> {
        render(content, Some("alice"))
            .into_iter()
            .flat_map(|line| line.spans)
            .map(|span| (span.content.into_owned(), span.style))
            .collect()
    }

    fn texts(content: &str) -> Vec<String> {
        render(content, None)
            .iter()
            .map(|line| line.to_string())
            .collect()
    }

    #[test]
    fn styles_nested_inline_markup() {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        assert_eq!(
            spans("a **b ~~c~~** `d`"),
            vec![
                ("a ".into(), Style::new()),
                ("b ".into(), bold),
                ("c".into(), bold.add_modifier(Modifier::CROSSED_OUT)),
                (" ".into(), Style::new()),
                ("d".into(), CODE),
            ]
        );
    }

    #[test]
    fn renders_code_blocks_and_quotes_per_line() {
        assert_eq!(
            texts("```\nlet x = **1**;\n```\n> quoted"),
            ["let x = **1**;", "│ quoted"]
        );
    }

    #[test]
    fn keeps_only_text_of_functions_and_plain() {
        assert_eq!(
            texts("$[tada $[spin.speed=2s hi]] <plain>**raw**</plain>"),
            ["hi **raw**"]
        );
        assert_eq!(texts("$[ broken"), ["$[ broken"]);
    }

    #[test]
    fn links_mentions_hashtags_and_shortcodes() {
        assert_eq!(
            texts(
                "see [docs](https://example.com) (https://a.example/x).\n@bob. #general :thumbsup: :nope:"
            ),
            [
                "see docs <https://example.com> (https://a.example/x).",
                "@bob. #general 👍 :nope:",
            ]
        );
        let styled = spans("hi @alice and @bob and a@b");
        assert!(styled.contains(&("@alice".into(), MENTION.add_modifier(Modifier::REVERSED))));
        assert!(styled.contains(&("@bob".into(), MENTION)));
        assert!(!styled.iter().any(|(text, _)| text == "@b"));
    }
}
