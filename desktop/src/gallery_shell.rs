//! アプリシェルの確認画面 (`zoo2-desktop gallery shell [loading|welcome|thread]`)。
//!
//! サーバーなしでワークスペースを描くため、固定のデータで `AppState` を作る。
//! データは `frontend/.storybook/fixtures.ts` (`ChannelSidebar.stories.tsx` と VRT のカタログ) に合わせてある。
//! 本番の画面からは使わない。

use gpui::{
    App, Application, Bounds, Entity, Focusable, KeyBinding, Window, WindowBounds, WindowOptions,
    prelude::*, px, size,
};
use serde_json::{Value, json};
use zoo2_client_core::{
    api::Me,
    config::{AuthConfig, Config},
    electric::{Batch, Change, ShapeId},
};

use crate::app::AppState;
use crate::assets::Assets;
use crate::fonts;
use crate::theme::ThemeMode as AppThemeMode;
use crate::tokens::{ThemeMode, initial_mode};
use crate::view::WorkspaceView;

/// 表示する状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// チャンネルを開いた状態。
    Workspace,
    /// 最初の同期の前 (`ShellSkeleton`)。
    Loading,
    /// チャンネルを選んでいない状態 (`WelcomePanel`)。
    Welcome,
    /// チャンネルを開き、右にスレッドペインも開いた状態。
    Thread,
    /// 入力欄にフォーカスがあり、メンション補完が出た状態。
    Composer,
    /// スレッドを開き、返信の入力欄に文字を入れた状態。
    ThreadReply,
    /// チャンネル作成ダイアログ。
    CreateChannel,
    /// 絵文字ピッカー。
    Emoji,
    /// メッセージ削除の確認。
    Delete,
    /// ⌘K のチャンネルスイッチャー。
    Switcher,
    /// メッセージ検索。
    Search,
}

impl Scenario {
    pub fn parse(arg: Option<&str>) -> Option<Self> {
        match arg {
            None | Some("workspace") => Some(Self::Workspace),
            Some("loading") => Some(Self::Loading),
            Some("welcome") => Some(Self::Welcome),
            Some("thread") => Some(Self::Thread),
            Some("composer") => Some(Self::Composer),
            Some("thread-reply") => Some(Self::ThreadReply),
            Some("create-channel") => Some(Self::CreateChannel),
            Some("emoji") => Some(Self::Emoji),
            Some("delete") => Some(Self::Delete),
            Some("switcher") => Some(Self::Switcher),
            Some("search") => Some(Self::Search),
            Some(_) => None,
        }
    }
}

fn row(value: Value) -> serde_json::Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn batch(rows: Vec<(String, Value)>) -> Batch {
    Batch {
        reset: true,
        changes: rows
            .into_iter()
            .map(|(key, value)| Change::Upsert {
                key,
                row: row(value),
            })
            .collect(),
        up_to_date: true,
    }
}

fn user(id: &str, user_name: &str, display_name: &str) -> (String, Value) {
    (
        id.to_string(),
        json!({"id": id, "user_name": user_name, "display_name": display_name}),
    )
}

fn bot(id: &str, user_name: &str, display_name: &str) -> (String, Value) {
    (
        id.to_string(),
        json!({"id": id, "user_name": user_name, "display_name": display_name, "is_bot": true}),
    )
}

#[allow(clippy::too_many_arguments)]
fn channel(
    id: &str,
    name: &str,
    description: Option<&str>,
    is_private: bool,
    is_dm: bool,
    is_nsfw: bool,
    archive_at: Option<&str>,
) -> (String, Value) {
    (
        id.to_string(),
        json!({
            "id": id, "channel_name": name, "description": description,
            "is_private": is_private, "is_dm": is_dm, "is_nsfw": is_nsfw,
            "archive_at": archive_at, "deleted_at": null,
        }),
    )
}

fn member(user_id: &str, channel_id: &str) -> (String, Value) {
    (
        format!("{user_id}:{channel_id}"),
        json!({"user_id": user_id, "channel_id": channel_id}),
    )
}

fn message(id: &str, user_id: &str, channel_id: &str, content: &str, at: &str) -> (String, Value) {
    message_with(id, user_id, channel_id, content, at, None, None)
}

/// 返信先 (`parent_id`) や連携カード (`card`) を持つメッセージ。
fn message_with(
    id: &str,
    user_id: &str,
    channel_id: &str,
    content: &str,
    at: &str,
    parent_id: Option<&str>,
    card: Option<&str>,
) -> (String, Value) {
    (
        id.to_string(),
        json!({
            "id": id, "user_id": user_id, "channel_id": channel_id, "content": content,
            "card": card,
            "created_at": at, "updated_at": null, "deleted_at": null, "parent_id": parent_id,
        }),
    )
}

fn reaction(id: &str, message_id: &str, user_id: &str, emoji: &str, at: &str) -> (String, Value) {
    (
        id.to_string(),
        json!({
            "id": id, "message_id": message_id, "user_id": user_id, "emoji": emoji, "created_at": at,
        }),
    )
}

#[allow(clippy::too_many_arguments)]
fn attachment(
    id: &str,
    message_id: &str,
    channel_id: &str,
    file_name: &str,
    file_size: i64,
    content_type: &str,
    at: &str,
) -> (String, Value) {
    (
        id.to_string(),
        json!({
            "id": id, "message_id": message_id, "uploader_id": "user-alice", "channel_id": channel_id,
            "file_name": file_name, "file_size": file_size, "content_type": content_type,
            "storage_key": format!("fixtures/{id}"), "is_sensitive": false, "created_at": at,
        }),
    )
}

/// `now` から `minutes` 分前の UTC の日時 (Electric の形)。「今日」の区切りを出すのに使う。
fn minutes_ago(minutes: i64) -> String {
    let at = jiff::Timestamp::now() - jiff::SignedDuration::from_mins(minutes);
    at.to_zoned(jiff::tz::TimeZone::UTC)
        .strftime("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 固定データの `AppState`。`frontend/.storybook/fixtures.ts` の人とチャンネルに、DM と未読を足したもの。
pub fn fixture_state(scenario: Scenario, mode: ThemeMode) -> AppState {
    let config = Config {
        api_base_url: "http://localhost:0".to_string(),
        auth: AuthConfig::Dev {
            user_id: "user-alice".to_string(),
        },
        check_updates: false,
    };
    let mut state = AppState::from_config(&config);
    state.login_phase = None;
    state.theme_mode = match mode {
        ThemeMode::Dark => AppThemeMode::Dark,
        ThemeMode::Light => AppThemeMode::Light,
    };
    if scenario == Scenario::Loading {
        return state;
    }
    state.me = Some(Me {
        id: "user-alice".to_string(),
        user_name: "alice".to_string(),
        display_name: "Alice".to_string(),
    });
    state.store.apply(
        &ShapeId::Users,
        batch(vec![
            user("user-alice", "alice", "Alice"),
            user("user-bob", "bob", "Bob"),
            user("user-carol", "carol", "Carol"),
            user("user-dave", "dave", "Dave"),
            user("user-admin", "admin", "Admin"),
            bot("user-deploy-bot", "deploy-bot", "Deploy Bot"),
        ]),
    );
    state.store.apply(
        &ShapeId::Channels,
        batch(vec![
            channel(
                "channel-general",
                "general",
                Some("チーム全体のお知らせと雑談"),
                false,
                false,
                false,
                None,
            ),
            channel(
                "channel-design",
                "design",
                Some("デザインの相談"),
                false,
                false,
                false,
                None,
            ),
            channel(
                "channel-secret",
                "secret",
                Some("非公開の作戦会議"),
                true,
                false,
                false,
                None,
            ),
            channel(
                "channel-random",
                "random",
                Some("雑談と気軽な共有"),
                false,
                false,
                true,
                None,
            ),
            channel(
                "channel-archive",
                "old-announcements",
                Some("アーカイブ済みのお知らせ"),
                false,
                false,
                false,
                Some("2025-01-01 01:00:00"),
            ),
            channel(
                "channel-dm",
                "dm:user-alice:user-bob",
                None,
                true,
                true,
                false,
                None,
            ),
            channel(
                "channel-dm-group",
                "dm:group:internal-id",
                None,
                true,
                true,
                false,
                None,
            ),
        ]),
    );
    let mut members = Vec::new();
    for (ch, users) in [
        (
            "channel-general",
            &["user-alice", "user-bob", "user-carol", "user-admin"][..],
        ),
        ("channel-design", &["user-alice", "user-carol", "user-dave"]),
        ("channel-secret", &["user-alice", "user-bob"]),
        ("channel-random", &["user-alice", "user-bob"]),
        ("channel-archive", &["user-alice", "user-admin"]),
        ("channel-dm", &["user-alice", "user-bob"]),
        (
            "channel-dm-group",
            &["user-alice", "user-bob", "user-carol", "user-dave"],
        ),
    ] {
        for u in users {
            members.push(member(u, ch));
        }
    }
    state.store.apply(&ShapeId::ChannelUsers, batch(members));
    // 前日のやりとり (連投のまとめ、メンション、リンク)、今日の投稿 (スレッド、リアクション、添付、連携カード)。
    let card = json!({
        "title": "Deploy complete",
        "description": "production へのデプロイが完了しました。",
        "url": "https://example.com/deploys/42",
        "source": "Deployment service",
        "badge": "success",
        "color": "good",
        "fields": [
            {"title": "Environment", "value": "production", "short": true},
            {"title": "Commit", "value": "79aba0ee", "short": true, "code": true},
        ],
    })
    .to_string();
    state.store.apply(
        &ShapeId::Messages("channel-general".to_string()),
        batch(vec![
            message(
                "message-hello",
                "user-alice",
                "channel-general",
                "おはようございます。今日のリリースは 18:00 です。",
                "2025-01-02 09:00:00",
            ),
            message(
                "message-hello-2",
                "user-alice",
                "channel-general",
                "リリースノートの下書きは https://example.com/release-notes にあります。",
                "2025-01-02 09:01:00",
            ),
            message(
                "message-bob-reply",
                "user-bob",
                "channel-general",
                "了解です！デプロイ後に確認します",
                "2025-01-02 09:02:00",
            ),
            message(
                "message-carol-mention",
                "user-carol",
                "channel-general",
                "@alice さん、リリース前にレビューをお願いします。@bob も見てください。",
                "2025-01-02 09:20:00",
            ),
            message(
                "message-deploy",
                "user-bob",
                "channel-general",
                "デプロイが完了しました 🎉",
                &minutes_ago(30),
            ),
            message_with(
                "message-reply-1",
                "user-carol",
                "channel-general",
                "リリースノートの下書きを共有しました。レビューをお願いします。",
                &minutes_ago(25),
                Some("message-deploy"),
                None,
            ),
            message_with(
                "message-reply-2",
                "user-bob",
                "channel-general",
                "確認しました。2 点だけコメントしました。",
                &minutes_ago(24),
                Some("message-deploy"),
                None,
            ),
            message_with(
                "message-reply-3",
                "user-alice",
                "channel-general",
                "ありがとうございます。修正して再共有します。",
                &minutes_ago(23),
                Some("message-deploy"),
                None,
            ),
            message(
                "message-own",
                "user-alice",
                "channel-general",
                "リリースノートの最終版です。",
                &minutes_ago(20),
            ),
            message_with(
                "message-bot",
                "user-deploy-bot",
                "channel-general",
                "Deploy complete",
                &minutes_ago(10),
                None,
                Some(card.as_str()),
            ),
        ]),
    );
    state.store.apply(
        &ShapeId::Reactions("channel-general".to_string()),
        batch(vec![
            reaction("r1", "message-deploy", "user-alice", "👍", &minutes_ago(29)),
            reaction("r2", "message-deploy", "user-bob", "👍", &minutes_ago(29)),
            reaction("r3", "message-deploy", "user-carol", "👍", &minutes_ago(28)),
            reaction("r4", "message-deploy", "user-dave", "🎉", &minutes_ago(28)),
            reaction(
                "r5",
                "message-carol-mention",
                "user-bob",
                "👀",
                "2025-01-02 09:21:00",
            ),
        ]),
    );
    state.store.apply(
        &ShapeId::Attachments("channel-general".to_string()),
        batch(vec![attachment(
            "attachment-notes",
            "message-own",
            "channel-general",
            "release-notes.pdf",
            1_258_291,
            "application/pdf",
            &minutes_ago(20),
        )]),
    );
    // 前日までは読んだ。今日の最初の他人の投稿から未読。
    state.read_boundary.insert(
        "channel-general".to_string(),
        "2025-01-02T09:30:00".parse().unwrap(),
    );
    state.unread.insert("channel-design".to_string());
    state.unread.insert("channel-dm".to_string());
    if !matches!(scenario, Scenario::Loading | Scenario::Welcome) {
        state.current_channel = Some("channel-general".to_string());
    }
    if matches!(scenario, Scenario::Thread | Scenario::ThreadReply) {
        state.thread = Some(crate::app::ThreadState {
            root_id: "message-deploy".to_string(),
            selected: None,
        });
    }
    state
}

/// 開いた直後の画面を、シナリオの状態にする (ダイアログを開く、入力欄に文字を入れるなど)。
/// 入力欄の中身とフォーカスはビューが持つので、`AppState` だけでは作れない。
fn prepare_scenario(
    scenario: Scenario,
    view: &Entity<WorkspaceView>,
    window: &mut Window,
    cx: &mut App,
) {
    let state = view.read(cx).state.clone();
    let composer = view.read(cx).composer_input.clone();
    match scenario {
        Scenario::Composer => {
            let text = "レビューをお願いします @a";
            composer.update(cx, |i, cx| {
                i.set_text_and_cursor(text.to_string(), text.len(), cx)
            });
            // WorkspaceView の observe (補完中の ↑↓ と Tab の横取り) を動かすため通知する。
            state.update(cx, |s, cx| {
                s.on_composer_changed(text.to_string());
                s.update_mention_autocomplete(text, text.len());
                cx.notify();
            });
            window.focus(&composer.focus_handle(cx));
        }
        Scenario::ThreadReply => {
            let text = "ありがとうございます。確認して返信します。";
            state.update(cx, |s, _| {
                s.focus_thread_reply("message-deploy".to_string());
            });
            composer.update(cx, |i, cx| {
                i.set_text_and_cursor(text.to_string(), text.len(), cx)
            });
            state.update(cx, |s, _| s.on_composer_changed(text.to_string()));
            window.focus(&composer.focus_handle(cx));
        }
        Scenario::CreateChannel => {
            let name = view.read(cx).create_channel_name_input.clone();
            let desc = view.read(cx).create_channel_desc_input.clone();
            state.update(cx, |s, _| {
                s.open_create_channel_modal();
                if let Some(m) = &mut s.create_channel_modal {
                    m.channel_name = "design-review".to_string();
                    m.description = "デザインレビューの連絡用".to_string();
                }
            });
            name.update(cx, |i, cx| i.set_text("design-review", cx));
            desc.update(cx, |i, cx| i.set_text("デザインレビューの連絡用", cx));
            window.focus(&name.focus_handle(cx));
        }
        Scenario::Emoji => {
            state.update(cx, |s, _| {
                s.open_emoji_picker();
                s.emoji_picker_selected = 11;
            });
        }
        Scenario::Delete => {
            state.update(cx, |s, _| s.request_delete("message-own".to_string()));
        }
        Scenario::Switcher => {
            let input = view.read(cx).quick_switcher_input.clone();
            state.update(cx, |s, _| s.open_quick_switcher());
            window.focus(&input.focus_handle(cx));
        }
        Scenario::Search => {
            let input = view.read(cx).message_search_input.clone();
            state.update(cx, |s, _| {
                s.open_message_search();
                s.update_message_search_query("リリース".to_string());
            });
            input.update(cx, |i, cx| i.set_text("リリース", cx));
            window.focus(&input.focus_handle(cx));
        }
        _ => {}
    }
}

/// 確認画面のウィンドウを開く。モードは `tokens::initial_mode()` (`--theme`) に従う。
pub fn run(scenario: Scenario) {
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            if let Err(err) = fonts::register(cx) {
                eprintln!("フォントを登録できませんでした: {err:#}");
            }
            crate::input::bind_input_keys(cx);
            cx.on_action(|_: &crate::Quit, cx| cx.quit());
            cx.bind_keys([
                KeyBinding::new("cmd-q", crate::Quit, None),
                KeyBinding::new("ctrl-q", crate::Quit, None),
            ]);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let mode = initial_mode();
            crate::ui::set_theme(cx, mode);
            let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
            let state: Entity<AppState> = cx.new(|_| fixture_state(scenario, mode));
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |window, cx| {
                    let view = cx.new(|cx| WorkspaceView::new(state, cx));
                    prepare_scenario(scenario, &view, window, cx);
                    view
                },
            );
            if let Err(err) = opened {
                eprintln!("ウィンドウを開けませんでした: {err:#}");
                std::process::exit(1);
            }
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoo2_client_core::electric::ShapeId;

    #[test]
    fn scenario_parses_known_names_only() {
        assert_eq!(Scenario::parse(None), Some(Scenario::Workspace));
        assert_eq!(Scenario::parse(Some("loading")), Some(Scenario::Loading));
        assert_eq!(Scenario::parse(Some("welcome")), Some(Scenario::Welcome));
        assert_eq!(Scenario::parse(Some("composer")), Some(Scenario::Composer));
        assert_eq!(
            Scenario::parse(Some("create-channel")),
            Some(Scenario::CreateChannel)
        );
        assert_eq!(Scenario::parse(Some("switcher")), Some(Scenario::Switcher));
        assert_eq!(Scenario::parse(Some("x")), None);
    }

    #[test]
    fn workspace_fixture_has_every_row_kind() {
        let s = fixture_state(Scenario::Workspace, ThemeMode::Light);
        assert!(s.store.is_synced(&ShapeId::Channels));
        let channels = s.store.channels();
        assert!(channels.iter().any(|c| c.is_private && !c.is_dm));
        assert!(channels.iter().any(|c| c.archive_at.is_some()));
        assert!(
            channels
                .iter()
                .any(|c| c.is_dm && c.channel_name.starts_with("dm:group:"))
        );
        assert!(
            channels
                .iter()
                .any(|c| c.is_dm && !c.channel_name.starts_with("dm:group:"))
        );
        assert_eq!(s.unread.len(), 2);
        assert_eq!(s.current_channel.as_deref(), Some("channel-general"));
        assert!(s.login_phase.is_none());
    }

    #[test]
    fn loading_fixture_has_nothing_synced() {
        let s = fixture_state(Scenario::Loading, ThemeMode::Dark);
        assert!(!s.store.is_synced(&ShapeId::Channels));
        assert!(s.store.channels().is_empty());
    }

    #[test]
    fn welcome_fixture_selects_nothing() {
        let s = fixture_state(Scenario::Welcome, ThemeMode::Dark);
        assert!(s.current_channel.is_none());
        assert!(!s.store.channels().is_empty());
    }
}
