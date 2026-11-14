//! 画面の描画。左にチャンネル一覧、中央にタイムライン、右にスレッド (開いたときだけ)、
//! 下に入力欄と状態表示を置く。

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use super::{
    app::{App, Focus, Mode, SidebarItem, Target},
    encryption::Screen,
    timeline::{self, Context},
};
use crate::e2ee::reauth;
use crate::e2ee::setup::{E2eeStatus, LockReason};
use crate::electric::ShapeId;

const ACCENT: Color = Color::Cyan;
const DIM: Style = Style::new().fg(Color::DarkGray);

pub fn draw(frame: &mut Frame, app: &mut App) {
    let composer_height = (app.composer.lines().len() as u16).clamp(1, 6) + 2;
    let [main, composer, status] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(composer_height),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let sidebar_width = (main.width / 4).clamp(18, 32);
    let [sidebar, rest] =
        Layout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(20)]).areas(main);
    draw_sidebar(frame, app, sidebar);
    if app.thread.is_some() {
        let [messages, thread] =
            Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
                .areas(rest);
        draw_messages(frame, app, messages);
        draw_thread(frame, app, thread);
    } else {
        draw_messages(frame, app, rest);
    }
    draw_composer(frame, app, composer);
    draw_status(frame, app, status);

    match &app.mode {
        Mode::Help => draw_help(frame),
        Mode::ConfirmDelete { .. } => draw_popup(
            frame,
            "削除の確認",
            vec![
                Line::from("このメッセージを削除しますか?"),
                Line::from(""),
                Line::from("y: 削除 / その他: やめる"),
            ],
        ),
        Mode::Encryption => draw_encryption(frame, app),
        _ => {}
    }
}

fn pane(title: Line<'static>, focused: bool) -> Block<'static> {
    let style = if focused {
        Style::new().fg(ACCENT)
    } else {
        DIM
    };
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(style)
        .title(title)
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, area: Rect) {
    let matches = match &app.mode {
        Mode::Search(query) => Some(app.search_matches(query)),
        _ => None,
    };
    let sidebar_items = app.sidebar_items();
    let items: Vec<ListItem> = sidebar_items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            if let SidebarItem::Thread { id, channel_id } = item {
                let thread = app.threads().find(|thread| thread.id == *id);
                let archived = app.thread_is_archived(channel_id, id);
                let content = app.thread_content(channel_id, id)?;
                let name = content.split_whitespace().collect::<Vec<_>>().join(" ");
                let name = if name.is_empty() {
                    "無題のスレッド"
                } else {
                    &name
                };
                let selected = app
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.root_id == *id);
                let style = if selected {
                    Style::new().fg(ACCENT)
                } else if archived {
                    DIM
                } else {
                    Style::new()
                };
                let style = if matches
                    .as_ref()
                    .is_some_and(|matches| !matches.contains(&index))
                {
                    style.add_modifier(Modifier::DIM)
                } else {
                    style
                };
                let marker = if archived { "▣" } else { "↳" };
                return Some(ListItem::new(Line::styled(
                    format!(
                        "    {marker} {name} ({})",
                        thread.map(|thread| thread.reply_count).unwrap_or_else(|| {
                            app.store
                                .reply_counts(channel_id)
                                .get(id)
                                .copied()
                                .unwrap_or(0) as u64
                        })
                    ),
                    style,
                )));
            }
            if let SidebarItem::MoreThreads { archived } = item {
                return Some(ListItem::new(
                    if app.thread_failed_append[usize::from(*archived)].is_some() {
                        if *archived {
                            "  履歴の取得を再試行"
                        } else {
                            "  スレッドの取得を再試行"
                        }
                    } else if *archived {
                        "  履歴をさらに表示"
                    } else {
                        "  スレッドをさらに表示"
                    },
                ));
            }
            let SidebarItem::Channel(id) = item else {
                return None;
            };
            let channel = app.store.channel(id)?;
            let member = app.store.is_member(&channel.id, &app.me.id);
            let open = app.current.as_deref() == Some(channel.id.as_str());
            let mut style = if member { Style::new() } else { DIM };
            if open {
                style = style.add_modifier(Modifier::BOLD);
            }
            if matches
                .as_ref()
                .is_some_and(|matches| !matches.contains(&index))
            {
                style = style.add_modifier(Modifier::DIM);
            }
            let unread = app.unread.contains(&channel.id);
            if unread {
                style = style.add_modifier(Modifier::BOLD);
            }
            let mut spans = vec![
                Span::styled(if unread { "● " } else { "  " }, Style::new().fg(ACCENT)),
                Span::styled(app.store.channel_label(channel, &app.me.id), style),
            ];
            if channel.is_private && !channel.is_dm {
                spans.push(Span::styled(" 🔒", DIM));
            }
            if channel.is_nsfw {
                spans.push(Span::styled(" NSFW", DIM));
            }
            if channel.archive_at.is_some() {
                spans.push(Span::styled(" (アーカイブ)", DIM));
            }
            Some(ListItem::new(Line::from(spans)))
        })
        .collect();

    let title = if app.store.is_synced(&ShapeId::Channels) {
        Line::from(" チャンネル ")
    } else {
        Line::from(" チャンネル (読み込み中) ")
    };
    let list = List::new(items)
        .block(pane(title, app.focus == Focus::Sidebar))
        .highlight_style(Style::new().bg(Color::Rgb(40, 44, 52)).fg(ACCENT));
    let mut state = ListState::default()
        .with_selected((!sidebar_items.is_empty()).then_some(app.sidebar_index));
    frame.render_stateful_widget(list, area, &mut state);
}

fn draw_messages(frame: &mut Frame, app: &mut App, area: Rect) {
    let Some(channel_id) = app.current.clone() else {
        let block = pane(Line::from(" zoo2 "), app.focus == Focus::Messages);
        let help = Paragraph::new(vec![
            Line::from(""),
            Line::from("  j/k でチャンネルを選び、Enter で開きます。"),
            Line::from("  ? でキー操作の一覧を表示します。"),
        ])
        .block(block);
        frame.render_widget(help, area);
        return;
    };

    let channel = app.store.channel(&channel_id);
    let mut title = vec![Span::raw(" ")];
    title.push(Span::styled(
        channel
            .map(|c| app.store.channel_label(c, &app.me.id))
            .unwrap_or_default(),
        Style::new().add_modifier(Modifier::BOLD),
    ));
    if let Some(description) = channel
        .and_then(|c| c.description.as_deref())
        .filter(|d| !d.is_empty())
    {
        title.push(Span::styled(format!(" — {description}"), DIM));
    }
    title.push(Span::raw(" "));
    let block = pane(Line::from(title), app.focus == Focus::Messages);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let shape = ShapeId::Messages(channel_id.clone());
    if !app.store.is_synced(&shape) {
        frame.render_widget(Paragraph::new(Line::styled("読み込み中…", DIM)), inner);
        return;
    }

    let messages = app.store.root_messages(&channel_id);
    if messages.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled("まだメッセージがありません", DIM)),
            inner,
        );
        return;
    }
    let counts = app.store.reply_counts(&channel_id);
    let reactions = app
        .store
        .reaction_groups_by_message(&channel_id, &app.me.id);
    let ctx = Context {
        store: &app.store,
        me_name: &app.me.user_name,
        selected: app.selected.as_deref(),
        reply_counts: Some(&counts),
        reactions: &reactions,
    };
    let built = timeline::build(&messages, &ctx, inner.width);
    let height = usize::from(inner.height);
    let (scroll, inner_scroll) = match app.selected.as_deref().and_then(|id| built.range_of(id)) {
        Some(range) => timeline::scroll_to(app.scroll, range, height, app.inner_scroll),
        // 未選択なら最新を表示し続ける。
        None => (built.lines.len().saturating_sub(height), 0),
    };
    app.scroll = scroll;
    app.inner_scroll = inner_scroll;
    app.newest_shown = built
        .ranges
        .iter()
        .rfind(|(_, range)| range.start < scroll + height)
        .map(|(id, _)| (channel_id.clone(), id.clone()));
    let visible: Vec<Line> = built.lines.into_iter().skip(scroll).take(height).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

fn draw_thread(frame: &mut Frame, app: &mut App, area: Rect) {
    let (Some(channel_id), Some(thread)) = (app.current.clone(), app.thread.as_ref()) else {
        return;
    };
    let archived = app.thread_is_archived(&channel_id, &thread.root_id);
    let title = if archived {
        " スレッド (アーカイブ / u:再開) "
    } else {
        " スレッド "
    };
    let block = pane(Line::from(title), app.focus == Focus::Thread);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(root) = app.store.message(&channel_id, &thread.root_id) else {
        frame.render_widget(
            Paragraph::new(Line::styled(
                if app.store.is_synced(&ShapeId::Messages(channel_id.clone())) {
                    "元のメッセージは削除されました"
                } else {
                    "読み込み中…"
                },
                DIM,
            )),
            inner,
        );
        return;
    };
    let mut messages = vec![root];
    messages.extend(app.store.replies(&channel_id, &thread.root_id));
    let reactions = app
        .store
        .reaction_groups_by_message(&channel_id, &app.me.id);
    let ctx = Context {
        store: &app.store,
        me_name: &app.me.user_name,
        selected: thread.selected.as_deref(),
        reply_counts: None,
        reactions: &reactions,
    };
    let built = timeline::build(&messages, &ctx, inner.width);
    let height = usize::from(inner.height);
    let (scroll, inner_scroll) = match thread.selected.as_deref().and_then(|id| built.range_of(id))
    {
        Some(range) => timeline::scroll_to(thread.scroll, range, height, thread.inner_scroll),
        None => (built.lines.len().saturating_sub(height), 0),
    };
    if let Some(thread) = app.thread.as_mut() {
        thread.scroll = scroll;
        thread.inner_scroll = inner_scroll;
    }
    let visible: Vec<Line> = built.lines.into_iter().skip(scroll).take(height).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

fn draw_composer(frame: &mut Frame, app: &mut App, area: Rect) {
    let insert = app.mode == Mode::Insert;
    let target = match &app.target {
        Target::Channel => app
            .current
            .as_deref()
            .and_then(|id| app.store.channel(id))
            .map(|channel| format!(" {} へ送信 ", app.store.channel_label(channel, &app.me.id)))
            .unwrap_or_else(|| " 入力 ".to_string()),
        Target::Reply(_) => " スレッドに返信 ".to_string(),
        Target::Edit(_) => " メッセージを編集 ".to_string(),
    };
    let hint = if insert {
        " Enter: 送信 / Alt+Enter: 改行 / Esc: 戻る "
    } else {
        " i: 書く "
    };
    let mut block =
        pane(Line::from(target), insert).title_bottom(Line::styled(hint, DIM).right_aligned());
    if let Some(typing) = typing_line(app) {
        block = block.title_bottom(Line::styled(typing, Style::new().fg(ACCENT)).left_aligned());
    }
    app.composer.set_block(block);
    let cursor = if insert {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        Style::new()
    };
    app.composer.set_cursor_style(cursor);
    frame.render_widget(&app.composer, area);
}

/// 入力欄の送り先 (タイムラインかスレッド) で入力中の人。
fn typing_line(app: &App) -> Option<String> {
    let scope = match &app.target {
        Target::Reply(root_id) => Some(root_id.clone()),
        _ => None,
    };
    let users = app.typing.get(&scope).filter(|users| !users.is_empty())?;
    let names: Vec<&str> = users
        .iter()
        .map(|user| user.display_name.as_str())
        .collect();
    Some(match names.as_slice() {
        [one] => format!(" {one} が入力中… "),
        [a, b] => format!(" {a}、{b} が入力中… "),
        _ => format!(" {} 人が入力中… ", names.len()),
    })
}

fn draw_status(frame: &mut Frame, app: &App, area: Rect) {
    let line = if let Mode::React { input, .. } = &app.mode {
        Line::from(vec![
            Span::styled(" リアクション: ", Style::new().fg(ACCENT)),
            Span::raw(input.clone()),
            Span::styled("▏", Style::new().fg(ACCENT)),
            Span::styled(
                "  例: thumbsup, :tada:, 👍  Enter: 付ける/外す / Esc: やめる",
                DIM,
            ),
        ])
    } else if let Mode::Search(query) = &app.mode {
        Line::from(vec![
            Span::styled(" 検索: ", Style::new().fg(ACCENT)),
            Span::raw(query.clone()),
            Span::styled("▏", Style::new().fg(ACCENT)),
            Span::styled("  Enter: 開く / Esc: やめる", DIM),
        ])
    } else if let Some((notice, _)) = &app.notice {
        Line::styled(format!(" {notice}"), Style::new().fg(Color::Yellow))
    } else {
        let mut spans = vec![Span::styled(
            format!(" {} (@{})", app.me.display_name, app.me.user_name),
            DIM,
        )];
        if app.store.invalid_rows > 0 {
            spans.push(Span::styled(
                format!("  読めない行: {}", app.store.invalid_rows),
                Style::new().fg(Color::Red),
            ));
        }
        if let Some(version) = &app.update_available {
            spans.push(Span::styled(
                format!("  新しい版 v{version} があります"),
                Style::new().fg(Color::Green),
            ));
        }
        match app.encryption.status {
            Some(E2eeStatus::NotSetUp) => spans.push(Span::styled(
                "  暗号化: 未設定 (E)",
                Style::new().fg(Color::Yellow),
            )),
            Some(E2eeStatus::Locked(_)) => spans.push(Span::styled(
                "  暗号化: 解錠が必要 (E)",
                Style::new().fg(Color::Yellow),
            )),
            _ => {}
        }
        spans.push(Span::styled("  ?: ヘルプ  q: 終了", DIM));
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), area);
}

const HELP: &[(&str, &str)] = &[
    ("Tab / Shift+Tab", "ペインを移動"),
    ("j k / ↓ ↑", "選択を移動"),
    ("g / G", "先頭 / 末尾へ"),
    ("Ctrl+D / Ctrl+U", "長いメッセージの中を下 / 上へ"),
    ("Enter", "チャンネルを開く / スレッドを開く"),
    ("Esc", "選択を外す / スレッドを閉じる"),
    ("i", "書く (スレッドでは返信)"),
    ("e", "自分のメッセージを編集"),
    ("d", "自分のメッセージを削除"),
    ("r", "リアクションを付ける / 外す"),
    ("J / L", "チャンネルに参加 / 退出"),
    ("/", "チャンネル・スレッドを検索"),
    ("u", "選択中のスレッドを再開"),
    ("R", "送れなかった本文を入力欄に戻す"),
    ("E", "暗号化（初回設定・解錠）"),
    ("q / Ctrl+C", "終了"),
];

fn draw_help(frame: &mut Frame) {
    let lines = HELP
        .iter()
        .map(|(key, action)| {
            Line::from(vec![
                Span::styled(format!("{key:<16}"), Style::new().fg(ACCENT)),
                Span::raw(*action),
            ])
        })
        .collect();
    draw_popup(frame, "キー操作", lines);
}

/// 暗号化の画面（#360）。
fn draw_encryption(frame: &mut Frame, app: &mut App) {
    let encryption = &app.encryption;
    let key = |text: &str| Span::styled(text.to_owned(), Style::new().fg(ACCENT));
    let warn = |text: String| Line::styled(text, Style::new().fg(Color::Yellow));
    let error = |text: String| Line::styled(text, Style::new().fg(Color::Red));
    let mut lines: Vec<Line<'static>> = Vec::new();
    match &encryption.screen {
        Screen::Loading => lines.push(Line::from("状態を読んでいます…")),
        Screen::Unavailable(message) => {
            lines.push(error(message.clone()));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("Enter"),
                Span::raw(": 読み直す / "),
                key("Esc"),
                Span::raw(": 閉じる"),
            ]));
        }
        Screen::NotSetUp => {
            lines.push(Line::from(
                "暗号化チャンネルを使うには、最初に鍵を作ります。",
            ));
            lines.push(Line::from(
                "リカバリーキーを表示するので、控えてください。新しい端末の解錠に使います。",
            ));
            lines.push(Line::from(
                "保存の直前に、ブラウザーでログインし直します（再認証）。",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("Enter"),
                Span::raw(": 初回設定を始める / "),
                key("Esc"),
                Span::raw(": 閉じる"),
            ]));
        }
        Screen::RecoveryKey(recovery_key) => {
            lines.push(Line::from(
                "リカバリーキー（紙やパスワードマネージャーに控えてください）:",
            ));
            lines.push(Line::from(""));
            lines.push(Line::styled(
                format!("  {}", recovery_key.as_str()),
                Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(
                "このキーをなくし、ほかに解錠できる端末もないと、暗号化したメッセージを読めなくなります。",
            ));
            lines.push(Line::from(
                "この画面を閉じると、このキーは二度と表示しません。",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("y"),
                Span::raw(": 控えたので保存する / "),
                key("Esc"),
                Span::raw(": やめる"),
            ]));
        }
        Screen::Reauth { url, port } => {
            lines.push(Line::from(
                "ブラウザーでログインし直してください（再認証）。",
            ));
            match (url, port) {
                (Some(url), Some(port)) => {
                    if !encryption.over_ssh {
                        lines.push(Line::from(
                            "ブラウザーが開かないときは、次の URL を開いてください。",
                        ));
                    }
                    lines.push(Line::from(""));
                    lines.push(Line::styled(url.clone(), Style::new().fg(ACCENT)));
                    lines.push(Line::from(""));
                    if encryption.over_ssh {
                        for line in reauth::ssh_guidance(*port) {
                            lines.push(warn(line));
                        }
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::styled(
                        format!(
                            "戻り先: http://127.0.0.1:{port}/callback（この端末で待っています）"
                        ),
                        DIM,
                    ));
                }
                _ => lines.push(Line::from("準備しています…")),
            }
            lines.push(Line::from(vec![
                key("Esc"),
                Span::raw(": 再認証をやめる（鍵はそのまま）"),
            ]));
        }
        Screen::Working(message) => lines.push(Line::from(*message)),
        Screen::Locked {
            reason,
            input,
            error: message,
        } => {
            lines.push(Line::from(match reason {
                LockReason::New => "この端末はまだ解錠していません。",
                LockReason::Resume => "初回設定が途中です。リカバリーキーで開け直して続けます。",
                LockReason::Mismatch => "鍵が変わりました。リカバリーキーで解錠し直してください。",
            }));
            lines.push(Line::from(
                "リカバリーキーを入力してください（空白は無視します）。",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::raw("> "),
                Span::raw(input.as_str().to_owned()),
                Span::styled("▏", Style::new().fg(ACCENT)),
            ]));
            if let Some(message) = message {
                lines.push(Line::from(""));
                lines.push(error(message.clone()));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("Enter"),
                Span::raw(": 解錠する / "),
                key("Ctrl+U"),
                Span::raw(": 消す / "),
                key("Esc"),
                Span::raw(": 閉じる"),
            ]));
        }
        Screen::NeedsReauth => {
            lines.push(Line::from(
                "解錠しました。初回設定の続きを保存するには、再認証が必要です。",
            ));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("Enter"),
                Span::raw(": 再認証して保存する / "),
                key("Esc"),
                Span::raw(": 閉じる"),
            ]));
        }
        Screen::SaveFailed { message, retry } => {
            lines.push(error(message.clone()));
            lines.push(Line::from(""));
            if *retry {
                lines.push(Line::from(vec![
                    key("Enter"),
                    Span::raw(": 同じ鍵のまま、再認証して保存し直す / "),
                    key("Esc"),
                    Span::raw(": 閉じる"),
                ]));
            } else {
                lines.push(Line::from(vec![
                    key("Enter"),
                    Span::raw(": 読み直す / "),
                    key("Esc"),
                    Span::raw(": 閉じる"),
                ]));
            }
        }
        Screen::Stopped(message) => {
            lines.push(error(message.clone()));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("Enter"),
                Span::raw(": 読み直す / "),
                key("Esc"),
                Span::raw(": 閉じる"),
            ]));
        }
        Screen::Unlocked => {
            lines.push(Line::from("この端末は解錠済みです。"));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                key("Enter"),
                Span::raw(" / "),
                key("Esc"),
                Span::raw(": 閉じる"),
            ]));
        }
    }
    // リカバリーキーを全部表示できない（端末が小さい）ときは、キーを出さずに広げるよう求め、保存にも
    // 進めない（見えていないキーを控えたことにしない）。
    if matches!(app.encryption.screen, Screen::RecoveryKey(_)) {
        let width = ENCRYPTION_POPUP_WIDTH.min(frame.area().width);
        let fits = popup_rows(&lines, width) + 2 <= usize::from(frame.area().height);
        app.encryption.recovery_key_shown = fits;
        if !fits {
            lines = vec![
                error("端末が小さいため、リカバリーキーを全部表示できません。".into()),
                Line::from("端末を広げてください（広げるまで保存には進めません）。"),
                Line::from(vec![key("Esc"), Span::raw(": やめる")]),
            ];
        }
    }
    draw_popup_sized(frame, "暗号化", lines, ENCRYPTION_POPUP_WIDTH);
}

const ENCRYPTION_POPUP_WIDTH: u16 = 76;

/// 幅 `width` の枠に入れたときの行数（枠を除く）。単語の区切りで折り返すと 1 行増えうるので、
/// 折り返す行には 1 行足す。
fn popup_rows(lines: &[Line<'_>], width: u16) -> usize {
    let inner = usize::from(width.saturating_sub(2)).max(1);
    lines
        .iter()
        .map(|line| {
            let width = line.width();
            if width <= inner {
                1
            } else {
                width.div_ceil(inner) + 1
            }
        })
        .sum()
}

fn draw_popup(frame: &mut Frame, title: &str, lines: Vec<Line<'static>>) {
    draw_popup_sized(frame, title, lines, 56);
}

fn draw_popup_sized(frame: &mut Frame, title: &str, lines: Vec<Line<'static>>, width: u16) {
    let width = width.min(frame.area().width);
    // 折り返す行（再認証の URL など）も入る高さにする。
    let rows = popup_rows(&lines, width);
    let height = (rows as u16).saturating_add(2).min(frame.area().height);
    let area = Rect {
        x: frame.area().width.saturating_sub(width) / 2,
        y: frame.area().height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);
    let block = pane(Line::from(format!(" {title} ")), true);
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(block),
        area,
    );
}

#[cfg(test)]
mod tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::{Value, json};
    use tokio::sync::mpsc;
    use unicode_width::UnicodeWidthStr;

    use super::*;
    use crate::{
        api::{ApiClient, Me, ReadState, ThreadSummary, TypingUser},
        electric::{Batch, Change, ShapeEvent},
        http::Credentials,
        ui::{app::AppEvent, encryption::EncryptionEvent},
    };

    fn batch(rows: Vec<(&str, Value)>) -> Batch {
        Batch {
            reset: false,
            changes: rows
                .into_iter()
                .map(|(key, row)| Change::Upsert {
                    key: key.into(),
                    row: row.as_object().unwrap().clone(),
                })
                .collect(),
            up_to_date: true,
        }
    }

    fn app() -> App {
        // 接続できない宛先なので、シェイプの購読は再試行を続けるだけで画面には影響しない。
        let base = "http://127.0.0.1:9".to_string();
        let http = reqwest::Client::new();
        let credentials = Credentials::Dev("u1".into());
        let api = ApiClient::new(http.clone(), &base, credentials.clone());
        let me = Me {
            id: "u1".into(),
            user_name: "alice".into(),
            display_name: "Alice".into(),
        };
        let (shape_tx, _) = mpsc::unbounded_channel();
        let (app_tx, _) = mpsc::unbounded_channel();
        let mut app = App::new(api, http, base, credentials, me, shape_tx, app_tx);

        let feed =
            |app: &mut App, id: ShapeId, rows| app.on_shape(ShapeEvent::Batch(id, batch(rows)));
        feed(
            &mut app,
            ShapeId::Channels,
            vec![
                (
                    "c1",
                    json!({ "id": "c1", "channel_name": "general", "is_private": "f", "description": "雑談" }),
                ),
                (
                    "c2",
                    json!({ "id": "c2", "channel_name": "secret", "is_private": "t" }),
                ),
            ],
        );
        feed(
            &mut app,
            ShapeId::Users,
            vec![
                (
                    "u1",
                    json!({ "id": "u1", "user_name": "alice", "display_name": "Alice" }),
                ),
                (
                    "u2",
                    json!({ "id": "u2", "user_name": "bob", "display_name": "Bob", "is_bot": "t" }),
                ),
            ],
        );
        feed(
            &mut app,
            ShapeId::ChannelUsers,
            vec![("m", json!({ "user_id": "u1", "channel_id": "c1" }))],
        );
        feed(
            &mut app,
            ShapeId::Reactions("c1".into()),
            vec![
                (
                    "x1",
                    json!({ "id": "x1", "message_id": "m1", "user_id": "u1", "emoji": "👍",
                               "created_at": "2026-11-01 03:02:00" }),
                ),
                (
                    "x2",
                    json!({ "id": "x2", "message_id": "m1", "user_id": "u2", "emoji": "👍",
                               "created_at": "2026-11-01 03:03:00" }),
                ),
            ],
        );
        feed(
            &mut app,
            ShapeId::Messages("c1".into()),
            vec![
                (
                    "m1",
                    json!({ "id": "m1", "user_id": "u2", "channel_id": "c1",
                               "content": "**hello** @alice", "created_at": "2026-11-01 03:00:00" }),
                ),
                (
                    "r1",
                    json!({ "id": "r1", "user_id": "u1", "channel_id": "c1", "content": "返信です",
                               "created_at": "2026-11-01 03:01:00", "parent_id": "m1" }),
                ),
            ],
        );
        app
    }

    /// 画面を文字列にする。全角文字の後ろのセルは空白で埋まっているので読み飛ばす。
    fn render(app: &mut App) -> String {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut screen = String::new();
        for y in 0..buffer.area.height {
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                screen.push_str(symbol);
                x += symbol.width().max(1) as u16;
            }
            screen.push('\n');
        }
        screen
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_terminal_event(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    }

    #[tokio::test]
    async fn archives_open_from_replies_and_appear_only_while_viewing() {
        let mut app = app();
        app.archived_threads.push(ThreadSummary {
            id: "r1".into(),
            channel_id: "c1".into(),
            content: "Nested history".into(),
            thread_archive_at: Some("2026-11-10T00:00:00".into()),
            reply_count: 1,
        });
        let items = app.sidebar_items();
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.sidebar_items(), items);
        app.current = Some("c1".into());
        app.focus = Focus::Messages;
        app.selected = Some("r1".into());
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.thread.as_ref().unwrap().root_id, "r1");
        assert!(render(&mut app).contains("アーカイブ / u:再開"));
        assert!(render(&mut app).contains("返信です"));
        let selected = SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "r1".into(),
        };
        assert!(app.sidebar_items().contains(&selected));
        press(&mut app, KeyCode::Esc);
        assert!(app.thread.is_none());
        assert!(!app.sidebar_items().contains(&selected));
        assert!(app.archived_threads[0].thread_archive_at.is_some());
    }

    #[tokio::test]
    async fn reply_free_posts_do_not_become_temporary_threads() {
        let mut app = app();
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "empty",
                json!({"id":"empty","user_id":"u2","channel_id":"c1",
                "content":"No replies","created_at":"2026-11-02 00:00:00"}),
            )]),
        ));
        app.current = Some("c1".into());
        app.focus = Focus::Messages;
        app.selected = Some("empty".into());
        press(&mut app, KeyCode::Enter);
        let row = SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "empty".into(),
        };
        assert!(app.thread.is_some());
        assert!(!app.sidebar_items().contains(&row));
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "empty",
                json!({"id":"empty","user_id":"u2","channel_id":"c1",
                "content":"No replies","created_at":"2026-11-02 00:00:00",
                "thread_archive_at":"2026-11-10 00:00:00"}),
            )]),
        ));
        assert!(app.sidebar_items().contains(&row));
    }

    #[tokio::test]
    async fn enter_opens_an_archived_nested_thread_from_its_parent_reply() {
        let mut app = app();
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "r1",
                    json!({"id":"r1","user_id":"u1","channel_id":"c1",
                "content":"Nested root","parent_id":"m1","created_at":"2026-11-01 03:01:00",
                "thread_archive_at":"2026-11-10 00:00:00"}),
                ),
                (
                    "r2",
                    json!({"id":"r2","user_id":"u2","channel_id":"c1",
                "content":"Nested reply","parent_id":"r1","created_at":"2026-11-01 03:02:00"}),
                ),
            ]),
        ));
        let row = SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "r1".into(),
        };
        assert!(!app.sidebar_items().contains(&row));
        app.current = Some("c1".into());
        app.focus = Focus::Messages;
        app.selected = Some("m1".into());
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Down);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.thread.as_ref().unwrap().selected.as_deref(), Some("r1"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.thread.as_ref().unwrap().root_id, "r1");
        assert!(app.sidebar_items().contains(&row));
        let screen = render(&mut app);
        assert!(screen.contains("Nested reply"), "{screen}");
        assert!(screen.contains("アーカイブ / u:再開"), "{screen}");
        press(&mut app, KeyCode::Esc);
        assert!(!app.sidebar_items().contains(&row));
        assert!(app.thread_is_archived("c1", "r1"));
    }

    #[tokio::test]
    async fn unlisted_selected_archive_stays_under_its_channel() {
        let mut app = app();
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1",
                "content":"Old conversation","created_at":"2026-11-01 03:00:00",
                "thread_archive_at":"2026-11-10 00:00:00"}),
            )]),
        ));
        assert!(app.active_threads.is_empty());
        assert!(app.archived_threads.is_empty());
        app.current = Some("c1".into());
        app.focus = Focus::Messages;
        app.selected = Some("m1".into());
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.sidebar_items(),
            vec![
                SidebarItem::Channel("c1".into()),
                SidebarItem::Thread {
                    channel_id: "c1".into(),
                    id: "m1".into()
                },
                SidebarItem::Channel("c2".into()),
            ]
        );
        assert!(app.thread_is_archived("c1", "m1"));
        assert_eq!(app.search_matches("Old conversation"), vec![1]);
        press(&mut app, KeyCode::Char('/'));
        for ch in "Old conversation".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.sidebar_index, 1);
        assert_eq!(app.thread.as_ref().unwrap().root_id, "m1");
        press(&mut app, KeyCode::Esc);
        assert_eq!(
            app.sidebar_items(),
            vec![
                SidebarItem::Channel("c1".into()),
                SidebarItem::Channel("c2".into()),
            ]
        );
    }

    #[tokio::test]
    async fn unlisted_archive_renders_loaded_reply_count() {
        let mut app = app();
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1",
                    "content":"Old","created_at":"2026-11-01 03:00:00",
                    "thread_archive_at":"2026-11-10 00:00:00"}),
            )]),
        ));
        assert!(app.active_threads.is_empty());
        assert!(app.archived_threads.is_empty());
        assert_eq!(app.store.reply_counts("c1").get("m1"), Some(&1));
        app.current = Some("c1".into());
        app.focus = Focus::Messages;
        app.selected = Some("m1".into());
        press(&mut app, KeyCode::Enter);
        let screen = render(&mut app);
        assert!(screen.contains("Old (1)"), "{screen}");
        assert!(screen.contains("返信 1 件"), "{screen}");
    }

    #[tokio::test]
    async fn archived_shape_state_survives_subscription_reset() {
        let mut app = app();
        app.open_channel("c1".into());
        let active = ThreadSummary {
            id: "m1".into(),
            channel_id: "c1".into(),
            content: "Old conversation".into(),
            thread_archive_at: None,
            reply_count: 1,
        };
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            threads: vec![active.clone()],
            next_cursor: None,
            requested_at: std::time::Instant::now(),
        });
        let stale_request = std::time::Instant::now();
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1","content":"Old conversation",
                "created_at":"2026-11-01 03:00:00","thread_archive_at":"2026-11-10 00:00:00"}),
            )]),
        ));
        app.open_channel("c2".into());
        assert!(app.store.message("c1", "m1").is_none());
        let row = SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "m1".into(),
        };
        assert!(!app.sidebar_items().contains(&row));
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: true,
            threads: vec![active.clone()],
            next_cursor: None,
            requested_at: stale_request,
        });
        assert!(!app.sidebar_items().contains(&row));
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            threads: vec![active],
            next_cursor: None,
            requested_at: std::time::Instant::now(),
        });
        assert!(app.sidebar_items().contains(&row));
    }

    #[tokio::test]
    async fn resync_reconciles_rows_added_by_an_index_response_during_the_snapshot() {
        for append in [false, true] {
            for replies in [0, 1] {
                let mut app = app();
                // 再同期開始時に親・返信・一覧のどれも存在しない状態にする。
                app.store.apply(
                    &ShapeId::Messages("c1".into()),
                    Batch {
                        reset: true,
                        ..Default::default()
                    },
                );
                app.open_channel("c1".into());
                app.on_shape(ShapeEvent::Batch(
                    ShapeId::Messages("c1".into()),
                    Batch {
                        reset: true,
                        ..Default::default()
                    },
                ));
                assert!(app.active_threads.is_empty());
                assert!(app.store.message("c1", "m1").is_none());
                app.on_app_event(AppEvent::Threads {
                    archived: false,
                    append,
                    requested_at: std::time::Instant::now(),
                    next_cursor: None,
                    threads: vec![ThreadSummary {
                        id: "m1".into(),
                        channel_id: "c1".into(),
                        content: "Late index row".into(),
                        thread_archive_at: None,
                        reply_count: 1,
                    }],
                });
                let row = SidebarItem::Thread {
                    channel_id: "c1".into(),
                    id: "m1".into(),
                };
                assert!(app.sidebar_items().contains(&row));
                let mut snapshot = batch(vec![(
                    "m1",
                    json!({"id":"m1","user_id":"u2","channel_id":"c1",
                    "content":"Late index row","created_at":"2026-11-01 03:00:00"}),
                )]);
                snapshot.up_to_date = false;
                app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), snapshot));
                assert!(app.sidebar_items().contains(&row));
                if replies > 0 {
                    let mut reply = batch(vec![(
                        "r1",
                        json!({"id":"r1","user_id":"u1","channel_id":"c1",
                        "content":"Still exists","parent_id":"m1","created_at":"2026-11-01 03:01:00"}),
                    )]);
                    reply.up_to_date = false;
                    app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), reply));
                }
                let stale_request = std::time::Instant::now();
                app.on_shape(ShapeEvent::Batch(
                    ShapeId::Messages("c1".into()),
                    Batch {
                        up_to_date: true,
                        ..Default::default()
                    },
                ));
                assert_eq!(app.store.replies("c1", "m1").len(), replies);
                assert_eq!(app.sidebar_items().contains(&row), replies > 0);
                app.on_app_event(AppEvent::ThreadsFailed {
                    archived: false,
                    append: false,
                    requested_at: std::time::Instant::now(),
                    message: "unavailable".into(),
                });
                app.on_app_event(AppEvent::Threads {
                    archived: false,
                    append,
                    requested_at: stale_request,
                    next_cursor: None,
                    threads: vec![ThreadSummary {
                        id: "m1".into(),
                        channel_id: "c1".into(),
                        content: "Late index row".into(),
                        thread_archive_at: None,
                        reply_count: 1,
                    }],
                });
                assert_eq!(app.sidebar_items().contains(&row), replies > 0);
            }
        }
    }

    #[tokio::test]
    async fn resync_keeps_rows_until_the_complete_snapshot_reconciles_reply_counts() {
        for count in [0, 1, 2] {
            let mut app = app();
            app.open_channel("c1".into());
            let summary = ThreadSummary {
                id: "m1".into(),
                channel_id: "c1".into(),
                content: "Resync conversation".into(),
                thread_archive_at: None,
                reply_count: 1,
            };
            app.on_app_event(AppEvent::Threads {
                archived: false,
                append: false,
                requested_at: std::time::Instant::now(),
                next_cursor: None,
                threads: vec![summary.clone()],
            });
            let row = SidebarItem::Thread {
                channel_id: "c1".into(),
                id: "m1".into(),
            };
            app.focus = Focus::Messages;
            app.selected = Some("m1".into());
            press(&mut app, KeyCode::Enter);
            for _ in 0..2 {
                app.on_shape(ShapeEvent::Batch(
                    ShapeId::Messages("c1".into()),
                    Batch {
                        reset: true,
                        ..Default::default()
                    },
                ));
                assert!(app.sidebar_items().contains(&row));
                assert_eq!(app.active_threads[0].reply_count, 1);
                assert_eq!(app.thread.as_ref().unwrap().root_id, "m1");
                assert!(render(&mut app).contains("読み込み中"));
            }
            let mut root = batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1",
                "content":"Resync conversation","created_at":"2026-11-01 03:00:00"}),
            )]);
            root.up_to_date = false;
            app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), root));
            assert!(app.sidebar_items().contains(&row));
            assert_eq!(app.active_threads[0].reply_count, 1);
            for i in 0..count {
                let id = format!("restored-{i}");
                let mut reply = batch(vec![(
                    &id,
                    json!({"id":id,"user_id":"u1","channel_id":"c1",
                    "content":"Restored reply","parent_id":"m1","created_at":"2026-11-01 03:01:00"}),
                )]);
                reply.up_to_date = false;
                app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), reply));
                assert_eq!(app.active_threads[0].reply_count, 1);
            }
            let stale_request = std::time::Instant::now();
            app.on_shape(ShapeEvent::Batch(
                ShapeId::Messages("c1".into()),
                Batch {
                    up_to_date: true,
                    ..Default::default()
                },
            ));
            assert_eq!(app.store.replies("c1", "m1").len(), count);
            assert_eq!(app.sidebar_items().contains(&row), count > 0);
            if count > 0 {
                assert_eq!(app.active_threads[0].reply_count, count as u64);
            }
            app.on_app_event(AppEvent::Threads {
                archived: false,
                append: false,
                requested_at: stale_request,
                next_cursor: None,
                threads: vec![summary],
            });
            app.on_app_event(AppEvent::ThreadsFailed {
                archived: false,
                append: false,
                requested_at: std::time::Instant::now(),
                message: "unavailable".into(),
            });
            assert_eq!(app.sidebar_items().contains(&row), count > 0);
            if count > 0 {
                assert_eq!(app.active_threads[0].reply_count, count as u64);
            }
        }
    }

    #[tokio::test]
    async fn resubscribing_reconciles_known_rows_even_when_the_store_was_unloaded() {
        for root_exists in [false, true] {
            let mut app = app();
            app.open_channel("c1".into());
            app.on_app_event(AppEvent::Threads {
                archived: false,
                append: false,
                requested_at: std::time::Instant::now(),
                next_cursor: None,
                threads: vec![ThreadSummary {
                    id: "m1".into(),
                    channel_id: "c1".into(),
                    content: "Known thread".into(),
                    thread_archive_at: None,
                    reply_count: 1,
                }],
            });
            app.open_channel("c2".into());
            app.open_channel("c1".into());
            assert!(app.store.message("c1", "m1").is_none());
            let row = SidebarItem::Thread {
                channel_id: "c1".into(),
                id: "m1".into(),
            };
            assert!(app.sidebar_items().contains(&row));
            if root_exists {
                let mut snapshot = batch(vec![
                    (
                        "m1",
                        json!({"id":"m1","user_id":"u2","channel_id":"c1",
                        "content":"Known thread","created_at":"2026-11-01 03:00:00"}),
                    ),
                    (
                        "r1",
                        json!({"id":"r1","user_id":"u1","channel_id":"c1",
                        "content":"Reply","parent_id":"m1","created_at":"2026-11-01 03:01:00"}),
                    ),
                ]);
                snapshot.up_to_date = false;
                app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), snapshot));
            }
            assert!(app.sidebar_items().contains(&row));
            app.on_shape(ShapeEvent::Batch(
                ShapeId::Messages("c1".into()),
                Batch {
                    up_to_date: true,
                    ..Default::default()
                },
            ));
            assert_eq!(app.sidebar_items().contains(&row), root_exists);
        }
    }

    #[tokio::test]
    async fn atomic_resync_keeps_a_pending_first_reply_and_reconciles_known_threads() {
        let mut app = app();
        app.open_channel("c1".into());
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            requested_at: std::time::Instant::now(),
            next_cursor: None,
            threads: vec![ThreadSummary {
                id: "m1".into(),
                channel_id: "c1".into(),
                content: "Known".into(),
                thread_archive_at: None,
                reply_count: 1,
            }],
        });
        let pending = json!({"id":"pending","user_id":"u2","channel_id":"c1",
            "content":"Pending first reply","created_at":"2026-11-01 03:00:00"});
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![("pending", pending.clone())]),
        ));
        app.focus = Focus::Messages;
        app.selected = Some("pending".into());
        press(&mut app, KeyCode::Enter);
        app.on_app_event(AppEvent::ThreadsChanged {
            reopened: Some("pending".into()),
        });
        let mut snapshot = batch(vec![
            ("pending", pending),
            (
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1",
                "content":"Known","created_at":"2026-11-01 03:00:00"}),
            ),
            (
                "a",
                json!({"id":"a","user_id":"u1","channel_id":"c1",
                "content":"A","parent_id":"m1","created_at":"2026-11-01 03:01:00"}),
            ),
            (
                "b",
                json!({"id":"b","user_id":"u1","channel_id":"c1",
                "content":"B","parent_id":"m1","created_at":"2026-11-01 03:02:00"}),
            ),
        ]);
        snapshot.reset = true;
        app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), snapshot));
        assert_eq!(
            app.active_threads
                .iter()
                .find(|row| row.id == "m1")
                .unwrap()
                .reply_count,
            2
        );
        assert_eq!(
            app.active_threads
                .iter()
                .find(|row| row.id == "pending")
                .unwrap()
                .reply_count,
            0
        );
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "first",
                json!({"id":"first","user_id":"u1","channel_id":"c1","content":"First",
                "parent_id":"pending","created_at":"2026-11-01 03:03:00"}),
            )]),
        ));
        assert_eq!(
            app.active_threads
                .iter()
                .find(|row| row.id == "pending")
                .unwrap()
                .reply_count,
            1
        );
        assert_eq!(app.thread.as_ref().unwrap().root_id, "pending");
    }

    #[tokio::test]
    async fn deleting_the_last_reply_removes_the_active_row_even_when_refresh_fails() {
        for (physical_delete, archived) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            let mut app = app();
            app.open_channel("c1".into());
            // fixtureの初期行が同期済みになってから、liveの削除を受信する。
            app.on_shape(ShapeEvent::Batch(
                ShapeId::Messages("c1".into()),
                Batch {
                    up_to_date: true,
                    ..Default::default()
                },
            ));
            let thread = ThreadSummary {
                id: "m1".into(),
                channel_id: "c1".into(),
                content: "Conversation".into(),
                thread_archive_at: None,
                reply_count: 1,
            };
            app.on_app_event(AppEvent::Threads {
                archived: false,
                append: false,
                requested_at: std::time::Instant::now(),
                next_cursor: None,
                threads: vec![thread.clone()],
            });
            let row = SidebarItem::Thread {
                channel_id: "c1".into(),
                id: "m1".into(),
            };
            assert!(app.sidebar_items().contains(&row));
            if archived {
                app.on_shape(ShapeEvent::Batch(
                    ShapeId::Messages("c1".into()),
                    batch(vec![(
                        "m1",
                        json!({"id":"m1","user_id":"u2","channel_id":"c1",
                        "content":"Conversation","created_at":"2026-11-01 03:00:00",
                        "thread_archive_at":"2026-11-10 00:00:00"}),
                    )]),
                ));
                app.focus = Focus::Messages;
                app.selected = Some("m1".into());
                press(&mut app, KeyCode::Enter);
            }
            let stale_request = std::time::Instant::now();
            let deletion = if physical_delete {
                Batch {
                    changes: vec![Change::Delete { key: "r1".into() }],
                    ..Default::default()
                }
            } else {
                batch(vec![(
                    "r1",
                    json!({"id":"r1","user_id":"u1","channel_id":"c1",
                    "content":"返信です","parent_id":"m1","created_at":"2026-11-01 03:01:00",
                    "deleted_at":"2026-11-11 00:00:00"}),
                )])
            };
            app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), deletion));
            assert_eq!(app.store.reply_counts("c1").get("m1"), None);
            assert!(!app.active_threads.iter().any(|thread| thread.id == "m1"));
            app.on_app_event(AppEvent::Threads {
                archived: false,
                append: false,
                requested_at: stale_request,
                next_cursor: None,
                threads: vec![thread],
            });
            app.on_app_event(AppEvent::ThreadsFailed {
                archived: false,
                append: false,
                requested_at: std::time::Instant::now(),
                message: "unavailable".into(),
            });
            assert_eq!(app.sidebar_items().contains(&row), archived);
            if archived {
                assert!(app.thread_is_archived("c1", "m1"));
                press(&mut app, KeyCode::Esc);
                assert!(!app.sidebar_items().contains(&row));
            }
            assert!(!render(&mut app).contains("Conversation (0)"));
        }
    }

    #[tokio::test]
    async fn first_reply_shape_updates_the_locally_created_thread_count() {
        let mut app = app();
        app.open_channel("c1".into());
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "new-root",
                json!({"id":"new-root","user_id":"u2","channel_id":"c1",
                "content":"New thread","created_at":"2026-11-01 03:00:00"}),
            )]),
        ));
        app.focus = Focus::Messages;
        app.selected = Some("new-root".into());
        press(&mut app, KeyCode::Enter);
        app.on_app_event(AppEvent::ThreadsChanged {
            reopened: Some("new-root".into()),
        });
        assert_eq!(app.active_threads[0].reply_count, 0);
        let stale_request = std::time::Instant::now();
        app.on_app_event(AppEvent::ThreadsFailed {
            archived: false,
            append: false,
            requested_at: stale_request,
            message: "unavailable".into(),
        });
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "first",
                json!({"id":"first","user_id":"u1","channel_id":"c1",
                "content":"First reply","parent_id":"new-root","created_at":"2026-11-01 03:01:00"}),
            )]),
        ));
        assert_eq!(app.store.reply_counts("c1").get("new-root"), Some(&1));
        assert_eq!(app.active_threads[0].reply_count, 1);
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            requested_at: stale_request,
            next_cursor: None,
            threads: vec![ThreadSummary {
                id: "new-root".into(),
                channel_id: "c1".into(),
                content: "New thread".into(),
                thread_archive_at: None,
                reply_count: 0,
            }],
        });
        press(&mut app, KeyCode::Esc);
        assert!(render(&mut app).contains("New thread (1)"));
    }

    #[tokio::test]
    async fn shape_reopen_survives_close_stale_index_and_failed_refresh() {
        let mut app = app();
        app.open_channel("c1".into());
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1",
                "content":"Reopened remotely","created_at":"2026-11-01 03:00:00",
                "thread_archive_at":"2026-11-10 00:00:00"}),
            )]),
        ));
        app.focus = Focus::Messages;
        app.selected = Some("m1".into());
        press(&mut app, KeyCode::Enter);
        assert!(app.active_threads.is_empty());
        assert!(app.archived_threads.is_empty());
        let stale_request = std::time::Instant::now();
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1",
                "content":"Reopened remotely","created_at":"2026-11-01 03:00:00",
                "thread_archive_at":null}),
            )]),
        ));
        press(&mut app, KeyCode::Esc);
        let row = SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "m1".into(),
        };
        assert!(app.thread.is_none());
        assert!(app.sidebar_items().contains(&row));
        assert!(!app.thread_is_archived("c1", "m1"));
        assert!(render(&mut app).contains("Reopened remotely"));
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            threads: vec![],
            next_cursor: None,
            requested_at: stale_request,
        });
        assert!(app.sidebar_items().contains(&row));
        app.on_app_event(AppEvent::ThreadsFailed {
            archived: false,
            append: false,
            requested_at: std::time::Instant::now(),
            message: "unavailable".into(),
        });
        assert!(app.sidebar_items().contains(&row));
        app.open_channel("c2".into());
        assert!(app.store.message("c1", "m1").is_none());
        assert!(app.sidebar_items().contains(&row));
    }

    #[tokio::test]
    async fn unlisted_reopen_success_survives_close_and_failed_refresh() {
        let mut app = app();
        app.open_channel("c1".into());
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({"id":"m1","user_id":"u2","channel_id":"c1","content":"Old conversation",
                "created_at":"2026-11-01 03:00:00","thread_archive_at":"2026-11-10 00:00:00"}),
            )]),
        ));
        app.focus = Focus::Messages;
        app.selected = Some("m1".into());
        press(&mut app, KeyCode::Enter);
        assert!(app.active_threads.is_empty());
        assert!(app.archived_threads.is_empty());
        app.on_app_event(AppEvent::ThreadsChanged {
            reopened: Some("m1".into()),
        });
        press(&mut app, KeyCode::Esc);
        assert!(app.thread.is_none());
        let row = SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "m1".into(),
        };
        assert!(app.sidebar_items().contains(&row));
        assert!(!app.thread_is_archived("c1", "m1"));
        app.on_app_event(AppEvent::ThreadsFailed {
            archived: false,
            append: false,
            requested_at: std::time::Instant::now(),
            message: "unavailable".into(),
        });
        assert!(app.sidebar_items().contains(&row));
    }

    #[tokio::test]
    async fn refreshing_lists_keeps_archives_in_history_only() {
        let mut app = app();
        let active = ThreadSummary {
            id: "m1".into(),
            channel_id: "c1".into(),
            content: "Active".into(),
            thread_archive_at: None,
            reply_count: 1,
        };
        let archived = ThreadSummary {
            id: "r1".into(),
            thread_archive_at: Some("2026-11-10".into()),
            ..active.clone()
        };
        for (archived_scope, append, threads, expected) in [
            (false, false, vec![active.clone()], vec!["m1"]),
            (true, false, vec![archived.clone()], vec!["m1"]),
            (false, false, vec![active.clone()], vec!["m1"]),
            (true, false, vec![archived], vec!["m1"]),
            (
                false,
                true,
                vec![ThreadSummary {
                    id: "older".into(),
                    ..active
                }],
                vec!["m1", "older"],
            ),
        ] {
            app.on_app_event(AppEvent::Threads {
                archived: archived_scope,
                append,
                threads,
                next_cursor: None,
                requested_at: std::time::Instant::now(),
            });
            let ids: Vec<_> = app
                .sidebar_items()
                .into_iter()
                .filter_map(|item| match item {
                    SidebarItem::Thread { id, .. } => Some(id),
                    _ => None,
                })
                .collect();
            assert_eq!(ids, expected);
        }
    }

    #[tokio::test]
    async fn archived_thread_changes_require_reopening_but_replies_remain_available() {
        let mut app = app();
        app.open_channel("c1".into());
        app.focus = Focus::Thread;
        app.thread = Some(crate::ui::app::ThreadView {
            root_id: "m1".into(),
            selected: Some("r1".into()),
            scroll: 0,
            inner_scroll: 0,
        });
        let archived = ThreadSummary {
            id: "m1".into(),
            channel_id: "c1".into(),
            content: "Archived".into(),
            thread_archive_at: Some("2026-11-10".into()),
            reply_count: 1,
        };
        app.on_app_event(AppEvent::Threads {
            archived: true,
            append: false,
            next_cursor: None,
            requested_at: std::time::Instant::now(),
            threads: vec![archived.clone()],
        });
        for key in ['r', 'e', 'd'] {
            press(&mut app, KeyCode::Char(key));
            assert_eq!(app.mode, Mode::Normal);
        }
        assert!(app.composer.is_empty());
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.target, crate::ui::app::Target::Reply("m1".into()));
        press(&mut app, KeyCode::Esc);
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            next_cursor: None,
            requested_at: std::time::Instant::now(),
            threads: vec![ThreadSummary {
                thread_archive_at: None,
                ..archived
            }],
        });
        press(&mut app, KeyCode::Char('r'));
        assert!(matches!(app.mode, Mode::React { .. }));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('d'));
        assert!(matches!(app.mode, Mode::ConfirmDelete { .. }));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.target, crate::ui::app::Target::Edit("r1".into()));
    }

    #[tokio::test]
    async fn an_open_thread_shows_archive_state_without_loading_history() {
        let mut app = app();
        app.open_channel("c1".into());
        app.thread = Some(crate::ui::app::ThreadView {
            root_id: "m1".into(),
            selected: None,
            scroll: 0,
            inner_scroll: 0,
        });
        app.on_shape(ShapeEvent::Batch(ShapeId::Messages("c1".into()), batch(vec![(
            "m1", serde_json::json!({"id":"m1", "user_id":"u1", "channel_id":"c1", "content":"Archived root", "created_at":"2026-11-01T00:00:00", "thread_archive_at":"2026-11-10T00:00:00"})
        )])));
        assert!(render(&mut app).contains("アーカイブ / u:再開"));
    }

    #[tokio::test]
    async fn sidebar_keeps_selection_when_threads_are_inserted_before_it() {
        let mut app = app();
        app.sidebar_index = 1;
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            next_cursor: None,
            requested_at: std::time::Instant::now(),
            threads: vec![ThreadSummary {
                id: "m1".into(),
                channel_id: "c1".into(),
                content: "Thread".into(),
                thread_archive_at: None,
                reply_count: 1,
            }],
        });
        assert_eq!(app.sidebar_index, 2);
        assert_eq!(app.sidebar_items()[2], SidebarItem::Channel("c2".into()));
    }

    #[tokio::test]
    async fn loading_another_thread_page_selects_the_first_added_thread() {
        let mut app = app();
        let first = ThreadSummary {
            id: "m1".into(),
            channel_id: "c1".into(),
            content: "First page".into(),
            thread_archive_at: None,
            reply_count: 1,
        };
        app.on_app_event(AppEvent::Threads {
            requested_at: std::time::Instant::now(),
            threads: vec![first.clone()],
            archived: false,
            append: false,
            next_cursor: Some("next".into()),
        });
        app.sidebar_index = app
            .sidebar_items()
            .iter()
            .position(|item| matches!(item, SidebarItem::MoreThreads { archived: false }))
            .unwrap();
        app.on_app_event(AppEvent::Threads {
            requested_at: std::time::Instant::now(),
            threads: vec![ThreadSummary {
                id: "older".into(),
                content: "Older thread".into(),
                ..first
            }],
            archived: false,
            append: true,
            next_cursor: None,
        });
        assert_eq!(
            app.sidebar_items()[app.sidebar_index],
            SidebarItem::Thread {
                channel_id: "c1".into(),
                id: "older".into()
            }
        );
        assert!(render(&mut app).contains("Older thread"));
    }

    #[tokio::test]
    async fn late_thread_refresh_does_not_restore_an_archived_state() {
        let mut app = app();
        let requested_at = std::time::Instant::now();
        let active = ThreadSummary {
            id: "m1".into(),
            channel_id: "c1".into(),
            content: "Thread".into(),
            thread_archive_at: None,
            reply_count: 1,
        };
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            next_cursor: None,
            requested_at,
            threads: vec![active.clone()],
        });
        app.on_app_event(AppEvent::Threads {
            archived: false,
            append: false,
            next_cursor: None,
            requested_at: requested_at - std::time::Duration::from_secs(1),
            threads: vec![ThreadSummary {
                thread_archive_at: Some("2026-11-10T00:00:00".into()),
                ..active
            }],
        });
        assert!(app.active_threads[0].thread_archive_at.is_none());
    }

    #[tokio::test]
    async fn opens_channel_and_thread_with_keys() {
        let mut app = app();
        let screen = render(&mut app);
        assert!(screen.contains("#general"));
        assert!(screen.contains("#secret"));
        assert!(screen.contains("Enter で開きます"));

        press(&mut app, KeyCode::Enter);
        let screen = render(&mut app);
        assert!(screen.contains("— 雑談"));
        assert!(screen.contains("Bob [bot]"));
        assert!(screen.contains("hello @alice"));
        assert!(screen.contains("返信 1 件"));
        assert!(screen.contains("👍 2"));
        assert!(!screen.contains("返信です"), "返信はタイムラインに出さない");

        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Enter);
        let screen = render(&mut app);
        assert!(screen.contains("スレッド"));
        assert!(screen.contains("返信です"));

        press(&mut app, KeyCode::Char('i'));
        assert!(render(&mut app).contains("スレッドに返信"));
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);
        assert!(app.thread.is_none());
        assert!(render(&mut app).contains("#general へ送信"));
    }

    #[tokio::test]
    async fn shows_unread_typing_and_reaction_prompt() {
        let mut app = app();
        app.on_app_event(AppEvent::ReadStates(vec![ReadState {
            channel_id: "c2".into(),
            has_unread: true,
            last_read_at: None,
        }]));
        assert!(render(&mut app).contains("● #secret"));

        press(&mut app, KeyCode::Enter);
        app.on_app_event(AppEvent::Typing {
            channel_id: "c1".into(),
            parent_id: None,
            users: vec![TypingUser {
                id: "u2".into(),
                display_name: "Bob".into(),
            }],
        });
        assert!(render(&mut app).contains("Bob が入力中…"));

        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Char('r'));
        for ch in "tada".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert!(render(&mut app).contains("リアクション: tada"));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[tokio::test]
    async fn marks_read_only_after_showing_the_latest_message() {
        let mut app = app();
        app.on_app_event(AppEvent::ReadStates(vec![ReadState {
            channel_id: "c1".into(),
            has_unread: true,
            last_read_at: None,
        }]));
        press(&mut app, KeyCode::Enter);
        // まだ描画していないので、最新を見たとは言えない。
        app.on_tick();
        assert!(app.unread.contains("c1"));

        render(&mut app);
        app.on_tick();
        assert!(!app.unread.contains("c1"));

        // 古いメッセージだけが見えているときは、新着が届いても既読にしない。
        app.newest_shown = Some(("c1".into(), "m0".into()));
        app.on_app_event(AppEvent::ReadStates(vec![ReadState {
            channel_id: "c1".into(),
            has_unread: true,
            last_read_at: None,
        }]));
        assert!(app.unread.contains("c1"));
    }

    #[tokio::test]
    async fn shows_available_update_in_status_line() {
        let mut app = app();
        assert!(!render(&mut app).contains("新しい版"));
        app.on_app_event(AppEvent::UpdateAvailable("0.2.0".into()));
        assert!(render(&mut app).contains("新しい版 v0.2.0 があります"));
    }

    #[tokio::test]
    async fn pastes_newlines_into_composer_without_sending() {
        let mut app = app();
        press(&mut app, KeyCode::Enter);
        app.on_terminal_event(Event::Paste("無視".into()));
        assert!(app.composer.is_empty());

        press(&mut app, KeyCode::Char('i'));
        app.on_terminal_event(Event::Paste("1行目\r\n2行目\r3行目".into()));
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.composer.lines(), ["1行目", "2行目", "3行目"]);
    }

    #[tokio::test]
    async fn resubscribing_drops_rows_of_the_stopped_shape() {
        let mut app = app();
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.store.reaction_groups("c1", "m1", "u1").len(), 1);
        let id = ShapeId::Messages("c1".into());
        assert!(
            !app.subscriptions
                .contains_key(&ShapeId::Reactions("c1".into()))
        );
        // 購読が 404 などで止まった状態にする。
        app.subscriptions[&id].abort();
        while !app.subscriptions[&id].is_finished() {
            tokio::task::yield_now().await;
        }
        app.on_shape(ShapeEvent::Failed(id.clone(), "404".into()));
        app.open_channel("c1".into());
        assert!(app.store.reaction_groups("c1", "m1", "u1").is_empty());
        // 同梱リアクションも本文の再同期と一緒に破棄する。
        assert!(app.store.root_messages("c1").is_empty());
        assert!(!app.store.is_synced(&id));
    }

    #[tokio::test]
    async fn channel_switch_releases_the_previous_subscription_and_focus_pauses_polling() {
        let mut app = app();
        app.open_channel("c1".into());
        app.open_channel("c2".into());
        assert!(
            !app.subscriptions
                .contains_key(&ShapeId::Messages("c1".into()))
        );
        assert!(
            app.subscriptions
                .contains_key(&ShapeId::Messages("c2".into()))
        );
        assert_eq!(app.subscriptions.len(), 4);
        app.on_terminal_event(Event::FocusLost);
        assert!(app.typing.is_empty());
        app.on_tick();
        app.on_terminal_event(Event::FocusGained);
    }

    #[tokio::test]
    async fn restores_failed_edit_with_its_target() {
        let mut app = app();
        press(&mut app, KeyCode::Enter);
        let failed = |channel_id: &str| AppEvent::RestoreDraft {
            channel_id: channel_id.into(),
            target: Target::Edit("r1".into()),
            content: "直した返信".into(),
        };

        // 別のチャンネルの失敗は入力欄に入れず、保存しておく。
        app.on_app_event(failed("c2"));
        assert!(app.composer.is_empty());
        assert_eq!(app.failed_drafts.len(), 1);
        assert!(render(&mut app).contains("送れなかった本文を保存しました"));
        app.failed_drafts.clear();

        app.on_app_event(failed("c1"));
        press(&mut app, KeyCode::Char('i'));
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.target, Target::Edit("r1".into()));
        assert_eq!(app.composer.lines(), ["直した返信"]);
        assert!(render(&mut app).contains("メッセージを編集"));
    }

    #[tokio::test]
    async fn keeps_failed_draft_while_writing_the_next_one() {
        let mut app = app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('i'));
        app.composer.insert_str("次の本文");

        app.on_app_event(AppEvent::RestoreDraft {
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "先の本文".into(),
        });
        assert_eq!(app.composer.lines(), ["次の本文"]);
        assert_eq!(app.failed_drafts.len(), 1);

        // 書きかけがある間は戻さない。
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Char('R'));
        assert_eq!(app.composer.lines(), ["次の本文"]);
        assert!(render(&mut app).contains("書きかけの下書きがあります"));

        app.composer = tui_textarea::TextArea::default();
        press(&mut app, KeyCode::Char('R'));
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.target, Target::Channel);
        assert_eq!(app.composer.lines(), ["先の本文"]);
        assert!(app.failed_drafts.is_empty());
    }

    #[tokio::test]
    async fn guards_edit_of_others_and_shows_help_and_search() {
        let mut app = app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('k'));
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.mode, Mode::Normal);
        assert!(render(&mut app).contains("自分のメッセージだけを編集できます"));

        // 自分の返信を選び、下書きがあるときは編集を始めない。
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('G'));
        app.composer.insert_str("下書き");
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.mode, Mode::Normal);
        assert_eq!(app.composer.lines(), ["下書き"]);
        assert!(render(&mut app).contains("書きかけの下書きがあります"));
        app.composer = tui_textarea::TextArea::default();
        press(&mut app, KeyCode::Char('e'));
        assert_eq!(app.mode, Mode::Insert);
        assert_eq!(app.composer.lines(), ["返信です"]);
        press(&mut app, KeyCode::Esc);
        press(&mut app, KeyCode::Esc);

        press(&mut app, KeyCode::Char('?'));
        assert!(render(&mut app).contains("キー操作"));
        press(&mut app, KeyCode::Char('x'));

        press(&mut app, KeyCode::Char('/'));
        for ch in "sec".chars() {
            press(&mut app, KeyCode::Char(ch));
        }
        assert!(render(&mut app).contains("検索: sec"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.current.as_deref(), Some("c2"));
    }

    fn render_sized(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        let mut screen = String::new();
        for y in 0..buffer.area.height {
            let mut x = 0;
            while x < buffer.area.width {
                let symbol = buffer[(x, y)].symbol();
                screen.push_str(symbol);
                x += symbol.width().max(1) as u16;
            }
            screen.push('\n');
        }
        screen
    }

    fn encryption_event(app: &mut App, event: impl FnOnce(u64) -> EncryptionEvent) {
        let generation = app.encryption.generation();
        app.on_app_event(AppEvent::Encryption(event(generation)));
    }

    #[tokio::test]
    async fn encryption_screen_walks_through_first_setup() {
        let mut app = app();
        app.me.id = "00000000-0000-4000-8000-00000000000b".into();
        // テストでブラウザーを開かない。
        app.encryption.over_ssh = true;
        press(&mut app, KeyCode::Char('E'));
        assert_eq!(app.mode, Mode::Encryption);
        // 端末の登録が済むまでは、暗号ストアがない。
        assert!(render_sized(&mut app, 100, 30).contains("端末を登録しています"));

        encryption_event(&mut app, |generation| EncryptionEvent::Status {
            generation,
            result: Ok(E2eeStatus::NotSetUp),
        });
        assert!(render_sized(&mut app, 100, 30).contains("初回設定を始める"));
        press(&mut app, KeyCode::Enter);
        let screen = render_sized(&mut app, 100, 30);
        assert!(screen.contains("リカバリーキー"), "{screen}");
        let Screen::RecoveryKey(key) = &app.encryption.screen else {
            panic!("リカバリーキーを表示していない");
        };
        assert!(screen.contains(key.as_str()), "{screen}");

        // 再認証の待ち受け。SSH の先では、ポートの転送か、Web か Android での初回設定を案内する。
        encryption_event(&mut app, |generation| EncryptionEvent::ReauthWaiting {
            generation,
            url: "https://account.example/authorize?max_age=0".into(),
            port: 41234,
        });
        let screen = render_sized(&mut app, 100, 30);
        assert!(screen.contains("ssh -L 41234:127.0.0.1:41234"), "{screen}");
        assert!(screen.contains("Web か Android"), "{screen}");
        assert!(screen.contains("max_age=0"), "{screen}");

        // 再認証をやめても、鍵は捨てずに、もう一度再認証できる。
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Encryption);
        assert!(render_sized(&mut app, 100, 30).contains("同じ鍵のまま"));

        // 保存が 403 になったら、同じ鍵のまま再認証してやり直せる。
        encryption_event(&mut app, |generation| EncryptionEvent::Failed {
            generation,
            failure: crate::ui::encryption::Failure::Reauth("再認証が必要です".into()),
        });
        assert!(render_sized(&mut app, 100, 30).contains("再認証して保存し直す"));
        encryption_event(&mut app, |generation| EncryptionEvent::Saved { generation });
        assert!(render_sized(&mut app, 100, 30).contains("解錠済み"));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
    }

    #[tokio::test]
    async fn encryption_screen_takes_a_recovery_key_and_shows_the_status() {
        let mut app = app();
        app.encryption.over_ssh = true;
        press(&mut app, KeyCode::Char('E'));
        encryption_event(&mut app, |generation| EncryptionEvent::Status {
            generation,
            result: Ok(E2eeStatus::Locked(LockReason::Resume)),
        });
        let screen = render_sized(&mut app, 100, 30);
        assert!(screen.contains("初回設定が途中です"), "{screen}");
        app.on_terminal_event(Event::Paste("EsTc 1234".into()));
        press(&mut app, KeyCode::Backspace);
        assert!(render_sized(&mut app, 100, 30).contains("> EsTc 123"));
        // 打ち間違いは、入力欄を空にして知らせる。
        encryption_event(&mut app, |generation| EncryptionEvent::Failed {
            generation,
            failure: crate::ui::encryption::Failure::WrongKey(
                "リカバリーキーの形式が違います".into(),
            ),
        });
        assert!(render_sized(&mut app, 100, 30).contains("リカバリーキーの形式が違います"));
        // 古い世代の結果は捨てる。
        app.on_app_event(AppEvent::Encryption(EncryptionEvent::Saved {
            generation: 0,
        }));
        assert!(!render_sized(&mut app, 100, 30).contains("解錠済み"));
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.mode, Mode::Normal);
        // 閉じても、解錠が要ることをステータス行に出す。
        assert!(render_sized(&mut app, 100, 30).contains("暗号化: 解錠が必要 (E)"));
    }

    #[tokio::test]
    async fn a_recovery_key_that_does_not_fit_cannot_be_confirmed() {
        let mut app = app();
        app.me.id = "00000000-0000-4000-8000-00000000000b".into();
        app.encryption.over_ssh = true;
        press(&mut app, KeyCode::Char('E'));
        encryption_event(&mut app, |generation| EncryptionEvent::Status {
            generation,
            result: Ok(E2eeStatus::NotSetUp),
        });
        press(&mut app, KeyCode::Enter);
        let Screen::RecoveryKey(key) = &app.encryption.screen else {
            panic!("リカバリーキーを表示していない");
        };
        let key = key.as_str().to_owned();
        // 表示中に端末を縮めると、キーを出さずに広げるよう求め、y では保存に進まない。
        let screen = render_sized(&mut app, 100, 8);
        assert!(screen.contains("端末を広げてください"), "{screen}");
        assert!(!screen.contains(&key));
        press(&mut app, KeyCode::Char('y'));
        assert!(matches!(app.encryption.screen, Screen::RecoveryKey(_)));
        // 広げてキー全体が見えたら、保存に進める。
        assert!(render_sized(&mut app, 100, 30).contains(&key));
        press(&mut app, KeyCode::Char('y'));
        assert!(!matches!(app.encryption.screen, Screen::RecoveryKey(_)));
    }
}
