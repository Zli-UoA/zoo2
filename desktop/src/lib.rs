pub mod app;
pub mod input;
pub mod login;
pub mod mfm_view;
pub mod theme;
pub mod view;

use std::sync::Arc;

use gpui::{
    App, Application, Bounds, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions, actions,
    prelude::*, px, size,
};
use zoo2_client_core::config::Config;

pub use app::AppState;
pub use view::WorkspaceView;

actions!(zoo2_desktop_app, [Quit]);

pub fn run(config: Config) {
    let runtime = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Tokio ランタイムの起動に失敗しました"),
    );

    Application::new().run(move |cx: &mut App| {
        input::bind_input_keys(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        let state = AppState::spawn_entity(config, runtime.clone(), cx);

        if let Err(err) = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("zoo2".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |_, cx| cx.new(|cx| WorkspaceView::new(state, cx)),
        ) {
            eprintln!("ウィンドウを開けませんでした: {err:#}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
}
