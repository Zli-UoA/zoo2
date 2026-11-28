//! 本文のアプリ内リンクの表示（docs/mfm.md の「チャンネル・メッセージのリンクの表示」）。リンク先の
//! 取得は `client-core` の `electric::LinkedCache`。

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use zoo2_client_core::{
    links::{self, LinkState, LinkTarget, MessagePreview, PreviewBody},
    store::Store,
    text::sanitize_display,
};

use super::timeline::{DIM, local};
use crate::electric::rows::Channel;

/// 本文中の短い表示と、投稿の下の埋め込みの内容。
#[derive(Debug, Clone, Default)]
pub struct LinkView {
    /// URL の文字そのものを書いたリンクの代わりに出す短い表示。行き先が分かったときだけ。
    pub inline: Option<String>,
    pub header: Vec<Line<'static>>,
    /// 本文の要約や説明。表示は `body_lines` 行まで。
    pub body: Option<Line<'static>>,
    pub body_lines: usize,
}

/// 1 回の描画の間だけ使う、リンクの表示の覚え書き（次の描画では作り直す）。
#[derive(Default)]
pub struct DrawCache {
    /// チャンネルごとの、読み込んでいる行に暗号文の行があるか。
    pub ciphertext: RefCell<HashMap<String, bool>>,
    /// リンク先ごとの表示。
    pub views: RefCell<HashMap<LinkTarget, LinkView>>,
}

/// 1 つの本文を調べて分かる、アプリ内リンク。
#[derive(Debug, Default)]
pub struct Parsed {
    /// 本文のアプリ内リンクのすべて。
    pub all: Vec<(String, LinkTarget)>,
    /// 投稿の下に埋め込むもの。
    pub embedded: Vec<(String, LinkTarget)>,
    /// 取得するもの。
    pub targets: Vec<LinkTarget>,
}

/// 本文ごとの [`Parsed`]。タイムラインは描画のたびに読み込んだ全部の投稿を組み立てるので、描画を
/// またいで覚え、同じ本文を調べ直さない。接続先は変わらないものとする。
///
/// 直前の描画で使った本文だけを次の描画へ引き継ぐので、覚える量は読み込んでいる本文の分で収まり、
/// 履歴がどれだけ長くても、調べるのは新しく出た本文だけになる。
#[derive(Default)]
pub struct ParsedLinks {
    /// いまの描画の番号。
    frame: u64,
    /// 本文ごとの、最後に使った描画の番号と結果。
    entries: RefCell<HashMap<String, (u64, Arc<Parsed>)>>,
}

impl ParsedLinks {
    /// 描画を始めるときに呼び、直前の描画で使わなかった本文を捨てる。直前の描画で本文を 1 つも
    /// 使っていなければ、そのまま残す。
    pub fn next_frame(&mut self) {
        let frame = self.frame;
        let entries = self.entries.get_mut();
        if entries.values().any(|(used, _)| *used == frame) {
            entries.retain(|_, (used, _)| *used == frame);
            self.frame += 1;
        }
    }

    pub fn get(&self, body: &str, app_base_url: &str) -> Arc<Parsed> {
        if let Some((used, parsed)) = self.entries.borrow_mut().get_mut(body) {
            *used = self.frame;
            return parsed.clone();
        }
        let parsed = Arc::new(parse(body, app_base_url));
        self.entries
            .borrow_mut()
            .insert(body.to_owned(), (self.frame, parsed.clone()));
        parsed
    }
}

fn parse(body: &str, app_base_url: &str) -> Parsed {
    let all = links::app_links(body, app_base_url);
    if all.is_empty() {
        return Parsed::default();
    }
    let embedded = links::embedded_links(body, app_base_url);
    Parsed {
        targets: links::fetch_targets(body, app_base_url, &embedded, &all),
        all,
        embedded,
    }
}

/// 1 つの本文のアプリ内リンクの表示と、取得の対象。
#[derive(Debug, Clone, Default)]
pub struct BodyLinks {
    /// URL ごとの短い表示。
    pub inline: HashMap<String, String>,
    /// 投稿の下の埋め込み（本文の URL の順）。
    pub embeds: Vec<LinkView>,
    pub targets: Vec<LinkTarget>,
}

/// チャンネルの呼び名。DM は名前が内部の識別子なので、総称を出す。
pub fn channel_name(channel: &Channel) -> String {
    if channel.is_dm {
        "ダイレクトメッセージ".to_owned()
    } else {
        format!("#{}", sanitize_display(&channel.channel_name))
    }
}

/// 埋め込みに添える日時。「11月23日 16:30」の形で、今年でなければ年も付ける。
fn timestamp(preview: &MessagePreview) -> String {
    let Some(time) = local(preview.created_at) else {
        return String::new();
    };
    let format = if time.year() == jiff::Zoned::now().year() {
        "%-m月%-d日 %H:%M"
    } else {
        "%Y年%-m月%-d日 %H:%M"
    };
    time.strftime(format).to_string()
}

pub fn view(state: &LinkState<'_>, store: &Store) -> LinkView {
    let note = |text: String| Line::styled(text, DIM);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    match state {
        LinkState::Loading => LinkView {
            header: vec![note("リンク先を読み込み中…".into())],
            ..Default::default()
        },
        LinkState::Failed => LinkView {
            header: vec![note("リンク先を読み込めませんでした".into())],
            ..Default::default()
        },
        // 理由は区別せず、チャンネル名と本文を出さない。
        LinkState::Unavailable { message } => LinkView {
            header: vec![note(if *message {
                "メッセージを表示できません".into()
            } else {
                "チャンネルを表示できません".into()
            })],
            ..Default::default()
        },
        LinkState::Channel { channel, encrypted } => {
            let name = channel_name(channel);
            let mut header = vec![Line::styled(name.clone(), bold)];
            if *encrypted {
                header.push(note("🔒 暗号化チャンネル".into()));
            } else if channel.is_private && !channel.is_dm {
                header.push(note("🔒 非公開チャンネル".into()));
            }
            LinkView {
                inline: Some(name),
                header,
                body: channel
                    .description
                    .as_deref()
                    .map(zoo2_client_core::links::excerpt)
                    .filter(|text| !text.is_empty())
                    .map(note),
                body_lines: 2,
            }
        }
        LinkState::Message { channel, preview } => {
            let author = store.display_name(&preview.author_id);
            let name = channel_name(channel);
            let place = if preview.parent_id.is_some() {
                format!("{name} 内のスレッド")
            } else {
                name.clone()
            };
            let body = match &preview.body {
                PreviewBody::Text(text) => Some(Line::from(text.clone())),
                PreviewBody::Encrypted => {
                    Some(note("🔒 このメッセージは暗号化されています".into()))
                }
                PreviewBody::App => Some(note("Appのメッセージ".into())),
                PreviewBody::Forwarded => Some(note("メッセージを転送しました".into())),
                PreviewBody::Attachments(count) => {
                    Some(note(format!("📎 添付ファイル {count} 件")))
                }
                PreviewBody::Empty => None,
            };
            LinkView {
                inline: Some(format!("💬 {author}さん: {name}")),
                header: vec![
                    Line::from(vec![
                        Span::styled(author, bold),
                        Span::raw("  "),
                        Span::styled(timestamp(preview), DIM),
                    ]),
                    note(place),
                ],
                body,
                body_lines: 3,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_bodies_are_kept_while_draws_use_them() {
        let mut cache = ParsedLinks::default();
        let get = |cache: &ParsedLinks, body: &str| cache.get(body, "https://zli.chat");
        let bodies: Vec<String> = (0..50_000).map(|n| n.to_string()).collect();
        let first: Vec<_> = bodies.iter().map(|body| get(&cache, body)).collect();
        // 毎回同じ本文を使う長い履歴は、件数によらず調べ直さない。
        for _ in 0..2 {
            cache.next_frame();
            for (body, parsed) in bodies.iter().zip(&first) {
                assert!(Arc::ptr_eq(parsed, &get(&cache, body)));
            }
        }
        cache.next_frame();
        get(&cache, "0");
        cache.next_frame();
        // 直前の描画で使わなかった本文は捨てる。
        assert_eq!(cache.entries.borrow().len(), 1);
        // 本文を使わない描画をはさんでも、残した本文は調べ直さない。
        cache.next_frame();
        cache.next_frame();
        assert!(Arc::ptr_eq(&first[0], &get(&cache, "0")));
        assert!(!Arc::ptr_eq(&first[1], &get(&cache, "1")));
    }

    #[test]
    fn views_do_not_reveal_unavailable_destinations() {
        let store = Store::default();
        let view = view(&LinkState::Unavailable { message: true }, &store);
        assert_eq!(view.inline, None);
        assert_eq!(view.header[0].to_string(), "メッセージを表示できません");
        assert!(view.body.is_none());
    }
}
