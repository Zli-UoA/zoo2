//! Misskey MFM の簡易パーサー。太字、斜体、取り消し線、コード、引用、リンク、
//! メンション、ハッシュタグ、絵文字ショートコードを UI に依存しないスタイル付きスパンへ
//! 変換し、アニメーションなどの `$[fn ...]` は中身の文字だけを取り出す。

#[derive(Default)]
struct EmojiCatalog {
    shortcodes: std::collections::HashMap<String, String>,
    custom: std::collections::HashSet<String>,
}
static EMOJI_CATALOG: std::sync::LazyLock<std::sync::RwLock<EmojiCatalog>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(EmojiCatalog::default()));
pub fn set_emoji_catalog(
    shortcodes: std::collections::HashMap<String, String>,
    custom: std::collections::HashSet<String>,
) {
    *EMOJI_CATALOG.write().unwrap() = EmojiCatalog { shortcodes, custom };
}
pub fn is_custom_shortcode(name: &str) -> bool {
    EMOJI_CATALOG.read().unwrap().custom.contains(name)
}
impl EmojiCatalog {
    fn resolve_emoji(&self, input: &str) -> Option<String> {
        let input = input.trim();
        let name = input.trim_matches(':');
        if self.custom.contains(name) {
            return Some(format!(":{name}:"));
        }
        self.unicode(name)
            .or_else(|| emojis::get(input).map(|emoji| emoji.as_str().to_owned()))
            .or_else(|| {
                (input.starts_with(':')
                    && input.ends_with(':')
                    && (1..=64).contains(&name.len())
                    && name.bytes().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'-'
                    }))
                .then(|| format!(":{name}:"))
            })
    }
    fn unicode(&self, name: &str) -> Option<String> {
        if self.custom.contains(name) {
            return None;
        }
        if !self.shortcodes.is_empty() {
            return self.shortcodes.get(name).cloned();
        }
        emojis::get_by_shortcode(name).map(|e| e.as_str().to_owned())
    }
}
pub fn resolve_emoji(input: &str) -> Option<String> {
    EMOJI_CATALOG.read().unwrap().resolve_emoji(input)
}
pub fn unicode_shortcode(name: &str) -> Option<String> {
    EMOJI_CATALOG.read().unwrap().unicode(name)
}

#[cfg(test)]
mod shared_shortcode_tests {
    use super::*;
    #[test]
    fn remote_names_override_local_dictionary_but_custom_names_win() {
        let catalog = EmojiCatalog {
            shortcodes: [
                ("remote_alias".to_owned(), "👩‍💻".to_owned()),
                ("smile".to_owned(), "😄".to_owned()),
            ]
            .into(),
            custom: ["smile".to_owned()].into(),
        };
        assert_eq!(
            catalog.resolve_emoji(":remote_alias:").as_deref(),
            Some("👩‍💻")
        );
        assert_eq!(catalog.resolve_emoji(":smile:").as_deref(), Some(":smile:"));
        assert_eq!(
            catalog.resolve_emoji(":custom_unknown:").as_deref(),
            Some(":custom_unknown:")
        );
        assert_eq!(catalog.resolve_emoji(":../cat:"), None);
        assert_eq!(catalog.unicode("remote_alias").as_deref(), Some("👩‍💻"));
        assert_eq!(catalog.unicode("smile"), None);
        assert_eq!(catalog.unicode("thumbsup"), None);
        assert_eq!(
            EmojiCatalog::default().unicode("thumbsup").as_deref(),
            Some("👍")
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Default,
    Code,
    Link,
    Mention,
    Hashtag,
    Math,
    Quote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    pub tone: Tone,
    pub bold: bool,
    pub italic: bool,
    pub underlined: bool,
    pub crossed_out: bool,
    pub dim: bool,
    pub reversed: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self::new()
    }
}

impl Style {
    pub const fn new() -> Self {
        Self {
            tone: Tone::Default,
            bold: false,
            italic: false,
            underlined: false,
            crossed_out: false,
            dim: false,
            reversed: false,
        }
    }

    pub const fn with_tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    pub const fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub const fn underlined(mut self) -> Self {
        self.underlined = true;
        self
    }

    pub const fn crossed_out(mut self) -> Self {
        self.crossed_out = true;
        self
    }

    pub const fn dim(mut self) -> Self {
        self.dim = true;
        self
    }

    pub const fn reversed(mut self) -> Self {
        self.reversed = true;
        self
    }

    pub const fn patch(mut self, other: Self) -> Self {
        if !matches!(other.tone, Tone::Default) {
            self.tone = other.tone;
        }
        self.bold |= other.bold;
        self.italic |= other.italic;
        self.underlined |= other.underlined;
        self.crossed_out |= other.crossed_out;
        self.dim |= other.dim;
        self.reversed |= other.reversed;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub content: String,
    pub style: Style,
    /// リンクの行き先 (`http://` か `https://` で始まるものだけ)。`[label](url)` では
    /// `content` がラベルになるので、行き先はこちらを使う。
    pub href: Option<String>,
}

impl Span {
    pub fn styled(content: String, style: Style) -> Self {
        Self {
            content,
            style,
            href: None,
        }
    }

    fn link(content: String, href: &str, style: Style) -> Self {
        Self {
            content,
            style,
            href: Some(href.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub spans: Vec<Span>,
}

impl Line {
    pub fn from_spans(spans: Vec<Span>) -> Self {
        Self { spans }
    }
}

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for span in &self.spans {
            f.write_str(&span.content)?;
        }
        Ok(())
    }
}

pub const CODE: Style = Style::new().with_tone(Tone::Code);
pub const LINK: Style = Style::new().with_tone(Tone::Link).underlined();
pub const MENTION: Style = Style::new().with_tone(Tone::Mention).bold();
pub const HASHTAG: Style = Style::new().with_tone(Tone::Hashtag);
pub const MATH: Style = Style::new().with_tone(Tone::Math);
pub const QUOTE: Style = Style::new().with_tone(Tone::Quote);

/// 本文を行ごとのスタイル付きスパンにする。`me` は自分の `user_name` で、自分宛ての
/// メンションを反転表示する。
pub fn parse(content: &str, me: Option<&str>) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut in_code = false;
    for raw in content.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            lines.push(Line::from_spans(vec![Span::styled(line.to_string(), CODE)]));
            continue;
        }
        let (prefix, body, base) = match line.strip_prefix('>') {
            Some(rest) => (
                Some(Span::styled("│ ".to_string(), QUOTE)),
                rest.strip_prefix(' ').unwrap_or(rest),
                QUOTE,
            ),
            None => (None, line, Style::new()),
        };
        let mut spans: Vec<Span> = prefix.into_iter().collect();
        Inline { me }.parse(body, base, 0, &mut spans);
        lines.push(Line::from_spans(spans));
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
    ("**", "**", Style::new().bold()),
    ("~~", "~~", Style::new().crossed_out()),
    ("<b>", "</b>", Style::new().bold()),
    ("<i>", "</i>", Style::new().italic()),
    ("<s>", "</s>", Style::new().crossed_out()),
    ("<small>", "</small>", Style::new().dim()),
    ("<center>", "</center>", Style::new()),
];

impl Inline<'_> {
    fn parse(&self, text: &str, base: Style, depth: usize, out: &mut Vec<Span>) {
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
    ) -> Option<(usize, Vec<Span>)> {
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
                vec![Span::link(url.to_string(), url, base.patch(LINK))],
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
    ) -> Option<(usize, Vec<Span>)> {
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

    fn mention(&self, rest: &str, base: Style) -> Option<(usize, Vec<Span>)> {
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
            base.patch(MENTION).reversed()
        } else {
            base.patch(MENTION)
        };
        Some((
            name.len() + 1,
            vec![Span::styled(format!("@{name}"), style)],
        ))
    }
}

fn flush(plain: &mut String, style: Style, out: &mut Vec<Span>) {
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

/// `[label](url)` と `?[label](url)`。ラベルの後に URL を添える。
fn link(scan: &mut Scan, i: usize, base: Style) -> Option<(usize, Vec<Span>)> {
    let offset = usize::from(scan.text[i..].starts_with('?'));
    let label = scan.between(i + offset, "[", "]")?;
    let url = scan.between(i + offset + label.len() + 2, "(", ")")?;
    if !(url.starts_with("https://") || url.starts_with("http://")) || label.contains('\n') {
        return None;
    }
    let consumed = offset + label.len() + 2 + url.len() + 2;
    let mut spans = vec![Span::link(label.to_string(), url, base.patch(LINK))];
    if label != url {
        spans.push(Span::styled(format!(" <{url}>"), base.dim()));
    }
    Some((consumed, spans))
}

fn shortcode(scan: &mut Scan, i: usize, base: Style) -> Option<(usize, Vec<Span>)> {
    let name = scan.between(i, ":", ":")?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_+-".contains(c))
    {
        return None;
    }
    let emoji = unicode_shortcode(name)?;
    Some((name.len() + 2, vec![Span::styled(emoji, base)]))
}

fn unixtime(body: &str, base: Style) -> Vec<Span> {
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
        parse(content, Some("alice"))
            .into_iter()
            .flat_map(|line| line.spans)
            .map(|span| (span.content, span.style))
            .collect()
    }

    fn texts(content: &str) -> Vec<String> {
        parse(content, None)
            .iter()
            .map(|line| line.to_string())
            .collect()
    }

    #[test]
    fn styles_nested_inline_markup() {
        let bold = Style::new().bold();
        assert_eq!(
            spans("a **b ~~c~~** `d`"),
            vec![
                ("a ".into(), Style::new()),
                ("b ".into(), bold),
                ("c".into(), bold.crossed_out()),
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
        assert!(styled.contains(&("@alice".into(), MENTION.reversed())));
        assert!(styled.contains(&("@bob".into(), MENTION)));
        assert!(!styled.iter().any(|(text, _)| text == "@b"));
    }

    #[test]
    fn links_keep_their_destination() {
        let hrefs: Vec<(String, Option<String>)> =
            parse("[docs](https://example.com) https://a.example/x", None)
                .into_iter()
                .flat_map(|line| line.spans)
                .filter(|span| span.style.tone == Tone::Link)
                .map(|span| (span.content, span.href))
                .collect();
        assert_eq!(
            hrefs,
            [
                ("docs".into(), Some("https://example.com".into())),
                (
                    "https://a.example/x".into(),
                    Some("https://a.example/x".into())
                ),
            ]
        );
    }
}
