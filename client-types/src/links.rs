//! 本文に書いた、このアプリのチャンネル・メッセージへのリンク。仕様は docs/mfm.md の
//! 「チャンネル・メッセージのリンクの表示」。TUI とデスクトップ版の画面で共有する。

use jiff::civil::DateTime;
use percent_encoding::percent_decode_str;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use crate::rows::{Channel, Message};

/// 要約の最大文字数（Web の `messageExcerpt` と同じ）。画面側でも行数で切るので、これは保険の上限。
pub const EXCERPT_MAX_CHARS: usize = 200;

/// 投稿の下に埋め込みの対象にする、本文の URL の数（Web と同じく、外部の URL も数える）。
const MAX_PREVIEW_URLS: usize = 3;

/// 1 つの本文から取得する、リンク先のメッセージの数。他の人が書いた 1 件の投稿で、購読や要求が
/// 増え続けないようにする。超えた分は、すでに分かっていなければ URL のまま表示する。
pub const MAX_FETCHED_PER_BODY: usize = 3;

/// アプリ内リンクの行き先。`message_id` は `?message=`、`thread_id` は `?thread=`。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkTarget {
    pub channel_id: String,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
}

impl LinkTarget {
    /// リンクが指すメッセージ。`?message=` を優先し、`?thread=` だけのリンクはスレッドの親を指す。
    pub fn linked_message_id(&self) -> Option<&str> {
        self.message_id.as_deref().or(self.thread_id.as_deref())
    }
}

/// `value` が、接続先 (`app_base_url`) と同じ origin の `/channels/<ID>` なら行き先を返す。
/// 別の origin、別のパス、ユーザー情報の付いた URL は `None`（外部のリンクとして扱う）。
pub fn parse_channel_link(value: &str, app_base_url: &str) -> Option<LinkTarget> {
    let link = Url::parse(value).ok()?;
    let app = Url::parse(app_base_url).ok()?;
    if !matches!(link.scheme(), "http" | "https")
        || link.scheme() != app.scheme()
        || link.host_str() != app.host_str()
        || link.port_or_known_default() != app.port_or_known_default()
        || !link.username().is_empty()
        || link.password().is_some()
    {
        return None;
    }
    let mut segments = link.path_segments()?;
    if segments.next()? != "channels" {
        return None;
    }
    let channel = segments.next()?;
    if !matches!(
        (segments.next(), segments.next()),
        (None, _) | (Some(""), None)
    ) {
        return None;
    }
    let channel_id = percent_decode_str(channel).decode_utf8().ok()?.into_owned();
    if channel_id.is_empty() {
        return None;
    }
    let parameter = |name: &str| {
        link.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
            .filter(|value| !value.is_empty())
    };
    Some(LinkTarget {
        channel_id,
        message_id: parameter("message"),
        thread_id: parameter("thread"),
    })
}

/// 本文の中の、記法を解釈しない部分（コードブロック、インラインコード、`<plain>`）かどうかで分ける。
/// コードブロックは [`crate::mfm`] の表示と同じく、```` ``` ```` で始まる行で開閉し、閉じていなければ
/// 末尾まで続く。
fn segments(content: &str) -> Vec<(bool, &str)> {
    let mut result = Vec::new();
    let mut in_code = false;
    // コードブロックでない行が続いている部分の先頭。
    let mut text = 0;
    let mut at = 0;
    for line in content.split_inclusive('\n') {
        let fence = line.trim_start().starts_with("```");
        if fence || in_code {
            result.extend(inline_segments(&content[text..at]));
            result.push((true, line));
            text = at + line.len();
            in_code ^= fence;
        }
        at += line.len();
    }
    result.extend(inline_segments(&content[text..]));
    result
}

/// コードブロックの外を、インラインコードと `<plain>` かどうかで分ける。
fn inline_segments(content: &str) -> Vec<(bool, &str)> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < content.len() {
        let rest = &content[at..];
        let literal = [("```", "```"), ("<plain>", "</plain>"), ("`", "`")]
            .into_iter()
            .find(|(open, _)| rest.starts_with(open))
            .and_then(|(open, close)| {
                let end = rest[open.len()..].find(close)?;
                if open == "`" && rest[1..1 + end].contains('\n') {
                    return None;
                }
                Some(open.len() + end + close.len())
            });
        match literal {
            Some(length) => {
                if start < at {
                    result.push((false, &content[start..at]));
                }
                result.push((true, &content[at..at + length]));
                at += length;
                start = at;
            }
            None => at += rest.chars().next().map_or(1, char::len_utf8),
        }
    }
    if start < content.len() {
        result.push((false, &content[start..]));
    }
    result
}

/// `[文字](URL)` と `?[文字](URL)` を、`replace(文字, silent)` の結果に置き換える。
fn replace_markdown_links(text: &str, replace: impl Fn(&str, &str, bool) -> String) -> String {
    let mut result = String::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        let silent = rest.starts_with("?[");
        let open = if silent { 2 } else { 1 };
        let link = (silent || rest.starts_with('['))
            .then(|| {
                let label_end = open + rest[open..].find("](")?;
                let label = &rest[open..label_end];
                let url_start = label_end + 2;
                let url = &rest[url_start..url_start + rest[url_start..].find(')')?];
                let valid = (url.starts_with("https://") || url.starts_with("http://"))
                    && !url.contains(char::is_whitespace)
                    && !label.contains(['\n', '[']);
                valid.then(|| (label, url, url_start + url.len() + 1))
            })
            .flatten();
        match link {
            Some((label, url, length)) => {
                result.push_str(&replace(label, url, silent));
                at += length;
            }
            None => {
                let ch = rest.chars().next().expect("at は文字境界");
                result.push(ch);
                at += ch.len_utf8();
            }
        }
    }
    result
}

/// 本文に出てくる順の URL（重複を除く）。コードと `<plain>` の中は除く。
fn urls(content: &str, include_silent: bool) -> Vec<String> {
    let mut result: Vec<String> = Vec::new();
    for (literal, part) in segments(content) {
        if literal {
            continue;
        }
        // `[文字](URL)` は行き先だけを残す（文字に書いた URL は行き先ではない）。
        let visible = replace_markdown_links(part, |_, url, silent| {
            if silent && !include_silent {
                " ".to_owned()
            } else {
                format!(" {url} ")
            }
        });
        let mut rest = visible.as_str();
        while let Some(index) = rest.find("http") {
            let candidate = &rest[index..];
            let Some(url) = crate::mfm::leading_url(candidate) else {
                rest = &candidate["http".len()..];
                continue;
            };
            if Url::parse(url).is_ok_and(|url| {
                url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
            }) && !result.iter().any(|seen| seen == url)
            {
                result.push(url.to_owned());
            }
            rest = &candidate[url.len()..];
        }
    }
    result
}

/// 投稿の下に埋め込むアプリ内リンク。本文の URL を先頭から [`MAX_PREVIEW_URLS`] 件まで数え、
/// サイレントリンク `?[文字](URL)` は除く。
pub fn embedded_links(content: &str, app_base_url: &str) -> Vec<(String, LinkTarget)> {
    urls(content, false)
        .into_iter()
        .take(MAX_PREVIEW_URLS)
        .filter_map(|url| parse_channel_link(&url, app_base_url).map(|target| (url, target)))
        .collect()
}

/// 本文に出てくる、開けるアプリ内リンクのすべて（サイレントリンクも含む）。
pub fn app_links(content: &str, app_base_url: &str) -> Vec<(String, LinkTarget)> {
    urls(content, true)
        .into_iter()
        .filter_map(|url| parse_channel_link(&url, app_base_url).map(|target| (url, target)))
        .collect()
}

/// 本文から取得するリンク先。埋め込みの対象を先に、続けて URL の文字そのものを書いたリンクを、
/// メッセージごとに数えて [`MAX_FETCHED_PER_BODY`] 件まで。`embedded` と `all` は、同じ本文の
/// [`embedded_links`] と [`app_links`]（描画のたびに本文を調べ直さないよう、呼ぶ側から渡す）。
pub fn fetch_targets(
    content: &str,
    app_base_url: &str,
    embedded: &[(String, LinkTarget)],
    all: &[(String, LinkTarget)],
) -> Vec<LinkTarget> {
    let mut result: Vec<LinkTarget> = Vec::new();
    let mut add = |links: &[(String, LinkTarget)]| {
        for (_, target) in links {
            let Some(message_id) = target.linked_message_id() else {
                continue;
            };
            if result.len() < MAX_FETCHED_PER_BODY
                && !result.iter().any(|seen| {
                    seen.channel_id == target.channel_id
                        && seen.linked_message_id() == Some(message_id)
                })
            {
                result.push(target.clone());
            }
        }
        result.len() < MAX_FETCHED_PER_BODY
    };
    // URL の文字そのものを書いたリンクを調べるのは本文の解析が要るので、ほかのリンクがあるときだけ。
    if add(embedded)
        && all
            .iter()
            .any(|(url, _)| !embedded.iter().any(|(shown, _)| shown == url))
    {
        add(&bare_links(content, app_base_url));
    }
    result
}

/// URL の文字そのものを書いたアプリ内リンク（短い表示にするもの）。
pub fn bare_links(content: &str, app_base_url: &str) -> Vec<(String, LinkTarget)> {
    crate::mfm::parse(content, None)
        .into_iter()
        .flat_map(|line| line.spans)
        .filter_map(|span| {
            let url = span.href.filter(|href| *href == span.content)?;
            let target = parse_channel_link(&url, app_base_url)?;
            Some((url, target))
        })
        .collect()
}

/// 本文を、埋め込みに出す短い素の文字列にする。装飾・リンク・コードの記法を外し、改行や連続する空白を
/// 1 つにまとめ、長いときは切って `…` を付ける。
pub fn excerpt(content: &str) -> String {
    // `[文字](URL)` は文字だけにする（MFM の解析は URL を後ろに添えるため）。
    let source: String = segments(content)
        .into_iter()
        .map(|(literal, part)| {
            if literal {
                part.to_owned()
            } else {
                replace_markdown_links(part, |label, _, _| label.to_owned())
            }
        })
        .collect();
    let mut text = String::new();
    for line in crate::mfm::parse(&source, None) {
        for (index, span) in line.spans.iter().enumerate() {
            // 引用の行の先頭の縦線は出さない。
            if index == 0 && span.content == "│ " && span.style.tone == crate::mfm::Tone::Quote {
                continue;
            }
            text.push_str(&span.content);
        }
        text.push(' ');
    }
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match collapsed.char_indices().nth(EXCERPT_MAX_CHARS) {
        Some((end, _)) => format!("{}…", collapsed[..end].trim_end()),
        None => collapsed,
    }
}

/// リンク先のメッセージの取得の状態。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkedFetch {
    /// 取得中。`stalled` は、始めてから時間がたっても終わらない。
    Loading {
        stalled: bool,
    },
    Found(Box<Message>),
    /// 取得できたが、その ID のメッセージがない。
    Missing,
    /// 再試行しても取得できない失敗（認証の期限切れ、権限の不足などの 4xx）。
    Failed,
}

/// 端末の暗号化チャンネルの記録を確かめたか。確かめ終わるまで、平文の本文は出さない。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionCheck {
    Done,
    Waiting,
    /// 待っても確かめられない。
    Stalled,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PreviewBody {
    Text(String),
    /// 暗号化されたメッセージ。本文は出さない。
    Encrypted,
    /// 連携カード・部品・App のブロックの投稿。`content` は出さない。
    App,
    /// コメントのない転送。
    Forwarded,
    Attachments(usize),
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessagePreview {
    pub author_id: String,
    pub created_at: DateTime,
    /// 返信なら、そのスレッドの親。
    pub parent_id: Option<String>,
    pub body: PreviewBody,
}

/// アプリ内リンクの行き先が、いま分かっていること。
#[derive(Debug, Clone, PartialEq)]
pub enum LinkState<'a> {
    Loading,
    /// 見られない。存在しない・削除済み・非公開で未参加を区別しない。
    Unavailable {
        message: bool,
    },
    /// 取得できなかった。見られるかは分からない。
    Failed,
    Channel {
        channel: &'a Channel,
        encrypted: bool,
    },
    Message {
        channel: &'a Channel,
        preview: MessagePreview,
    },
}

/// 暗号化チャンネルとして扱うか（docs/e2ee.md の「暗号化の格下げを防ぐ」）。`recorded` は端末が
/// 以前に `encrypted = true` を見た記録があるか、`has_ciphertext` は読み込んでいる行に暗号文の行があるか。
pub fn treats_as_encrypted(channel: &Channel, recorded: bool, has_ciphertext: bool) -> bool {
    channel.encrypted || recorded || has_ciphertext || channel.channel_name.starts_with("edm:")
}

/// リンク先の状態を決める。
///
/// - `channel` は見られるチャンネルの一覧の行。ないときは、一覧の同期 (`directory_ready`) が
///   終わるまで読み込み中とする。
/// - `encrypted` は暗号化チャンネルとして扱うか（docs/e2ee.md の「暗号化の格下げを防ぐ」。
///   サーバーの `encrypted` だけで決めない）。
pub fn resolve<'a>(
    target: &LinkTarget,
    channel: Option<&'a Channel>,
    directory_ready: bool,
    fetch: &LinkedFetch,
    encrypted: bool,
    check: EncryptionCheck,
) -> LinkState<'a> {
    let message_id = target.linked_message_id();
    let Some(channel) = channel.filter(|channel| channel.deleted_at.is_none()) else {
        return if directory_ready {
            LinkState::Unavailable {
                message: message_id.is_some(),
            }
        } else {
            LinkState::Loading
        };
    };
    let encrypted = encrypted || channel.encrypted;
    let Some(message_id) = message_id else {
        return LinkState::Channel { channel, encrypted };
    };
    let message = match fetch {
        LinkedFetch::Loading { stalled: false } => return LinkState::Loading,
        LinkedFetch::Loading { stalled: true } | LinkedFetch::Failed => return LinkState::Failed,
        LinkedFetch::Missing => return LinkState::Unavailable { message: true },
        LinkedFetch::Found(message) => message,
    };
    if message.id != message_id || message.channel_id != channel.id || message.deleted_at.is_some()
    {
        return LinkState::Unavailable { message: true };
    }
    // 暗号文のある行は、チャンネルの扱いがどうあっても暗号化されたメッセージにする。
    let body = if encrypted || message.ciphertext.is_some() {
        PreviewBody::Encrypted
    } else if replaces_content(message) {
        PreviewBody::App
    } else {
        let text = excerpt(&message.content);
        let attachments = message.display_attachments.as_ref().map_or(0, Vec::len);
        if !text.is_empty() {
            PreviewBody::Text(text)
        } else if is_forward(message) {
            PreviewBody::Forwarded
        } else if attachments > 0 {
            PreviewBody::Attachments(attachments)
        } else {
            PreviewBody::Empty
        }
    };
    if matches!(body, PreviewBody::Text(_)) {
        match check {
            EncryptionCheck::Done => {}
            EncryptionCheck::Waiting => return LinkState::Loading,
            EncryptionCheck::Stalled => return LinkState::Failed,
        }
    }
    LinkState::Message {
        channel,
        preview: MessagePreview {
            author_id: message.user_id.clone(),
            created_at: message.created_at,
            parent_id: message.parent_id.clone(),
            body,
        },
    }
}

fn is_forward(message: &Message) -> bool {
    message
        .forwarded_message_ids
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|ids| !ids.is_empty())
}

/// 本文（`content`）の代わりに、部品・App のカード・連携カードを描く投稿か（Web の `messageBodySource`）。
pub fn replaces_content(message: &Message) -> bool {
    if let Some(document) = message
        .components
        .as_deref()
        .filter(|json| !json.trim().is_empty())
        .and_then(zoo_components::Document::parse)
    {
        return document.hides_content();
    }
    let json = |raw: &Option<String>| {
        raw.as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
    };
    if json(&message.app_blocks)
        .as_ref()
        .and_then(Value::as_array)
        .is_some_and(|blocks| blocks.iter().any(|block| block["type"] == "card"))
    {
        return true;
    }
    // 連携カードは、1 枚か `cards` の配列で、どのカードにも題名がある。
    let Some(card) = json(&message.card).filter(Value::is_object) else {
        return false;
    };
    let cards = match card.get("cards") {
        Some(cards) => cards.as_array().cloned().unwrap_or_default(),
        None => vec![card],
    };
    !cards.is_empty()
        && cards.iter().all(|card| {
            card["title"]
                .as_str()
                .is_some_and(|title| !title.trim().is_empty())
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const APP: &str = "https://zli.chat";

    fn target(url: &str) -> Option<LinkTarget> {
        parse_channel_link(url, APP)
    }

    fn channel() -> Channel {
        serde_json::from_value(json!({
            "id": "general", "channel_name": "general", "description": "雑談",
            "is_private": false, "is_dm": false, "archive_at": null, "deleted_at": null,
        }))
        .unwrap()
    }

    fn message(extra: Value) -> Message {
        let mut value = json!({
            "id": "m1", "user_id": "alice", "channel_id": "general", "content": "**こんにちは**",
            "card": null, "created_at": "2026-11-23T07:30:00", "updated_at": null,
            "deleted_at": null, "parent_id": null,
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(value).unwrap()
    }

    fn link() -> LinkTarget {
        LinkTarget {
            channel_id: "general".into(),
            message_id: Some("m1".into()),
            thread_id: None,
        }
    }

    fn state(fetch: LinkedFetch, encrypted: bool, check: EncryptionCheck) -> &'static str {
        let channel = channel();
        match resolve(&link(), Some(&channel), true, &fetch, encrypted, check) {
            LinkState::Loading => "loading",
            LinkState::Failed => "failed",
            LinkState::Unavailable { .. } => "unavailable",
            LinkState::Channel { .. } => "channel",
            LinkState::Message { preview, .. } => match preview.body {
                PreviewBody::Text(_) => "text",
                PreviewBody::Encrypted => "encrypted",
                PreviewBody::App => "app",
                PreviewBody::Forwarded => "forwarded",
                PreviewBody::Attachments(_) => "attachments",
                PreviewBody::Empty => "empty",
            },
        }
    }

    fn found(extra: Value) -> LinkedFetch {
        LinkedFetch::Found(Box::new(message(extra)))
    }

    #[test]
    fn only_channel_pages_of_this_app_are_links_into_it() {
        let only = |channel: &str| LinkTarget {
            channel_id: channel.into(),
            message_id: None,
            thread_id: None,
        };
        assert_eq!(target("https://zli.chat/channels/c1"), Some(only("c1")));
        assert_eq!(
            target("https://ZLI.chat:443/channels/c1/"),
            Some(only("c1"))
        );
        assert_eq!(target("https://zli.chat/channels/a%20b"), Some(only("a b")));
        let reply = target("https://zli.chat/channels/c1?thread=m1&message=m2&x=1").unwrap();
        assert_eq!(reply.linked_message_id(), Some("m2"));
        assert_eq!(reply.thread_id.as_deref(), Some("m1"));
        let thread = target("https://zli.chat/channels/c1?thread=m1&message=").unwrap();
        assert_eq!(thread.linked_message_id(), Some("m1"));
        for other in [
            "http://zli.chat/channels/c1",
            "https://zli.chat:8443/channels/c1",
            "https://zli.chat.example/channels/c1",
            "https://user@zli.chat/channels/c1",
            "https://example.com/channels/c1",
            "https://zli.chat/channels",
            "https://zli.chat/channels/",
            "https://zli.chat/channels/c1/settings",
            "https://zli.chat/api/channels/c1",
            "ftp://zli.chat/channels/c1",
            "/channels/c1",
        ] {
            assert_eq!(target(other), None, "{other}");
        }
    }

    #[test]
    fn embeds_come_from_the_first_three_visible_urls() {
        let content = "https://zli.chat/channels/a `https://zli.chat/channels/code` \
            ?[静か](https://zli.chat/channels/quiet)\n```\nhttps://zli.chat/channels/fence\n```\n\
            <plain>https://zli.chat/channels/plain</plain> https://example.com/x と \
            [資料](https://zli.chat/channels/b)。 https://zli.chat/channels/c";
        let ids = |links: Vec<(String, LinkTarget)>| {
            links
                .into_iter()
                .map(|(_, target)| target.channel_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(embedded_links(content, APP)), ["a", "b"]);
        assert_eq!(ids(app_links(content, APP)), ["a", "quiet", "b", "c"]);
        assert_eq!(
            embedded_links(
                "https://zli.chat/channels/a https://zli.chat/channels/a",
                APP
            )
            .len(),
            1
        );
    }

    #[test]
    fn only_urls_written_as_text_are_shortened() {
        let content = "https://zli.chat/channels/a `https://zli.chat/channels/code` \
            [資料](https://zli.chat/channels/b) https://example.com/x";
        let bare: Vec<String> = bare_links(content, APP)
            .into_iter()
            .map(|(url, _)| url)
            .collect();
        assert_eq!(bare, ["https://zli.chat/channels/a"]);
    }

    #[test]
    fn encryption_does_not_rely_on_the_server_flag_alone() {
        let mut row = channel();
        assert!(!treats_as_encrypted(&row, false, false));
        assert!(treats_as_encrypted(&row, true, false));
        assert!(treats_as_encrypted(&row, false, true));
        row.channel_name = "edm:a:b".into();
        assert!(treats_as_encrypted(&row, false, false));
        row = channel();
        row.encrypted = true;
        assert!(treats_as_encrypted(&row, false, false));
    }

    #[test]
    fn fetch_states_survive_the_ipc() {
        for fetch in [
            LinkedFetch::Loading { stalled: true },
            found(json!({ "ciphertext": { "a": 1 }, "forwarded_message_ids": ["x"] })),
            LinkedFetch::Missing,
            LinkedFetch::Failed,
        ] {
            let text = serde_json::to_string(&fetch).unwrap();
            assert_eq!(serde_json::from_str::<LinkedFetch>(&text).unwrap(), fetch);
        }
    }

    #[test]
    fn urls_follow_how_the_body_is_shown() {
        let ids = |content: &str| {
            app_links(content, APP)
                .into_iter()
                .map(|(_, target)| target.channel_id)
                .collect::<Vec<_>>()
        };
        // 文字に書いた URL は行き先ではない。
        assert_eq!(
            ids("[https://zli.chat/channels/a](https://zli.chat/channels/b)"),
            ["b"]
        );
        // 閉じていないコードブロックは末尾まで。行の頭でなくなる閉じ方はない。
        assert!(ids("```\nhttps://zli.chat/channels/a").is_empty());
        assert!(ids("  ```rs\nhttps://zli.chat/channels/a\n本文```").is_empty());
        assert_eq!(ids("```\nx\n```\nhttps://zli.chat/channels/c"), ["c"]);
        assert_eq!(excerpt("前\n```\n**コード**"), "前 **コード**");
    }

    #[test]
    fn one_body_fetches_only_a_few_messages() {
        let link = |n: u32| format!("https://zli.chat/channels/c{n}?message=m{n}");
        let content = format!(
            "https://zli.chat/channels/only {0}と{0} https://example.com/x \
             詳しくは{1}。 {2} {3} {4}",
            link(1),
            link(2),
            link(3),
            link(4),
            link(5),
        );
        let ids: Vec<String> = fetch_targets(
            &content,
            APP,
            &embedded_links(&content, APP),
            &app_links(&content, APP),
        )
        .into_iter()
        .map(|target| target.channel_id)
        .collect();
        assert_eq!(ids, ["c1", "c2", "c3"]);
    }

    #[test]
    fn excerpts_drop_markup() {
        assert_eq!(
            excerpt(
                "**太字** [リンク](https://example.com/x) https://example.com\n```rs\ncode\n```\n\
                 <plain>**本文**</plain>\n> 引用\n<center>中央</center> $[x2 大きい] `a` @bob"
            ),
            "太字 リンク https://example.com code **本文** 引用 中央 大きい a @bob"
        );
        assert_eq!(excerpt(" \n "), "");
        assert_eq!(excerpt(&"あ".repeat(300)), format!("{}…", "あ".repeat(200)));
    }

    #[test]
    fn unlisted_channels_are_unavailable_only_after_the_list_synced() {
        let fetch = LinkedFetch::Loading { stalled: false };
        let check = EncryptionCheck::Done;
        assert_eq!(
            resolve(&link(), None, false, &fetch, false, check),
            LinkState::Loading
        );
        assert_eq!(
            resolve(&link(), None, true, &fetch, false, check),
            LinkState::Unavailable { message: true }
        );
        let channel = channel();
        let only = LinkTarget {
            channel_id: "general".into(),
            message_id: None,
            thread_id: None,
        };
        assert_eq!(
            resolve(&only, Some(&channel), true, &fetch, true, check),
            LinkState::Channel {
                channel: &channel,
                encrypted: true
            }
        );
    }

    #[test]
    fn fetch_states_differ_from_unavailable() {
        let done = EncryptionCheck::Done;
        assert_eq!(
            state(LinkedFetch::Loading { stalled: false }, false, done),
            "loading"
        );
        assert_eq!(
            state(LinkedFetch::Loading { stalled: true }, false, done),
            "failed"
        );
        assert_eq!(state(LinkedFetch::Failed, false, done), "failed");
        assert_eq!(state(LinkedFetch::Missing, false, done), "unavailable");
        let deleted = found(json!({ "deleted_at": "2026-11-24T00:00:00" }));
        assert_eq!(state(deleted, false, done), "unavailable");
        assert_eq!(
            state(found(json!({ "channel_id": "other" })), false, done),
            "unavailable"
        );
        assert_eq!(
            state(found(json!({ "id": "other" })), false, done),
            "unavailable"
        );
    }

    #[test]
    fn bodies_depend_on_the_kind_of_message() {
        let done = EncryptionCheck::Done;
        assert_eq!(state(found(json!({})), false, done), "text");
        assert_eq!(state(found(json!({})), true, done), "encrypted");
        assert_eq!(
            state(found(json!({ "ciphertext": {} })), false, done),
            "encrypted"
        );
        for card in [r#"{"title":"通知"}"#, r#"{"cards":[{"title":"通知"}]}"#] {
            assert_eq!(state(found(json!({ "card": card })), false, done), "app");
        }
        assert_eq!(state(found(json!({ "card": "{}" })), false, done), "text");
        let blocks = found(json!({ "app_blocks": r#"[{"type":"card"}]"# }));
        assert_eq!(state(blocks, false, done), "app");
        let hidden = r#"{"flags":32768,"components":[{"type":10,"content":"x"}]}"#;
        assert_eq!(
            state(found(json!({ "components": hidden })), false, done),
            "app"
        );
        let shown = r#"{"components":[{"type":10,"content":"x"}]}"#;
        let both = found(json!({ "components": shown, "card": r#"{"title":"通知"}"# }));
        assert_eq!(state(both, false, done), "text");
        let forward = found(json!({ "content": "", "forwarded_message_ids": ["x"] }));
        assert_eq!(state(forward, false, done), "forwarded");
        let files = found(json!({ "content": "", "display_attachments": [{
            "id": "a", "message_id": "m1", "uploader_id": "u", "channel_id": "general",
            "file_name": "a.png", "file_size": 1, "content_type": "image/png",
            "storage_key": "k", "created_at": "2026-11-23T07:30:00" }] }));
        assert_eq!(state(files, false, done), "attachments");
        assert_eq!(state(found(json!({ "content": "" })), false, done), "empty");
    }

    #[test]
    fn plaintext_waits_for_the_encryption_records() {
        assert_eq!(
            state(found(json!({})), false, EncryptionCheck::Waiting),
            "loading"
        );
        assert_eq!(
            state(found(json!({})), false, EncryptionCheck::Stalled),
            "failed"
        );
        // 本文を出さない種類は待たない。
        assert_eq!(
            state(found(json!({})), true, EncryptionCheck::Waiting),
            "encrypted"
        );
    }
}
