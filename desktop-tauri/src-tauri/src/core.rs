//! ログイン状態、Shape の購読、通知の判定。
//!
//! 状態は 1 つの `Mutex<Inner>` にまとめる。Shape のイベントは 1 本のループで、ロックを
//! 取ったまま「通知の判定 → 適用 → WebView へ emit」の順に処理する。`zoo_resync` も
//! 同じロックの下で送るので、WebView に届く順番は Shape に届いた順と同じになる。
//!
//! ログインやログアウトのたびに `generation` を進める。古い世代のタスクが遅れて
//! 終わっても、今の状態を書き換えないようにするため。

use std::{sync::Arc, time::Duration};

use anyhow::{Result, anyhow};
use jiff::{civil::DateTime, tz::TimeZone};
use tauri::{AppHandle, Emitter};
use tokio::{
    sync::{Mutex, mpsc},
    task::JoinHandle,
};
use zoo2_client_core::{
    api::ApiClient,
    auth::{
        self,
        device::{self, Poller},
        token::{self, StoredTokens, TokenManager},
    },
    config::{AuthConfig, Config, OidcConfig, config_dir_for},
    electric::{self, ShapeEvent},
    http::{Credentials, HttpError},
};
use zoo2_client_types::{
    api::Me,
    ipc::{
        DeviceLogin, ErrorCode, FocusState, IpcError, OpenTarget, PROTOCOL_VERSION, SessionInfo,
        SessionState, ShapeBatch, ShapeFailed, UiMode, event,
    },
    notify::{self, NotifyContext, Toast},
    shape::{Batch, Replica, ShapeId},
    store::Store,
};

use crate::{APP_NAME, toast};

/// DND (`dnd_until`) を取り直す間隔。
const DND_REFRESH: Duration = Duration::from_secs(5 * 60);

pub struct Core {
    app: AppHandle,
    ui: UiMode,
    config: Result<Config, String>,
    http: reqwest::Client,
    inner: Mutex<Inner>,
}

struct Inner {
    generation: u64,
    state: SessionState,
    session: Option<Session>,
    login_task: Option<JoinHandle<()>>,
    /// WebView から `zoo_set_focus` で受け取った、表示中のチャンネル。
    focus: FocusState,
    /// ウィンドウが前面にあるか (Tauri のウィンドウのイベントで知る)。
    window_focused: bool,
    dnd_until: Option<DateTime>,
    /// ログインしてから、DND を 1 回は取りに行ったか。それまでは通知を出さない
    /// (DND の間に起動したとき、取れる前に届いた通知を出さないため)。
    dnd_checked: bool,
}

/// ログインしている間だけある状態。捨てるとすべてのタスクが止まる。
struct Session {
    tokens: Option<Arc<TokenManager>>,
    api: ApiClient,
    credentials: Credentials,
    me: Me,
    events: mpsc::UnboundedSender<ShapeEvent>,
    tasks: Vec<JoinHandle<()>>,
    /// 開いているチャンネルと、そのメッセージの Shape を購読するタスク。
    messages: Option<(String, JoinHandle<()>)>,
    replica: Replica,
    store: Store,
}

impl Drop for Session {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        if let Some((_, task)) = &self.messages {
            task.abort();
        }
    }
}

impl Core {
    pub fn new(app: AppHandle, ui: UiMode, config: Result<Config>) -> Self {
        Self {
            app,
            ui,
            config: config.map_err(|err| format!("{err:#}")),
            http: reqwest::Client::new(),
            inner: Mutex::new(Inner {
                generation: 0,
                state: SessionState::Starting,
                session: None,
                login_task: None,
                focus: FocusState::default(),
                window_focused: true,
                dnd_until: None,
                dnd_checked: false,
            }),
        }
    }

    pub fn ui(&self) -> UiMode {
        self.ui
    }

    /// backend の起点。設定が読めなかったときは None。
    pub fn api_base_url(&self) -> Option<&str> {
        self.config.as_ref().ok().map(|c| c.api_base_url.as_str())
    }

    fn info(&self, state: &SessionState) -> SessionInfo {
        SessionInfo {
            protocol: PROTOCOL_VERSION,
            ui: self.ui,
            api_base_url: self.api_base_url().unwrap_or_default().to_string(),
            state: state.clone(),
        }
    }

    pub async fn session(&self) -> SessionInfo {
        let inner = self.inner.lock().await;
        self.info(&inner.state)
    }

    fn set_state(&self, inner: &mut Inner, state: SessionState) {
        inner.state = state;
        let _ = self.app.emit(event::SESSION, self.info(&inner.state));
    }

    fn token_path(&self) -> Result<std::path::PathBuf> {
        config_dir_for(APP_NAME)
            .map(|dir| token::token_path(&dir))
            .ok_or_else(|| anyhow!("設定ディレクトリが見つかりません"))
    }

    fn oidc(&self) -> Result<(&Config, &OidcConfig), IpcError> {
        let config = self.config.as_ref().map_err(|message| {
            IpcError::new(ErrorCode::Internal, format!("設定を読めません: {message}"))
        })?;
        match &config.auth {
            AuthConfig::Oidc(oidc) => Ok((config, oidc)),
            AuthConfig::Dev { .. } => Err(IpcError::new(
                ErrorCode::WrongMode,
                "dev モード (ZOO_DEV_USER) ではログインしません",
            )),
        }
    }

    /// 起動時に、保存済みのトークン (または dev モード) でログインを試みる。
    pub fn start(self: &Arc<Self>) {
        let core = self.clone();
        tauri::async_runtime::spawn(async move {
            let config = match &core.config {
                Ok(config) => config.clone(),
                Err(message) => {
                    let mut inner = core.inner.lock().await;
                    let state = SessionState::Failed {
                        message: format!("設定を読めません: {message}"),
                    };
                    core.set_state(&mut inner, state);
                    return;
                }
            };
            let credentials = match &config.auth {
                AuthConfig::Dev { user_id } => Some((Credentials::Dev(user_id.clone()), None)),
                AuthConfig::Oidc(oidc) => {
                    let loaded = core.token_path().and_then(|path| {
                        Ok(token::load(&path, oidc, Some(&config.api_base_url))?
                            .map(|tokens| (path, tokens)))
                    });
                    match loaded {
                        Ok(Some((path, tokens))) => {
                            let manager = Arc::new(TokenManager::new(
                                core.http.clone(),
                                oidc.clone(),
                                path,
                                tokens,
                            ));
                            Some((Credentials::Bearer(manager.clone()), Some(manager)))
                        }
                        Ok(None) => None,
                        Err(err) => {
                            let mut inner = core.inner.lock().await;
                            let state = SessionState::Failed {
                                message: format!("{err:#}"),
                            };
                            core.set_state(&mut inner, state);
                            return;
                        }
                    }
                }
            };
            match credentials {
                Some((credentials, tokens)) => core.finish_login(0, credentials, tokens).await,
                None => {
                    let mut inner = core.inner.lock().await;
                    core.set_state(&mut inner, SessionState::LoggedOut);
                }
            }
        });
    }

    /// トークンで `/api/users/me` を取り、取れたら購読を始めて `Ready` にする。
    /// 通信のエラーは間隔を延ばしながら取り直す。401 ならログアウトの状態にする。
    async fn finish_login(
        self: &Arc<Self>,
        generation: u64,
        credentials: Credentials,
        tokens: Option<Arc<TokenManager>>,
    ) {
        let Some(base) = self.api_base_url().map(str::to_string) else {
            return;
        };
        let api = ApiClient::new(self.http.clone(), &base, credentials.clone());
        let mut backoff = Duration::from_secs(1);
        let me = loop {
            match api.me().await {
                Ok(me) => break me,
                Err(err) if is_unauthorized(&err) => {
                    let mut inner = self.inner.lock().await;
                    if inner.generation == generation {
                        self.set_state(&mut inner, SessionState::LoggedOut);
                    }
                    return;
                }
                Err(_) => {
                    if self.inner.lock().await.generation != generation {
                        return;
                    }
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(30));
                }
            }
        };

        let mut inner = self.inner.lock().await;
        if inner.generation != generation {
            return;
        }
        let (events, receiver) = mpsc::unbounded_channel();
        let mut session = Session {
            tokens,
            api: api.clone(),
            credentials: credentials.clone(),
            me: me.clone(),
            events: events.clone(),
            tasks: Vec::new(),
            messages: None,
            replica: Replica::default(),
            store: Store::default(),
        };
        // (b) React では Shape を WebView 側が購読するので、Rust 側は購読しない。
        if self.ui == UiMode::Leptos {
            for id in [
                ShapeId::Channels,
                ShapeId::Users,
                ShapeId::ChannelUsers,
                ShapeId::Notifications,
            ] {
                session.tasks.push(electric::spawn(
                    self.http.clone(),
                    &base,
                    credentials.clone(),
                    id,
                    events.clone(),
                ));
            }
        }
        session
            .tasks
            .push(self.spawn_event_loop(generation, receiver));
        session.tasks.push(self.spawn_dnd_refresh(generation, api));
        inner.session = Some(session);
        inner.dnd_until = None;
        inner.dnd_checked = false;
        self.set_state(&mut inner, SessionState::Ready { me });
    }

    fn spawn_event_loop(
        self: &Arc<Self>,
        generation: u64,
        mut receiver: mpsc::UnboundedReceiver<ShapeEvent>,
    ) -> JoinHandle<()> {
        let core = self.clone();
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                match event {
                    ShapeEvent::Batch(id, batch) => core.handle_batch(generation, id, batch).await,
                    // 401 は、トークンが切れて更新もできない。ログイン画面に戻す。
                    ShapeEvent::Failed(_, message) if is_unauthorized_message(&message) => {
                        core.expire_session(generation).await;
                        return;
                    }
                    ShapeEvent::Failed(id, message) => {
                        let _ = core
                            .app
                            .emit(event::SHAPE_FAILED, ShapeFailed { shape: id, message });
                    }
                }
            }
        })
    }

    fn spawn_dnd_refresh(self: &Arc<Self>, generation: u64, api: ApiClient) -> JoinHandle<()> {
        let core = self.clone();
        tokio::spawn(async move {
            loop {
                let profile = api.my_profile().await;
                let mut inner = core.inner.lock().await;
                if inner.generation != generation {
                    return;
                }
                // 取れなかったときは前の値のままにする。初回に取れなかったときは、
                // 通知を止め続けないよう、DND なしとして通知を出し始める。
                if let Ok(profile) = profile {
                    inner.dnd_until = profile
                        .dnd_until
                        .as_deref()
                        .and_then(notify::parse_dnd_until);
                }
                inner.dnd_checked = true;
                drop(inner);
                tokio::time::sleep(DND_REFRESH).await;
            }
        })
    }

    /// Shape のバッチを 1 つ処理する。(b) の `zoo_notifications_observed` もここを通る。
    pub async fn handle_batch(&self, generation: u64, id: ShapeId, batch: Batch) {
        let toasts = {
            let mut inner = self.inner.lock().await;
            if inner.generation != generation {
                return;
            }
            let focused_channel = inner
                .window_focused
                .then(|| inner.focus.channel_id.clone())
                .flatten();
            let dnd_until = inner.dnd_until;
            let dnd_checked = inner.dnd_checked;
            let Some(session) = inner.session.as_mut() else {
                return;
            };
            // チャンネルを切り替えた後に届いた、前のチャンネルの行は捨てる。
            if let ShapeId::Messages(channel) = &id
                && session.messages.as_ref().map(|(open, _)| open) != Some(channel)
            {
                return;
            }
            // 判定は適用より前に行う (適用した後では、新しく届いた行を区別できない)。
            let toasts = if id == ShapeId::Notifications {
                let ctx = NotifyContext {
                    me: session.me.id.clone(),
                    enabled: dnd_checked,
                    now: Some(now_utc()),
                    dnd_until,
                    focused_channel,
                };
                notify::notifications_to_show(&session.store, &batch, &ctx)
            } else {
                Vec::new()
            };
            session.replica.apply(&id, &batch);
            session.store.apply(&id, batch.clone());
            if self.ui == UiMode::Leptos {
                let _ = self.app.emit(event::SHAPE, ShapeBatch { shape: id, batch });
            }
            toasts
        };
        for toast in toasts {
            self.show_toast(toast);
        }
    }

    fn show_toast(&self, toast: Toast) {
        let target = OpenTarget {
            channel_id: toast.channel_id.clone(),
            message_id: Some(toast.message_id.clone()),
        };
        toast::show(&self.app, &toast.title, &toast.body, target);
    }

    pub async fn generation(&self) -> u64 {
        self.inner.lock().await.generation
    }

    pub async fn start_login(self: &Arc<Self>) -> Result<DeviceLogin, IpcError> {
        let (config, oidc) = self.oidc()?;
        let generation = {
            let mut inner = self.inner.lock().await;
            if matches!(inner.state, SessionState::Ready { .. }) {
                return Err(IpcError::new(ErrorCode::WrongMode, "ログイン済みです"));
            }
            if let Some(task) = inner.login_task.take() {
                task.abort();
            }
            inner.generation += 1;
            inner.generation
        };
        let discovery = auth::discover(&self.http, &oidc.issuer)
            .await
            .map_err(to_ipc)?;
        let authorization = device::start(&self.http, &discovery, oidc)
            .await
            .map_err(to_ipc)?;
        let login = DeviceLogin {
            user_code: authorization.user_code.clone(),
            verification_uri: authorization.verification_uri.clone(),
            verification_uri_complete: authorization.verification_uri_complete.clone(),
            expires_at: token::now() + authorization.expires_in as i64,
        };

        let mut inner = self.inner.lock().await;
        if inner.generation != generation {
            return Err(IpcError::new(ErrorCode::Internal, "ログインを中止しました"));
        }
        let core = self.clone();
        let (config, oidc) = (config.clone(), oidc.clone());
        inner.login_task = Some(tokio::spawn(async move {
            let result = async {
                let response = Poller::default()
                    .poll(&core.http, &discovery, &oidc, &authorization)
                    .await?;
                let tokens = StoredTokens::new(&oidc, &config.api_base_url, response, token::now());
                let path = core.token_path()?;
                token::save(&path, &tokens)?;
                anyhow::Ok(Arc::new(TokenManager::new(
                    core.http.clone(),
                    oidc.clone(),
                    path,
                    tokens,
                )))
            }
            .await;
            match result {
                Ok(manager) => {
                    core.finish_login(
                        generation,
                        Credentials::Bearer(manager.clone()),
                        Some(manager),
                    )
                    .await
                }
                Err(_) => {
                    let mut inner = core.inner.lock().await;
                    if inner.generation == generation {
                        core.set_state(&mut inner, SessionState::LoggedOut);
                    }
                }
            }
        }));
        self.set_state(
            &mut inner,
            SessionState::LoggingIn {
                login: login.clone(),
            },
        );
        Ok(login)
    }

    pub async fn cancel_login(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(task) = inner.login_task.take() {
            task.abort();
        }
        if matches!(inner.state, SessionState::LoggingIn { .. }) {
            inner.generation += 1;
            self.set_state(&mut inner, SessionState::LoggedOut);
        }
    }

    pub async fn logout(&self) -> Result<(), IpcError> {
        let mut inner = self.inner.lock().await;
        let result = self.end_session(&mut inner);
        self.set_state(&mut inner, SessionState::LoggedOut);
        result.map_err(to_ipc)
    }

    /// 購読が 401 で止まったとき。その世代のセッションのままなら、ログアウトする。
    async fn expire_session(&self, generation: u64) {
        let mut inner = self.inner.lock().await;
        if inner.generation != generation {
            return;
        }
        let _ = self.end_session(&mut inner);
        self.set_state(&mut inner, SessionState::LoggedOut);
    }

    /// セッションを捨て、保存したトークンを消す。セッションの `TokenManager` を
    /// `sign_out` するので、コマンドが持っている `ApiClient` で進行中だった更新が
    /// 後から終わっても、`token.json` は作り直されない。
    fn end_session(&self, inner: &mut Inner) -> Result<()> {
        inner.generation += 1;
        if let Some(task) = inner.login_task.take() {
            task.abort();
        }
        let session = inner.session.take();
        // 画面はチャンネル未選択に戻るので、前のチャンネルの通知を抑え続けないよう消す。
        inner.focus = FocusState::default();
        inner.dnd_until = None;
        inner.dnd_checked = false;
        if !matches!(
            self.config.as_ref().map(|c| &c.auth),
            Ok(AuthConfig::Oidc(_))
        ) {
            return Ok(());
        }
        match session.and_then(|session| session.tokens.clone()) {
            Some(tokens) => tokens.sign_out()?,
            None => token::delete(&self.token_path()?)?,
        };
        Ok(())
    }

    pub async fn set_focus(&self, focus: FocusState) {
        let mut inner = self.inner.lock().await;
        inner.window_focused = focus.focused;
        inner.focus = focus;
    }

    pub async fn set_window_focused(&self, focused: bool) {
        self.inner.lock().await.window_focused = focused;
    }

    pub async fn open_channel(&self, channel_id: String) -> Result<(), IpcError> {
        let base = self.api_base_url().unwrap_or_default().to_string();
        let mut inner = self.inner.lock().await;
        let session = inner.session.as_mut().ok_or_else(not_logged_in)?;
        if session.messages.as_ref().map(|(open, _)| open) == Some(&channel_id) {
            return Ok(());
        }
        if let Some((old, task)) = session.messages.take() {
            task.abort();
            let id = ShapeId::Messages(old);
            session.replica.forget(&id);
            session.store.forget(&id);
        }
        let task = electric::spawn(
            self.http.clone(),
            &base,
            session.credentials.clone(),
            ShapeId::Messages(channel_id.clone()),
            session.events.clone(),
        );
        session.messages = Some((channel_id, task));
        Ok(())
    }

    /// 持っている行を `reset` 付きで送り直す。WebView を読み込み直したときに呼ばれる。
    pub async fn resync(&self) -> Result<(), IpcError> {
        let inner = self.inner.lock().await;
        let session = inner.session.as_ref().ok_or_else(not_logged_in)?;
        for (shape, batch) in session.replica.snapshot() {
            let _ = self.app.emit(event::SHAPE, ShapeBatch { shape, batch });
        }
        Ok(())
    }

    pub async fn api(&self) -> Result<ApiClient, IpcError> {
        let inner = self.inner.lock().await;
        Ok(inner
            .session
            .as_ref()
            .ok_or_else(not_logged_in)?
            .api
            .clone())
    }

    pub async fn access_token(&self) -> Result<(String, i64), IpcError> {
        let tokens = {
            let inner = self.inner.lock().await;
            let session = inner.session.as_ref().ok_or_else(not_logged_in)?;
            session.tokens.clone().ok_or_else(|| {
                IpcError::new(ErrorCode::WrongMode, "dev モードではトークンがありません")
            })?
        };
        tokens.access_token_with_expiry().await.map_err(to_ipc)
    }
}

fn now_utc() -> DateTime {
    jiff::Timestamp::now().to_zoned(TimeZone::UTC).datetime()
}

fn not_logged_in() -> IpcError {
    IpcError::new(ErrorCode::NotLoggedIn, "ログインしていません")
}

/// `electric::spawn` は 401 を `HttpError::Unauthorized` の文言で `Failed` に入れる。
fn is_unauthorized_message(message: &str) -> bool {
    message == HttpError::Unauthorized.to_string()
}

fn is_unauthorized(err: &anyhow::Error) -> bool {
    matches!(
        err.downcast_ref::<HttpError>(),
        Some(HttpError::Unauthorized)
    )
}

/// anyhow のエラーを、WebView が振る舞いを変えられる形にする。
pub fn to_ipc(err: anyhow::Error) -> IpcError {
    let code = match err.downcast_ref::<HttpError>() {
        Some(HttpError::Unauthorized) => ErrorCode::NotLoggedIn,
        Some(HttpError::Status { status, .. }) if status.is_client_error() => ErrorCode::Rejected,
        Some(HttpError::Status { .. }) => ErrorCode::Network,
        None if err.downcast_ref::<reqwest::Error>().is_some() => ErrorCode::Network,
        None => ErrorCode::Internal,
    };
    IpcError::new(code, format!("{err:#}"))
}
