//! デスクトップ版の状態管理 (`AppState`) と Electric Shape / REST API の非同期連携。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use gpui::{AppContext as _, Context, Entity};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    task::AbortHandle,
};
use zoo2_client_core::{
    api::{ApiClient, Me, ReadState, TypingUser},
    config::{AuthConfig, Config},
    electric::{self, ShapeEvent, ShapeId, rows::Channel},
    http::{Credentials, HttpError},
    store::Store,
    update,
};

use crate::login::{self, AuthEvent, LoginPhase};

const READ_STATE_POLL: Duration = Duration::from_secs(10);
const TYPING_SEND_INTERVAL: Duration = Duration::from_secs(2);
const TYPING_IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const NOTICE_TIMEOUT: Duration = Duration::from_secs(5);
const FAILED_DRAFT_LIMIT: usize = 20;
const INITIAL_TIMELINE_LIMIT: usize = 100;
const TIMELINE_LOAD_STEP: usize = 100;

/// クイックリアクション候補。
pub const QUICK_EMOJIS: &[&str] = &["👍", "🎉", "❤️", "👀", "😂", "🙏"];

/// 入力欄の送信先。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    /// 開いているチャンネルへの通常投稿。
    Channel,
    /// スレッドのルートメッセージへの返信。
    Reply(String),
    /// 自分の既存メッセージの編集。
    Edit(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadState {
    pub root_id: String,
    pub selected: Option<String>,
}

/// 送信・編集に失敗した本文の退避データ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedDraft {
    pub channel_id: String,
    pub target: Target,
    pub content: String,
}

/// バックグラウンドタスクから `AppState` へ届くイベント。
#[derive(Debug)]
pub enum AppEvent {
    Notice(String),
    Unauthorized,
    RestoreDraft {
        channel_id: String,
        target: Target,
        content: String,
    },
    ReadStates(Vec<ReadState>),
    ReadMarkFailed {
        channel_id: String,
        message_id: String,
    },
    Typing {
        channel_id: String,
        parent_id: Option<String>,
        users: Vec<TypingUser>,
    },
    TypingPollFinished,
    UpdateAvailable(String),
}

/// Tokio タスク群から GPUI の `Entity<AppState>` へ橋渡しする統合イベント。
#[derive(Debug)]
pub enum DesktopEvent {
    Auth(AuthEvent),
    Shape(ShapeEvent),
    App(AppEvent),
    Tick,
}

/// デスクトップ版のアプリケーション状態 (`Entity<AppState>`)。
pub struct AppState {
    pub config: Config,
    pub api_base_url: String,
    pub auth_summary: String,
    pub store: Store,
    pub http: reqwest::Client,
    pub runtime: Option<Arc<tokio::runtime::Runtime>>,
    pub event_tx: UnboundedSender<DesktopEvent>,

    /// ログイン画面のフェーズ (`None` のときはログイン済みでチャット画面を表示)。
    pub login_phase: Option<LoginPhase>,
    pub login_task: Option<AbortHandle>,

    pub credentials: Option<Credentials>,
    pub api: Option<ApiClient>,
    pub me: Option<Me>,
    pub session_id: String,

    pub current_channel: Option<String>,
    pub selected_message: Option<String>,
    pub thread: Option<ThreadState>,
    pub target: Target,

    /// リアクション入力欄を開いている対象の `(ペイン識別子, メッセージ ID)`。
    pub active_reaction_message: Option<(String, String)>,
    /// 削除確認を出している対象のメッセージ ID。
    pub confirm_delete_message: Option<String>,

    /// サイドバーのチャンネル絞り込みクエリ。
    pub search_query: String,
    /// `AppState` 側から検索欄 (`TextInput`) へ反映すべき文字列。
    pub pending_search_text: Option<String>,
    /// メッセージ入力欄の現在の文字列。
    pub composer_draft: String,
    /// `(チャンネル ID, 送信先)` ごとの書きかけ下書き。
    pub saved_drafts: HashMap<(String, Target), String>,
    /// `AppState` 側から入力欄 (`TextInput`) へ反映すべき文字列。
    pub pending_composer_text: Option<String>,
    pub failed_drafts: Vec<FailedDraft>,

    pub unread: HashSet<String>,
    pub last_read_marked: HashMap<String, String>,
    pub newest_shown: Option<(String, String)>,
    /// ウィンドウが前面（アクティブ）かどうか。非アクティブの間は自動既読にしない。
    pub window_active: bool,
    /// タイムラインが最下部を表示している（新着メッセージに追従する）かどうか。
    pub timeline_at_bottom: bool,
    /// 次の描画でタイムラインのスクロール位置を最下部へ移動させるフラグ。
    pub pending_scroll_to_bottom: bool,
    /// タイムラインで一度に描画する末尾からの最大ルートメッセージ数。
    pub timeline_render_limit: usize,

    pub typing: HashMap<Option<String>, Vec<TypingUser>>,
    pub typing_sent: Option<(String, Option<String>, Instant)>,
    pub last_key_typed: Option<Instant>,
    pub typing_poll_in_flight: bool,

    pub notice: Option<(String, Instant)>,
    pub update_available: Option<String>,

    pub subscriptions: HashMap<ShapeId, AbortHandle>,
    pub background_tasks: Vec<AbortHandle>,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("api_base_url", &self.api_base_url)
            .field("auth_summary", &self.auth_summary)
            .field("login_phase", &self.login_phase)
            .field("current_channel", &self.current_channel)
            .field("target", &self.target)
            .finish()
    }
}

impl AppState {
    /// 設定から初期状態を作る (単体テストおよび GPUI 起動の共通初期化)。
    pub fn from_config(config: &Config) -> Self {
        let (event_tx, _) = mpsc::unbounded_channel();
        Self::with_channel(config.clone(), None, event_tx)
    }

    pub fn with_channel(
        config: Config,
        runtime: Option<Arc<tokio::runtime::Runtime>>,
        event_tx: UnboundedSender<DesktopEvent>,
    ) -> Self {
        let auth_summary = match &config.auth {
            AuthConfig::Dev { user_id } => format!("Dev モード (X-Dev-User: {user_id})"),
            AuthConfig::Oidc(oidc) => format!("OIDC ({})", oidc.issuer),
        };
        Self {
            api_base_url: config.api_base_url.clone(),
            auth_summary,
            config,
            store: Store::default(),
            http: reqwest::Client::new(),
            runtime,
            event_tx,
            login_phase: Some(LoginPhase::Checking),
            login_task: None,
            credentials: None,
            api: None,
            me: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            current_channel: None,
            selected_message: None,
            thread: None,
            target: Target::Channel,
            active_reaction_message: None,
            confirm_delete_message: None,
            search_query: String::new(),
            pending_search_text: None,
            composer_draft: String::new(),
            saved_drafts: HashMap::new(),
            pending_composer_text: None,
            failed_drafts: Vec::new(),
            unread: HashSet::new(),
            last_read_marked: HashMap::new(),
            newest_shown: None,
            window_active: true,
            timeline_at_bottom: true,
            pending_scroll_to_bottom: false,
            timeline_render_limit: INITIAL_TIMELINE_LIMIT,
            typing: HashMap::new(),
            typing_sent: None,
            last_key_typed: None,
            typing_poll_in_flight: false,
            notice: None,
            update_available: None,
            subscriptions: HashMap::new(),
            background_tasks: Vec::new(),
        }
    }

    /// GPUI の `Entity<AppState>` を生成し、Tokio ランタイムからのイベント受信ループを開始する。
    pub fn spawn_entity(
        config: Config,
        runtime: Arc<tokio::runtime::Runtime>,
        cx: &mut gpui::App,
    ) -> Entity<Self> {
        let (event_tx, event_rx) = mpsc::unbounded_channel();
        cx.new(|cx| {
            let mut state = Self::with_channel(config, Some(runtime), event_tx);
            state.start_event_pump(event_rx, cx);
            state.start_initial_auth();
            state
        })
    }

    fn start_event_pump(
        &self,
        mut event_rx: UnboundedReceiver<DesktopEvent>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            while let Some(event) = event_rx.recv().await {
                if this
                    .update(cx, |state, cx| {
                        if state.on_desktop_event(event) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// Tokio ランタイム上で非同期タスクを起動する。
    /// テスト環境ですでに `#[tokio::test]` ランタイム内にいる場合は `tokio::spawn` を使う。
    fn spawn_tokio<F>(&self, future: F) -> Option<AbortHandle>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if let Some(rt) = &self.runtime {
            Some(rt.spawn(future).abort_handle())
        } else if tokio::runtime::Handle::try_current().is_ok() {
            Some(tokio::spawn(future).abort_handle())
        } else {
            None
        }
    }

    /// 起動時または再試行時に初期認証チェックを開始する。
    pub fn start_initial_auth(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.login_phase = Some(LoginPhase::Checking);
        let http = self.http.clone();
        let config = self.config.clone();
        let tx = self.event_tx.clone();
        let (auth_tx, mut auth_rx) = mpsc::unbounded_channel();
        self.login_task = self.spawn_tokio(async move {
            let forward = tokio::spawn(async move {
                while let Some(ev) = auth_rx.recv().await {
                    if tx.send(DesktopEvent::Auth(ev)).is_err() {
                        break;
                    }
                }
            });
            login::connect_initial(http, config, auth_tx).await;
            let _ = forward.await;
        });
    }

    /// OIDC デバイスコード認証を開始する。
    pub fn start_device_login(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.login_phase = Some(LoginPhase::Checking);
        let http = self.http.clone();
        let config = self.config.clone();
        let tx = self.event_tx.clone();
        let (auth_tx, mut auth_rx) = mpsc::unbounded_channel();
        self.login_task = self.spawn_tokio(async move {
            let forward = tokio::spawn(async move {
                while let Some(ev) = auth_rx.recv().await {
                    if tx.send(DesktopEvent::Auth(ev)).is_err() {
                        break;
                    }
                }
            });
            login::run_device_login(http, config, auth_tx).await;
            let _ = forward.await;
        });
    }

    /// デバイスコード認証の待機をキャンセルする。
    pub fn cancel_device_login(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.login_phase = Some(LoginPhase::Ready { error: None });
    }

    /// ログアウトしてトークンを削除し、ログイン画面へ戻る。
    pub fn logout(&mut self) {
        self.stop_typing();
        self.abort_all_tasks();
        if let AuthConfig::Oidc(oidc) = &self.config.auth {
            let oidc = oidc.clone();
            let http = self.http.clone();
            if let Ok((_, Some(refresh_token))) = login::delete_local_token_for_logout(&self.config)
            {
                self.spawn_tokio(async move {
                    login::revoke_refresh_token(http, oidc, refresh_token).await;
                });
            }
        }
        self.reset_to_login(None);
    }

    fn abort_all_tasks(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        for (_, handle) in self.subscriptions.drain() {
            handle.abort();
        }
        for handle in self.background_tasks.drain(..) {
            handle.abort();
        }
    }

    fn reset_to_login(&mut self, error: Option<String>) {
        self.abort_all_tasks();
        self.store = Store::default();
        self.credentials = None;
        self.api = None;
        self.me = None;
        self.current_channel = None;
        self.selected_message = None;
        self.thread = None;
        self.target = Target::Channel;
        self.active_reaction_message = None;
        self.confirm_delete_message = None;
        self.search_query.clear();
        self.pending_search_text = Some(String::new());
        self.composer_draft.clear();
        self.saved_drafts.clear();
        self.pending_composer_text = Some(String::new());
        self.failed_drafts.clear();
        self.unread.clear();
        self.last_read_marked.clear();
        self.newest_shown = None;
        self.timeline_at_bottom = true;
        self.pending_scroll_to_bottom = false;
        self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
        self.typing.clear();
        self.typing_sent = None;
        self.typing_poll_in_flight = false;
        self.login_phase = Some(LoginPhase::Ready { error });
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    /// イベントを処理し、画面の再描画が必要な状態変化があった場合に `true` を返す。
    pub fn on_desktop_event(&mut self, event: DesktopEvent) -> bool {
        match event {
            DesktopEvent::Auth(auth) => {
                self.on_auth_event(auth);
                true
            }
            DesktopEvent::Shape(shape) => {
                self.on_shape(shape);
                true
            }
            DesktopEvent::App(app) => self.on_app_event(app),
            DesktopEvent::Tick => self.on_tick(),
        }
    }

    pub fn on_auth_event(&mut self, event: AuthEvent) {
        match event {
            AuthEvent::Connected { credentials, me } => {
                self.connect_session(credentials, me);
            }
            AuthEvent::DevicePrompt {
                verification_uri,
                user_code,
            } => {
                self.login_phase = Some(LoginPhase::AwaitingApproval {
                    verification_uri,
                    user_code,
                });
            }
            AuthEvent::LoginRequired { error } => {
                self.login_phase = Some(LoginPhase::Ready { error });
            }
        }
    }

    /// 認証完了後にセッションを開始し、Electric Shape 購読と定期タスクを立ち上げる。
    pub fn connect_session(&mut self, credentials: Credentials, me: Me) {
        self.abort_all_tasks();
        let api = ApiClient::new(self.http.clone(), &self.api_base_url, credentials.clone());
        self.credentials = Some(credentials);
        self.api = Some(api);
        self.me = Some(me);
        self.login_phase = None;

        self.subscribe(ShapeId::Channels);
        self.subscribe(ShapeId::Users);
        self.subscribe(ShapeId::ChannelUsers);
        self.poll_read_states();
        self.start_periodic_tasks();
    }

    fn start_periodic_tasks(&mut self) {
        // 既読状態の定期取得
        {
            let tx = self.event_tx.clone();
            let api = self.api.clone();
            if let Some(api) = api
                && let Some(handle) = self.spawn_tokio(async move {
                    let mut interval = tokio::time::interval(READ_STATE_POLL);
                    loop {
                        interval.tick().await;
                        if let Ok(states) = api.read_states().await
                            && tx
                                .send(DesktopEvent::App(AppEvent::ReadStates(states)))
                                .is_err()
                        {
                            break;
                        }
                    }
                })
            {
                self.background_tasks.push(handle);
            }
        }

        // 1秒ごとの Tick (通知消去・入力中解除・既読送信)
        {
            let tx = self.event_tx.clone();
            if let Some(handle) = self.spawn_tokio(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if tx.send(DesktopEvent::Tick).is_err() {
                        break;
                    }
                }
            }) {
                self.background_tasks.push(handle);
            }
        }

        // 新バージョン確認
        if self.config.check_updates {
            let http = self.http.clone();
            let tx = self.event_tx.clone();
            if let Some(handle) = self.spawn_tokio(async move {
                let cache = login::token_file()
                    .ok()
                    .and_then(|p| p.parent().map(update::cache_path));
                let current = env!("CARGO_PKG_VERSION");
                let user_agent = format!("{}/{current}", login::APP_NAME);
                if let Ok(Some(version)) = update::check_for(
                    &http,
                    update::RELEASES_URL,
                    cache.as_deref(),
                    current,
                    zoo2_client_core::auth::token::now(),
                    "desktop-v",
                    &user_agent,
                )
                .await
                {
                    let _ = tx.send(DesktopEvent::App(AppEvent::UpdateAvailable(version)));
                }
            }) {
                self.background_tasks.push(handle);
            }
        }
    }

    pub fn subscribe(&mut self, id: ShapeId) {
        let Some(credentials) = self.credentials.clone() else {
            return;
        };
        let stopped = match self.subscriptions.get(&id) {
            None => false,
            Some(handle) if handle.is_finished() => true,
            Some(_) => return,
        };
        if stopped {
            self.store.apply(
                &id,
                electric::Batch {
                    reset: true,
                    ..Default::default()
                },
            );
        }

        let http = self.http.clone();
        let base_url = self.api_base_url.clone();
        let tx = self.event_tx.clone();
        let shape_id = id.clone();

        let (shape_tx, mut shape_rx) = mpsc::unbounded_channel();
        if let Some(rt) = &self.runtime {
            let _guard = rt.enter();
            let stream_handle =
                electric::spawn(http, &base_url, credentials, shape_id, shape_tx).abort_handle();
            let forward_handle = rt
                .spawn(async move {
                    struct AbortOnDrop(AbortHandle);
                    impl Drop for AbortOnDrop {
                        fn drop(&mut self) {
                            self.0.abort();
                        }
                    }
                    let _abort_guard = AbortOnDrop(stream_handle);
                    while let Some(ev) = shape_rx.recv().await {
                        if tx.send(DesktopEvent::Shape(ev)).is_err() {
                            break;
                        }
                    }
                })
                .abort_handle();
            self.subscriptions.insert(id, forward_handle);
        } else if tokio::runtime::Handle::try_current().is_ok() {
            let stream_handle =
                electric::spawn(http, &base_url, credentials, shape_id, shape_tx).abort_handle();
            let forward_handle = tokio::spawn(async move {
                struct AbortOnDrop(AbortHandle);
                impl Drop for AbortOnDrop {
                    fn drop(&mut self) {
                        self.0.abort();
                    }
                }
                let _abort_guard = AbortOnDrop(stream_handle);
                while let Some(ev) = shape_rx.recv().await {
                    if tx.send(DesktopEvent::Shape(ev)).is_err() {
                        break;
                    }
                }
            })
            .abort_handle();
            self.subscriptions.insert(id, forward_handle);
        }
    }

    pub fn on_shape(&mut self, event: ShapeEvent) {
        match event {
            ShapeEvent::Batch(id, batch) => {
                self.store.apply(&id, batch);
                match &id {
                    ShapeId::Channels => {
                        if self.store.is_synced(&ShapeId::Channels)
                            && let Some(current) = self.current_channel.clone()
                            && self
                                .store
                                .channel(&current)
                                .is_none_or(|ch| ch.deleted_at.is_some())
                        {
                            self.evict_channel(&current);
                            self.notify("このチャンネルへのアクセス権がなくなりました");
                        }
                    }
                    ShapeId::Messages(ch) => {
                        if self.store.is_synced(&ShapeId::Messages(ch.clone())) {
                            self.reconcile_channel_targets(ch);
                        }
                        if self.current_channel.as_deref() == Some(ch.as_str())
                            && self.timeline_at_bottom
                        {
                            self.pending_scroll_to_bottom = true;
                            if self.window_active
                                && let Some(latest) = self.store.latest_root(ch)
                            {
                                self.newest_shown = Some((ch.clone(), latest.id.clone()));
                            }
                        }
                    }
                    _ => {}
                }
                self.sync_unread_with_latest();
            }
            ShapeEvent::Failed(id, message) => {
                if message == HttpError::Unauthorized.to_string() {
                    self.reset_to_login(Some(
                        "セッションの有効期限が切れました。再度ログインしてください".to_string(),
                    ));
                    return;
                }
                if let ShapeId::Messages(ch) | ShapeId::Reactions(ch) = &id
                    && is_access_or_not_found_error(&message)
                {
                    let ch = ch.clone();
                    self.evict_channel(&ch);
                    self.notify("このチャンネルへのアクセス権がなくなりました");
                    return;
                }
                self.notify(format!("同期が止まりました ({id:?}): {message}"));
            }
        }
    }

    /// 特定のチャンネルのメッセージ・リアクション購読を停止し、キャッシュされた行を破棄する。
    fn unsubscribe_channel(&mut self, channel_id: &str) {
        for shape_id in [
            ShapeId::Messages(channel_id.to_string()),
            ShapeId::Reactions(channel_id.to_string()),
        ] {
            if let Some(handle) = self.subscriptions.remove(&shape_id) {
                handle.abort();
            }
            self.store.apply(
                &shape_id,
                electric::Batch {
                    reset: true,
                    ..Default::default()
                },
            );
        }
    }

    /// チャンネルの閲覧権限を失った、またはチャンネルが削除されたときに状態を片付ける。
    fn evict_channel(&mut self, channel_id: &str) {
        self.unsubscribe_channel(channel_id);
        self.unread.remove(channel_id);
        self.last_read_marked.remove(channel_id);
        self.saved_drafts.retain(|(ch, _), _| ch != channel_id);
        if self
            .newest_shown
            .as_ref()
            .is_some_and(|(ch, _)| ch == channel_id)
        {
            self.newest_shown = None;
        }
        if self.current_channel.as_deref() == Some(channel_id) {
            self.stop_typing();
            self.typing.clear();
            self.current_channel = None;
            self.selected_message = None;
            self.thread = None;
            self.active_reaction_message = None;
            self.confirm_delete_message = None;
            self.composer_draft.clear();
            self.pending_composer_text = Some(String::new());
            self.target = Target::Channel;
            self.timeline_at_bottom = true;
            self.pending_scroll_to_bottom = false;
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
        }
    }

    /// メッセージ削除によりスレッド親や編集対象・削除確認対象が消えた場合に状態を整合させる。
    fn reconcile_channel_targets(&mut self, channel_id: &str) {
        if self.current_channel.as_deref() != Some(channel_id) {
            return;
        }
        if let Some(msg_id) = self.confirm_delete_message.clone()
            && self.store.message(channel_id, &msg_id).is_none()
        {
            self.confirm_delete_message = None;
        }
        if let Some((_, msg_id)) = self.active_reaction_message.clone()
            && self.store.message(channel_id, &msg_id).is_none()
        {
            self.active_reaction_message = None;
        }
        if let Some(thread) = self.thread.clone()
            && self.store.message(channel_id, &thread.root_id).is_none()
        {
            self.thread = None;
            if matches!(self.target, Target::Reply(_)) {
                self.target = Target::Channel;
                self.notify(
                    "返信元のメッセージが削除されたため、チャンネルへの投稿に切り替えました",
                );
            }
        }
        if let Target::Edit(msg_id) = self.target.clone()
            && self.store.message(channel_id, &msg_id).is_none()
        {
            self.target = match &self.thread {
                Some(thread) => Target::Reply(thread.root_id.clone()),
                None => Target::Channel,
            };
            self.notify("編集中のメッセージが削除されました。入力中の本文はそのまま残しています");
        }
    }

    pub fn on_app_event(&mut self, event: AppEvent) -> bool {
        match event {
            AppEvent::Notice(message) => {
                self.notify(message);
                true
            }
            AppEvent::Unauthorized => {
                self.reset_to_login(Some(
                    "認証が切れました。再度ログインしてください".to_string(),
                ));
                true
            }
            AppEvent::RestoreDraft {
                channel_id,
                target,
                content,
            } => {
                if self.current_channel.as_deref() == Some(channel_id.as_str())
                    && self.composer_draft.is_empty()
                {
                    self.composer_draft = content.clone();
                    self.pending_composer_text = Some(content);
                    self.target = target;
                } else {
                    if self.failed_drafts.len() >= FAILED_DRAFT_LIMIT {
                        self.failed_drafts.remove(0);
                    }
                    self.failed_drafts.push(FailedDraft {
                        channel_id,
                        target,
                        content,
                    });
                    self.notify(
                        "送信に失敗した本文を保存しました。「下書きを復元」で入力欄に戻せます",
                    );
                }
                true
            }
            AppEvent::ReadStates(states) => {
                let next_unread: HashSet<String> = states
                    .into_iter()
                    .filter(|s| s.has_unread)
                    .map(|s| s.channel_id)
                    .collect();
                let changed = self.unread != next_unread;
                self.unread = next_unread;
                self.sync_unread_with_latest();
                changed
            }
            AppEvent::ReadMarkFailed {
                channel_id,
                message_id,
            } => {
                if self.last_read_marked.get(&channel_id) == Some(&message_id) {
                    self.last_read_marked.remove(&channel_id);
                }
                false
            }
            AppEvent::Typing {
                channel_id,
                parent_id,
                users,
            } => {
                if self.current_channel.as_deref() == Some(channel_id.as_str()) {
                    let prev_names: Vec<&str> = self
                        .typing
                        .get(&parent_id)
                        .map(|u| u.iter().map(|x| x.display_name.as_str()).collect())
                        .unwrap_or_default();
                    let next_names: Vec<&str> =
                        users.iter().map(|x| x.display_name.as_str()).collect();
                    let changed = prev_names != next_names;
                    self.typing.insert(parent_id, users);
                    changed
                } else {
                    false
                }
            }
            AppEvent::TypingPollFinished => {
                self.typing_poll_in_flight = false;
                false
            }
            AppEvent::UpdateAvailable(version) => {
                let changed = self.update_available.as_deref() != Some(version.as_str());
                self.update_available = Some(version);
                changed
            }
        }
    }

    pub fn on_tick(&mut self) -> bool {
        let mut changed = false;
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= NOTICE_TIMEOUT)
        {
            self.notice = None;
            changed = true;
        }
        if self
            .last_key_typed
            .is_some_and(|at| at.elapsed() >= TYPING_IDLE_TIMEOUT)
        {
            self.stop_typing();
        }
        if self.mark_current_read() {
            changed = true;
        }
        self.poll_typing();
        changed
    }

    /// 開いているチャンネルの最新メッセージを画面に表示済みであれば既読にする。
    fn sync_unread_with_latest(&mut self) -> bool {
        let Some(channel_id) = self.current_channel.clone() else {
            return false;
        };
        if self.is_latest_shown(&channel_id) {
            self.mark_current_read()
        } else {
            false
        }
    }

    fn is_latest_shown(&self, channel_id: &str) -> bool {
        let Some(latest) = self.store.latest_root(channel_id) else {
            return false;
        };
        self.newest_shown
            .as_ref()
            .is_some_and(|(ch, id)| ch == channel_id && id == &latest.id)
    }

    pub fn mark_current_read(&mut self) -> bool {
        if !self.window_active {
            return false;
        }
        let Some(me) = self.me.as_ref() else {
            return false;
        };
        let Some(channel_id) = self.current_channel.clone() else {
            return false;
        };
        if !self.store.is_synced(&ShapeId::Messages(channel_id.clone()))
            || !self.store.is_member(&channel_id, &me.id)
            || !self.is_latest_shown(&channel_id)
        {
            return false;
        }
        let Some(latest) = self.store.latest_root(&channel_id).map(|m| m.id.clone()) else {
            return false;
        };
        let unread_removed = self.unread.remove(&channel_id);
        if self.last_read_marked.get(&channel_id) == Some(&latest) {
            return unread_removed;
        }
        self.last_read_marked
            .insert(channel_id.clone(), latest.clone());
        if let Some(api) = self.api.clone() {
            let tx = self.event_tx.clone();
            self.spawn_tokio(async move {
                if let Err(err) = api.mark_read(&channel_id, &latest).await {
                    if matches!(
                        err.downcast_ref::<HttpError>(),
                        Some(HttpError::Unauthorized)
                    ) {
                        let _ = tx.send(DesktopEvent::App(AppEvent::Unauthorized));
                    } else {
                        let _ = tx.send(DesktopEvent::App(AppEvent::ReadMarkFailed {
                            channel_id,
                            message_id: latest,
                        }));
                    }
                }
            });
        }
        unread_removed
    }

    pub fn poll_read_states(&self) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let tx = self.event_tx.clone();
        self.spawn_tokio(async move {
            if let Ok(states) = api.read_states().await {
                let _ = tx.send(DesktopEvent::App(AppEvent::ReadStates(states)));
            }
        });
    }

    pub fn poll_typing(&mut self) {
        if self.typing_poll_in_flight {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        let Some(me) = self.me.as_ref() else {
            return;
        };
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if !self.store.is_member(&channel_id, &me.id) {
            return;
        }
        let mut scopes = vec![None];
        if let Some(thread) = &self.thread {
            scopes.push(Some(thread.root_id.clone()));
        }
        let tx = self.event_tx.clone();
        if self
            .spawn_tokio(async move {
                for parent_id in scopes {
                    if let Ok(users) = api.typing(&channel_id, parent_id.as_deref()).await {
                        let _ = tx.send(DesktopEvent::App(AppEvent::Typing {
                            channel_id: channel_id.clone(),
                            parent_id,
                            users,
                        }));
                    }
                }
                let _ = tx.send(DesktopEvent::App(AppEvent::TypingPollFinished));
            })
            .is_some()
        {
            self.typing_poll_in_flight = true;
        }
    }

    pub fn on_composer_changed(&mut self, text: String) {
        let non_empty = !text.trim().is_empty();
        self.composer_draft = text;
        if non_empty {
            self.touch_typing();
        } else {
            self.stop_typing();
        }
    }

    fn touch_typing(&mut self) {
        if matches!(self.target, Target::Edit(_)) {
            return;
        }
        let Some(me) = self.me.as_ref() else {
            return;
        };
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if !self.store.is_member(&channel_id, &me.id) {
            return;
        }
        let parent_id = match &self.target {
            Target::Reply(root_id) => Some(root_id.clone()),
            _ => None,
        };
        let now = Instant::now();
        self.last_key_typed = Some(now);
        if let Some((sent_ch, sent_parent, at)) = &self.typing_sent
            && sent_ch == &channel_id
            && sent_parent == &parent_id
            && now.duration_since(*at) < TYPING_SEND_INTERVAL
        {
            return;
        }
        self.typing_sent = Some((channel_id.clone(), parent_id.clone(), now));
        self.send_typing(channel_id, parent_id, true);
    }

    pub fn stop_typing(&mut self) {
        self.last_key_typed = None;
        if let Some((channel_id, parent_id, _)) = self.typing_sent.take() {
            self.send_typing(channel_id, parent_id, false);
        }
    }

    fn send_typing(&self, channel_id: String, parent_id: Option<String>, active: bool) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let session_id = self.session_id.clone();
        self.spawn_tokio(async move {
            let _ = api
                .set_typing(&channel_id, parent_id.as_deref(), &session_id, active)
                .await;
        });
    }

    /// API 呼び出しをバックグラウンドで実行し、エラー時は通知または再ログインへ遷移する。
    fn spawn_api<F, T>(&self, label: &'static str, future: F)
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let tx = self.event_tx.clone();
        self.spawn_tokio(async move {
            if let Err(err) = future.await {
                if matches!(
                    err.downcast_ref::<HttpError>(),
                    Some(HttpError::Unauthorized)
                ) {
                    let _ = tx.send(DesktopEvent::App(AppEvent::Unauthorized));
                } else {
                    let _ = tx.send(DesktopEvent::App(AppEvent::Notice(format!(
                        "{label}に失敗しました: {err}"
                    ))));
                }
            }
        });
    }

    /// 検索クエリで絞り込んだチャンネル一覧。
    pub fn filtered_channels(&self) -> Vec<&Channel> {
        let query = self.search_query.trim().to_lowercase();
        let me_id = self.me.as_ref().map(|m| m.id.as_str()).unwrap_or_default();
        self.store
            .channels()
            .into_iter()
            .filter(|ch| {
                if query.is_empty() {
                    return true;
                }
                self.store
                    .channel_label(ch, me_id)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }

    /// 現在の `(チャンネル ID, 送信先)` の下書きを退避し、新しい送信先の下書きを入力欄へ反映する。
    fn switch_composer_context(&mut self, next_channel: Option<&str>, next_target: Target) {
        if let Some(ch) = self.current_channel.clone()
            && !matches!(self.target, Target::Edit(_))
        {
            if self.composer_draft.trim().is_empty() {
                self.saved_drafts.remove(&(ch, self.target.clone()));
            } else {
                self.saved_drafts
                    .insert((ch, self.target.clone()), self.composer_draft.clone());
            }
        }
        self.target = next_target.clone();
        let next_draft = match next_channel {
            Some(ch) if !matches!(next_target, Target::Edit(_)) => self
                .saved_drafts
                .remove(&(ch.to_string(), next_target))
                .unwrap_or_default(),
            _ => String::new(),
        };
        if self.composer_draft != next_draft || self.pending_composer_text.is_some() {
            self.composer_draft = next_draft.clone();
            self.pending_composer_text = Some(next_draft);
        }
    }

    /// チャンネルを開き、メッセージとリアクションのシェイプ購読を開始する。
    pub fn open_channel(&mut self, channel_id: String) {
        if let Some(old_ch) = self.current_channel.clone()
            && old_ch != channel_id
        {
            self.stop_typing();
            self.typing.clear();
            self.selected_message = None;
            self.thread = None;
            self.active_reaction_message = None;
            self.confirm_delete_message = None;
            self.switch_composer_context(Some(&channel_id), Target::Channel);
            // 別のチャンネルへ切り替えるときは前のチャンネルの Messages / Reactions 購読とキャッシュを破棄し、
            // バックグラウンドタスクとメモリがチャンネル数に応じて増え続けるのを防ぐ。
            self.unsubscribe_channel(&old_ch);
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
        } else if self.current_channel.is_none() {
            self.switch_composer_context(Some(&channel_id), Target::Channel);
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
        }
        self.subscribe(ShapeId::Messages(channel_id.clone()));
        self.subscribe(ShapeId::Reactions(channel_id.clone()));
        self.current_channel = Some(channel_id.clone());
        self.timeline_at_bottom = true;
        self.pending_scroll_to_bottom = true;

        // ウィンドウがアクティブであれば最新メッセージを表示済みとして記録し、既読化を試みる。
        if self.window_active
            && let Some(latest) = self.store.latest_root(&channel_id)
        {
            self.newest_shown = Some((channel_id, latest.id.clone()));
        } else {
            self.newest_shown = None;
        }
        self.sync_unread_with_latest();
        self.poll_typing();
    }

    /// ウィンドウのアクティブ状態とタイムラインのスクロール位置を更新し、
    /// アクティブかつ最下部を表示しているときだけ最新メッセージを既読化する。
    pub fn update_timeline_view_state(&mut self, window_active: bool, at_bottom: bool) {
        self.window_active = window_active;
        self.timeline_at_bottom = at_bottom;
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if at_bottom && window_active {
            if let Some(latest) = self.store.latest_root(&channel_id) {
                self.newest_shown = Some((channel_id, latest.id.clone()));
            }
            self.sync_unread_with_latest();
        } else if !at_bottom
            && self
                .newest_shown
                .as_ref()
                .is_some_and(|(ch, _)| ch == &channel_id)
        {
            self.newest_shown = None;
        }
    }

    /// タイムラインのスクロール位置が最下部にあるかどうかを更新する。
    pub fn update_timeline_scroll_state(&mut self, at_bottom: bool) {
        self.update_timeline_view_state(self.window_active, at_bottom);
    }

    /// タイムラインのさらに古いメッセージを `TIMELINE_LOAD_STEP` 件分追加で表示する。
    pub fn load_older_messages(&mut self) {
        self.timeline_render_limit = self
            .timeline_render_limit
            .saturating_add(TIMELINE_LOAD_STEP);
    }

    /// チャンネルへの参加・退出を実行する。
    pub fn join_or_leave(&mut self, channel_id: String, join: bool) {
        let Some(api) = self.api.clone() else {
            return;
        };
        if join {
            self.spawn_api("参加", async move { api.join_channel(&channel_id).await });
        } else {
            self.spawn_api("退出", async move { api.leave_channel(&channel_id).await });
        }
    }

    /// スレッドペインを開き、編集中でなければ送信先をスレッド返信に切り替える。
    pub fn open_thread(&mut self, root_id: String) {
        self.thread = Some(ThreadState {
            root_id: root_id.clone(),
            selected: None,
        });
        if !matches!(self.target, Target::Edit(_)) {
            let current = self.current_channel.clone();
            self.switch_composer_context(current.as_deref(), Target::Reply(root_id));
        }
        self.poll_typing();
    }

    /// スレッドペインを閉じ、送信先がスレッド返信ならチャンネルに戻す。
    pub fn close_thread(&mut self) {
        self.thread = None;
        if matches!(self.target, Target::Reply(_)) {
            let current = self.current_channel.clone();
            self.switch_composer_context(current.as_deref(), Target::Channel);
        }
    }

    /// 送信先を指定したスレッドへの返信に切り替える（現在の下書きは退避し、対象スレッドの下書きを復元する）。
    pub fn focus_thread_reply(&mut self, root_id: String) {
        let current = self.current_channel.clone();
        self.switch_composer_context(current.as_deref(), Target::Reply(root_id));
    }

    /// 送信先を編集または返信からキャンセルして通常投稿へ戻す。
    pub fn cancel_target(&mut self) {
        let next_target = match &self.thread {
            Some(thread) => Target::Reply(thread.root_id.clone()),
            None => Target::Channel,
        };
        let current = self.current_channel.clone();
        self.switch_composer_context(current.as_deref(), next_target);
    }

    /// 自分のメッセージの編集を開始する。
    pub fn start_edit(&mut self, message_id: String) {
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        let Some(me) = self.me.as_ref() else {
            return;
        };
        let Some(message) = self.store.message(&channel_id, &message_id) else {
            return;
        };
        if message.user_id != me.id {
            self.notify("自分のメッセージだけを編集できます");
            return;
        }
        if !matches!(self.target, Target::Edit(_)) && !self.composer_draft.trim().is_empty() {
            self.notify("書きかけの下書きがあります。送信するか消してから編集してください");
            return;
        }
        let content = message.content.clone();
        self.composer_draft = content.clone();
        self.pending_composer_text = Some(content);
        self.target = Target::Edit(message_id);
    }

    /// 自分のメッセージの削除確認を開始する。
    pub fn request_delete(&mut self, message_id: String) {
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        let Some(me) = self.me.as_ref() else {
            return;
        };
        if self
            .store
            .message(&channel_id, &message_id)
            .is_some_and(|m| m.user_id == me.id)
        {
            self.confirm_delete_message = Some(message_id);
        } else {
            self.notify("自分のメッセージだけを削除できます");
        }
    }

    /// 削除確認で「削除する」が押されたときに削除 API を呼ぶ。
    pub fn confirm_delete(&mut self) {
        let Some(message_id) = self.confirm_delete_message.take() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        self.spawn_api("削除", async move { api.delete_message(&message_id).await });
    }

    pub fn cancel_delete(&mut self) {
        self.confirm_delete_message = None;
    }

    /// 保存した送信失敗の下書きを新しい順に1件入力欄へ戻す。
    pub fn restore_failed_draft(&mut self) {
        if self.failed_drafts.is_empty() {
            self.notify("送れなかった本文はありません");
            return;
        }
        if !self.composer_draft.trim().is_empty() {
            self.notify("書きかけの下書きがあります。送信するか消してから戻してください");
            return;
        }
        let index = self
            .failed_drafts
            .iter()
            .rposition(|draft| self.current_channel.as_deref() == Some(draft.channel_id.as_str()))
            .unwrap_or(self.failed_drafts.len() - 1);
        let draft = self.failed_drafts.remove(index);
        if self.current_channel.as_deref() != Some(draft.channel_id.as_str()) {
            self.open_channel(draft.channel_id);
        }
        self.composer_draft = draft.content.clone();
        self.pending_composer_text = Some(draft.content);
        self.target = draft.target;
    }

    /// 入力欄のメッセージを送信または編集する。送信を開始した場合は `true` を返す。
    pub fn submit_message(&mut self, content: String) -> bool {
        if content.trim().is_empty() {
            return false;
        }
        let Some(channel_id) = self.current_channel.clone() else {
            self.notify("先にチャンネルを開いてください");
            return false;
        };
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルには投稿できません");
            return false;
        }
        let Some(api) = self.api.clone() else {
            return false;
        };
        let tx = self.event_tx.clone();
        let target = self.target.clone();
        self.composer_draft.clear();
        self.saved_drafts
            .remove(&(channel_id.clone(), target.clone()));
        self.stop_typing();

        if matches!(target, Target::Edit(_)) {
            let next_target = match &self.thread {
                Some(thread) => Target::Reply(thread.root_id.clone()),
                None => Target::Channel,
            };
            self.switch_composer_context(Some(&channel_id), next_target);
        }

        self.spawn_tokio(async move {
            let id = uuid::Uuid::new_v4().to_string();
            let result = match &target {
                Target::Channel => api.send_message(&id, &channel_id, &content, None).await,
                Target::Reply(root_id) => {
                    api.send_message(&id, &channel_id, &content, Some(root_id))
                        .await
                }
                Target::Edit(message_id) => api.edit_message(message_id, &content).await,
            };
            if let Err(err) = result {
                if matches!(
                    err.downcast_ref::<HttpError>(),
                    Some(HttpError::Unauthorized)
                ) {
                    let _ = tx.send(DesktopEvent::App(AppEvent::Unauthorized));
                } else {
                    let _ = tx.send(DesktopEvent::App(AppEvent::Notice(format!(
                        "送信に失敗しました: {err}"
                    ))));
                }
                let _ = tx.send(DesktopEvent::App(AppEvent::RestoreDraft {
                    channel_id,
                    target,
                    content,
                }));
            }
        });
        true
    }

    /// 指定メッセージに対する絵文字リアクションをトグルする。
    pub fn toggle_reaction(&mut self, message_id: &str, emoji: &str) {
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        let Some(me) = self.me.as_ref() else {
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let mine = self
            .store
            .reaction_groups(&channel_id, message_id, &me.id)
            .iter()
            .any(|g| g.emoji == emoji && g.mine);
        let message_id = message_id.to_string();
        let emoji = emoji.to_string();
        if mine {
            self.spawn_api("リアクション解除", async move {
                api.remove_reaction(&message_id, &emoji).await
            });
        } else {
            let reaction_id = uuid::Uuid::new_v4().to_string();
            self.spawn_api("リアクション追加", async move {
                api.add_reaction(&reaction_id, &message_id, &emoji).await
            });
        }
    }

    /// ショートコードまたは絵文字入力を受け取ってリアクションをトグルする。
    pub fn submit_reaction_input(&mut self, message_id: &str, input: &str) -> bool {
        let Some(emoji) = resolve_emoji(input) else {
            self.notify(format!("絵文字が見つかりません: {}", input.trim()));
            return false;
        };
        self.active_reaction_message = None;
        self.toggle_reaction(message_id, &emoji);
        true
    }

    /// 現在のターゲットに応じた入力中表示テキスト。
    pub fn typing_text(&self, scope: &Option<String>) -> Option<String> {
        let users = self.typing.get(scope).filter(|u| !u.is_empty())?;
        let names: Vec<&str> = users.iter().map(|u| u.display_name.as_str()).collect();
        Some(match names.as_slice() {
            [one] => format!("{one} が入力中…"),
            [a, b] => format!("{a}、{b} が入力中…"),
            _ => format!("{} 人が入力中…", names.len()),
        })
    }
}

/// `thumbsup` や `:thumbsup:` のショートコード、または絵文字そのものを絵文字に変換する。
pub fn resolve_emoji(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    let name = input.trim_matches(':');
    emojis::get_by_shortcode(name)
        .or_else(|| emojis::get(input))
        .map(|emoji| emoji.as_str().to_string())
}

/// Emoji Kitchen の合成絵文字 (`kitchen:<a>_<b>`) は元の2つを `+` で並べて示す。
pub fn reaction_label(emoji: &str) -> String {
    match emoji.strip_prefix("kitchen:") {
        Some(pair) => pair.replacen('_', "+", 1),
        None => emoji.to_string(),
    }
}

/// `ShapeEvent::Failed` のメッセージが HTTP 403 / 404 (閲覧権限喪失または削除済みチャンネル) を示すか判定する。
fn is_access_or_not_found_error(message: &str) -> bool {
    let forbidden = reqwest::StatusCode::FORBIDDEN.to_string();
    let not_found = reqwest::StatusCode::NOT_FOUND.to_string();
    message.starts_with(&forbidden) || message.starts_with(&not_found)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use zoo2_client_core::{
        config::OidcConfig,
        electric::{Batch, Change},
    };

    use super::*;

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

    fn connected_state() -> AppState {
        let config = Config {
            api_base_url: "http://127.0.0.1:9".into(),
            auth: AuthConfig::Dev {
                user_id: "u1".into(),
            },
            check_updates: false,
        };
        let mut state = AppState::from_config(&config);
        state.connect_session(
            Credentials::Dev("u1".into()),
            Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
        );
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![
                (
                    "c1",
                    json!({ "id": "c1", "channel_name": "general", "is_private": "f", "description": "雑談" }),
                ),
                (
                    "c2",
                    json!({ "id": "c2", "channel_name": "secret", "is_private": "t" }),
                ),
            ]),
        ));
        state.on_shape(ShapeEvent::Batch(
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
            ]
            .into_iter()
            .collect::<Vec<_>>()
            .pipe(batch),
        ));
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![("m1", json!({ "user_id": "u1", "channel_id": "c1" }))]),
        ));
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "m1",
                    json!({ "id": "m1", "user_id": "u2", "channel_id": "c1",
                            "content": "こんにちは @alice", "created_at": "2026-11-01 03:00:00" }),
                ),
                (
                    "r1",
                    json!({ "id": "r1", "user_id": "u1", "channel_id": "c1",
                            "content": "スレッド返信", "created_at": "2026-11-01 03:01:00", "parent_id": "m1" }),
                ),
            ]),
        ));
        state
    }

    trait Pipe: Sized {
        fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
            f(self)
        }
    }
    impl<T> Pipe for T {}

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

    #[tokio::test]
    async fn transitions_login_phases_and_connects() {
        let mut state = connected_state();
        assert!(state.login_phase.is_none());
        assert_eq!(state.me.as_ref().unwrap().user_name, "alice");

        state.on_auth_event(AuthEvent::DevicePrompt {
            verification_uri: "https://example.com/device".into(),
            user_code: "ABCD-1234".into(),
        });
        assert_eq!(
            state.login_phase,
            Some(LoginPhase::AwaitingApproval {
                verification_uri: "https://example.com/device".into(),
                user_code: "ABCD-1234".into(),
            })
        );

        state.composer_draft = "残っていた下書き".into();
        state.search_query = "general".into();
        state.failed_drafts.push(FailedDraft {
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "失敗分".into(),
        });

        state.on_shape(ShapeEvent::Failed(
            ShapeId::Channels,
            HttpError::Unauthorized.to_string(),
        ));
        assert!(matches!(
            state.login_phase,
            Some(LoginPhase::Ready { error: Some(_) })
        ));
        assert!(state.composer_draft.is_empty());
        assert_eq!(state.pending_composer_text.as_deref(), Some(""));
        assert!(state.search_query.is_empty());
        assert_eq!(state.pending_search_text.as_deref(), Some(""));
        assert!(state.failed_drafts.is_empty());
    }

    #[tokio::test]
    async fn opens_channel_thread_and_filters_sidebar() {
        let mut state = connected_state();
        assert_eq!(state.filtered_channels().len(), 2);
        state.search_query = "sec".into();
        assert_eq!(state.filtered_channels().len(), 1);
        assert_eq!(state.filtered_channels()[0].id, "c2");
        state.search_query.clear();

        state.open_channel("c1".into());
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert_eq!(state.target, Target::Channel);

        state.open_thread("m1".into());
        assert!(state.thread.is_some());
        assert_eq!(state.target, Target::Reply("m1".into()));

        state.close_thread();
        assert!(state.thread.is_none());
        assert_eq!(state.target, Target::Channel);
    }

    #[tokio::test]
    async fn guards_edit_and_delete_to_own_messages_and_restores_failed_drafts() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 他人のメッセージ m1 は編集・削除できない
        state.start_edit("m1".into());
        assert_eq!(state.target, Target::Channel);
        assert!(
            state
                .notice
                .as_ref()
                .unwrap()
                .0
                .contains("自分のメッセージだけを編集できます")
        );

        state.request_delete("m1".into());
        assert!(state.confirm_delete_message.is_none());

        // 自分のメッセージ r1 は編集・削除確認できる
        state.start_edit("r1".into());
        assert_eq!(state.target, Target::Edit("r1".into()));
        assert_eq!(state.pending_composer_text.as_deref(), Some("スレッド返信"));

        state.cancel_target();
        assert_eq!(state.target, Target::Channel);
        assert!(state.composer_draft.is_empty());

        state.request_delete("r1".into());
        assert_eq!(state.confirm_delete_message.as_deref(), Some("r1"));
        state.cancel_delete();
        assert!(state.confirm_delete_message.is_none());

        // 失敗した下書きの保存と復元
        state.on_app_event(AppEvent::RestoreDraft {
            channel_id: "c2".into(),
            target: Target::Channel,
            content: "保存された本文".into(),
        });
        assert_eq!(state.failed_drafts.len(), 1);
        state.restore_failed_draft();
        assert_eq!(state.current_channel.as_deref(), Some("c2"));
        assert_eq!(state.composer_draft, "保存された本文");
        assert!(state.failed_drafts.is_empty());
    }

    #[tokio::test]
    async fn marks_channel_read_when_latest_message_is_shown() {
        let mut state = connected_state();
        state.on_app_event(AppEvent::ReadStates(vec![
            ReadState {
                channel_id: "c1".into(),
                has_unread: true,
            },
            ReadState {
                channel_id: "c2".into(),
                has_unread: true,
            },
        ]));
        assert!(state.unread.contains("c1"));
        assert!(state.unread.contains("c2"));

        // (a) すでに同期済みの c1 を開くと、最下部へ追従して m1 が既読になる
        state.open_channel("c1".into());
        assert!(!state.unread.contains("c1"));
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m1")
        );

        // (b) 最下部を表示中に新着メッセージ m2 の Batch が届くと、newest_shown が更新され m2 も既読になる
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m2",
                json!({ "id": "m2", "user_id": "u2", "channel_id": "c1",
                        "content": "新着メッセージ", "created_at": "2026-11-01 03:02:00" }),
            )]),
        ));
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m2")
        );

        // (c) 上へスクロールして最下部から離れているときは、新着メッセージ m3 が来ても自動では既読にならない
        state.update_timeline_scroll_state(false);
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m3",
                json!({ "id": "m3", "user_id": "u2", "channel_id": "c1",
                        "content": "スクロール中の新着", "created_at": "2026-11-01 03:03:00" }),
            )]),
        ));
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m2")
        );
        // 再び最下部へスクロールすると m3 が既読になる
        state.update_timeline_scroll_state(true);
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m3")
        );

        // (d) ウィンドウが非アクティブのときは、最下部を表示中でも新着 m4 は自動既読にならない
        state.update_timeline_view_state(false, true);
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m4",
                json!({ "id": "m4", "user_id": "u2", "channel_id": "c1",
                        "content": "背面ウィンドウ中の新着", "created_at": "2026-11-01 03:04:00" }),
            )]),
        ));
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m3")
        );
        // ウィンドウが再びアクティブになると m4 が既読になる
        state.update_timeline_view_state(true, true);
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m4")
        );

        // (e) mark_read API が一時失敗したときは last_read_marked が巻き戻り、次の判定で再送できる
        state.on_app_event(AppEvent::ReadMarkFailed {
            channel_id: "c1".into(),
            message_id: "m4".into(),
        });
        assert!(!state.last_read_marked.contains_key("c1"));
        state.mark_current_read();
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m4")
        );

        // (f) メッセージ同期前にチャンネル c2 を開き、あとから初回 Batch が届いたときも既読になる
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![("m2", json!({ "user_id": "u1", "channel_id": "c2" }))]),
        ));
        state.open_channel("c2".into());
        assert!(state.unread.contains("c2"));
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c2".into()),
            batch(vec![(
                "c2m1",
                json!({ "id": "c2m1", "user_id": "u2", "channel_id": "c2",
                        "content": "初回同期メッセージ", "created_at": "2026-11-01 03:05:00" }),
            )]),
        ));
        assert!(!state.unread.contains("c2"));
        assert_eq!(
            state.last_read_marked.get("c2").map(String::as_str),
            Some("c2m1")
        );
    }

    #[tokio::test]
    async fn reconciles_targets_and_evicts_channel_on_deletion_or_access_loss() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        assert!(
            state
                .subscriptions
                .contains_key(&ShapeId::Messages("c1".into()))
        );

        // 409 / must-refetch による再同期途中の reset バッチ (up_to_date: false) では
        // チャンネルが誤って閉じられないことを確認する
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            Batch {
                reset: true,
                changes: vec![],
                up_to_date: false,
            },
        ));
        assert_eq!(state.current_channel.as_deref(), Some("c1"));

        // 再同期が完了 (up_to_date: true) して c1 と c2 が戻ってくる
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![
                (
                    "c1",
                    json!({ "id": "c1", "channel_name": "general", "is_private": "f" }),
                ),
                (
                    "c2",
                    json!({ "id": "c2", "channel_name": "secret", "is_private": "t" }),
                ),
            ]),
        ));
        assert_eq!(state.current_channel.as_deref(), Some("c1"));

        // スレッドを開いて自分のメッセージ r1 を編集中に r1 が削除された場合、下書きを残したまま返信に戻る
        state.open_thread("m1".into());
        state.start_edit("r1".into());
        state.on_composer_changed("編集途中のテキスト".into());
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "r1".into() }],
                up_to_date: true,
            },
        ));
        assert_eq!(state.target, Target::Reply("m1".into()));
        assert_eq!(state.composer_draft, "編集途中のテキスト");

        // スレッドのルート m1 が削除された場合、スレッドを閉じて通常投稿に切り替える
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));
        assert!(state.thread.is_none());
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "編集途中のテキスト");

        // 別のチャンネル c2 に切り替えると c1 の購読は停止・破棄され、c1 の下書きは退避される
        state.open_channel("c2".into());
        assert!(
            !state
                .subscriptions
                .contains_key(&ShapeId::Messages("c1".into()))
        );
        assert!(
            state
                .subscriptions
                .contains_key(&ShapeId::Messages("c2".into()))
        );
        assert!(state.composer_draft.is_empty());

        // c2 からアクセス権を失って HttpError::Status (404 Not Found / 403 Forbidden) になった場合、
        // c2 の表示とキャッシュを閉じてクリアする
        state.on_shape(ShapeEvent::Failed(
            ShapeId::Messages("c2".into()),
            HttpError::Status {
                status: reqwest::StatusCode::NOT_FOUND,
                message: "Channel not found".into(),
            }
            .to_string(),
        ));
        assert!(state.current_channel.is_none());
        assert!(
            !state
                .subscriptions
                .contains_key(&ShapeId::Messages("c2".into()))
        );

        // c1 を再度開くと退避されていた下書きが復元され、論理削除 (deleted_at) された場合は閉じられる
        state.open_channel("c1".into());
        assert_eq!(state.composer_draft, "編集途中のテキスト");
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "f", "deleted_at": "2026-11-01 04:00:00" }),
            )]),
        ));
        assert!(state.current_channel.is_none());
    }

    #[tokio::test]
    async fn isolates_drafts_across_targets_and_channels() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.on_composer_changed("チャンネル本文の下書き".into());

        // スレッドを開くとチャンネル用の下書きが退避され、スレッド返信の入力欄は空から始まる
        state.open_thread("m1".into());
        assert_eq!(state.target, Target::Reply("m1".into()));
        assert!(state.composer_draft.is_empty());

        // スレッド返信の下書きを入力してからスレッドを閉じると、チャンネル用の下書きが戻る
        state.on_composer_changed("スレッド返信の下書き".into());
        state.close_thread();
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "チャンネル本文の下書き");

        // もう一度同じスレッドを開くと、スレッド返信の下書きが復元される
        state.open_thread("m1".into());
        assert_eq!(state.composer_draft, "スレッド返信の下書き");
    }

    #[test]
    fn resolves_emoji_shortcodes_and_kitchen_labels() {
        assert_eq!(resolve_emoji(":thumbsup:").as_deref(), Some("👍"));
        assert_eq!(resolve_emoji("tada").as_deref(), Some("🎉"));
        assert_eq!(resolve_emoji("❤️").as_deref(), Some("❤️"));
        assert_eq!(resolve_emoji("invalid_shortcode_xyz"), None);
        assert_eq!(reaction_label("kitchen:🐱_🐢"), "🐱+🐢");
        assert_eq!(reaction_label("👍"), "👍");
    }
}
