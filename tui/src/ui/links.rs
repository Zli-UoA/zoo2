//! 本文のアプリ内リンクの表示（docs/mfm.md の「チャンネル・メッセージのリンクの表示」）。リンク先の
//! 取得は `client-core` の `electric::LinkedCache`。

use std::{cell::RefCell, collections::HashMap};

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use zoo2_client_core::{
    links::{LinkState, LinkTarget, MessagePreview, PreviewBody},
    store::Store,
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

/// チャンネルごとの、読み込んでいる行に暗号文の行があるかの結果（1 回の描画の間だけ使う）。
pub type CiphertextRows = RefCell<HashMap<String, bool>>;

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
        format!("#{}", channel.channel_name)
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
    fn views_do_not_reveal_unavailable_destinations() {
        let store = Store::default();
        let view = view(&LinkState::Unavailable { message: true }, &store);
        assert_eq!(view.inline, None);
        assert_eq!(view.header[0].to_string(), "メッセージを表示できません");
        assert!(view.body.is_none());
    }
}
