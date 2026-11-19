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
    task::{AbortHandle, JoinHandle},
};
use zoo2_client_core::{
    api::ApiClient,
    auth::{
        self,
        device::{self, DeviceAuthorization, Poller},
        reauth::{LoopbackReauth, REAUTH_HTTP_TIMEOUT, REAUTH_TIMEOUT, ReauthError},
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
    /// ログイン URL の自動起動前リダイレクト検証専用クライアント（自動リダイレクト無効）。
    no_redirect_http: Option<reqwest::Client>,
    inner: Mutex<Inner>,
    /// ブラウザー起動とキャンセルを順序付ける同期ゲート（`inner` とは別に保ち、ブロッキング I/O で `inner` を止めない）。
    browser_launch: Arc<BrowserLaunchGate>,
    /// OS の通知を出すか (`notifications.json` に保存する)。
    notifications: std::sync::Mutex<NotificationSettings>,
    /// 待っている E2EE の再認証。新しく始めたら、前のものは取り消す。
    reauth: std::sync::Mutex<Option<AbortHandle>>,
}

#[derive(Default)]
struct BrowserLaunchGate {
    state: std::sync::Mutex<BrowserLaunchState>,
}

#[derive(Default)]
struct BrowserLaunchState {
    active_gen: Option<u64>,
    cancelled_through: u64,
}

impl BrowserLaunchGate {
    fn arm(&self, generation: u64) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if generation > guard.cancelled_through
            && guard.active_gen.is_none_or(|cur| generation > cur)
        {
            guard.active_gen = Some(generation);
        }
    }

    fn is_active(&self, generation: u64) -> bool {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.active_gen == Some(generation) && generation > guard.cancelled_through
    }

    fn cancel(&self, generation: u64) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.cancelled_through = guard.cancelled_through.max(generation);
        if guard.active_gen.is_some_and(|cur| cur <= generation) {
            guard.active_gen = None;
        }
    }

    /// 状態遷移・通知より先にゲートを無効化し、遷移通知後に旧試行の起動タスクが走っても抑止される順序を保証する。
    fn cancel_before_transition<F>(&self, generation: u64, transition: F)
    where
        F: FnOnce(),
    {
        self.cancel(generation);
        transition();
    }

    fn launch_if_active<F>(&self, generation: u64, launcher: F) -> bool
    where
        F: FnOnce(),
    {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard.active_gen != Some(generation) || generation <= guard.cancelled_through {
            return false;
        }
        guard.active_gen = None;
        launcher();
        true
    }
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
            no_redirect_http: build_no_redirect_http_client(),
            browser_launch: Arc::new(BrowserLaunchGate::default()),
            notifications: std::sync::Mutex::new(load_notification_settings()),
            reauth: std::sync::Mutex::new(None),
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

    /// OIDC の発行者 (`https://account.zli.works/...` など)。dev ログインや設定のエラーでは None。
    pub fn oidc_issuer(&self) -> Option<&str> {
        match &self.config.as_ref().ok()?.auth {
            AuthConfig::Oidc(oidc) => Some(oidc.issuer.as_str()),
            AuthConfig::Dev { .. } => None,
        }
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
                    ShapeEvent::Failed(ShapeId::CallInvites, message) => {
                        eprintln!(
                            "通話の招待を購読できません (着信のトーストは出ません): {message}"
                        );
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
                let first_check = !inner.dnd_checked;
                inner.dnd_checked = true;
                // DND を取る前に届いた着信は、まだ判定していない。初めて取れたときに、判定し直す。
                if first_check {
                    core.reconcile_call_toasts(&mut inner, false);
                }
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
        // 着信のダイアログがあるのは React の画面だけ。Leptos では、前面でもトーストを出す。
        let in_front = self.ui == UiMode::Remote && self.window_in_front(inner);
        let api = session.api.clone();
        for toast in candidates {
            if inner.call_toast_seen.contains(&toast.invite_id) {
                continue;
            }
            inner.call_toast_seen.insert(toast.invite_id.clone());
            if in_dnd || in_front {
                continue;
            }
            let handle = CallToast::default();
            inner
                .call_toasts
                .insert(toast.invite_id.clone(), handle.clone());
            self.show_call_toast(&toast, handle, api.clone(), inner.generation);
        }
    }

    fn show_call_toast(
        &self,
        invite: &notify::CallInviteToast,
        handle: CallToast,
        api: ApiClient,
        generation: u64,
    ) {
        let core = self.app.state::<Arc<Core>>().inner().clone();
        let id = invite.invite_id.clone();
        let decline_core = core.clone();
        let decline_id = id.clone();
        let accept = (self.ui == UiMode::Remote).then(|| {
            let core = core.clone();
            let id = id.clone();
            Box::new(move || {
                tauri::async_runtime::spawn(async move {
                    core.accept_call_invite(generation, id).await
                });
            }) as Box<dyn FnOnce() + Send>
        });
        let app = self.app.clone();
        let actions = CallActions {
            accept,
            decline: Some(Box::new(move || {
                tauri::async_runtime::spawn(async move {
                    decline_core.decline_call_invite(&decline_id).await;
                });
            })),
            open: Box::new(move || crate::window::show_main(&app)),
        };
        if invite.inviter_known {
            toast::show_call_invite(&handle, &invite.title, &invite.body, actions);
        } else {
            // users の Shape を持たない (React の) 画面では、呼んだ人の名前を、通話の参加者から API で引く。
            // 引けなければ、決まった文言のまま出す。閉じる指示が先に来ていたら、出さずに閉じる。
            let (inviter_id, channel_id) = (invite.inviter_id.clone(), invite.channel_id.clone());
            let (title, body) = (invite.title.clone(), invite.body.clone());
            tauri::async_runtime::spawn(async move {
                if handle.is_closed() {
                    return;
                }
                let name = tokio::time::timeout(
                    Duration::from_secs(3),
                    api.call_participant_name(&channel_id, &inviter_id),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
                match name {
                    Some(name) => toast::show_call_invite(&handle, &name, "着信", actions),
                    None => toast::show_call_invite(&handle, &title, &body, actions),
                }
            });
        }
        // Shape から消えなかったときの保険。受け取った時刻 + 招待の長さ + 数秒で閉じる。
        if let Some(ms) = invite.duration_ms.filter(|ms| *ms > 0) {
            let guard = core;
            let id = id.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(ms as u64) + CALL_TOAST_GRACE).await;
                guard.close_call_toast(&id).await;
            });
        }
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
    async fn accept_call_invite(&self, generation: u64, invite_id: String) {
        {
            let mut inner = self.inner.lock().await;
            // ログアウトのあと（世代が違う）や、すでに終わった招待では、印を残さない（別のユーザーのログインに
            // 残らないように）。ウィンドウが前面に来てトーストを閉じた直後でも、招待が呼び出し中なら残す。
            if inner.generation != generation {
                return;
            }
            let Some(session) = inner.session.as_ref() else {
                return;
            };
            let me = session.me.id.as_str();
            let pending = session.store.call_invites().into_iter().any(|invite| {
                invite.id == invite_id && invite.invitee_id == me && invite.status == "pending"
            });
            if !pending {
                return;
            }
            // 出ているトーストは、応答を受け付けた時点で閉じる（前面化の通知が遅れる環境でも閉じる）。
            if let Some(toast) = inner.call_toasts.remove(&invite_id) {
                toast.close();
            }
            inner.pending_accept = Some((invite_id, Instant::now()));
        }
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
            let prev = inner.generation;
            inner.generation += 1;
            // 前の試行がまだブラウザー起動待ちにあれば、置き換え時点で失効させる。
            self.browser_launch.cancel(prev);
            inner.generation
        };
        let discovery = tokio::time::timeout(
            REAUTH_HTTP_TIMEOUT,
            auth::discover(&self.http, &oidc.issuer),
        )
        .await
        .map_err(|_| {
            IpcError::new(
                ErrorCode::Timeout,
                "認証サーバーの応答がタイムアウトしました",
            )
        })?
        .map_err(to_ipc)?;
        let authorization = tokio::time::timeout(
            REAUTH_HTTP_TIMEOUT,
            device::start(&self.http, &discovery, oidc),
        )
        .await
        .map_err(|_| {
            IpcError::new(
                ErrorCode::Timeout,
                "認証サーバーの応答がタイムアウトしました",
            )
        })?
        .map_err(to_ipc)?;
        let authorized_at = Instant::now();
        let authorized_unix = token::now();
        let (login, auto_open_url, remaining_expires_in) = prepare_verified_device_login(
            self.no_redirect_http.as_ref(),
            &authorization,
            &oidc.issuer,
            authorized_at,
            authorized_unix,
        )
        .await?;
        let mut authorization = authorization;
        authorization.expires_in = remaining_expires_in;

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
                    core.browser_launch.cancel(generation);
                    core.finish_login(
                        generation,
                        Credentials::Bearer(manager.clone()),
                        Some(manager),
                    )
                    .await
                }
                Err(_) => {
                    let mut inner = core.inner.lock().await;
                    core.browser_launch
                        .cancel_before_transition(generation, || {
                            if inner.generation == generation {
                                core.set_state(&mut inner, SessionState::LoggedOut);
                            }
                        });
                }
            }
        }));
        if auto_open_url.is_some() {
            // 状態遷移・通知前にゲートを有効化し、直後の cancel_login と順序を揃える。
            self.browser_launch.arm(generation);
        }
        self.set_state(
            &mut inner,
            SessionState::LoggingIn {
                login: login.clone(),
            },
        );
        drop(inner);
        // 既定のブラウザーを自動で開く。開けなくても画面上のコードとリンクから手動で進められる。
        // 確認済みオリジンかつ GET リダイレクト先検証を通過した URL のみ自動で開き、中止済みなら実行時にも抑止する。
        if let Some(url) = auto_open_url {
            let gate = self.browser_launch.clone();
            let app = self.app.clone();
            tauri::async_runtime::spawn(async move {
                if !gate.is_active(generation) {
                    return;
                }
                if let Err(err) = tokio::task::spawn_blocking(move || {
                    gate.launch_if_active(generation, || {
                        if let Err(err) = open_in_browser(&app, &url) {
                            eprintln!("ログイン URL をブラウザーで開けませんでした: {err}");
                        }
                    });
                })
                .await
                {
                    eprintln!("ログイン URL をブラウザーで開く処理が失敗しました: {err}");
                }
            });
        }
        Ok(login)
    }

    pub async fn cancel_login(&self) {
        let mut inner = self.inner.lock().await;
        if let Some(task) = inner.login_task.take() {
            task.abort();
        }
        if matches!(inner.state, SessionState::LoggingIn { .. }) {
            let cancelled = inner.generation;
            inner.generation += 1;
            self.browser_launch.cancel_before_transition(cancelled, || {
                self.set_state(&mut inner, SessionState::LoggedOut);
            });
        }
    }

    pub async fn logout(&self) -> Result<(), IpcError> {
        let mut inner = self.inner.lock().await;
        let ending = self.prepare_end_session(&mut inner).map_err(to_ipc)?;
        let cancelled = inner.generation;
        self.browser_launch.cancel_before_transition(cancelled, || {
            self.set_state(&mut inner, SessionState::LoggedOut);
        });
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
    ///
    /// 待っている E2EE の再認証も取り消す (ループバックの待ち受けも閉じる)。ログアウトと 401 の
    /// 失効の両方がここを通る。`inner` のロックの下で取り消すので、`reauthenticate` が
    /// 同じロックの下で始めた再認証とは前後が決まる。
    fn prepare_end_session(&self, inner: &mut Inner) -> Result<EndingSession> {
        if let Some(reauth) = self.reauth.lock().expect("reauth の lock").take() {
            reauth.abort();
        }
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
        // Leptos の画面には着信のダイアログがないので、閉じない（「拒否」を押せなくなる）。
        if focused && self.ui == UiMode::Remote {
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

    /// E2EE の直前の再認証 (`docs/e2ee.md` の「初回設定と解錠の流れ」の 7)。ID トークンを返す。
    ///
    /// (b) の Web の画面は、Web 版と同じ別のウィンドウ (`window.open`) を開けない (wry は新しい
    /// ウィンドウを作らない)。また、AuthZ が再認証として認めるのはパスキーでのログインだけで、
    /// アプリの WebView ではパスキーを使えない。そこで TUI と同じく、既定のブラウザーで
    /// `max_age=0` の認可リクエストを開き、ループバックのリダイレクトで受け取る
    /// (`client-core` の `auth/reauth.rs`)。今のセッションのトークンは変えない。
    ///
    /// 新しく始めたら、待っている前の再認証は取り消す (Web で同じ名前のウィンドウを開き直すのと同じ)。
    pub async fn reauthenticate(self: &Arc<Self>) -> Result<String, IpcError> {
        let (_, oidc) = self.oidc()?;
        let oidc = oidc.clone();
        // ログイン中かを確かめてから、始めたタスクを `reauth` に置くまでを、`inner` と `reauth` の
        // ロックの下で行う (どちらも、その間に await しない)。ログアウトや 401 の失効
        // (`prepare_end_session`) と、ほかの再認証の開始が、間に入らないようにするため。
        // これで、後から始めた再認証が必ず残り、セッションが終わったら必ず取り消される。
        let task = {
            let inner = self.inner.lock().await;
            if !matches!(inner.state, SessionState::Ready { .. }) {
                return Err(not_logged_in());
            }
            let mut pending = self.reauth.lock().expect("reauth の lock");
            if let Some(previous) = pending.take() {
                previous.abort();
            }
            let core = self.clone();
            let task = tokio::spawn(async move {
                let discovery = tokio::time::timeout(
                    REAUTH_HTTP_TIMEOUT,
                    auth::discover(&core.http, &oidc.issuer),
                )
                .await
                .map_err(|_| ReauthError::Timeout)??;
                let pending = LoopbackReauth::start(&discovery, &oidc).await?;
                open_in_browser(&core.app, pending.url())?;
                pending
                    .finish(&core.http, &discovery, &oidc, REAUTH_TIMEOUT)
                    .await
            });
            *pending = Some(task.abort_handle());
            drop(inner);
            task
        };
        let attempt = task.id();
        let result = task.await;
        // 終わったタスクには `abort()` が効かないので、終わってからここに戻るまでの間に、新しい
        // 再認証やセッションの終わりで取り消されていたら、結果を使わない。まだ今の試行なら、
        // `reauth` から外す。
        let current = {
            let _inner = self.inner.lock().await;
            let mut pending = self.reauth.lock().expect("reauth の lock");
            let current = pending
                .as_ref()
                .is_some_and(|handle| handle.id() == attempt);
            if current {
                pending.take();
            }
            current
        };
        if !current && matches!(result, Ok(Ok(_))) {
            return Err(IpcError::new(
                ErrorCode::Cancelled,
                "新しい再認証を始めたか、ログアウトしたので、取り消しました",
            ));
        }
        match result {
            Ok(Ok(id_token)) => {
                crate::window::show_main(&self.app);
                Ok(id_token)
            }
            Ok(Err(error)) => Err(reauth_error(error)),
            Err(join) if join.is_cancelled() => Err(IpcError::new(
                ErrorCode::Cancelled,
                "新しい再認証を始めたので、取り消しました",
            )),
            Err(join) => Err(IpcError::new(ErrorCode::Internal, join.to_string())),
        }
    }
}

/// 再認証の認可リクエストを既定のブラウザーで開く。URL は IdP の認可エンドポイントなので、
/// 外部のサイトの確認 (`window::open_external`) はしない。
fn open_in_browser(app: &AppHandle, url: &str) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|error| anyhow!("ブラウザーを開けませんでした: {error}"))
}

/// 再認証の失敗を、Web の画面が見分けられる IPC のエラーにする。
fn reauth_error(error: anyhow::Error) -> IpcError {
    match error.downcast_ref::<ReauthError>() {
        Some(ReauthError::Timeout) => IpcError::new(ErrorCode::Timeout, format!("{error:#}")),
        Some(ReauthError::Denied(_) | ReauthError::InvalidIdToken(_)) => {
            IpcError::new(ErrorCode::Rejected, format!("{error:#}"))
        }
        None => to_ipc(error),
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

const MAX_LOGIN_REDIRECT_HOPS: usize = 5;
const LOGIN_REDIRECT_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

fn build_no_redirect_http_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(LOGIN_REDIRECT_CHECK_TIMEOUT)
        .timeout(LOGIN_REDIRECT_CHECK_TIMEOUT)
        .build()
        .ok()
}

fn is_loopback_host(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// ログイン URL が IdP (`issuer`) と同じオリジンの HTTPS URL（開発用ループバックのみ HTTP 許可）かを確かめる。
fn is_same_issuer_origin(url: &str, issuer: &str) -> bool {
    let (Ok(target), Ok(issuer)) = (url::Url::parse(url), url::Url::parse(issuer)) else {
        return false;
    };
    if target.origin() != issuer.origin() {
        return false;
    }
    match target.scheme() {
        "https" => true,
        // 開発時のローカル検証のみ HTTP を許可し、リモートでの平文送信を防ぐ。
        "http" => is_loopback_host(&target),
        _ => false,
    }
}

/// 未認証時・認証後の承認画面遷移まで含めて `http://` や外部オリジンへリダイレクトしないことが
/// IdP 実装 (`Zli-UoA/AuthZ`)・実機で確認済みの `issuer` オリジン（公式 AuthZ または開発用ループバック）かを返す。
const CONFIRMED_OIDC_ISSUER_ORIGIN: &str = "https://account.zli.works";

fn is_confirmed_login_issuer(issuer: &str) -> bool {
    let Ok(parsed) = url::Url::parse(issuer) else {
        return false;
    };
    if let Ok(confirmed) = url::Url::parse(CONFIRMED_OIDC_ISSUER_ORIGIN)
        && parsed.origin() == confirmed.origin()
    {
        return true;
    }
    matches!(parsed.scheme(), "http" | "https") && is_loopback_host(&parsed)
}

fn resolve_and_validate_redirect(
    current: &url::Url,
    location: &str,
    issuer: &str,
) -> std::result::Result<url::Url, String> {
    let next = current
        .join(location.trim())
        .map_err(|err| format!("リダイレクト先 URL を解釈できませんでした: {err}"))?;
    if !is_same_issuer_origin(next.as_str(), issuer) {
        return Err(format!(
            "リダイレクト先のスキームまたはオリジンが不正です: {}",
            next.origin().ascii_serialization()
        ));
    }
    Ok(next)
}

/// ブラウザー自動起動およびコード付き URL 採用の前に、`GET` リクエストの `Location` リダイレクトを
/// 最大 `MAX_LOGIN_REDIRECT_HOPS` 回まで手動追跡し、すべての遷移先が IdP と同一オリジンの
/// HTTPS（開発用ループバックのみ HTTP）に留まることを確認する。
async fn verify_login_redirect_chain(
    client: &reqwest::Client,
    url: &str,
    issuer: &str,
) -> std::result::Result<(), String> {
    if !is_same_issuer_origin(url, issuer) {
        return Err("初期 URL のスキームまたはオリジンが不正です".to_string());
    }
    let mut current = url::Url::parse(url)
        .map_err(|err| format!("ログイン URL を解釈できませんでした: {err}"))?;
    for hop in 0..=MAX_LOGIN_REDIRECT_HOPS {
        let res = client.get(current.clone()).send().await.map_err(|err| {
            format!(
                "ログイン URL の事前確認 (GET) に失敗しました: {}",
                err.without_url()
            )
        })?;
        let status = res.status();
        if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
            if hop == MAX_LOGIN_REDIRECT_HOPS {
                return Err("リダイレクト回数が上限を超えました".to_string());
            }
            let location = res
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| "リダイレクト応答に Location ヘッダーがありません".to_string())?
                .to_str()
                .map_err(|_| "Location ヘッダーに不正な文字が含まれています".to_string())?;
            current = resolve_and_validate_redirect(&current, location, issuer)?;
            continue;
        }
        if status.is_success() {
            return Ok(());
        }
        return Err(format!(
            "ログイン URL の事前確認が予期しないステータスを返しました (HTTP {})",
            status.as_u16()
        ));
    }
    Err("リダイレクト回数が上限を超えました".to_string())
}

/// デバイス認可応答の URL を検証し、UI へ渡す `DeviceLogin`・自動起動対象 URL・残りの有効秒数を返す。
/// - `verification_uri` が `issuer` と別オリジン（または非ループバック HTTP）の場合や、
///   リダイレクト安全性が未確認の外部カスタム `issuer` では、ログイン自体は失敗させず
///   ユーザーコードと手動リンク表示へフォールバックし、ブラウザー自動起動と `verification_uri_complete` を抑止する。
/// - 確認済み `issuer` かつ同一オリジンの場合のみ、自動起動および `verification_uri_complete` の採用前に
///   `GET` でリダイレクト先を検証する。
/// - 事前検証完了時点で認可コードの有効期限が切れていた場合は、`LoggingIn` 遷移やブラウザー自動起動へ進まず
///   タイムアウトエラーを返す。
async fn prepare_verified_device_login(
    no_redirect_http: Option<&reqwest::Client>,
    authorization: &DeviceAuthorization,
    issuer: &str,
    authorized_at: Instant,
    authorized_unix: i64,
) -> Result<(DeviceLogin, Option<String>, u64), IpcError> {
    let mut verification_uri_complete = None;
    let mut auto_open_url = None;

    if !is_same_issuer_origin(&authorization.verification_uri, issuer) {
        eprintln!(
            "ログイン確認 URL のスキームまたはオリジンが issuer と一致しないため、コード付き URL とブラウザー自動起動を抑止します"
        );
    } else if !is_confirmed_login_issuer(issuer) {
        eprintln!(
            "リダイレクト挙動が未確認のカスタム issuer のため、コード付き URL とブラウザー自動起動を抑止します"
        );
    } else if let Some(client) = no_redirect_http {
        match authorization.verification_uri_complete.as_deref() {
            Some(complete) => match verify_login_redirect_chain(client, complete, issuer).await {
                Ok(()) => {
                    verification_uri_complete = Some(complete.to_string());
                    auto_open_url = Some(complete.to_string());
                }
                Err(err) => {
                    eprintln!(
                        "コード付きログイン URL のリダイレクト先検証に失敗したため、コード付き URL と自動起動を抑止します: {err}"
                    );
                }
            },
            None => {
                match verify_login_redirect_chain(client, &authorization.verification_uri, issuer)
                    .await
                {
                    Ok(()) => {
                        auto_open_url = Some(authorization.verification_uri.clone());
                    }
                    Err(err) => {
                        eprintln!(
                            "ログイン URL のリダイレクト先検証に失敗したため、自動では開きません: {err}"
                        );
                    }
                }
            }
        }
    } else {
        eprintln!(
            "リダイレクト検証クライアントを初期化できないため、コード付き URL とブラウザー自動起動をスキップします"
        );
    }

    let expires_at = authorized_unix.saturating_add(authorization.expires_in as i64);
    let elapsed = authorized_at.elapsed();
    let Some(remaining) = Duration::from_secs(authorization.expires_in).checked_sub(elapsed) else {
        return Err(IpcError::new(
            ErrorCode::Timeout,
            "コードの有効期限が切れました。もう一度ログインしてください",
        ));
    };
    let remaining_secs = remaining.as_secs();
    if remaining_secs == 0 || token::now() >= expires_at {
        return Err(IpcError::new(
            ErrorCode::Timeout,
            "コードの有効期限が切れました。もう一度ログインしてください",
        ));
    }

    let login = DeviceLogin {
        user_code: authorization.user_code.clone(),
        verification_uri: authorization.verification_uri.clone(),
        verification_uri_complete,
        expires_at,
    };
    Ok((login, auto_open_url, remaining_secs))
}

#[cfg(test)]
mod tests {
    use super::{
        BrowserLaunchGate, DeviceAuthorization, ErrorCode, Instant, build_no_redirect_http_client,
        is_confirmed_login_issuer, is_same_issuer_origin, prepare_verified_device_login,
        resolve_and_validate_redirect, token, verify_login_redirect_chain,
    };
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };

    #[test]
    fn login_url_origin_validation_requires_same_https_or_loopback_origin() {
        assert!(is_same_issuer_origin(
            "https://account.zli.works/device?user_code=ABCD-1234",
            "https://account.zli.works/realms/zli"
        ));
        assert!(is_same_issuer_origin(
            "http://localhost:8080/device",
            "http://localhost:8080"
        ));
        assert!(is_same_issuer_origin(
            "http://127.0.0.1:8080/device",
            "http://127.0.0.1:8080"
        ));
        assert!(is_same_issuer_origin(
            "http://[::1]:8080/device",
            "http://[::1]:8080"
        ));
        assert!(!is_same_issuer_origin(
            "http://account.zli.works/device",
            "http://account.zli.works/realms/zli"
        ));
        assert!(!is_same_issuer_origin(
            "http://localhost.evil.example:8080/device",
            "http://localhost.evil.example:8080"
        ));
        assert!(!is_same_issuer_origin(
            "https://evil.example/device",
            "https://account.zli.works"
        ));
        assert!(!is_same_issuer_origin(
            "https://account.zli.works:8443/device",
            "https://account.zli.works"
        ));
        assert!(!is_same_issuer_origin(
            "http://account.zli.works/device",
            "https://account.zli.works"
        ));
        assert!(!is_same_issuer_origin(
            "file:///C:/Windows/System32/calc.exe",
            "https://account.zli.works"
        ));
        assert!(!is_same_issuer_origin(
            "javascript:alert(1)",
            "https://account.zli.works"
        ));
        assert!(!is_same_issuer_origin(
            "not a url",
            "https://account.zli.works"
        ));

        assert!(is_confirmed_login_issuer(
            "https://account.zli.works/api/auth"
        ));
        assert!(is_confirmed_login_issuer("http://127.0.0.1:8080/api/auth"));
        assert!(is_confirmed_login_issuer("http://localhost:8080/api/auth"));
        assert!(is_confirmed_login_issuer("https://[::1]:8443/api/auth"));
        assert!(!is_confirmed_login_issuer(
            "https://custom-idp.example/api/auth"
        ));
        assert!(!is_confirmed_login_issuer(
            "http://account.zli.works/api/auth"
        ));
        assert!(!is_confirmed_login_issuer(
            "https://account.zli.works:8443/api/auth"
        ));
    }

    #[tokio::test]
    async fn login_redirect_validation_allows_same_origin_and_blocks_downgrade_or_cross_origin() {
        let base = url::Url::parse("https://account.zli.works/device?user_code=ABCD-1234").unwrap();
        let issuer = "https://account.zli.works/api/auth";
        assert_eq!(
            resolve_and_validate_redirect(&base, "/login?next=%2Fdevice", issuer)
                .unwrap()
                .as_str(),
            "https://account.zli.works/login?next=%2Fdevice"
        );
        assert!(
            resolve_and_validate_redirect(&base, "http://account.zli.works/login", issuer).is_err()
        );
        assert!(
            resolve_and_validate_redirect(&base, "https://evil.example/login", issuer).is_err()
        );
        assert!(resolve_and_validate_redirect(&base, "//evil.example/login", issuer).is_err());

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_server = stop.clone();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop_server.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                // Windows では nonblocking なリスナーから accept したソケットも nonblocking を継承する。
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut req_bytes = Vec::new();
                let mut buf = [0u8; 2048];
                while !req_bytes.windows(4).any(|w| w == b"\r\n\r\n") && req_bytes.len() < 8192 {
                    match stream.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => req_bytes.extend_from_slice(&buf[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&req_bytes);
                let first_line = req.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("");
                let path = parts.next().unwrap_or("/");
                let resp = match (method, path) {
                    ("GET", "/device") => {
                        "HTTP/1.1 303 See Other\r\nLocation: /login?next=%2Fdevice\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    ("GET", "/device?user_code=OK") => {
                        "HTTP/1.1 303 See Other\r\nLocation: /login?next=%2Fdevice\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    ("GET", "/login?next=%2Fdevice") => {
                        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    // HEAD では 200 を返すが GET では http:// へダウングレードするケースも GET 検査で拒否する。
                    ("HEAD", "/get-only-downgrade?user_code=LEAK") => {
                        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    ("GET", "/get-only-downgrade?user_code=LEAK") => {
                        "HTTP/1.1 302 Found\r\nLocation: http://account.zli.works/login?user_code=LEAK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    (_, "/downgrade") => {
                        "HTTP/1.1 302 Found\r\nLocation: http://account.zli.works/login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    (_, "/cross-origin") => {
                        "HTTP/1.1 302 Found\r\nLocation: https://evil.example/device\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    (_, "/loop") => {
                        "HTTP/1.1 302 Found\r\nLocation: /loop\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    }
                    _ => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                };
                let _ = stream.write_all(resp.as_bytes());
            }
        });

        let client = build_no_redirect_http_client().unwrap();
        let local_issuer = format!("http://{addr}/api/auth");
        assert!(
            verify_login_redirect_chain(
                &client,
                &format!("http://{addr}/device?user_code=OK"),
                &local_issuer
            )
            .await
            .is_ok()
        );
        assert!(
            verify_login_redirect_chain(
                &client,
                &format!("http://{addr}/get-only-downgrade?user_code=LEAK"),
                &local_issuer
            )
            .await
            .is_err()
        );
        assert!(
            verify_login_redirect_chain(
                &client,
                &format!("http://{addr}/downgrade"),
                &local_issuer
            )
            .await
            .is_err()
        );
        assert!(
            verify_login_redirect_chain(
                &client,
                &format!("http://{addr}/cross-origin"),
                &local_issuer
            )
            .await
            .is_err()
        );
        assert!(
            verify_login_redirect_chain(&client, &format!("http://{addr}/loop"), &local_issuer)
                .await
                .is_err()
        );

        // 1) 確認済み issuer + GET リダイレクト検証通過時は verification_uri_complete と自動起動 URL を保持する
        let now_unix = token::now();
        let ok_auth = DeviceAuthorization {
            device_code: "dev-ok".to_string(),
            user_code: "OK".to_string(),
            verification_uri: format!("http://{addr}/device"),
            verification_uri_complete: Some(format!("http://{addr}/device?user_code=OK")),
            expires_in: 600,
            interval: Some(5),
        };
        let (ok_login, ok_auto_open, ok_remaining) = prepare_verified_device_login(
            Some(&client),
            &ok_auth,
            &local_issuer,
            Instant::now(),
            now_unix,
        )
        .await
        .unwrap();
        assert_eq!(
            ok_login.verification_uri_complete.as_deref(),
            Some(format!("http://{addr}/device?user_code=OK").as_str())
        );
        assert_eq!(
            ok_auto_open.as_deref(),
            Some(format!("http://{addr}/device?user_code=OK").as_str())
        );
        assert!(ok_remaining > 0 && ok_remaining <= 600);

        // 2) GET でダウングレードが発生するコード付き URL は、UI 向け verification_uri_complete から除外し自動起動も抑止する
        let downgrade_auth = DeviceAuthorization {
            device_code: "dev-downgrade".to_string(),
            user_code: "LEAK".to_string(),
            verification_uri: format!("http://{addr}/device"),
            verification_uri_complete: Some(format!(
                "http://{addr}/get-only-downgrade?user_code=LEAK"
            )),
            expires_in: 600,
            interval: Some(5),
        };
        let (downgrade_login, downgrade_auto_open, _) = prepare_verified_device_login(
            Some(&client),
            &downgrade_auth,
            &local_issuer,
            Instant::now(),
            now_unix,
        )
        .await
        .unwrap();
        assert!(downgrade_login.verification_uri_complete.is_none());
        assert!(downgrade_auto_open.is_none());

        // 3) リダイレクト挙動が未確認の外部カスタム issuer では、自動起動を抑止し verification_uri_complete も落とす
        let unconfirmed_issuer = "https://custom-idp.example/api/auth";
        let unconfirmed_auth = DeviceAuthorization {
            device_code: "dev-custom".to_string(),
            user_code: "CUSTOM12".to_string(),
            verification_uri: "https://custom-idp.example/device".to_string(),
            verification_uri_complete: Some(
                "https://custom-idp.example/device?user_code=CUSTOM12".to_string(),
            ),
            expires_in: 600,
            interval: Some(5),
        };
        let (unconfirmed_login, unconfirmed_auto_open, _) = prepare_verified_device_login(
            Some(&client),
            &unconfirmed_auth,
            unconfirmed_issuer,
            Instant::now(),
            now_unix,
        )
        .await
        .unwrap();
        assert_eq!(
            unconfirmed_login.verification_uri,
            "https://custom-idp.example/device"
        );
        assert!(unconfirmed_login.verification_uri_complete.is_none());
        assert!(unconfirmed_auto_open.is_none());

        // 4) 別オリジンの verification_uri を返す IdP でもログインは失敗させず、手動リンクのみ表示して自動起動と verification_uri_complete を抑止する
        let cross_origin_auth = DeviceAuthorization {
            device_code: "dev-cross".to_string(),
            user_code: "CROSS123".to_string(),
            verification_uri: "https://login.custom-idp.example/device".to_string(),
            verification_uri_complete: Some(
                "https://login.custom-idp.example/device?user_code=CROSS123".to_string(),
            ),
            expires_in: 600,
            interval: Some(5),
        };
        let (cross_login, cross_auto_open, _) = prepare_verified_device_login(
            Some(&client),
            &cross_origin_auth,
            unconfirmed_issuer,
            Instant::now(),
            now_unix,
        )
        .await
        .unwrap();
        assert_eq!(cross_login.user_code, "CROSS123");
        assert_eq!(
            cross_login.verification_uri,
            "https://login.custom-idp.example/device"
        );
        assert!(cross_login.verification_uri_complete.is_none());
        assert!(cross_auto_open.is_none());

        // 5) 事前検証完了時点で認可コードの有効期限が切れている場合は Timeout エラーを返し、残り秒数は経過分を差し引く
        let deducted_start = Instant::now() - std::time::Duration::from_secs(12);
        let (_, _, deducted_remaining) = prepare_verified_device_login(
            Some(&client),
            &ok_auth,
            &local_issuer,
            deducted_start,
            now_unix,
        )
        .await
        .unwrap();
        assert!(
            deducted_remaining <= 588,
            "remaining_secs should deduct elapsed time: {deducted_remaining}"
        );

        let expired_start = Instant::now() - std::time::Duration::from_secs(600);
        let expired_err = prepare_verified_device_login(
            Some(&client),
            &ok_auth,
            &local_issuer,
            expired_start,
            now_unix - 600,
        )
        .await
        .unwrap_err();
        assert_eq!(expired_err.code, ErrorCode::Timeout);

        stop.store(true, Ordering::SeqCst);
        let _ = server.join();

        // 通信エラー時のログ用エラー文字列に user_code 付き URL が含まれないことを確認する。
        let net_err = verify_login_redirect_chain(
            &client,
            &format!("http://{addr}/device?user_code=SECRET-CODE-999"),
            &local_issuer,
        )
        .await
        .unwrap_err();
        assert!(
            !net_err.contains("SECRET-CODE-999"),
            "error leaked user_code: {net_err}"
        );
    }

    #[test]
    fn browser_launch_gate_skips_launch_when_cancelled_before_blocking_task_runs() {
        let gate = BrowserLaunchGate::default();
        let launched = Arc::new(AtomicUsize::new(0));

        // 状態確認後にブロッキングタスク実行前に中止されたケース
        gate.arm(1);
        gate.cancel(1);
        assert!(!gate.launch_if_active(1, || {
            launched.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(launched.load(Ordering::SeqCst), 0);

        // 中止されていないログイン試行は通常どおり起動し、同一世代は二重起動しない
        gate.arm(2);
        assert!(gate.launch_if_active(2, || {
            launched.fetch_add(1, Ordering::SeqCst);
        }));
        assert!(!gate.launch_if_active(2, || {
            launched.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(launched.load(Ordering::SeqCst), 1);

        // 先行世代の中止通知が遅れて届いても新しい世代の起動を妨げない
        gate.arm(3);
        gate.cancel(2);
        assert!(gate.launch_if_active(3, || {
            launched.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(launched.load(Ordering::SeqCst), 2);

        // 次の試行へ置き換えられた (new arm の前に prev を失効させた) 直後の旧タスクも拒否される
        gate.arm(4);
        gate.cancel(4);
        assert!(!gate.launch_if_active(4, || {
            launched.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(launched.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn browser_launch_gate_orders_cancel_before_state_transition_and_blocks_concurrent_launch() {
        // 1) cancel_before_transition 内の状態変更・通知コールバック実行中に別スレッドが起動を試みても拒否される
        let gate = Arc::new(BrowserLaunchGate::default());
        gate.arm(10);
        let launched = Arc::new(AtomicUsize::new(0));
        let (tx_transition, rx_transition) = mpsc::channel();
        let (tx_attempt_done, rx_attempt_done) = mpsc::channel();

        let gate_worker = gate.clone();
        let launched_worker = launched.clone();
        let worker = std::thread::spawn(move || {
            rx_transition.recv().unwrap();
            let ran = gate_worker.launch_if_active(10, || {
                launched_worker.fetch_add(1, Ordering::SeqCst);
            });
            tx_attempt_done.send(ran).unwrap();
        });

        gate.cancel_before_transition(10, || {
            // LoggedOut 通知と同タイミングで起動タスクが走っても、既にゲートが無効化されている。
            tx_transition.send(()).unwrap();
            let ran = rx_attempt_done.recv().unwrap();
            assert!(
                !ran,
                "cancelled generation must not launch during LoggedOut transition"
            );
        });
        worker.join().unwrap();
        assert_eq!(launched.load(Ordering::SeqCst), 0);

        // 2) 既に launch_if_active が起動処理に入っている場合は、その完了後に状態遷移コールバックが走る
        gate.arm(11);
        let entered = Arc::new(Barrier::new(2));
        let release_launch = Arc::new(Barrier::new(2));
        let transitioned = Arc::new(AtomicBool::new(false));

        let gate_launch = gate.clone();
        let entered_launch = entered.clone();
        let release_worker = release_launch.clone();
        let transitioned_check = transitioned.clone();
        let launcher_thread = std::thread::spawn(move || {
            assert!(gate_launch.launch_if_active(11, || {
                entered_launch.wait();
                std::thread::sleep(std::time::Duration::from_millis(30));
                assert!(
                    !transitioned_check.load(Ordering::SeqCst),
                    "LoggedOut transition must wait for in-flight launch gate lock"
                );
                release_worker.wait();
            }));
        });

        entered.wait();
        let gate_cancel = gate.clone();
        let transitioned_cancel = transitioned.clone();
        let cancel_thread = std::thread::spawn(move || {
            gate_cancel.cancel_before_transition(11, || {
                transitioned_cancel.store(true, Ordering::SeqCst);
            });
        });

        release_launch.wait();
        launcher_thread.join().unwrap();
        cancel_thread.join().unwrap();
        assert!(transitioned.load(Ordering::SeqCst));
    }
}
