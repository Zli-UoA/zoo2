use gpui::{
    App, Application, Bounds, Context, SharedString, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, prelude::*, px, rgb, size,
};
use zoo2_client_core::{
    config::{AuthConfig, Config},
    store::Store,
};

/// デスクトップ版のウィンドウ初期状態。
/// Phase 2 以降で Electric のシェイプ購読と REST API クライアントをここに接続する。
#[derive(Debug)]
pub struct AppState {
    pub api_base_url: String,
    pub auth_summary: String,
    pub store: Store,
}

impl AppState {
    pub fn from_config(config: &Config) -> Self {
        let auth_summary = match &config.auth {
            AuthConfig::Dev { user_id } => format!("Dev モード (X-Dev-User: {user_id})"),
            AuthConfig::Oidc(oidc) => format!("OIDC ({})", oidc.issuer),
        };
        Self {
            api_base_url: config.api_base_url.clone(),
            auth_summary,
            store: Store::default(),
        }
    }
}

pub struct WorkspaceView {
    state: AppState,
}

impl WorkspaceView {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let api_url: SharedString = self.state.api_base_url.clone().into();
        let auth_summary: SharedString = self.state.auth_summary.clone().into();
        let channel_count = self.state.store.channels().len();

        div()
            .flex()
            .size_full()
            .bg(rgb(0x0f172a))
            .text_color(rgb(0xf8fafc))
            // 左ペイン: チャンネル・DM サイドバー領域
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(240.0))
                    .h_full()
                    .bg(rgb(0x1e293b))
                    .border_r_1()
                    .border_color(rgb(0x334155))
                    .p_4()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .text_color(rgb(0xf8fafc))
                            .child("zoo2 Desktop"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x94a3b8))
                            .child(format!("チャンネル ({channel_count})")),
                    ),
            )
            // 中央ペイン: タイムライン領域
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .h_full()
                    .justify_center()
                    .items_center()
                    .p_6()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .text_color(rgb(0xf8fafc))
                            .child("zoo2 Desktop (GPUI)"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x94a3b8))
                            .child(format!("接続先: {api_url}")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(rgb(0x94a3b8))
                            .child(format!("認証設定: {auth_summary}")),
                    ),
            )
    }
}

pub fn run(config: Config) {
    let state = AppState::from_config(&config);
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        if let Err(err) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("zoo2".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| cx.new(|_| WorkspaceView::new(state)),
        ) {
            eprintln!("ウィンドウを開けませんでした: {err:#}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use zoo2_client_core::config::OidcConfig;

    use super::*;

    #[test]
    fn builds_app_state_from_dev_and_oidc_configs() {
        let dev_config = Config {
            api_base_url: "http://localhost:8000".into(),
            auth: AuthConfig::Dev {
                user_id: "00000000-0000-0000-0000-000000000001".into(),
            },
            check_updates: false,
        };
        let dev_state = AppState::from_config(&dev_config);
        assert_eq!(dev_state.api_base_url, "http://localhost:8000");
        assert!(dev_state.auth_summary.contains("X-Dev-User"));
        assert_eq!(dev_state.store.channels().len(), 0);

        let oidc_config = Config {
            api_base_url: "https://zli.chat".into(),
            auth: AuthConfig::Oidc(OidcConfig {
                issuer: "https://account.zli.works/api/auth".into(),
                client_id: "client".into(),
                resource: Some("https://zli.chat/api".into()),
                scope: "openid profile offline_access".into(),
            }),
            check_updates: true,
        };
        let oidc_state = AppState::from_config(&oidc_config);
        assert_eq!(oidc_state.api_base_url, "https://zli.chat");
        assert!(
            oidc_state
                .auth_summary
                .contains("https://account.zli.works/api/auth")
        );
    }
}
