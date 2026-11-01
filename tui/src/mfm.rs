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
        Inline { me }.parse(body, base, 0, &mut spans);
        lines.push(Line::from(spans));
    }
    lines
}

/// 入れ子にした記法を解析する深さの上限。これより深い部分は文字のまま表示する。
/// Misskey の `nestLimit` と同じ値。
const MAX_DEPTH: usize = 20;

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
    fn parse(&self, text: &str, base: Style, depth: usize, out: &mut Vec<Span<'static>>) {
        let mut scan = Scan::new(text);
        let mut plain = String::new();
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            let previous = text[..i].chars().next_back();
            if let Some((consumed, spans)) = self.token(&mut scan, i, previous, base, depth) {
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

    /// `scan.text[i..]` の先頭が記法なら、消費したバイト数と変換結果を返す。
    fn token(
        &self,
        scan: &mut Scan,
        i: usize,
        previous: Option<char>,
        base: Style,
        depth: usize,
    ) -> Option<(usize, Vec<Span<'static>>)> {
        let rest = &scan.text[i..];
        let nestable = depth < MAX_DEPTH;
        if let Some(inner) = scan
            .between(i, "`", "`")
            .filter(|inner| !inner.contains('\n'))
        {
            return Some((
                inner.len() + 2,
                vec![Span::styled(inner.to_string(), base.patch(CODE))],
            ));
        }
        for (open, close, style) in WRAPPERS.iter().filter(|_| nestable) {
            if let Some(inner) = scan
                .between(i, open, close)
                .filter(|inner| !inner.is_empty())
            {
                let mut spans = Vec::new();
                self.parse(inner, base.patch(*style), depth + 1, &mut spans);
                return Some((open.len() + inner.len() + close.len(), spans));
            }
        }
        if let Some(inner) = scan.between(i, "<plain>", "</plain>") {
            return Some((
                inner.len() + 15,
                vec![Span::styled(inner.to_string(), base)],
            ));
        }
        if let Some(inner) = scan.between(i, "\\(", "\\)") {
            return Some((
                inner.len() + 4,
                vec![Span::styled(inner.to_string(), base.patch(MATH))],
            ));
        }
        if nestable && rest.starts_with("$[") {
            return self.function(scan, i, base, depth);
        }
        if rest.starts_with('[') || rest.starts_with("?[") {
            return link(scan, i, base);
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
            return shortcode(scan, i, base);
        }
        None
    }

    /// `$[name.args text]`。対応する `]` までを探し、中身の text だけを解析する。
    fn function(
        &self,
        scan: &mut Scan,
        i: usize,
        base: Style,
        depth: usize,
    ) -> Option<(usize, Vec<Span<'static>>)> {
        let end = scan.closing_bracket(i + 1)? - i;
        let rest = &scan.text[i..];
        let inner = &rest[2..end];
        let (name, body) = inner.split_once(' ').unwrap_or((inner, ""));
        if name.is_empty() {
            return None;
        }
        if name.split('.').next() == Some("unixtime") {
            return Some((end + 1, unixtime(body, base)));
        }
        let mut spans = Vec::new();
        self.parse(body, base, depth + 1, &mut spans);
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

/// 1行分の本文の走査結果を覚えておき、閉じ記号の探索を本文の長さに比例する量に抑える。
/// 閉じ記号のない開き記号が並んでも、同じ範囲を何度も探し直さない。
struct Scan<'t> {
    text: &'t str,
    /// 閉じ記号ごとの、直近の探索の開始位置と見つかった位置。
    found: Vec<(&'static str, usize, Option<usize>)>,
    /// `[` の位置から対応する `]` の位置。初めて必要になったときに一度だけ求める。
    brackets: Option<std::collections::HashMap<usize, usize>>,
}

impl<'t> Scan<'t> {
    fn new(text: &'t str) -> Self {
        Self {
            text,
            found: Vec::new(),
            brackets: None,
        }
    }

    /// `from` 以降で最初に `close` が現れる位置。
    fn find(&mut self, close: &'static str, from: usize) -> Option<usize> {
        let cached = self.found.iter_mut().find(|(c, _, _)| *c == close);
        if let Some((_, start, found)) = &cached
            && *start <= from
            && found.is_none_or(|found| found >= from)
        {
            return *found;
        }
        let found = self.text[from..].find(close).map(|index| from + index);
        match cached {
            Some(entry) => *entry = (close, from, found),
            None => self.found.push((close, from, found)),
        }
        found
    }

    /// `i` から `open` で始まるなら、次の `close` までの中身。
    fn between(&mut self, i: usize, open: &str, close: &'static str) -> Option<&'t str> {
        if !self.text[i..].starts_with(open) {
            return None;
        }
        let start = i + open.len();
        let end = self.find(close, start)?;
        Some(&self.text[start..end])
    }

    /// `open` にある `[` に対応する `]` の位置。
    fn closing_bracket(&mut self, open: usize) -> Option<usize> {
        let text = self.text;
        let brackets = self.brackets.get_or_insert_with(|| {
            let mut pairs = std::collections::HashMap::new();
            let mut stack = Vec::new();
            for (index, ch) in text.char_indices() {
                match ch {
                    '[' => stack.push(index),
                    ']' => {
                        if let Some(start) = stack.pop() {
                            pairs.insert(start, index);
                        }
                    }
                    _ => {}
                }
            }
            pairs
        });
        brackets.get(&open).copied()
    }
}

fn is_boundary(ch: char) -> bool {
    ch.is_whitespace() || "([{<（「『\"'。、".contains(ch)
}

/// URL の末尾の句読点と、対応しない閉じ括弧を除く。
fn trim_url(url: &str) -> &str {
    let count = |ch: char| url.chars().filter(|c| *c == ch).count();
    // 括弧の数は一度だけ数え、末尾を削るたびに減らす。
    let (open_paren, mut close_paren) = (count('('), count(')'));
    let (open_bracket, mut close_bracket) = (count('['), count(']'));
    let mut url = url;
    loop {
        let Some(last) = url.chars().next_back() else {
            return url;
        };
        let trim = match last {
            ')' => open_paren < close_paren,
            ']' => open_bracket < close_bracket,
            '(' | '[' => false,
            _ => ".,!?;:'\"".contains(last),
        };
        if !trim {
            return url;
        }
        match last {
            ')' => close_paren -= 1,
            ']' => close_bracket -= 1,
            _ => {}
        }
        url = &url[..url.len() - last.len_utf8()];
    }
}

/// `[label](url)` と `?[label](url)`。ターミナルでは URL を開けないので、ラベルの後に URL を添える。
fn link(scan: &mut Scan, i: usize, base: Style) -> Option<(usize, Vec<Span<'static>>)> {
    let offset = usize::from(scan.text[i..].starts_with('?'));
    let label = scan.between(i + offset, "[", "]")?;
    let url = scan.between(i + offset + label.len() + 2, "(", ")")?;
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

fn shortcode(scan: &mut Scan, i: usize, base: Style) -> Option<(usize, Vec<Span<'static>>)> {
    let name = scan.between(i, ":", ":")?;
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
    fn stops_nesting_at_depth_limit() {
        // backend の文字数上限に収まる深い入れ子でも、スタックを使い切らない。
        let depth = 7_999;
        let content = format!("{}hi{}", "$[x ".repeat(depth), "]".repeat(depth));
        let text = texts(&content).concat();
        assert!(text.starts_with("$[x "), "上限より深い部分は文字のまま出る");
        assert!(text.contains("hi"));

        let bold = format!("{}hi{}", "<b>".repeat(depth), "</b>".repeat(depth));
        assert!(texts(&bold).concat().contains("hi"));
        assert_eq!(texts("$[x $[y ok]]"), ["ok"]);
    }

    #[test]
    fn unclosed_markup_does_not_rescan_the_line() {
        // 閉じ記号のない開き記号を backend の上限近くまで並べても、すぐに描画し終える。
        let started = std::time::Instant::now();
        for unit in ["$[", "<b>", "[", "?[", "\\(", "<plain>"] {
            let content = unit.repeat(40_000 / unit.len());
            assert_eq!(texts(&content).concat(), content);
        }
        let url = format!("(https://a.example/{}", ")".repeat(40_000));
        assert_eq!(texts(&url).concat(), url);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
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
