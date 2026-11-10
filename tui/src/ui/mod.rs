mod app;
mod timeline;
mod view;

use std::{future::Future, io, pin::Pin};

use anyhow::{Result, anyhow};
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyCode, KeyEventKind,
    KeyModifiers,
};
use futures_util::{Stream, StreamExt};
use ratatui::{
    Terminal,
    backend::Backend,
    text::Line,
    widgets::{Block, Borders, Paragraph, Wrap},
};
use tokio::sync::mpsc;

use crate::{
    api::{ApiClient, Me},
    auth::token,
    config,
    http::{Credentials, HttpError, TokenPersistenceReport, TokenPersistenceStatus},
    update,
};
pub use app::App;

const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// チャット画面を開き、終了するまでイベントを処理する。
pub async fn run(
    http: reqwest::Client,
    api_base_url: String,
    credentials: Credentials,
    check_updates: bool,
) -> Result<()> {
    let api = ApiClient::new(http.clone(), &api_base_url, credentials.clone());
    // 画面を開く前に自分を確かめる。認証の失敗をターミナルの通常表示で伝えるため。
    let mut ctrl_c: CtrlCSignal = Box::pin(tokio::signal::ctrl_c());
    let me = startup_me_or_interrupt(&api, &credentials, &mut ctrl_c).await?;

    let (shape_tx, mut shape_rx) = mpsc::unbounded_channel();
    let (app_tx, mut app_rx) = mpsc::unbounded_channel();
    let updates = check_updates.then(|| spawn_update_check(http.clone(), app_tx.clone()));
    let mut app = App::new(
        api,
        http,
        api_base_url,
        credentials.clone(),
        me,
        shape_tx,
        app_tx,
    );

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
            signal = ctrl_c.as_mut() => match signal {
                Ok(()) => app.should_quit = true,
                Err(error) => break Err(anyhow!("Ctrl+C signal handler failed: {error}")),
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
    if let Some(updates) = updates {
        updates.abort();
    }
    let (result, shutdown_warning) = finish_application_shutdown(result, &credentials).await;
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    restore_terminal_then_report(ratatui::restore, shutdown_warning, |warning| {
        eprintln!("{warning}");
    });
    result
}

type CtrlCSignal = Pin<Box<dyn Future<Output = io::Result<()>> + Send>>;

async fn startup_me_or_interrupt(
    api: &ApiClient,
    credentials: &Credentials,
    ctrl_c: &mut CtrlCSignal,
) -> Result<Me> {
    let me_result = tokio::select! {
        biased;
        signal = ctrl_c.as_mut() => {
            let report = credentials.finish_token_tasks().await;
            return match signal {
                Ok(()) => Err(startup_interrupt_error(report)),
                Err(error) => Err(startup_interrupt_error(report)
                    .context(format!("Ctrl+C signal handler failed: {error}"))),
            };
        }
        result = api.me() => result,
    };
    match me_result {
        Ok(me) => Ok(me),
        Err(error) if token::is_refresh_persistence_pending(&error) => {
            run_startup_recovery(api, error, ctrl_c).await
        }
        Err(error) => {
            let report = credentials.persistence_report().await;
            if report.status == TokenPersistenceStatus::Unknown {
                Err(startup_unknown_refresh_error(report))
            } else {
                Err(error)
            }
        }
    }
}

fn startup_interrupt_error(report: TokenPersistenceReport) -> anyhow::Error {
    match report.status {
        TokenPersistenceStatus::Saved => {
            anyhow!("Ctrl+C を受けて起動を中断しました。token の保存は完了しています")
        }
        TokenPersistenceStatus::SessionChanged => anyhow!(
            "Ctrl+C を受けて起動を中断しました。logout または別 session への変更を確認しました"
        ),
        TokenPersistenceStatus::Pending => anyhow!(
            "Ctrl+C を受けて起動を中断しました。{} 新 refresh token は終了後に失われ、別 process の logout では失効できない可能性があります",
            report.reason.unwrap_or("token 保存が完了していません。")
        ),
        TokenPersistenceStatus::Unknown => startup_unknown_refresh_error(report),
    }
}

fn startup_unknown_refresh_error(report: TokenPersistenceReport) -> anyhow::Error {
    anyhow!(
        "{} 起動時の refresh 結果を確認できません。通信断や timeout の時点で IdP が token をローテーションしていた場合、未保存 token が失われ、別 process の logout でも失効できない可能性があります",
        report
            .reason
            .unwrap_or("IdP との refresh 通信に失敗しました。")
    )
}

async fn finish_application_shutdown(
    result: Result<()>,
    credentials: &Credentials,
) -> (Result<()>, Option<String>) {
    let report = credentials.finish_token_tasks().await;
    match (result, token_shutdown_error(report)) {
        (Ok(()), Some(error)) => (Err(error), None),
        (Err(result), Some(warning)) => (Err(result), Some(warning.to_string())),
        (result, None) => (result, None),
    }
}

fn restore_terminal_then_report(
    restore: impl FnOnce(),
    warning: Option<String>,
    report: impl FnOnce(&str),
) {
    restore();
    if let Some(warning) = warning {
        report(&warning);
    }
}

fn token_shutdown_error(report: TokenPersistenceReport) -> Option<anyhow::Error> {
    match report.status {
        TokenPersistenceStatus::Pending => Some(anyhow!(
            "{} アプリ終了時にも token 保存が完了していません。新 refresh token は終了後に失われ、別 process の logout では失効できない可能性があります",
            report
                .reason
                .unwrap_or("token 保存の再試行に失敗しました。")
        )),
        TokenPersistenceStatus::Unknown => Some(anyhow!(
            "{} IdP の refresh 結果を確認できません。refresh token がローテーション済みでも保存できていない可能性があります",
            report
                .reason
                .unwrap_or("token manager の終了状態を確認できません。")
        )),
        TokenPersistenceStatus::Saved | TokenPersistenceStatus::SessionChanged => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupAction {
    Retry,
    Quit,
    Ignore,
}

struct StartupRecovery {
    message: String,
    reason: &'static str,
}

impl StartupRecovery {
    fn new(error: &anyhow::Error) -> Self {
        let reason = token::refresh_persistence_retry_reason(error);
        Self {
            message: reason.to_string(),
            reason,
        }
    }

    fn action(&self, event: &Event) -> StartupAction {
        let Event::Key(key) = event else {
            return StartupAction::Ignore;
        };
        if key.kind != KeyEventKind::Press {
            return StartupAction::Ignore;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c')
            || matches!(key.code, KeyCode::Char('q' | 'Q'))
        {
            StartupAction::Quit
        } else if matches!(key.code, KeyCode::Char('r' | 'R')) {
            StartupAction::Retry
        } else {
            StartupAction::Ignore
        }
    }

    fn quit_error(&self, report: TokenPersistenceReport) -> anyhow::Error {
        match report.status {
            TokenPersistenceStatus::Pending => anyhow!(
                "{} 認証情報の保存が完了する前に終了しました。新 refresh token はこの process のメモリにだけあり、終了後は失われます。別 process の logout では失効できない可能性があります",
                report.reason.unwrap_or(self.reason)
            ),
            TokenPersistenceStatus::SessionChanged => self.session_changed_error(),
            TokenPersistenceStatus::Saved => {
                anyhow!("起動時の再試行を中断しました。token の保存は完了しています")
            }
            TokenPersistenceStatus::Unknown => anyhow!(
                "{} 起動時の再試行を中断しました。IdP の refresh 結果を確認できず、未保存 token が失われる可能性があります",
                report
                    .reason
                    .unwrap_or("token の保存状態を確認できません。")
            ),
        }
    }

    fn session_changed_error(&self) -> anyhow::Error {
        anyhow!(
            "ログイン状態が logout または別 session に変わったため、起動時の復旧を終了します。未保存の refresh token は別 process の logout で失効していない可能性があります"
        )
    }

    fn draw<B: Backend>(
        &self,
        terminal: &mut Terminal<B>,
        retrying: bool,
        exiting: bool,
    ) -> Result<()> {
        terminal.draw(|frame| {
            let lines = vec![
                Line::from(if exiting {
                    "token 更新の完了を待って終了しています。"
                } else if retrying {
                    "再試行中です。追加の r は無視されます。"
                } else {
                    "r: 再試行 (保存先を直したら)"
                }),
                Line::from(if exiting {
                    "refresh token の保存結果を確認しています。"
                } else {
                    "q / Ctrl+C: 終了"
                }),
                Line::from(if exiting {
                    "終了処理中はキー入力を受け付けません。"
                } else {
                    "終了すると新 token はメモリから失われます。"
                }),
                Line::from("別 process の logout では失効できない可能性があります。"),
                Line::from(""),
                Line::from(self.message.clone()),
            ];
            frame.render_widget(
                Paragraph::new(lines).wrap(Wrap { trim: true }).block(
                    Block::default()
                        .title(" 認証情報を保存できません ")
                        .borders(Borders::ALL),
                ),
                frame.area(),
            );
        })?;
        Ok(())
    }
}

#[cfg(test)]
async fn drive_startup_recovery<S, D>(
    api: ApiClient,
    recovery: StartupRecovery,
    events: S,
    draw: D,
) -> (Result<Me>, StartupRecovery)
where
    S: Stream<Item = std::io::Result<Event>> + Unpin,
    D: FnMut(&StartupRecovery, bool, bool) -> Result<()>,
{
    let mut ctrl_c: CtrlCSignal = Box::pin(std::future::pending::<io::Result<()>>());
    drive_startup_recovery_with_ctrl_c(api, recovery, events, &mut ctrl_c, draw).await
}

async fn drive_startup_recovery_with_ctrl_c<S, D>(
    api: ApiClient,
    mut recovery: StartupRecovery,
    mut events: S,
    ctrl_c: &mut CtrlCSignal,
    mut draw: D,
) -> (Result<Me>, StartupRecovery)
where
    S: Stream<Item = std::io::Result<Event>> + Unpin,
    D: FnMut(&StartupRecovery, bool, bool) -> Result<()>,
{
    let (retry_tx, mut retry_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel::<TokenPersistenceReport>();
    let mut retry_in_flight = false;
    let mut retry_abort = None;
    let mut quitting = false;
    let mut input_closed = false;
    let mut quit_context = None;

    loop {
        if let Err(error) = draw(&recovery, retry_in_flight, quitting)
            && !quitting
        {
            quit_context = Some(format!("起動時の復旧画面を描画できません: {error}"));
            start_startup_quit_status(&api, &status_tx, &mut retry_abort, &mut retry_in_flight);
            quitting = true;
        }

        tokio::select! {
            biased;
            event = events.next(), if !input_closed => {
                match event {
                    Some(Ok(event)) => match recovery.action(&event) {
                        StartupAction::Retry if !retry_in_flight && !quitting => {
                            retry_in_flight = true;
                            let api = api.clone();
                            let tx = retry_tx.clone();
                            let task = tokio::spawn(async move {
                                let _ = tx.send(api.me().await);
                            });
                            retry_abort = Some(task.abort_handle());
                        }
                        StartupAction::Quit if !quitting => {
                            start_startup_quit_status(
                                &api,
                                &status_tx,
                                &mut retry_abort,
                                &mut retry_in_flight,
                            );
                            quitting = true;
                        }
                        _ => {}
                    },
                    Some(Err(error)) => {
                        quit_context = Some(format!("端末イベントを読み取れません: {error}"));
                        start_startup_quit_status(
                            &api,
                            &status_tx,
                            &mut retry_abort,
                            &mut retry_in_flight,
                        );
                        quitting = true;
                        input_closed = true;
                    }
                    None => {
                        quit_context = Some("端末イベントが終了しました".into());
                        start_startup_quit_status(
                            &api,
                            &status_tx,
                            &mut retry_abort,
                            &mut retry_in_flight,
                        );
                        quitting = true;
                        input_closed = true;
                    }
                }
            }
            result = retry_rx.recv(), if retry_in_flight => {
                retry_in_flight = false;
                retry_abort = None;
                match result {
                    Some(Ok(me)) => return (Ok(me), recovery),
                    Some(Err(error)) if token::is_refresh_persistence_pending(&error) => {
                        recovery.reason = token::refresh_persistence_retry_reason(&error);
                        recovery.message = recovery.reason.to_string();
                    }
                    Some(Err(error))
                        if error.downcast_ref::<HttpError>()
                            .is_some_and(|error| matches!(error, HttpError::Unauthorized)) =>
                    {
                        return (Err(recovery.session_changed_error()), recovery);
                    }
                    Some(Err(error)) => return (Err(error), recovery),
                    None => {
                        recovery.message = "再試行処理が応答しませんでした。r で再試行してください。".into();
                    }
                }
            }
            status = status_rx.recv(), if quitting => {
                let report = status.unwrap_or(TokenPersistenceReport {
                    status: TokenPersistenceStatus::Unknown,
                    reason: None,
                });
                let mut error = recovery.quit_error(report);
                if let Some(context) = quit_context.take() {
                    error = error.context(context);
                }
                return (Err(error), recovery);
            }
            signal = ctrl_c.as_mut(), if !quitting => {
                match signal {
                    Ok(()) => quit_context = Some("Ctrl+C を受けました".into()),
                    Err(error) => quit_context = Some(format!("Ctrl+C signal handler failed: {error}")),
                }
                start_startup_quit_status(
                    &api,
                    &status_tx,
                    &mut retry_abort,
                    &mut retry_in_flight,
                );
                quitting = true;
            }
        }
    }
}

fn start_startup_quit_status(
    api: &ApiClient,
    status_tx: &mpsc::UnboundedSender<TokenPersistenceReport>,
    retry_abort: &mut Option<tokio::task::AbortHandle>,
    retry_in_flight: &mut bool,
) {
    if let Some(abort) = retry_abort.take() {
        abort.abort();
    }
    *retry_in_flight = false;
    let api = api.clone();
    let tx = status_tx.clone();
    tokio::spawn(async move {
        let _ = tx.send(api.finish_token_tasks().await);
    });
}

fn initialize_startup_terminal<B: Backend>(
    recovery: &StartupRecovery,
    initialize: impl FnOnce() -> std::io::Result<Terminal<B>>,
    warn: impl FnOnce(&str),
) -> Result<Terminal<B>> {
    match initialize() {
        Ok(terminal) => Ok(terminal),
        Err(error) => {
            let _ = ratatui::try_restore();
            let warning = recovery.quit_error(TokenPersistenceReport {
                status: TokenPersistenceStatus::Pending,
                reason: Some(recovery.reason),
            });
            let warning_message = warning.to_string();
            warn(&warning_message);
            Err(warning.context(format!("起動時の端末を初期化できません: {error}")))
        }
    }
}

async fn run_startup_recovery(
    api: &ApiClient,
    error: anyhow::Error,
    ctrl_c: &mut CtrlCSignal,
) -> Result<Me> {
    let recovery = StartupRecovery::new(&error);
    let mut terminal = initialize_startup_terminal(&recovery, ratatui::try_init, |warning| {
        eprintln!("{warning}");
    })?;
    let (result, _) = drive_startup_recovery_with_ctrl_c(
        api.clone(),
        recovery,
        EventStream::new(),
        ctrl_c,
        |recovery, retrying, exiting| recovery.draw(&mut terminal, retrying, exiting),
    )
    .await;
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

/// 起動時と、そのあと一定の間隔で新しい版を調べ、見つかれば画面に知らせる。
/// 調べられなかったときは何も出さない。
fn spawn_update_check(
    http: reqwest::Client,
    tx: mpsc::UnboundedSender<app::AppEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let cache = config::config_dir().map(|dir| {
            let _ = std::fs::create_dir_all(&dir);
            update::cache_path(&dir)
        });
        loop {
            let now = jiff::Timestamp::now().as_second();
            if let Ok(Some(version)) = update::check(
                &http,
                update::RELEASES_URL,
                cache.as_deref(),
                CURRENT_VERSION,
                now,
            )
            .await
            {
                let _ = tx.send(app::AppEvent::UpdateAvailable(version));
            }
            tokio::time::sleep(update::CHECK_INTERVAL).await;
        }
    })
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        path::PathBuf,
        pin::Pin,
        process::{Command, Stdio},
        sync::Arc,
        time::Duration,
    };

    use crossterm::event::{KeyEvent, KeyModifiers};
    use futures_util::stream;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use tokio::sync::mpsc;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;
    use crate::{
        auth::token::{self, StoredTokens, TokenManager},
        config::OidcConfig,
        http::Credentials,
    };

    struct ReadOnlyDirectory {
        path: PathBuf,
        original_mode: u32,
    }

    impl ReadOnlyDirectory {
        fn new(path: &std::path::Path) -> Self {
            use std::os::unix::fs::PermissionsExt;

            let original_mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
            std::fs::set_permissions(
                path,
                std::fs::Permissions::from_mode(original_mode & !0o222),
            )
            .unwrap();
            Self {
                path: path.to_path_buf(),
                original_mode,
            }
        }

        fn allow_writes(&self) {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(self.original_mode),
            )
            .unwrap();
        }
    }

    impl Drop for ReadOnlyDirectory {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;

            let _ = std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(self.original_mode),
            );
        }
    }

    fn oidc(issuer: String) -> OidcConfig {
        OidcConfig {
            issuer,
            client_id: "zoo2-tui".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        }
    }

    fn stored_tokens(oidc: &OidcConfig, api_base_url: &str) -> StoredTokens {
        StoredTokens {
            session_id: "startup-recovery-session".into(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: api_base_url.into(),
            access_token: "expired-access".into(),
            refresh_token: Some("startup-refresh".into()),
            expires_at: token::now() + 30,
        }
    }

    fn rendered(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent::new(code, modifiers))
    }

    fn event_stream(
        receiver: mpsc::UnboundedReceiver<Event>,
    ) -> Pin<Box<dyn Stream<Item = std::io::Result<Event>> + Send>> {
        Box::pin(stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|event| (Ok(event), receiver))
        }))
    }

    type DrawSnapshot = (bool, bool, String);

    fn test_drawer() -> (
        impl FnMut(&StartupRecovery, bool, bool) -> Result<()> + Send + 'static,
        mpsc::UnboundedReceiver<DrawSnapshot>,
    ) {
        let mut terminal = Terminal::new(TestBackend::new(72, 14)).unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        let draw = move |recovery: &StartupRecovery, retrying, exiting| {
            recovery.draw(&mut terminal, retrying, exiting)?;
            let text = rendered(&terminal, 72, 14)
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            let _ = tx.send((retrying, exiting, text));
            Ok(())
        };
        (draw, rx)
    }

    async fn wait_for_draw(
        receiver: &mut mpsc::UnboundedReceiver<DrawSnapshot>,
        predicate: impl Fn(&DrawSnapshot) -> bool,
    ) -> DrawSnapshot {
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(snapshot) = receiver.recv().await {
                if predicate(&snapshot) {
                    return snapshot;
                }
            }
            panic!("draw channel closed before expected state")
        })
        .await
        .expect("startup recovery state should be drawn")
    }

    async fn wait_for_request(server: &MockServer, predicate: impl Fn(&wiremock::Request) -> bool) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if server
                    .received_requests()
                    .await
                    .unwrap_or_default()
                    .iter()
                    .any(&predicate)
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("controlled HTTP request should arrive")
    }

    struct PendingStartup {
        server: MockServer,
        oidc: OidcConfig,
        api_base_url: String,
        path: PathBuf,
        dir: PathBuf,
        readonly: ReadOnlyDirectory,
        api: ApiClient,
        recovery: StartupRecovery,
    }

    struct NormalShutdown {
        server: MockServer,
        oidc: OidcConfig,
        api_base_url: String,
        path: PathBuf,
        dir: PathBuf,
        manager: Arc<TokenManager>,
        credentials: Credentials,
        readonly: Option<ReadOnlyDirectory>,
    }

    async fn normal_shutdown_fixture(
        readonly: bool,
        response_delay: Duration,
        client_timeout: Option<Duration>,
    ) -> NormalShutdown {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/api/auth/oauth2/token", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=startup-refresh"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(response_delay)
                    .set_body_json(json!({
                        "access_token": "shutdown-rotated-access",
                        "refresh_token": "shutdown-rotated-refresh",
                        "expires_in": 300
                    })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let oidc = oidc(format!("{}/api/auth", server.uri()));
        let api_base_url = server.uri();
        let dir =
            std::env::temp_dir().join(format!("zoo2-ui-token-shutdown-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token.json");
        let mut tokens = stored_tokens(&oidc, &api_base_url);
        tokens.expires_at = token::now() - 1;
        token::save(&path, &tokens).unwrap();
        let readonly = readonly.then(|| ReadOnlyDirectory::new(&dir));

        let http = match client_timeout {
            Some(timeout) => reqwest::Client::builder().timeout(timeout).build().unwrap(),
            None => reqwest::Client::new(),
        };
        let manager = Arc::new(TokenManager::new(http, oidc.clone(), path.clone(), tokens));
        let credentials = Credentials::Bearer(manager.clone());
        NormalShutdown {
            server,
            oidc,
            api_base_url,
            path,
            dir,
            manager,
            credentials,
            readonly,
        }
    }

    async fn pending_startup() -> PendingStartup {
        pending_startup_with(300, Duration::ZERO, Duration::ZERO).await
    }

    async fn pending_startup_with(
        initial_expires_in: u64,
        retry_refresh_delay: Duration,
        me_delay: Duration,
    ) -> PendingStartup {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/api/auth/oauth2/token", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=startup-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "rotated-access",
                "refresh_token": "rotated-refresh",
                "expires_in": initial_expires_in
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut retry_refresh = Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=rotated-refresh"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(retry_refresh_delay)
                    .set_body_json(json!({
                        "access_token": "retry-rotated-access",
                        "refresh_token": "retry-rotated-refresh",
                        "expires_in": 300
                    })),
            );
        if !retry_refresh_delay.is_zero() {
            retry_refresh = retry_refresh.expect(1);
        }
        retry_refresh.mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/api/users/me"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(me_delay)
                    .set_body_json(json!({
                        "id": "u1", "user_name": "alice", "display_name": "Alice"
                    })),
            )
            .mount(&server)
            .await;

        let oidc = oidc(format!("{}/api/auth", server.uri()));
        let api_base_url = server.uri();
        let dir =
            std::env::temp_dir().join(format!("zoo2-ui-startup-pending-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("token.json");
        token::save(&path, &stored_tokens(&oidc, &api_base_url)).unwrap();

        let readonly = ReadOnlyDirectory::new(&dir);
        let http = reqwest::Client::new();
        let manager = Arc::new(TokenManager::new(
            http.clone(),
            oidc.clone(),
            path.clone(),
            stored_tokens(&oidc, &api_base_url),
        ));
        let api = ApiClient::new(http, &api_base_url, Credentials::Bearer(manager));
        let error = api.me().await.unwrap_err();
        assert!(token::is_refresh_persistence_pending(&error), "{error:#}");
        let recovery = StartupRecovery::new(&error);

        PendingStartup {
            server,
            oidc,
            api_base_url,
            path,
            dir,
            readonly,
            api,
            recovery,
        }
    }

    #[tokio::test]
    async fn startup_keeps_manager_and_retries_persistence_on_real_r_key() {
        use std::os::unix::fs::PermissionsExt;
        let fixture = pending_startup().await;
        let token_mode = std::fs::metadata(&fixture.path)
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(0o0)).unwrap();

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (draw, mut draw_rx) = test_drawer();
        let run = tokio::spawn(drive_startup_recovery(
            fixture.api.clone(),
            fixture.recovery,
            event_stream(event_rx),
            draw,
        ));
        event_tx
            .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();
        wait_for_draw(&mut draw_rx, |state| state.0 && !state.1).await;
        let failed = wait_for_draw(&mut draw_rx, |state| {
            !state.0 && !state.1 && state.2.contains("アクセスが拒否")
        })
        .await;
        assert!(failed.2.contains("token"));
        std::fs::set_permissions(&fixture.path, std::fs::Permissions::from_mode(token_mode))
            .unwrap();
        fixture.readonly.allow_writes();
        event_tx
            .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();

        let (result, _) = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap().user_name, "alice");
        let persisted =
            token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(persisted.refresh_token.as_deref(), Some("rotated-refresh"));
        fixture.server.verify().await;
        drop(fixture.readonly);
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_quit_during_refresh_waits_for_owned_rotation_to_persist_and_ignores_double_r()
    {
        let fixture = pending_startup_with(1, Duration::from_millis(700), Duration::ZERO).await;
        fixture.readonly.allow_writes();
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (draw, mut draw_rx) = test_drawer();
        let run = tokio::spawn(drive_startup_recovery(
            fixture.api.clone(),
            fixture.recovery,
            event_stream(event_rx),
            draw,
        ));

        event_tx
            .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();
        wait_for_draw(&mut draw_rx, |state| state.0 && !state.1).await;
        wait_for_request(&fixture.server, |request| {
            String::from_utf8_lossy(&request.body).contains("refresh_token=rotated-refresh")
        })
        .await;
        event_tx
            .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();
        event_tx
            .send(key(KeyCode::Char('q'), KeyModifiers::NONE))
            .unwrap();
        wait_for_draw(&mut draw_rx, |state| state.1).await;

        let (result, _) = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .unwrap()
            .unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("保存は完了しています"), "{error}");
        assert!(!error.contains("終了後は失われ"), "{error}");
        let persisted =
            token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            persisted.refresh_token.as_deref(),
            Some("retry-rotated-refresh")
        );
        let retry_refreshes = fixture
            .server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| {
                String::from_utf8_lossy(&request.body).contains("refresh_token=rotated-refresh")
            })
            .count();
        assert_eq!(retry_refreshes, 1);
        fixture.server.verify().await;
        drop(fixture.readonly);
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_ctrl_c_cancels_hung_me_request_after_token_is_saved() {
        for exit_event in [
            key(KeyCode::Char('q'), KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let fixture = pending_startup_with(300, Duration::ZERO, Duration::from_secs(3)).await;
            fixture.readonly.allow_writes();
            let (event_tx, event_rx) = mpsc::unbounded_channel();
            let (draw, _) = test_drawer();
            let run = tokio::spawn(drive_startup_recovery(
                fixture.api.clone(),
                fixture.recovery,
                event_stream(event_rx),
                draw,
            ));

            event_tx
                .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
                .unwrap();
            wait_for_request(&fixture.server, |request| {
                request.method == "GET" && request.url.path() == "/api/users/me"
            })
            .await;
            event_tx.send(exit_event).unwrap();

            let (result, _) = tokio::time::timeout(Duration::from_secs(1), run)
                .await
                .expect("q or Ctrl+C should not wait for the unresponsive /me endpoint")
                .unwrap();
            let error = result.unwrap_err().to_string();
            assert!(error.contains("保存は完了しています"), "{error}");
            assert!(!error.contains("終了後は失われ"), "{error}");
            let persisted =
                token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(persisted.refresh_token.as_deref(), Some("rotated-refresh"));
            fixture.server.verify().await;
            drop(fixture.readonly);
            token::delete(&fixture.path).unwrap();
            std::fs::remove_dir_all(fixture.dir).unwrap();
        }
    }

    #[tokio::test]
    async fn startup_timeout_reports_uncertain_rotation_and_user_risk() {
        let fixture = normal_shutdown_fixture(
            false,
            Duration::from_secs(2),
            Some(Duration::from_millis(50)),
        )
        .await;
        let result = run(
            reqwest::Client::builder()
                .timeout(Duration::from_millis(50))
                .build()
                .unwrap(),
            fixture.api_base_url.clone(),
            fixture.credentials.clone(),
            false,
        )
        .await;
        let message = result.unwrap_err().to_string();
        assert!(message.contains("起動時"), "{message}");
        assert!(
            message.contains("refresh 結果を確認できません"),
            "{message}"
        );
        assert!(message.contains("未保存 token が失われ"), "{message}");
        assert!(!message.contains("shutdown-rotated-refresh"));
        fixture.server.verify().await;
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_sigint_waits_for_registered_rotation_to_persist_in_child_process() {
        if std::env::var_os("ZOO2_TUI_STARTUP_SIGINT_WORKER_PATH").is_some() {
            return;
        }
        let fixture = normal_shutdown_fixture(false, Duration::from_millis(500), None).await;
        run_startup_sigint_child(&fixture, |request| {
            request.method == "POST" && request.url.path() == "/api/auth/oauth2/token"
        })
        .await;
        let persisted =
            token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            persisted.refresh_token.as_deref(),
            Some("shutdown-rotated-refresh")
        );
        fixture.server.verify().await;
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_sigint_interrupts_an_unresponsive_initial_me() {
        let fixture = normal_shutdown_fixture(false, Duration::ZERO, None).await;
        Mock::given(method("GET"))
            .and(path("/api/users/me"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(3))
                    .set_body_json(json!({
                        "id": "u1", "user_name": "alice", "display_name": "Alice"
                    })),
            )
            .mount(&fixture.server)
            .await;
        run_startup_sigint_child(&fixture, |request| {
            request.method == "GET" && request.url.path() == "/api/users/me"
        })
        .await;
        let persisted =
            token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            persisted.refresh_token.as_deref(),
            Some("shutdown-rotated-refresh")
        );
        fixture.server.verify().await;
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_signal_handler_error_still_waits_for_owned_rotation() {
        let fixture = normal_shutdown_fixture(false, Duration::from_millis(250), None).await;
        let api = ApiClient::new(
            reqwest::Client::new(),
            &fixture.api_base_url,
            fixture.credentials.clone(),
        );
        let (signal_tx, signal_rx) = tokio::sync::oneshot::channel();
        let mut signal: CtrlCSignal = Box::pin(async move { signal_rx.await.unwrap() });
        let credentials = fixture.credentials.clone();
        let run =
            tokio::spawn(
                async move { startup_me_or_interrupt(&api, &credentials, &mut signal).await },
            );
        wait_for_request(&fixture.server, |request| {
            request.method == "POST" && request.url.path() == "/api/auth/oauth2/token"
        })
        .await;
        signal_tx
            .send(Err(std::io::Error::other("injected signal error")))
            .unwrap();
        let started = tokio::time::Instant::now();
        let error = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(error.contains("signal handler failed"), "{error}");
        assert!(started.elapsed() >= Duration::from_millis(200));
        let persisted =
            token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            persisted.refresh_token.as_deref(),
            Some("shutdown-rotated-refresh")
        );
        fixture.server.verify().await;
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    async fn run_startup_sigint_child(
        fixture: &NormalShutdown,
        request_predicate: impl Fn(&wiremock::Request) -> bool,
    ) {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ui::tests::startup_sigint_worker", "--nocapture"])
            .env("ZOO2_TUI_STARTUP_SIGINT_WORKER_PATH", &fixture.path)
            .env("ZOO2_TUI_STARTUP_SIGINT_WORKER_API", &fixture.api_base_url)
            .env(
                "ZOO2_TUI_STARTUP_SIGINT_WORKER_ISSUER",
                &fixture.oidc.issuer,
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_for_request(&fixture.server, request_predicate).await;
        let signal_result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
        assert_eq!(
            signal_result, 0,
            "SIGINT should be delivered to startup worker"
        );
        let status = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("startup worker should finish after handling SIGINT");
        let output = child.wait_with_output().unwrap();
        assert!(
            status.success(),
            "worker failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[tokio::test]
    async fn startup_sigint_worker() {
        let (Some(path), Some(api), Some(issuer)) = (
            std::env::var_os("ZOO2_TUI_STARTUP_SIGINT_WORKER_PATH"),
            std::env::var("ZOO2_TUI_STARTUP_SIGINT_WORKER_API").ok(),
            std::env::var("ZOO2_TUI_STARTUP_SIGINT_WORKER_ISSUER").ok(),
        ) else {
            return;
        };
        let path = PathBuf::from(path);
        let oidc = oidc(issuer);
        let tokens = token::load_async(&path, &oidc, Some(&api))
            .await
            .unwrap()
            .unwrap();
        let manager = Arc::new(TokenManager::new(
            reqwest::Client::new(),
            oidc,
            path,
            tokens,
        ));
        let result = run(
            reqwest::Client::new(),
            api,
            Credentials::Bearer(manager),
            false,
        )
        .await;
        let message = result.unwrap_err().to_string();
        assert!(message.contains("Ctrl+C"), "{message}");
        assert!(message.contains("保存は完了しています"), "{message}");
        assert!(!message.contains("shutdown-rotated-refresh"));
    }

    #[tokio::test]
    async fn draw_and_event_error_warnings_are_reported_only_after_terminal_restore() {
        use std::future::Future;

        for failure in ["injected draw failure", "injected event failure"] {
            let fixture = normal_shutdown_fixture(true, Duration::from_millis(100), None).await;
            let mut access = Box::pin(fixture.manager.access_token());
            let first_poll = std::future::poll_fn(|context| {
                std::task::Poll::Ready(access.as_mut().poll(context))
            })
            .await;
            assert!(first_poll.is_pending());
            drop(access);
            wait_for_request(&fixture.server, |request| {
                request.method == "POST" && request.url.path() == "/api/auth/oauth2/token"
            })
            .await;

            let (result, warning) =
                finish_application_shutdown(Err(anyhow!(failure)), &fixture.credentials).await;
            assert!(result.unwrap_err().to_string().contains(failure));
            let order = std::cell::RefCell::new(Vec::new());
            restore_terminal_then_report(
                || order.borrow_mut().push("restore"),
                warning,
                |message| {
                    assert!(message.contains("終了後に失われ"));
                    assert!(!message.contains("shutdown-rotated-refresh"));
                    order.borrow_mut().push("warning");
                },
            );
            assert_eq!(*order.borrow(), ["restore", "warning"]);
            fixture.server.verify().await;
            drop(fixture.readonly);
            token::delete(&fixture.path).unwrap();
            std::fs::remove_dir_all(fixture.dir).unwrap();
        }
    }

    #[tokio::test]
    async fn startup_quit_before_retry_reports_pending_token_loss() {
        let fixture = pending_startup().await;
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (draw, _) = test_drawer();
        let run = tokio::spawn(drive_startup_recovery(
            fixture.api.clone(),
            fixture.recovery,
            event_stream(event_rx),
            draw,
        ));
        event_tx
            .send(key(KeyCode::Char('q'), KeyModifiers::NONE))
            .unwrap();
        let (result, _) = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .unwrap()
            .unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("終了後は失われ"), "{error}");
        fixture.server.verify().await;
        drop(fixture.readonly);
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn normal_q_and_ctrl_c_wait_for_registered_rotation_and_reject_new_tasks() {
        use std::future::Future;

        for event in [
            key(KeyCode::Char('q'), KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let fixture = normal_shutdown_fixture(false, Duration::from_millis(350), None).await;
            let api = ApiClient::new(
                reqwest::Client::new(),
                &fixture.api_base_url,
                fixture.credentials.clone(),
            );
            let (shape_tx, _) = mpsc::unbounded_channel();
            let (app_tx, _) = mpsc::unbounded_channel();
            let mut app = App::new(
                api,
                reqwest::Client::new(),
                fixture.api_base_url.clone(),
                fixture.credentials.clone(),
                Me {
                    id: "u1".into(),
                    user_name: "alice".into(),
                    display_name: "Alice".into(),
                },
                shape_tx,
                app_tx,
            );

            // Poll access_token exactly far enough to register and spawn its
            // owned worker, without yielding for that worker to lock State.
            let mut access = Box::pin(fixture.manager.access_token());
            let first_poll = std::future::poll_fn(|context| {
                std::task::Poll::Ready(access.as_mut().poll(context))
            })
            .await;
            assert!(first_poll.is_pending());
            drop(access);

            app.on_terminal_event(event);
            assert!(app.should_quit);
            let started = tokio::time::Instant::now();
            finish_application_shutdown(Ok(()), &fixture.credentials)
                .await
                .0
                .unwrap();
            assert!(started.elapsed() >= Duration::from_millis(250));

            let persisted =
                token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                persisted.refresh_token.as_deref(),
                Some("shutdown-rotated-refresh")
            );
            let new_request = fixture.manager.access_token().await.unwrap_err();
            assert!(new_request.to_string().contains("shutting down"));
            fixture.server.verify().await;
            drop(fixture.readonly);
            token::delete(&fixture.path).unwrap();
            std::fs::remove_dir_all(fixture.dir).unwrap();
        }
    }

    #[tokio::test]
    async fn normal_quit_reports_the_cause_and_risk_when_rotation_cannot_be_saved() {
        use std::future::Future;

        let fixture = normal_shutdown_fixture(true, Duration::from_millis(250), None).await;
        let api = ApiClient::new(
            reqwest::Client::new(),
            &fixture.api_base_url,
            fixture.credentials.clone(),
        );
        let (shape_tx, _) = mpsc::unbounded_channel();
        let (app_tx, _) = mpsc::unbounded_channel();
        let mut app = App::new(
            api,
            reqwest::Client::new(),
            fixture.api_base_url.clone(),
            fixture.credentials.clone(),
            Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
            shape_tx,
            app_tx,
        );
        let mut access = Box::pin(fixture.manager.access_token());
        let first_poll =
            std::future::poll_fn(|context| std::task::Poll::Ready(access.as_mut().poll(context)))
                .await;
        assert!(first_poll.is_pending());
        drop(access);
        app.on_terminal_event(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(app.should_quit);

        let error = finish_application_shutdown(Ok(()), &fixture.credentials)
            .await
            .0
            .unwrap_err()
            .to_string();
        assert!(error.contains("token 保存先へのアクセスが拒否"), "{error}");
        assert!(error.contains("終了後に失われ"), "{error}");
        assert!(!error.contains("shutdown-rotated-refresh"));
        fixture.server.verify().await;
        drop(fixture.readonly);
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn normal_quit_reports_uncertain_rotation_after_refresh_timeout() {
        use std::future::Future;

        let fixture = normal_shutdown_fixture(
            false,
            Duration::from_secs(2),
            Some(Duration::from_millis(50)),
        )
        .await;
        let api = ApiClient::new(
            reqwest::Client::new(),
            &fixture.api_base_url,
            fixture.credentials.clone(),
        );
        let (shape_tx, _) = mpsc::unbounded_channel();
        let (app_tx, _) = mpsc::unbounded_channel();
        let mut app = App::new(
            api,
            reqwest::Client::new(),
            fixture.api_base_url.clone(),
            fixture.credentials.clone(),
            Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
            shape_tx,
            app_tx,
        );
        let mut access = Box::pin(fixture.manager.access_token());
        let first_poll =
            std::future::poll_fn(|context| std::task::Poll::Ready(access.as_mut().poll(context)))
                .await;
        assert!(first_poll.is_pending());
        drop(access);
        app.on_terminal_event(key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            finish_application_shutdown(Ok(()), &fixture.credentials),
        )
        .await
        .unwrap()
        .0
        .unwrap_err()
        .to_string();
        assert!(error.contains("IdP との通信に失敗"), "{error}");
        assert!(error.contains("refresh 結果を確認できません"), "{error}");
        assert!(!error.contains("shutdown-rotated-refresh"));
        fixture.server.verify().await;
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_retry_exits_recovery_when_another_session_replaces_pending_tokens() {
        let fixture = pending_startup().await;
        fixture.readonly.allow_writes();

        let mut replacement = stored_tokens(&fixture.oidc, &fixture.api_base_url);
        replacement.session_id = "different-session".into();
        replacement.access_token = "different-access".into();
        replacement.refresh_token = Some("different-refresh".into());
        replacement.expires_at = token::now() + 300;
        token::save(&fixture.path, &replacement).unwrap();

        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (draw, _) = test_drawer();
        let run = tokio::spawn(drive_startup_recovery(
            fixture.api.clone(),
            fixture.recovery,
            event_stream(event_rx),
            draw,
        ));
        event_tx
            .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();
        let (result, _) = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .unwrap()
            .unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("別 session"), "{error}");
        let on_disk = token::load_async(&fixture.path, &fixture.oidc, Some(&fixture.api_base_url))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(on_disk.session_id, "different-session");
        assert_eq!(on_disk.refresh_token.as_deref(), Some("different-refresh"));
        fixture.server.verify().await;
        drop(fixture.readonly);
        token::delete(&fixture.path).unwrap();
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[tokio::test]
    async fn startup_retry_exits_recovery_after_explicit_logout() {
        let fixture = pending_startup().await;
        fixture.readonly.allow_writes();
        assert!(token::delete(&fixture.path).unwrap());
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        let (draw, _) = test_drawer();
        let run = tokio::spawn(drive_startup_recovery(
            fixture.api.clone(),
            fixture.recovery,
            event_stream(event_rx),
            draw,
        ));
        event_tx
            .send(key(KeyCode::Char('r'), KeyModifiers::NONE))
            .unwrap();
        let (result, _) = tokio::time::timeout(Duration::from_secs(2), run)
            .await
            .unwrap()
            .unwrap();
        let error = result.unwrap_err().to_string();
        assert!(error.contains("logout または別 session"), "{error}");
        fixture.server.verify().await;
        drop(fixture.readonly);
        std::fs::remove_dir_all(fixture.dir).unwrap();
    }

    #[test]
    fn terminal_initialization_failure_reports_pending_token_loss_without_secrets() {
        let recovery = StartupRecovery {
            message: "refresh-secret-sentinel".into(),
            reason: "token 保存に失敗しました。",
        };
        let mut stderr_warnings = Vec::new();
        let result: Result<Terminal<TestBackend>> = initialize_startup_terminal(
            &recovery,
            || {
                Err(std::io::Error::other(
                    "injected terminal initialization failure",
                ))
            },
            |warning| stderr_warnings.push(warning.to_owned()),
        );

        let error = result.unwrap_err();
        assert_eq!(stderr_warnings.len(), 1);
        assert!(stderr_warnings[0].contains("終了後は失われ"));
        assert!(stderr_warnings[0].contains("別 process の logout"));
        assert!(!stderr_warnings[0].contains("refresh-secret-sentinel"));
        assert!(error.to_string().contains("端末を初期化できません"));
    }
}
