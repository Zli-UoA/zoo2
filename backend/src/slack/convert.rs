//! Slack の mrkdwn を zoo の本文 (MFM) に変換する。

use crate::validation::MAX_MESSAGE_CONTENT_LENGTH;

/// `<...>` 内の参照を解決するための対応表
pub trait Resolver {
    /// Slack ユーザー ID → zoo の user_name
    fn user_name(&self, slack_user_id: &str) -> Option<String>;
    /// Slack チャンネル ID → zoo のチャンネル名
    fn channel_name(&self, slack_channel_id: &str) -> Option<String>;
}

/// mrkdwn を MFM に変換する。
///
/// - `<@U123>` / `<@U123|name>` → `@user_name`
/// - `<#C123|name>` → `#name`
/// - `<!here>` / `<!channel>` / `<!everyone>` → `@here` など
/// - `<!subteam^S1|@team>` → `@team`
/// - `<https://a|label>` → `[label](https://a)`、`<https://a>` → `https://a`
/// - `&amp;` `&lt;` `&gt;` を元に戻す
/// - `*太字*` → `**太字**`、`~取り消し線~` → `~~取り消し線~~`、`_斜体_` → `<i>斜体</i>`。コードの中は変えない
/// - 書式でない文字のうち、MFM として解釈される並び (`**` `$[` `<b>` など) は、U+200B を挟んで無効にする
pub fn mrkdwn_to_mfm(text: &str, resolver: &impl Resolver) -> String {
    truncate_content(convert(text, resolver, false))
}

/// mrkdwn を、MFM として描画しない場所 (カードの説明など) 向けの文字列に変換する。
/// [`mrkdwn_to_mfm`] と同じだが、`<https://a|label>` は `label (https://a)` にする。切り詰めない。
pub fn mrkdwn_to_text(text: &str, resolver: &impl Resolver) -> String {
    convert(text, resolver, true)
}

/// 書式の変換で展開する文字数の上限 (変換後の文字数)。取り込みの JSON は 1 ファイル 64 MiB まで読めて、本文に
/// 長さの制限がないので、文字ごとの `Tok` に展開する量を制限する。変換後の本文は `MAX_MESSAGE_CONTENT_LENGTH` で
/// 切るので、その 2 倍まで (書式の記号が上限の少し先で閉じるものまで) 見る。入力の長さではなく、`&amp;` などを戻した
/// 後の長さで数えるので、変換後に上限へ収まる本文は欠けない。
const MAX_FORMAT_CHARS: usize = MAX_MESSAGE_CONTENT_LENGTH * 2;

/// 本文を、`<...>` の外側の文字 (unescape 済み) と、`<...>` を変換した文字列に分けて順に渡す。
/// 渡した先が `false` を返したら、そこで止める
fn scan(
    text: &str,
    resolver: &impl Resolver,
    plain: bool,
    mut on_text: impl FnMut(&str) -> bool,
    mut on_atom: impl FnMut(String) -> bool,
) {
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        if !on_text(&unescape(&rest[..start])) {
            return;
        }
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else {
            on_text(&unescape(&rest[start..]));
            return;
        };
        if !on_atom(convert_token(&after[..end], resolver, plain)) {
            return;
        }
        rest = &after[end + 1..];
    }
    on_text(&unescape(rest));
}

fn convert(text: &str, resolver: &impl Resolver, plain: bool) -> String {
    // カードの説明は書式を変換しないので、文字ごとに展開せず、そのままつなげる
    if plain {
        let out = std::cell::RefCell::new(String::with_capacity(text.len()));
        scan(
            text,
            resolver,
            true,
            |t| {
                out.borrow_mut().push_str(t);
                true
            },
            |a| {
                out.borrow_mut().push_str(&a);
                true
            },
        );
        return out.into_inner();
    }
    let toks = std::cell::RefCell::new(Vec::<Tok>::new());
    // 残りの文字数。0 になったら、そこで止める
    let budget = std::cell::Cell::new(MAX_FORMAT_CHARS);
    scan(
        text,
        resolver,
        false,
        |t| {
            for c in t.chars() {
                if budget.get() == 0 {
                    return false;
                }
                toks.borrow_mut().push(Tok::Ch(c));
                budget.set(budget.get() - 1);
            }
            true
        },
        |a| {
            let n = a.chars().count();
            let mut toks = toks.borrow_mut();
            // 空の文字列になる参照 (`<!x>`) は、書式の区切りとして 1 個だけ残し、連続する分は積まない
            // (トークンが際限なく増えないように。変換後の文字数は増えないので、予算も使わない)
            if n == 0 && matches!(toks.last(), Some(Tok::Atom(s)) if s.is_empty()) {
                return true;
            }
            budget.set(budget.get().saturating_sub(n));
            toks.push(Tok::Atom(a));
            budget.get() > 0
        },
    );
    let toks = toks.into_inner();
    let mut out = String::with_capacity(toks.len() + 16);
    format(&toks, 0, &mut out);
    out
}

/// 書式の変換で扱う単位。`Atom` は `<...>` を変換した文字列で、書式の対象にせず、そのまま出す。
enum Tok {
    Ch(char),
    Atom(String),
}

fn push_text(toks: &mut Vec<Tok>, s: &str) {
    toks.extend(s.chars().map(Tok::Ch));
}

fn push_verbatim(toks: &[Tok], out: &mut String) {
    for tok in toks {
        match tok {
            Tok::Ch(c) => out.push(*c),
            Tok::Atom(s) => out.push_str(s),
        }
    }
}

/// 書式の入れ子の上限。MFM の `nestLimit` (20) に合わせる
const MAX_FORMAT_DEPTH: usize = 20;

/// MFM の記法を無効にするために挟む、幅のない文字 (U+200B)
const ZWSP: char = '\u{200b}';

/// MFM として解釈される `<` の続き。Slack の本文に文字として書かれていても、表示が変わらないようにする
const MFM_TAGS: &[&str] = &[
    "b>", "i>", "s>", "small>", "center>", "plain>", "/b>", "/i>", "/s>", "/small>", "/center>",
    "/plain>",
];

fn is_space(tok: Option<&Tok>) -> bool {
    matches!(tok, Some(Tok::Ch(c)) if c.is_whitespace())
}

fn is_ascii_alnum(tok: Option<&Tok>) -> bool {
    matches!(tok, Some(Tok::Ch(c)) if c.is_ascii_alphanumeric())
}

fn is_char(tok: Option<&Tok>, expected: char) -> bool {
    matches!(tok, Some(Tok::Ch(c)) if *c == expected)
}

/// Slack の書式 (`*太字*` `_斜体_` `~取り消し線~`) を MFM にして `out` に出す。
///
/// - 開く記号は、直後が空白でなく、直前が ASCII の英数字でないときだけ書式にする。閉じる記号は、直前が空白でなく、
///   直後が ASCII の英数字でないとき。これで `2*3*4` や `snake_case_name` は変換しない。日本語に隣り合っていても書式にする
///   (Slack の入力欄で書いた投稿は、`これは*太字*です` のように、日本語に隣り合ったまま記号が入る)。
/// - コード (`` ` `` と ```` ``` ````) の中は、そのまま出す。
/// - 書式にならなかった文字のうち、MFM として解釈される並びは、U+200B を挟んで無効にする。
fn format(toks: &[Tok], depth: usize, out: &mut String) {
    // 閉じる記号が行末まで見つからなかった記号と、探索を止めた位置。同じ行の後ろの開く記号は探し直さない
    let mut dead: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    // `later[k][m]`: `k` 以降、同じ行に記号 `*` (m=0) / `_` (m=1) があるか
    let mut later = vec![[false; 2]; toks.len() + 1];
    for k in (0..toks.len()).rev() {
        later[k] = later[k + 1];
        match &toks[k] {
            Tok::Ch('\n') => later[k] = [false; 2],
            Tok::Ch('*') => later[k][0] = true,
            Tok::Ch('_') => later[k][1] = true,
            _ => {}
        }
    }
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Atom(s) => {
                out.push_str(s);
                i += 1;
            }
            Tok::Ch('`') => i = code(toks, i, out),
            Tok::Ch(c @ ('*' | '~' | '_')) => {
                let c = *c;
                let run = toks[i..].iter().take_while(|t| is_char(Some(t), c)).count();
                if run >= 2 {
                    // `**` `~~` `__` は Slack では書式でなく、MFM では書式になる並び
                    for k in 0..run {
                        if k > 0 {
                            out.push(ZWSP);
                        }
                        out.push(c);
                    }
                    // 後ろの文字と組んで、`*x*` が斜体にならないようにする
                    if toks.get(i + run).is_some() {
                        out.push(ZWSP);
                    }
                    i += run;
                    continue;
                }
                let opens = depth < MAX_FORMAT_DEPTH
                    && !is_ascii_alnum(i.checked_sub(1).and_then(|p| toks.get(p)))
                    && toks.get(i + 1).is_some_and(|t| !is_space(Some(t)))
                    && dead.get(&c).is_none_or(|stop| i >= *stop);
                if opens {
                    match closing(toks, i, c) {
                        Ok(j) => {
                            let (open, close) = match c {
                                '*' => ("**", "**"),
                                '~' => ("~~", "~~"),
                                _ => ("<i>", "</i>"),
                            };
                            out.push_str(open);
                            format(&toks[i + 1..j], depth + 1, out);
                            out.push_str(close);
                            i = j + 1;
                            continue;
                        }
                        Err(stop) => {
                            dead.insert(c, stop);
                        }
                    }
                }
                out.push(c);
                // MFM は、`* 3 *` や `_x_` を斜体にする。同じ行に同じ記号がまだあるときは、後ろに U+200B を挟んで防ぐ
                let mfm_italic_open = c != '~'
                    && !is_ascii_alnum(i.checked_sub(1).and_then(|p| toks.get(p)))
                    && (is_ascii_alnum(toks.get(i + 1))
                        || matches!(toks.get(i + 1), Some(Tok::Ch(' ' | '\t' | '\u{3000}'))))
                    && later[i + 1][usize::from(c == '_')];
                if mfm_italic_open {
                    out.push(ZWSP);
                }
                i += 1;
            }
            Tok::Ch(c) => {
                out.push(*c);
                let next = toks.get(i + 1);
                let neutralize = match c {
                    '$' => is_char(next, '['),
                    '\\' => is_char(next, '(') || is_char(next, '['),
                    ']' => is_char(next, '('),
                    '<' => {
                        let ahead: String = toks[i + 1..]
                            .iter()
                            .take(8)
                            .map(|t| match t {
                                Tok::Ch(c) => *c,
                                Tok::Atom(_) => '\0',
                            })
                            .collect();
                        MFM_TAGS.iter().any(|tag| ahead.starts_with(tag))
                    }
                    _ => false,
                };
                if neutralize {
                    out.push(ZWSP);
                }
                i += 1;
            }
        }
    }
}

/// `toks[open]` の記号 `mark` に対応する閉じる記号の位置。見つからなければ、探索を止めた位置 (行末か末尾)
fn closing(toks: &[Tok], open: usize, mark: char) -> Result<usize, usize> {
    // 開き記号の直後からコードが始まる場合も、コードを飛ばす
    let mut j = open + 1;
    while j < toks.len() {
        if is_char(toks.get(j), '\n') {
            return Err(j);
        }
        // コードの中の記号は、外側の書式の終わりにしない
        if is_char(toks.get(j), '`')
            && let Some(end) = code_end(toks, j)
        {
            // コードが改行をまたぐなら、そこで探索を終える (改行をまたぐ記号は書式にしない)
            if let Some(newline) = toks[j..end].iter().position(|t| is_char(Some(t), '\n')) {
                return Err(j + newline);
            }
            j = end;
            continue;
        }
        if is_char(toks.get(j), mark)
            && !is_space(toks.get(j - 1))
            && !is_char(toks.get(j - 1), mark)
            && !is_char(toks.get(j + 1), mark)
            && !is_ascii_alnum(toks.get(j + 1))
        {
            return Ok(j);
        }
        j += 1;
    }
    Err(toks.len())
}

/// `toks[i]` が `` ` `` のとき、閉じているコードの範囲の終わり (含まない位置)。閉じていなければ `None`
fn code_end(toks: &[Tok], i: usize) -> Option<usize> {
    let fence = |at: usize| (0..3).all(|k| is_char(toks.get(at + k), '`'));
    if fence(i) {
        return (i + 3..toks.len()).find(|&j| fence(j)).map(|j| j + 3);
    }
    let mut j = i + 1;
    while j < toks.len() && !is_char(toks.get(j), '`') && !is_char(toks.get(j), '\n') {
        j += 1;
    }
    (is_char(toks.get(j), '`') && j > i + 1).then_some(j + 1)
}

/// `toks[i]` が `` ` `` のとき、コードの範囲をそのまま出して、次の位置を返す。閉じていなければ文字のまま出す
fn code(toks: &[Tok], i: usize, out: &mut String) -> usize {
    if let Some(end) = code_end(toks, i) {
        push_verbatim(&toks[i..end], out);
        return end;
    }
    if is_char(toks.get(i), '`') && (1..3).all(|k| is_char(toks.get(i + k), '`')) {
        out.push_str("```");
        return i + 3;
    }
    out.push('`');
    i + 1
}

/// 本文でメンションされている Slack ユーザー ID (`<@U123>` / `<@W123|name>`) を出現順・重複なしで返す
pub fn mentioned_users(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (index, _) in text.match_indices("<@") {
        let token = &text[index + 2..];
        let end = token.find(['>', '|']).unwrap_or(token.len());
        let id = &token[..end];
        let valid = id.len() > 1
            && id.starts_with(['U', 'W'])
            && id
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
        if valid && !found.iter().any(|f| f == id) {
            found.push(id.to_string());
        }
    }
    found
}

fn convert_token(token: &str, resolver: &impl Resolver, plain: bool) -> String {
    let (target, label) = match token.split_once('|') {
        Some((t, l)) => (t, Some(unescape(l))),
        None => (token, None),
    };
    if let Some(user) = target.strip_prefix('@') {
        return match resolver.user_name(user) {
            Some(name) => format!("@{name}"),
            None => format!("@{}", label.unwrap_or_else(|| user.to_string())),
        };
    }
    if let Some(channel) = target.strip_prefix('#') {
        let name = resolver
            .channel_name(channel)
            .or(label)
            .unwrap_or_else(|| channel.to_string());
        return format!("#{name}");
    }
    if let Some(special) = target.strip_prefix('!') {
        return match special {
            "here" | "channel" | "everyone" => format!("@{special}"),
            _ if special.starts_with("subteam^") => label.unwrap_or_else(|| "@team".to_string()),
            _ => label.unwrap_or_default(),
        };
    }
    let url = unescape(target);
    match label {
        Some(label) if !label.is_empty() && label != url => {
            if plain && is_web_url(&url) {
                format!("{label} ({url})")
            } else if is_web_url(&url) {
                format!("[{}]({url})", label.replace(['[', ']'], ""))
            } else {
                label
            }
        }
        _ => url
            .strip_prefix("mailto:")
            .map(str::to_string)
            .unwrap_or(url),
    }
}

fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// zoo の本文上限に合わせて切り詰める
pub fn truncate_content(s: String) -> String {
    if s.chars().count() <= MAX_MESSAGE_CONTENT_LENGTH {
        return s;
    }
    s.chars().take(MAX_MESSAGE_CONTENT_LENGTH).collect()
}

/// Slack のリアクション名 (`+1::skin-tone-2` など) を zoo の絵文字表記に正規化する
pub fn reaction_emoji(name: &str) -> Option<String> {
    let base = name.split("::").next().unwrap_or(name).to_ascii_lowercase();
    crate::validation::normalize_and_validate_emoji(&format!(":{base}:")).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Map;
    impl Resolver for Map {
        fn user_name(&self, id: &str) -> Option<String> {
            (id == "U1").then(|| "alice".to_string())
        }
        fn channel_name(&self, id: &str) -> Option<String> {
            (id == "C1").then(|| "random".to_string())
        }
    }

    #[test]
    fn collects_mentioned_users_once() {
        assert_eq!(
            mentioned_users("<@U1> <@W2|bob> <@U1> <@lower> <#C1> <@U3"),
            vec!["U1", "W2", "U3"]
        );
    }

    #[test]
    fn converts_mentions_channels_and_links() {
        let text = "hi <@U1> and <@U9|bob> in <#C1|general> &amp; <#C9|other> <!here> \
                    <https://example.com|Example> <https://x.test> <mailto:a@b.c|a@b.c> \
                    <!subteam^S1|@devs> 1 &lt; 2";
        assert_eq!(
            mrkdwn_to_mfm(text, &Map),
            "hi @alice and @bob in #random & #other @here \
             [Example](https://example.com) https://x.test a@b.c \
             @devs 1 < 2"
        );
    }

    #[test]
    fn converts_to_plain_text_for_cards() {
        let text = "see <https://example.com|Example> <https://x.test> <@U1> in <#C1|general> &amp; *bold*";
        assert_eq!(
            mrkdwn_to_text(text, &Map),
            "see Example (https://example.com) https://x.test @alice in #random & *bold*"
        );
    }

    #[test]
    fn keeps_unclosed_brackets_and_non_web_links() {
        assert_eq!(mrkdwn_to_mfm("a < b", &Map), "a < b");
        assert_eq!(mrkdwn_to_mfm("<slack://open|Open>", &Map), "Open");
    }

    fn mfm(text: &str) -> String {
        mrkdwn_to_mfm(text, &Map)
    }

    #[test]
    fn converts_bold_strike_and_italic() {
        assert_eq!(mfm("a *bold* b"), "a **bold** b");
        assert_eq!(mfm("a ~gone~ b"), "a ~~gone~~ b");
        assert_eq!(mfm("a _slant_ b"), "a <i>slant</i> b");
        assert_eq!(mfm("*one* and *two*"), "**one** and **two**");
        assert_eq!(mfm("*a b c*"), "**a b c**");
    }

    #[test]
    fn converts_markers_next_to_japanese() {
        assert_eq!(mfm("これは*太字*です"), "これは**太字**です");
        assert_eq!(
            mfm("今度一緒に~ご範囲効果~ご飯"),
            "今度一緒に~~ご範囲効果~~ご飯"
        );
        assert_eq!(mfm("かなり_斜め_だ"), "かなり<i>斜め</i>だ");
        assert_eq!(mfm("*<!channel>* 開始"), "**@channel** 開始");
    }

    #[test]
    fn keeps_markers_that_are_not_formatting() {
        assert_eq!(mfm("2*3*4"), "2*3*4");
        assert_eq!(mfm("snake_case_name"), "snake_case_name");
        assert_eq!(mfm("a~b~c"), "a~b~c");
        assert_eq!(mfm("5 * 3"), "5 * 3");
        // 同じ行にもう一度ある `*` は、MFM で斜体にならないよう U+200B を挟む
        assert_eq!(mfm("5 * 3 * 4"), "5 *\u{200b} 3 * 4");
        assert_eq!(mfm("a _ b _ c"), "a _\u{200b} b _ c");
        assert_eq!(mfm("1~2 and 3~4"), "1~2 and 3~4");
        assert_eq!(mfm("* a *"), "*\u{200b} a *");
        assert_eq!(mfm("*unclosed"), "*unclosed");
        assert_eq!(mfm("a *b\nc* d"), "a *b\nc* d");
        assert_eq!(mfm("**"), "*\u{200b}*");
        assert_eq!(mfm("a *b"), "a *b");
    }

    #[test]
    fn nests_formats_and_skips_links() {
        assert_eq!(mfm("*bold ~and gone~*"), "**bold ~~and gone~~**");
        assert_eq!(mfm("_*both*_"), "<i>**both**</i>");
        assert_eq!(
            mfm("*see <https://a.test/x_y_z|a_b> now*"),
            "**see [a_b](https://a.test/x_y_z) now**"
        );
        assert_eq!(mfm("<https://a.test/_x_/~y~>"), "https://a.test/_x_/~y~");
        assert_eq!(mfm("*<@U1>*"), "**@alice**");
    }

    #[test]
    fn leaves_code_untouched() {
        assert_eq!(mfm("`*x* ~y~ $[a b]`"), "`*x* ~y~ $[a b]`");
        assert_eq!(
            mfm("```\n*x* **y**\n```\n*z*"),
            "```\n*x* **y**\n```\n**z**"
        );
        assert_eq!(mfm("*a `b*c` d*"), "**a `b*c` d**");
        // コードの中の記号は、外側の書式の終わりにしない (直後が英字でも空白でも)
        assert_eq!(mfm("*a `b* c` d*"), "**a `b* c` d**");
        assert_eq!(mfm("*a `x* y` z*"), "**a `x* y` z**");
        assert_eq!(mfm("~a `b~ c` d~"), "~~a `b~ c` d~~");
        assert_eq!(mfm("_a `b_ c` d_"), "<i>a `b_ c` d</i>");
        assert_eq!(mfm("*a ```b* c``` d*"), "**a ```b* c``` d**");
        // 開き記号の直後からコードが始まる場合も、コードの中を終わりにしない
        assert_eq!(mfm("*`b* c` d*"), "**`b* c` d**");
        // 改行をまたぐコードの後ろの記号は、閉じ記号にしない
        assert_eq!(mfm("~a ```\nb\n``` c~"), "~a ```\nb\n``` c~");
        assert_eq!(mfm("*a ```\nb\n``` c*"), "*a ```\nb\n``` c*");
        assert_eq!(mfm("~`b~ c` d~"), "~~`b~ c` d~~");
        assert_eq!(mfm("_`b_ c` d_"), "<i>`b_ c` d</i>");
        assert_eq!(mfm("a ` b *c*"), "a ` b **c**");
        assert_eq!(mfm("```*x"), "```*x");
    }

    #[test]
    fn neutralizes_mfm_syntax_that_slack_shows_as_text() {
        let z = "\u{200b}";
        assert_eq!(mfm("**x**"), format!("*{z}*{z}x*{z}*"));
        assert_eq!(mfm("~~x~~"), format!("~{z}~{z}x~{z}~"));
        assert_eq!(mfm("***x***"), format!("*{z}*{z}*{z}x*{z}*{z}*"));
        assert_eq!(mfm("$[spin x]"), format!("${z}[spin x]"));
        assert_eq!(mfm("&lt;b&gt;x&lt;/b&gt;"), format!("<{z}b>x<{z}/b>"));
        assert_eq!(mfm("&lt;plain&gt;"), format!("<{z}plain>"));
        assert_eq!(
            mfm("[a](https://x.test)"),
            format!("[a]{z}(https://x.test)")
        );
        // 数式は開きの `\(` を無効にすれば足りる
        assert_eq!(mfm("\\(x\\)"), format!("\\{z}(x\\)"));
        assert_eq!(mfm("1 &lt; 2 and a<b"), "1 < 2 and a<b");
        // 変換したリンクは、そのまま MFM のリンクになる
        assert_eq!(mfm("<https://a.test|A>"), "[A](https://a.test)");
    }

    #[test]
    fn caps_nesting_and_keeps_text_when_too_deep() {
        let deep = format!("{}x{}", "_*".repeat(15), "*_".repeat(15));
        let out = mfm(&deep);
        assert!(out.contains('x'));
        // 上限を超えた内側は、記号を文字のまま残す
        assert!(out.contains('*') || out.contains('_'));
    }

    #[test]
    fn many_unclosed_markers_stay_fast() {
        let text = "*a ".repeat(20_000);
        let started = std::time::Instant::now();
        assert_eq!(mfm(&text).chars().count(), 40_000);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn plain_text_keeps_slack_markers() {
        assert_eq!(
            mrkdwn_to_text("*b* ~s~ _i_ **x**", &Map),
            "*b* ~s~ _i_ **x**"
        );
    }

    #[test]
    fn bounds_the_expanded_text_by_output_length() {
        let huge = "あ*a* ".repeat(MAX_FORMAT_CHARS);
        let out = mfm(&huge);
        assert_eq!(out.chars().count(), MAX_MESSAGE_CONTENT_LENGTH);
        // カードの説明は切らない
        assert_eq!(mrkdwn_to_text(&huge, &Map), huge);
        // 入力が長くても、戻したあとの本文が上限に収まるなら、欠けない
        assert_eq!(
            mfm(&"&amp;".repeat(MAX_MESSAGE_CONTENT_LENGTH)),
            "&".repeat(MAX_MESSAGE_CONTENT_LENGTH)
        );
        assert_eq!(
            mfm(&"&amp;".repeat(MAX_MESSAGE_CONTENT_LENGTH * 10))
                .chars()
                .count(),
            MAX_MESSAGE_CONTENT_LENGTH
        );
        // 空になる参照だけの本文でも、トークンは際限なく増えない
        assert_eq!(mfm(&"<!x>".repeat(MAX_MESSAGE_CONTENT_LENGTH * 10)), "");
        // 空の参照が上限より多く続いても、後ろの本文は欠けない
        assert_eq!(
            mfm(&format!("{}END", "<!x>".repeat(MAX_FORMAT_CHARS * 2))),
            "END"
        );
        assert_eq!(
            mfm(&format!("{}*END*", "<>".repeat(MAX_FORMAT_CHARS * 2))),
            "**END**"
        );
        let links = "<@U1>".repeat(MAX_MESSAGE_CONTENT_LENGTH);
        assert_eq!(mfm(&links).chars().count(), MAX_MESSAGE_CONTENT_LENGTH);
    }

    #[test]
    fn truncation_counts_converted_characters() {
        let text = format!("*{}*", "a".repeat(MAX_MESSAGE_CONTENT_LENGTH));
        let out = mfm(&text);
        assert_eq!(out.chars().count(), MAX_MESSAGE_CONTENT_LENGTH);
        assert!(out.starts_with("**a"));
    }

    #[test]
    fn truncates_long_content() {
        let long = "a".repeat(MAX_MESSAGE_CONTENT_LENGTH + 10);
        assert_eq!(
            mrkdwn_to_mfm(&long, &Map).chars().count(),
            MAX_MESSAGE_CONTENT_LENGTH
        );
    }

    #[test]
    fn normalizes_reaction_names() {
        assert_eq!(reaction_emoji("+1::skin-tone-2").as_deref(), Some("👍"));
        assert_eq!(
            reaction_emoji("partyparrot").as_deref(),
            Some(":partyparrot:")
        );
        assert_eq!(reaction_emoji("bad name!"), None);
    }
}
