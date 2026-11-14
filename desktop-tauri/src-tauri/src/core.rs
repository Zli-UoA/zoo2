//! ログイン状態、Shape の購読、通知の判定。
//!
//! 状態は 1 つの `Mutex<Inner>` にまとめる。Shape のイベントは 1 本のループで、ロックを
//! 取ったまま「通知の判定 → 適用 → WebView へ emit」の順に処理する。`zoo_resync` も
//! 同じロックの下で送るので、WebView に届く順番は Shape に届いた順と同じになる。
//!
//! ログインやログアウトのたびに `generation` を進める。古い世代のタスクが遅れて
//! 終わっても、今の状態を書き換えないようにするため。

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow};
use jiff::{civil::DateTime, tz::TimeZone};
use tauri::{AppHandle, Emitter, Manager};
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
        DeviceLogin, ErrorCode, FocusState, IpcError, NotificationSettings, OpenTarget,
        PROTOCOL_VERSION, SessionInfo, SessionState, ShapeBatch, ShapeFailed, UiMode, event,
    },
    notify::{self, NotifyContext, Toast},
    shape::{Batch, Replica, ShapeId},
    store::Store,
};

use crate::{
    APP_NAME,
    toast::{self, CallActions, CallToast},
    window::MAIN,
};

/// DND (`dnd_until`) を取り直す間隔。
const DND_REFRESH: Duration = Duration::from_secs(5 * 60);

/// 「応答」の印の有効な長さ。
const CALL_ACCEPT_TTL: Duration = Duration::from_secs(60);

/// 招待の期限から、この時間だけ待っても Shape から消えなければ、トーストを閉じる（保険）。
const CALL_TOAST_GRACE: Duration = Duration::from_secs(3);

pub struct Core {
    app: AppHandle,
    ui: UiMode,
    config: Result<Config, String>,
    http: reqwest::Client,
    inner: Mutex<Inner>,
    /// OS の通知を出すか (`notifications.json` に保存する)。
    notifications: std::sync::Mutex<NotificationSettings>,
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
    /// 出している着信のトースト（招待の ID → トースト）。
    call_toasts: HashMap<String, CallToast>,
    /// 着信のトーストを一度出した（または出さないと決めた）招待。前面と背面を行き来しても出し直さない。
    call_toast_seen: HashSet<String>,
    /// トーストの「応答」を押した招待。Rust 側だけが残す印で、WebView が一度だけ取る。
    pending_accept: Option<(String, Instant)>,
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

struct EndingSession {
    tokens: Option<Arc<TokenManager>>,
    fallback: Option<(std::path::PathBuf, Option<StoredTokens>)>,
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
            notifications: std::sync::Mutex::new(load_notification_settings()),
            inner: Mutex::new(Inner {
                generation: 0,
                state: SessionState::Starting,
                session: None,
                login_task: None,
                focus: FocusState::default(),
                window_focused: true,
                dnd_until: None,
                dnd_checked: false,
                call_toasts: HashMap::new(),
                call_toast_seen: HashSet::new(),
                pending_accept: None,
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
        crate::window::follow_session(&self.app, self, &inner.state);
    }

    pub fn notification_settings(&self) -> NotificationSettings {
        *self
            .notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// 変えて保存する。保存できなかったら、変えない。
    pub fn set_notification_settings(&self, settings: NotificationSettings) -> Result<()> {
        // 書き込み、置き換え、メモリの更新を同じロックの中で行う。同時に呼ばれても、
        // ファイルとメモリの値が食い違わないように (一時ファイルの名前も 1 つで足りる)。
        let mut current = self
            .notifications
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = notification_settings_path()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // 同じディレクトリの一時ファイルに書いてから置き換える。途中で失敗しても、
        // 保存済みの設定 (通知オフなど) を壊さない。
        let tmp = path.with_extension("json.tmp");
        let written = std::fs::write(&tmp, serde_json::to_vec(&settings)?)
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(err) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(err.into());
        }
        *current = settings;
        Ok(())
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
        // 着信のトーストのために、通話の招待はどちらの画面でも Rust 側が購読する（WebView は、
        // 表示のための購読を別に持つ。招待の行は通常 0〜数行）。
        // (b) React では、それ以外の Shape を WebView 側が購読するので、Rust 側は購読しない。
        let mut shapes = vec![ShapeId::CallInvites];
        if self.ui == UiMode::Leptos {
            shapes.extend([
                ShapeId::Channels,
                ShapeId::Users,
                ShapeId::ChannelUsers,
                ShapeId::Notifications,
            ]);
        }
        for id in shapes {
            session.tasks.push(electric::spawn(
                self.http.clone(),
                &base,
                credentials.clone(),
                id,
                events.clone(),
            ));
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
                        let core = core.clone();
                        tokio::spawn(async move { core.expire_session(generation).await });
                        return;
                    }
                    // 古い backend など、通話の招待の購読だけが失敗しても、画面にはエラーを出さない。
                    ShapeEvent::Failed(ShapeId::CallInvites, _) => {}
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
        if id == ShapeId::CallInvites {
            let mut inner = self.inner.lock().await;
            if inner.generation != generation {
                return;
            }
            let Some(session) = inner.session.as_mut() else {
                return;
            };
            // 同期が済んだあとの差分は、届いたばかりの招待。初回と reset の行は、古いかもしれない。
            let fresh = session.store.is_synced(&id) && !batch.reset;
            session.store.apply(&id, batch);
            self.reconcile_call_toasts(&mut inner, fresh);
            return;
        }
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

    /// 前面にあるか（表示されていて、フォーカスがある）。前面なら、React のダイアログに任せる。
    fn window_in_front(&self, inner: &Inner) -> bool {
        match self.app.get_webview_window(MAIN) {
            Some(window) => {
                window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false)
            }
            None => inner.window_focused,
        }
    }

    /// 呼ばれている招待の集合を正として、出ているトーストと突き合わせる。
    /// 集合から消えた招待（応答・拒否・取り消し・不在）のトーストは閉じ、新しい招待は、条件を満たせば出す。
    /// 条件: 通知がオン、DND でない（Web Push と同じく、DND の間は出さず、あとから出すこともしない）、
    /// ウィンドウが前面にない。
    fn reconcile_call_toasts(&self, inner: &mut Inner, fresh: bool) {
        let Some(session) = inner.session.as_ref() else {
            return;
        };
        let me = session.me.id.clone();
        let now = now_utc();
        let current: HashSet<String> = session
            .store
            .call_invites()
            .into_iter()
            .filter(|invite| invite.invitee_id == me && invite.status == "pending")
            .map(|invite| invite.id.clone())
            .collect();
        let candidates: Vec<_> = notify::pending_call_invites(&session.store, &me, now, fresh)
            .into_iter()
            .map(|invite| notify::call_invite_toast(&session.store, invite))
            .collect();

        inner.call_toasts.retain(|id, toast| {
            let keep = current.contains(id);
            if !keep {
                toast.close();
            }
            keep
        });
        inner.call_toast_seen.retain(|id| current.contains(id));

        if !self.notification_settings().enabled || !inner.dnd_checked {
            // 設定がオフのときは、出さないと決める。DND を取る前は、あとで判定し直す。
            if inner.dnd_checked {
                inner
                    .call_toast_seen
                    .extend(candidates.iter().map(|toast| toast.invite_id.clone()));
            }
            return;
        }
        let in_dnd = NotifyContext {
            me: me.clone(),
            enabled: true,
            now: Some(now),
            dnd_until: inner.dnd_until,
            focused_channel: None,
        }
        .in_dnd();
        let in_front = self.window_in_front(inner);
        for toast in candidates {
            if inner.call_toast_seen.contains(&toast.invite_id) {
                continue;
            }
            inner.call_toast_seen.insert(toast.invite_id.clone());
            if in_dnd || in_front {
                continue;
            }
            let handle = self.show_call_toast(&toast);
            inner.call_toasts.insert(toast.invite_id.clone(), handle);
        }
    }

    fn show_call_toast(&self, invite: &notify::CallInviteToast) -> CallToast {
        let core = self.app.state::<Arc<Core>>().inner().clone();
        let id = invite.invite_id.clone();
        let decline_core = core.clone();
        let decline_id = id.clone();
        let accept = (self.ui == UiMode::Remote).then(|| {
            let core = core.clone();
            let id = id.clone();
            Box::new(move || {
                tauri::async_runtime::spawn(async move { core.accept_call_invite(id).await });
            }) as Box<dyn FnOnce() + Send>
        });
        let app = self.app.clone();
        let handle = toast::show_call_invite(
            &invite.title,
            &invite.body,
            CallActions {
                accept,
                decline: Some(Box::new(move || {
                    tauri::async_runtime::spawn(async move {
                        decline_core.decline_call_invite(&decline_id).await;
                    });
                })),
                open: Box::new(move || crate::window::show_main(&app)),
            },
        );
        // Shape から消えなかったときの保険。受け取った時刻 + 招待の長さ + 数秒で閉じる。
        if let Some(ms) = invite.duration_ms.filter(|ms| *ms > 0) {
            let guard = core;
            let id = id.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms as u64) + CALL_TOAST_GRACE).await;
                guard.close_call_toast(&id).await;
            });
        }
        handle
    }

    async fn close_call_toast(&self, invite_id: &str) {
        if let Some(toast) = self.inner.lock().await.call_toasts.remove(invite_id) {
            toast.close();
        }
    }

    fn close_all_call_toasts(inner: &mut Inner) {
        for (_, toast) in inner.call_toasts.drain() {
            toast.close();
        }
    }

    /// トーストの「応答」。ウィンドウを前に出し、WebView に印を取りに来る合図を送る。
    /// 応答そのもの（API と通話への参加）は、印を取った WebView が行う。このイベントだけでは応答しない。
    async fn accept_call_invite(&self, invite_id: String) {
        self.inner.lock().await.pending_accept = Some((invite_id, Instant::now()));
        crate::window::show_main(&self.app);
        let _ = self.app.emit_to(MAIN, event::CALL_ACCEPT, ());
    }

    /// 「応答」の印を、一度だけ返す。60 秒を過ぎた印は捨てる。
    pub async fn take_call_accept(&self) -> Option<String> {
        let (invite_id, at) = self.inner.lock().await.pending_accept.take()?;
        (at.elapsed() < CALL_ACCEPT_TTL).then_some(invite_id)
    }

    /// トーストの「拒否」。すでに終わっていても、失敗しても、期限で呼び出しは終わる。
    async fn decline_call_invite(&self, invite_id: &str) {
        if let Ok(api) = self.api().await {
            let _ = api.decline_call_invite(invite_id).await;
        }
    }

    fn show_toast(&self, toast: Toast) {
        if !self.notification_settings().enabled {
            return;
        }
        let target = OpenTarget {
            channel_id: toast.channel_id.clone(),
            message_id: toast.message_id.clone(),
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
        let ending = self.prepare_end_session(&mut inner).map_err(to_ipc)?;
        self.set_state(&mut inner, SessionState::LoggedOut);
        drop(inner);
        self.finish_end_session(ending).await.map_err(to_ipc)
    }

    /// 購読が 401 で止まったとき。その世代のセッションのままなら、ログアウトする。
    async fn expire_session(&self, generation: u64) {
        let mut inner = self.inner.lock().await;
        if inner.generation != generation {
            return;
        }
        let Ok(ending) = self.prepare_end_session(&mut inner) else {
            return;
        };
        self.set_state(&mut inner, SessionState::LoggedOut);
        drop(inner);
        let _ = self.finish_end_session(ending).await;
    }

    /// セッションを捨て、保存したトークンを消す。セッションの `TokenManager` を
    /// `sign_out` するので、コマンドが持っている `ApiClient` で進行中だった更新が
    /// 後から終わっても、`token.json` は作り直されない。
    fn prepare_end_session(&self, inner: &mut Inner) -> Result<EndingSession> {
        let oidc = matches!(
            self.config.as_ref().map(|c| &c.auth),
            Ok(AuthConfig::Oidc(_))
        );
        let fallback = if oidc
            && inner
                .session
                .as_ref()
                .and_then(|session| session.tokens.as_ref())
                .is_none()
        {
            let path = self.token_path()?;
            let expected = token::read_token_snapshot(&path)?;
            Some((path, expected))
        } else {
            None
        };
        inner.generation += 1;
        if let Some(task) = inner.login_task.take() {
            task.abort();
        }
        let session = inner.session.take();
        // 画面はチャンネル未選択に戻るので、前のチャンネルの通知を抑え続けないよう消す。
        inner.focus = FocusState::default();
        inner.dnd_until = None;
        inner.dnd_checked = false;
        Self::close_all_call_toasts(inner);
        inner.call_toast_seen.clear();
        inner.pending_accept = None;
        Ok(EndingSession {
            tokens: session.and_then(|session| session.tokens.clone()),
            fallback,
        })
    }

    async fn finish_end_session(&self, ending: EndingSession) -> Result<()> {
        if let Some(tokens) = ending.tokens {
            tokens.sign_out().await?;
        } else if let Some((path, expected)) = ending.fallback {
            tokio::task::spawn_blocking(move || {
                token::delete_if_unchanged(&path, expected.as_ref())
            })
            .await??;
        }
        Ok(())
    }

    pub async fn set_focus(&self, focus: FocusState) {
        let mut inner = self.inner.lock().await;
        inner.window_focused = focus.focused;
        inner.focus = focus;
    }

    pub async fn set_window_focused(&self, focused: bool) {
        let mut inner = self.inner.lock().await;
        inner.window_focused = focused;
        // 前面に来たら、着信は React のダイアログに任せて、トーストを閉じる。
        if focused {
            Self::close_all_call_toasts(&mut inner);
        }
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

fn notification_settings_path() -> Result<std::path::PathBuf> {
    config_dir_for(APP_NAME)
        .map(|dir| dir.join("notifications.json"))
        .ok_or_else(|| anyhow!("設定ディレクトリが見つかりません"))
}

/// 保存した通知の設定。ないときや読めないときは、既定 (出す)。
fn load_notification_settings() -> NotificationSettings {
    notification_settings_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
