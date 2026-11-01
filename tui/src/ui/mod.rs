mod app;
mod timeline;
mod view;

use anyhow::Result;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, EventStream};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::{api::ApiClient, http::Credentials};
pub use app::App;

/// チャット画面を開き、終了するまでイベントを処理する。
pub async fn run(
    http: reqwest::Client,
    api_base_url: String,
    credentials: Credentials,
) -> Result<()> {
    let api = ApiClient::new(http.clone(), &api_base_url, credentials.clone());
    // 画面を開く前に自分を確かめる。認証の失敗をターミナルの通常表示で伝えるため。
    let me = api.me().await?;

    let (shape_tx, mut shape_rx) = mpsc::unbounded_channel();
    let (app_tx, mut app_rx) = mpsc::unbounded_channel();
    let mut app = App::new(api, http, api_base_url, credentials, me, shape_tx, app_tx);

    let mut terminal = ratatui::init();
    // 貼り付けを1つのイベントで受け取り、中の改行を送信の Enter と区別する。
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste);
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(app::TICK);
    let result = loop {
        if let Err(err) = terminal.draw(|frame| view::draw(frame, &mut app)) {
            break Err(err.into());
        }
        tokio::select! {
            event = events.next() => match event {
                Some(Ok(event)) => app.on_terminal_event(event),
                Some(Err(err)) => break Err(err.into()),
                None => break Ok(()),
            },
            Some(event) = shape_rx.recv() => {
                app.on_shape(event);
                // 初回同期では多数のバッチが続けて届くので、まとめて反映してから描く。
                while let Ok(event) = shape_rx.try_recv() {
                    app.on_shape(event);
                }
            }
            Some(event) = app_rx.recv() => app.on_app_event(event),
            _ = tick.tick() => app.on_tick(),
        }
        if app.should_quit {
            break Ok(());
        }
    };
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}
