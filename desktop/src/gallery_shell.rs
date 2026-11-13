//! アプリシェルの確認画面 (`zoo2-desktop gallery shell [loading|welcome]`)。
//!
//! サーバーなしでワークスペースを描くため、固定のデータで `AppState` を作る。
//! データは `frontend/.storybook/fixtures.ts` (`ChannelSidebar.stories.tsx` と VRT のカタログ) に合わせてある。
//! 本番の画面からは使わない。

use gpui::{
    App, Application, Bounds, Entity, KeyBinding, WindowBounds, WindowOptions, prelude::*, px, size,
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
}

impl Scenario {
    pub fn parse(arg: Option<&str>) -> Option<Self> {
        match arg {
            None | Some("workspace") => Some(Self::Workspace),
            Some("loading") => Some(Self::Loading),
            Some("welcome") => Some(Self::Welcome),
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
    (
        id.to_string(),
        json!({
            "id": id, "user_id": user_id, "channel_id": channel_id, "content": content,
            "created_at": at, "updated_at": null, "deleted_at": null, "parent_id": null,
        }),
    )
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
                "message-bob-reply",
                "user-bob",
                "channel-general",
                "了解です！デプロイ後に確認します",
                "2025-01-02 09:02:00",
            ),
        ]),
    );
    state.unread.insert("channel-design".to_string());
    state.unread.insert("channel-dm".to_string());
    if scenario == Scenario::Workspace {
        state.current_channel = Some("channel-general".to_string());
    }
    state
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
                move |_, cx| cx.new(|cx| WorkspaceView::new(state, cx)),
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
