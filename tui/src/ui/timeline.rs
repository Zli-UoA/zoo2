//! メッセージ一覧を、幅に合わせて折り返した行の列にする。各メッセージが何行目から
//! 何行目までかを持つので、選択中のメッセージが見えるようにスクロールできる。

use std::{collections::HashMap, ops::Range};

use jiff::{Zoned, civil::DateTime, tz::TimeZone};
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::{
    electric::rows::Message,
    mfm,
    store::{ReactionGroup, Store},
};

const INDENT: usize = 2;
const SELECTED: Style = Style::new().bg(Color::Rgb(40, 44, 52));
const MARKER: Style = Style::new().fg(Color::Cyan);
const DIM: Style = Style::new().fg(Color::DarkGray);
const MINE: Style = Style::new().fg(Color::Cyan).bg(Color::Rgb(24, 48, 56));

pub struct Timeline {
    pub lines: Vec<Line<'static>>,
    /// メッセージ id と、それが占める行の範囲。
    pub ranges: Vec<(String, Range<usize>)>,
}

impl Timeline {
    pub fn range_of(&self, message_id: &str) -> Option<Range<usize>> {
        self.ranges
            .iter()
            .find(|(id, _)| id == message_id)
            .map(|(_, range)| range.clone())
    }
}

pub struct Context<'a> {
    pub store: &'a Store,
    pub me_name: &'a str,
    pub selected: Option<&'a str>,
    /// タイムラインでは返信数を出し、スレッドでは出さない。
    pub reply_counts: Option<&'a HashMap<String, usize>>,
    /// メッセージ ID ごとのリアクション。`Store::reaction_groups_by_message` で描画前に1回だけ作る。
    pub reactions: &'a HashMap<String, Vec<ReactionGroup>>,
}

pub fn build(messages: &[&Message], ctx: &Context, width: u16) -> Timeline {
    let width = usize::from(width).max(INDENT + 4);
    let mut timeline = Timeline {
        lines: Vec::new(),
        ranges: Vec::new(),
    };
    let mut last_date = None;
    for message in messages {
        let created = local(message.created_at);
        let date = created.as_ref().map(|zoned| zoned.date());
        if date.is_some() && date != last_date {
            if let Some(date) = date {
                timeline.lines.push(separator(&date.to_string(), width));
            }
            last_date = date;
        }

        let start = timeline.lines.len();
        let selected = ctx.selected == Some(message.id.as_str());
        for line in message_lines(message, created.as_ref(), ctx) {
            for wrapped in wrap(line, width - INDENT) {
                timeline.lines.push(gutter(wrapped, selected));
            }
        }
        timeline
            .ranges
            .push((message.id.clone(), start..timeline.lines.len()));
    }
    timeline
}

fn message_lines(message: &Message, created: Option<&Zoned>, ctx: &Context) -> Vec<Line<'static>> {
    let time = created
        .map(|zoned| zoned.strftime("%H:%M").to_string())
        .unwrap_or_default();
    let author = ctx.store.user(&message.user_id);
    let mut header = vec![
        Span::styled(time, DIM),
        Span::raw(" "),
        Span::styled(
            ctx.store.display_name(&message.user_id),
            Style::new().add_modifier(Modifier::BOLD),
        ),
    ];
    if author.is_some_and(|user| user.is_bot) {
        header.push(Span::styled(" [bot]", DIM));
    }
    if message.updated_at.is_some() {
        header.push(Span::styled(" (編集済み)", DIM));
    }

    let mut lines = vec![Line::from(header)];
    lines.extend(mfm::render(&message.content, Some(ctx.me_name)));

    let reactions = ctx
        .reactions
        .get(&message.id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !reactions.is_empty() {
        let mut spans = Vec::new();
        for group in reactions {
            let style = if group.mine { MINE } else { DIM };
            spans.push(Span::styled(
                format!(" {} {} ", reaction_label(&group.emoji), group.count),
                style,
            ));
            spans.push(Span::raw(" "));
        }
        lines.push(Line::from(spans));
    }

    if let Some(count) = ctx.reply_counts.and_then(|counts| counts.get(&message.id)) {
        lines.push(Line::from(Span::styled(
            format!("💬 返信 {count} 件"),
            Style::new().fg(Color::Cyan),
        )));
    }
    lines
}

/// Emoji Kitchen の合成絵文字 (`kitchen:<a>_<b>`) は画像なので、元の2つを並べて示す。
fn reaction_label(emoji: &str) -> String {
    match emoji.strip_prefix("kitchen:") {
        Some(pair) => pair.replacen('_', "+", 1),
        None => emoji.to_string(),
    }
}

/// 選択中のメッセージには左端に印を付け、背景色を変える。
fn gutter(line: Line<'static>, selected: bool) -> Line<'static> {
    let (marker, style) = if selected {
        (Span::styled("▌ ", MARKER), SELECTED)
    } else {
        (Span::raw("  "), Style::new())
    };
    let mut spans = vec![marker];
    spans.extend(line.spans);
    Line::from(spans).style(style)
}

fn separator(label: &str, width: usize) -> Line<'static> {
    let label = format!(" {label} ");
    let side = width.saturating_sub(label.len()) / 2;
    Line::from(Span::styled(
        format!("{}{label}{}", "─".repeat(side), "─".repeat(side)),
        DIM,
    ))
}

/// 時刻は UTC の naive 値で届くので、端末のタイムゾーンに直す。
pub fn local(at: DateTime) -> Option<Zoned> {
    at.to_zoned(TimeZone::UTC)
        .ok()
        .map(|zoned| zoned.with_time_zone(TimeZone::system()))
}

/// 文字の表示幅で折り返す。日本語は単語の区切りがないため、文字単位で折る。
/// ZWJ でつないだ絵文字などを途中で分けないよう、書記素クラスタを1文字として数える。
pub fn wrap(line: Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for span in line.spans {
        let mut chunk = String::new();
        for grapheme in span.content.graphemes(true) {
            let w = grapheme.width();
            if used + w > width && used > 0 {
                if !chunk.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut chunk), span.style));
                }
                lines.push(Line::from(std::mem::take(&mut current)).style(line.style));
                used = 0;
            }
            chunk.push_str(grapheme);
            used += w;
        }
        if !chunk.is_empty() {
            current.push(Span::styled(chunk, span.style));
        }
    }
    lines.push(Line::from(current).style(line.style));
    lines
}

/// `range` が高さ `height` の表示領域に入るよう、スクロール位置を調整する。
/// `range` が領域より長いときは、先頭から `inner` 行進めた位置を出す。`inner` は
/// 末尾が見える位置で止め、詰めた値も返す。
pub fn scroll_to(
    scroll: usize,
    range: Range<usize>,
    height: usize,
    inner: usize,
) -> (usize, usize) {
    if range.len() > height {
        let inner = inner.min(range.len() - height);
        return (range.start + inner, inner);
    }
    let scroll = if range.start < scroll {
        range.start
    } else if range.end > scroll + height {
        range.end - height
    } else {
        scroll
    };
    (scroll, 0)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::electric::{Batch, Change, ShapeId};

    #[test]
    fn keeps_grapheme_clusters_together() {
        let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
        let line = Line::from(format!("aaaa{family}b"));
        let wrapped: Vec<String> = wrap(line, 6).iter().map(|l| l.to_string()).collect();
        assert_eq!(wrapped, [format!("aaaa{family}"), "b".to_string()]);

        let thumbs = "👍🏽";
        let line = Line::from(format!("aaaaa{thumbs}"));
        let wrapped: Vec<String> = wrap(line, 6).iter().map(|l| l.to_string()).collect();
        assert_eq!(wrapped, ["aaaaa".to_string(), thumbs.to_string()]);
    }

    #[test]
    fn wraps_by_display_width() {
        let line = Line::from(vec![Span::raw("ab"), Span::styled("あいう", DIM)]);
        let wrapped: Vec<String> = wrap(line, 4).iter().map(|l| l.to_string()).collect();
        assert_eq!(wrapped, ["abあ", "いう"]);
        assert_eq!(wrap(Line::from(""), 4).len(), 1);
    }

    #[test]
    fn scroll_keeps_range_visible() {
        assert_eq!(scroll_to(10, 2..4, 5, 0), (2, 0));
        assert_eq!(scroll_to(0, 6..8, 5, 0), (3, 0));
        assert_eq!(scroll_to(3, 4..6, 5, 0), (3, 0));
        assert_eq!(scroll_to(0, 6..20, 5, 0), (6, 0));
        // 長いメッセージの中は末尾が見える位置までしか進まない。
        assert_eq!(scroll_to(0, 6..20, 5, 4), (10, 4));
        assert_eq!(scroll_to(0, 6..20, 5, 99), (15, 9));
        // 収まるメッセージでは内側の位置を使わない。
        assert_eq!(scroll_to(3, 4..6, 5, 7), (3, 0));
    }

    #[test]
    fn builds_ranges_with_date_separators_and_reply_counts() {
        let mut store = Store::default();
        let row = |id: &str, at: &str| Change::Upsert {
            key: id.into(),
            row: json!({ "id": id, "user_id": "u1", "channel_id": "c1", "content": "line1\nline2",
                         "created_at": at })
            .as_object()
            .unwrap()
            .clone(),
        };
        store.apply(
            &ShapeId::Messages("c1".into()),
            Batch {
                changes: vec![
                    row("m1", "2026-11-01 00:00:00"),
                    row("m2", "2026-11-03 00:00:00"),
                ],
                ..Default::default()
            },
        );
        let messages = store.root_messages("c1");
        let counts = HashMap::from([("m2".to_string(), 3)]);
        let ctx = Context {
            store: &store,
            me_name: "alice",
            selected: Some("m2"),
            reply_counts: Some(&counts),
            reactions: &HashMap::new(),
        };
        let timeline = build(&messages, &ctx, 40);

        // 区切り + (ヘッダー + 本文2行) / 区切り + (ヘッダー + 本文2行 + 返信数)
        assert_eq!(timeline.range_of("m1"), Some(1..4));
        assert_eq!(timeline.range_of("m2"), Some(5..9));
        assert!(timeline.lines[8].to_string().contains("返信 3 件"));
        assert!(timeline.lines[5].to_string().starts_with("▌ "));
        assert!(timeline.lines[1].to_string().starts_with("  "));
    }
}
