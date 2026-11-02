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
    app::{App, Focus, Mode, Target},
    timeline::{self, Context},
};
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
    let channels = app.store.channels();
    let items: Vec<ListItem> = channels
        .iter()
        .enumerate()
        .map(|(index, channel)| {
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
            if channel.archive_at.is_some() {
                spans.push(Span::styled(" (アーカイブ)", DIM));
            }
            ListItem::new(Line::from(spans))
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
    let mut state =
        ListState::default().with_selected((!channels.is_empty()).then_some(app.sidebar_index));
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
    let block = pane(Line::from(" スレッド "), app.focus == Focus::Thread);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(root) = app.store.message(&channel_id, &thread.root_id) else {
        frame.render_widget(
            Paragraph::new(Line::styled("元のメッセージは削除されました", DIM)),
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
    ("/", "チャンネルを検索"),
    ("R", "送れなかった本文を入力欄に戻す"),
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

fn draw_popup(frame: &mut Frame, title: &str, lines: Vec<Line<'static>>) {
    let width = 56.min(frame.area().width);
    let height = (lines.len() as u16 + 2).min(frame.area().height);
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
        api::{ApiClient, Me, ReadState, TypingUser},
        electric::{Batch, Change, ShapeEvent},
        http::Credentials,
        ui::app::AppEvent,
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
        let id = ShapeId::Reactions("c1".into());
        // 購読が 404 などで止まった状態にする。
        app.subscriptions[&id].abort();
        while !app.subscriptions[&id].is_finished() {
            tokio::task::yield_now().await;
        }
        app.on_shape(ShapeEvent::Failed(id.clone(), "404".into()));
        app.open_channel("c1".into());
        assert!(app.store.reaction_groups("c1", "m1", "u1").is_empty());
        // 動いている購読は作り直さないので、メッセージはそのまま残る。
        assert!(!app.store.root_messages("c1").is_empty());
        assert!(!app.store.is_synced(&id));
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
}
