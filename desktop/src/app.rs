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
    task::{AbortHandle, JoinHandle},
};
use zoo2_client_core::{
    api::{ApiClient, Me, ReadState, TypingUser},
    config::{AuthConfig, Config},
    electric::{
        self, Change, ShapeEvent, ShapeId,
        rows::{Channel, Message},
    },
    http::{Credentials, HttpError},
    store::{ReactionGroup, Store},
    update,
};

use crate::login::{self, AuthEvent, LoginPhase};

const READ_STATE_POLL: Duration = Duration::from_secs(10);
const READ_MARK_BACKOFF: Duration = Duration::from_secs(5);
const TYPING_SEND_INTERVAL: Duration = Duration::from_secs(2);
const TYPING_IDLE_TIMEOUT: Duration = Duration::from_secs(3);
const NOTICE_TIMEOUT: Duration = Duration::from_secs(5);
const FAILED_DRAFT_LIMIT: usize = 20;
const INITIAL_TIMELINE_LIMIT: usize = 100;
const TIMELINE_LOAD_STEP: usize = 100;
const MAX_TIMELINE_RENDER_LIMIT: usize = 500;

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

/// 現在開いているチャンネルの描画用キャッシュ。
/// `ShapeId::Messages` / `ShapeId::Reactions` の更新時またはチャンネル・スレッド切替時のみ再計算する。
#[derive(Debug, Default, Clone)]
pub struct ChannelViewCache {
    pub channel_id: Option<String>,
    pub me_id: String,
    pub root_messages: Arc<Vec<Message>>,
    pub reply_counts: Arc<HashMap<String, usize>>,
    pub reactions_by_message: Arc<HashMap<String, Vec<ReactionGroup>>>,
    pub thread_root_id: Option<String>,
    pub thread_root_message: Option<Message>,
    pub thread_replies: Arc<Vec<Message>>,
    pub messages_dirty: bool,
    pub reactions_dirty: bool,
}

/// バックグラウンドタスクから `AppState` へ届くイベント。
#[derive(Debug)]
pub enum AppEvent {
    Notice(String),
    Unauthorized,
    SubmissionSucceeded {
        submission_id: u64,
    },
    RestoreDraft {
        submission_id: u64,
        channel_id: String,
        target: Target,
        content: String,
    },
    ReadStates(Vec<ReadState>),
    ReadMarkSucceeded {
        channel_id: String,
        message_id: String,
    },
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
    Auth { session_gen: u64, event: AuthEvent },
    Shape { session_gen: u64, event: ShapeEvent },
    App { session_gen: u64, event: AppEvent },
    Tick { session_gen: u64 },
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
    pub login_task: Option<JoinHandle<()>>,

    pub credentials: Option<Credentials>,
    pub token_manager: Option<Arc<login::DesktopTokenManager>>,
    pub api: Option<ApiClient>,
    pub me: Option<Me>,
    pub session_id: String,
    /// セッション世代。ログアウトや再ログイン時にインクリメントし、古いタスクから届いたイベントを破棄する。
    pub session_gen: u64,

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
    /// 現在送信リクエスト実行中の下書き (`(submission_id, FailedDraft)`)。
    pub in_flight_submissions: Vec<(u64, FailedDraft)>,
    pub next_submission_id: u64,
    /// 認証切れ (`Unauthorized`) でログイン画面に戻った際に、同じユーザーの再ログイン時だけ復元する下書き。
    pub preserved_unauthorized_drafts: Option<(String, Vec<FailedDraft>)>,

    /// `Channels` シェイプのキーごとの `(チャンネル ID, 作成者ユーザー ID)`。
    pub channel_creators_by_key: HashMap<String, (String, Option<String>)>,
    /// 開いているチャンネルの描画用キャッシュ。
    pub channel_cache: ChannelViewCache,

    pub unread: HashSet<String>,
    pub last_read_marked: HashMap<String, String>,
    pub read_mark_in_flight: HashMap<String, String>,
    pub read_mark_backoff_until: HashMap<String, (String, Instant)>,
    pub newest_shown: Option<(String, String)>,
    /// ウィンドウが前面（アクティブ）かどうか。非アクティブの間は自動既読にしない。
    pub window_active: bool,
    /// タイムラインが最下部を表示している（新着メッセージに追従する）かどうか。
    pub timeline_at_bottom: bool,
    /// 次の描画でタイムラインのスクロール位置を最下部へ移動させるフラグ。
    pub pending_scroll_to_bottom: bool,
    /// タイムラインで一度に描画する最大ルートメッセージ数。
    pub timeline_render_limit: usize,
    /// 上へスクロールしている間の描画範囲の先頭メッセージ ID アンカー。
    pub timeline_anchor_first_id: Option<String>,
    /// スレッドペインが最下部を表示しているかどうか。
    pub thread_at_bottom: bool,
    /// 次の描画でスレッドペインのスクロール位置を最下部へ移動させるフラグ。
    pub pending_thread_scroll_to_bottom: bool,

    pub typing: HashMap<Option<String>, Vec<TypingUser>>,
    pub typing_sent: Option<(String, Option<String>, Instant)>,
    pub last_key_typed: Option<Instant>,
    pub typing_poll_in_flight: bool,

    pub notice: Option<(String, Instant)>,
    pub sync_errors: HashMap<ShapeId, String>,
    pub update_available: Option<String>,

    pub subscriptions: HashMap<ShapeId, JoinHandle<()>>,
    pub background_tasks: Vec<JoinHandle<()>>,
    pub typing_stop_tasks: Vec<JoinHandle<()>>,
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
            token_manager: None,
            api: None,
            me: None,
            session_id: uuid::Uuid::new_v4().to_string(),
            session_gen: 0,
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
            in_flight_submissions: Vec::new(),
            next_submission_id: 0,
            preserved_unauthorized_drafts: None,
            channel_creators_by_key: HashMap::new(),
            channel_cache: ChannelViewCache::default(),
            unread: HashSet::new(),
            last_read_marked: HashMap::new(),
            read_mark_in_flight: HashMap::new(),
            read_mark_backoff_until: HashMap::new(),
            newest_shown: None,
            window_active: true,
            timeline_at_bottom: true,
            pending_scroll_to_bottom: false,
            timeline_render_limit: INITIAL_TIMELINE_LIMIT,
            timeline_anchor_first_id: None,
            thread_at_bottom: true,
            pending_thread_scroll_to_bottom: false,
            typing: HashMap::new(),
            typing_sent: None,
            last_key_typed: None,
            typing_poll_in_flight: false,
            notice: None,
            sync_errors: HashMap::new(),
            update_available: None,
            subscriptions: HashMap::new(),
            background_tasks: Vec::new(),
            typing_stop_tasks: Vec::new(),
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
    fn spawn_tokio<F>(&self, future: F) -> Option<JoinHandle<()>>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if let Some(rt) = &self.runtime {
            Some(rt.spawn(future))
        } else if tokio::runtime::Handle::try_current().is_ok() {
            Some(tokio::spawn(future))
        } else {
            None
        }
    }

    /// 起動時または再試行時に初期認証チェックを開始する。
    pub fn start_initial_auth(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.session_gen = self.session_gen.wrapping_add(1);
        let session_gen = self.session_gen;
        self.login_phase = Some(LoginPhase::Checking);
        let http = self.http.clone();
        let config = self.config.clone();
        let tx = self.event_tx.clone();
        let (auth_tx, mut auth_rx) = mpsc::unbounded_channel();
        self.login_task = self.spawn_tokio(async move {
            let forward = tokio::spawn(async move {
                while let Some(ev) = auth_rx.recv().await {
                    if tx
                        .send(DesktopEvent::Auth {
                            session_gen,
                            event: ev,
                        })
                        .is_err()
                    {
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
        self.session_gen = self.session_gen.wrapping_add(1);
        let session_gen = self.session_gen;
        self.login_phase = Some(LoginPhase::Checking);
        let http = self.http.clone();
        let config = self.config.clone();
        let tx = self.event_tx.clone();
        let (auth_tx, mut auth_rx) = mpsc::unbounded_channel();
        self.login_task = self.spawn_tokio(async move {
            let forward = tokio::spawn(async move {
                while let Some(ev) = auth_rx.recv().await {
                    if tx
                        .send(DesktopEvent::Auth {
                            session_gen,
                            event: ev,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
            login::run_device_login(http, config, auth_tx).await;
            let _ = forward.await;
        });
    }

    /// デバイスコード認証の待機をキャンセルする。
    /// デバイスコード認証タスクは UI 側で現行世代として採用されるまで `token.json` に書き込まないため、
    /// ここで共有の `token.json` を削除する必要はない。
    pub fn cancel_device_login(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.session_gen = self.session_gen.wrapping_add(1);
        self.login_phase = Some(LoginPhase::Ready { error: None });
    }

    /// ログアウトしてトークンを削除し、ログイン画面へ戻る。
    /// ローカルトークンの削除をタスク中断より前に確定させるため、削除に失敗した場合はタスクや実行中フラグを壊さずにログイン状態を維持する。
    pub fn logout(&mut self) {
        if let AuthConfig::Oidc(oidc) = self.config.auth.clone() {
            let delete_res = match &self.token_manager {
                Some(tm) => tm.logout_and_delete(),
                None => login::delete_local_token_for_logout(&self.config).map(|(_, rt)| rt),
            };
            match delete_res {
                Ok(refresh_token) => {
                    self.stop_typing();
                    self.preserved_unauthorized_drafts = None;
                    self.reset_to_login(None);
                    if let Some(refresh_token) = refresh_token {
                        let http = self.http.clone();
                        self.spawn_tokio(async move {
                            login::revoke_refresh_token(http, oidc, refresh_token).await;
                        });
                    }
                }
                Err(err) => {
                    self.notify(format!(
                        "ログアウトに失敗しました（トークン削除エラー）: {err:#}"
                    ));
                }
            }
        } else {
            self.stop_typing();
            self.preserved_unauthorized_drafts = None;
            self.reset_to_login(None);
        }
    }

    fn drain_and_abort_all_tasks(&mut self) -> Vec<JoinHandle<()>> {
        let mut handles = Vec::new();
        if let Some(task) = self.login_task.take() {
            task.abort();
            handles.push(task);
        }
        for (_, handle) in self.subscriptions.drain() {
            handle.abort();
            handles.push(handle);
        }
        for handle in self.background_tasks.drain(..) {
            handle.abort();
            handles.push(handle);
        }
        self.typing_poll_in_flight = false;
        self.read_mark_in_flight.clear();
        handles
    }

    fn abort_all_tasks(&mut self) {
        let _ = self.drain_and_abort_all_tasks();
    }

    /// 現在のセッションに紐づくバックグラウンドタスクを起動し、`background_tasks` に登録する。
    fn spawn_session_task<F>(&mut self, future: F) -> bool
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.background_tasks.retain(|h| !h.is_finished());
        if let Some(handle) = self.spawn_tokio(future) {
            self.background_tasks.push(handle);
            true
        } else {
            false
        }
    }

    fn push_failed_draft(&mut self, draft: FailedDraft) {
        if draft.content.trim().is_empty() {
            return;
        }
        if self.failed_drafts.iter().any(|d| d == &draft) {
            return;
        }
        if self.failed_drafts.len() >= FAILED_DRAFT_LIMIT {
            self.failed_drafts.remove(0);
        }
        self.failed_drafts.push(draft);
    }

    /// 認証切れ (`Unauthorized`) でログイン画面へ戻る直前に、書きかけ・送信中・送信失敗の下書きをユーザー ID 付きで退避する。
    fn preserve_drafts_for_unauthorized(&mut self) {
        let Some(me) = self.me.as_ref() else {
            return;
        };
        let mut drafts = self.failed_drafts.clone();
        for (_, in_flight) in &self.in_flight_submissions {
            if !in_flight.content.trim().is_empty() && !drafts.iter().any(|d| d == in_flight) {
                drafts.push(in_flight.clone());
            }
        }
        for ((ch_id, target), content) in &self.saved_drafts {
            if !content.trim().is_empty()
                && !drafts
                    .iter()
                    .any(|d| &d.channel_id == ch_id && &d.target == target && &d.content == content)
            {
                drafts.push(FailedDraft {
                    channel_id: ch_id.clone(),
                    target: target.clone(),
                    content: content.clone(),
                });
            }
        }
        if !self.composer_draft.trim().is_empty()
            && let Some(ch_id) = self.current_channel.clone()
            && !drafts.iter().any(|d| {
                d.channel_id == ch_id && d.target == self.target && d.content == self.composer_draft
            })
        {
            drafts.push(FailedDraft {
                channel_id: ch_id,
                target: self.target.clone(),
                content: self.composer_draft.clone(),
            });
        }
        if drafts.len() > FAILED_DRAFT_LIMIT {
            let excess = drafts.len() - FAILED_DRAFT_LIMIT;
            drafts.drain(0..excess);
        }
        if !drafts.is_empty() {
            self.preserved_unauthorized_drafts = Some((me.id.clone(), drafts));
        }
    }

    fn clear_session_state(&mut self) {
        self.session_gen = self.session_gen.wrapping_add(1);
        if let Some(tm) = self.token_manager.take() {
            tm.disable_persistence();
        }
        self.abort_all_tasks();
        self.store = Store::default();
        self.credentials = None;
        self.api = None;
        self.me = None;
        self.session_id = uuid::Uuid::new_v4().to_string();
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
        self.in_flight_submissions.clear();
        self.channel_creators_by_key.clear();
        self.channel_cache = ChannelViewCache::default();
        self.unread.clear();
        self.last_read_marked.clear();
        self.read_mark_in_flight.clear();
        self.read_mark_backoff_until.clear();
        self.newest_shown = None;
        self.timeline_at_bottom = true;
        self.pending_scroll_to_bottom = false;
        self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
        self.timeline_anchor_first_id = None;
        self.thread_at_bottom = true;
        self.pending_thread_scroll_to_bottom = false;
        self.typing.clear();
        self.typing_sent = None;
        self.last_key_typed = None;
        self.typing_poll_in_flight = false;
        self.notice = None;
        self.sync_errors.clear();
    }

    fn reset_to_login(&mut self, error: Option<String>) {
        self.clear_session_state();
        self.login_phase = Some(LoginPhase::Ready { error });
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    /// 現在発生している Shape 同期エラーの表示文字列を返す。
    pub fn sync_error(&self) -> Option<String> {
        for shape in [ShapeId::Channels, ShapeId::Users, ShapeId::ChannelUsers] {
            if let Some(msg) = self.sync_errors.get(&shape) {
                return Some(msg.clone());
            }
        }
        if let Some(ch) = &self.current_channel {
            for shape in [
                ShapeId::Messages(ch.clone()),
                ShapeId::Reactions(ch.clone()),
            ] {
                if let Some(msg) = self.sync_errors.get(&shape) {
                    return Some(msg.clone());
                }
            }
        }
        self.sync_errors.values().next().cloned()
    }

    /// イベントを処理し、画面の再描画が必要な状態変化があった場合に `true` を返す。
    pub fn on_desktop_event(&mut self, event: DesktopEvent) -> bool {
        match event {
            DesktopEvent::Auth { session_gen, event } => {
                if session_gen != self.session_gen {
                    return false;
                }
                self.on_auth_event(event);
                true
            }
            DesktopEvent::Shape { session_gen, event } => {
                if session_gen != self.session_gen {
                    return false;
                }
                self.on_shape(event);
                true
            }
            DesktopEvent::App { session_gen, event } => {
                if session_gen != self.session_gen {
                    return false;
                }
                self.on_app_event(event)
            }
            DesktopEvent::Tick { session_gen } => {
                if session_gen != self.session_gen {
                    return false;
                }
                self.on_tick()
            }
        }
    }

    pub fn on_auth_event(&mut self, event: AuthEvent) {
        match event {
            AuthEvent::Connected {
                credentials,
                token_manager,
                me,
            } => {
                if let Some(tm) = &token_manager
                    && let Err(err) = tm.enable_persistence_and_save()
                {
                    self.login_phase = Some(LoginPhase::Ready {
                        error: Some(format!("トークンの保存に失敗しました: {err:#}")),
                    });
                    return;
                }
                self.connect_session(credentials, me);
                self.token_manager = token_manager;
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
        let preserved = self.preserved_unauthorized_drafts.take();
        self.clear_session_state();
        if let Some((user_id, drafts)) = preserved
            && user_id == me.id
        {
            self.failed_drafts = drafts;
        }
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

    /// 停止した Shape 購読を再試行する。
    pub fn retry_sync(&mut self) {
        self.sync_errors.clear();
        self.subscribe(ShapeId::Channels);
        self.subscribe(ShapeId::Users);
        self.subscribe(ShapeId::ChannelUsers);
        if let Some(ch) = self.current_channel.clone() {
            self.subscribe(ShapeId::Messages(ch.clone()));
            self.subscribe(ShapeId::Reactions(ch));
        }
        self.poll_read_states();
    }

    fn start_periodic_tasks(&mut self) {
        let session_gen = self.session_gen;
        // 既読状態の定期取得
        if let Some(api) = self.api.clone() {
            let tx = self.event_tx.clone();
            self.spawn_session_task(async move {
                let mut interval = tokio::time::interval(READ_STATE_POLL);
                loop {
                    interval.tick().await;
                    if let Ok(states) = api.read_states().await
                        && tx
                            .send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::ReadStates(states),
                            })
                            .is_err()
                    {
                        break;
                    }
                }
            });
        }

        // 1秒ごとの Tick (通知消去・入力中解除・既読送信)
        {
            let tx = self.event_tx.clone();
            self.spawn_session_task(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1));
                loop {
                    interval.tick().await;
                    if tx.send(DesktopEvent::Tick { session_gen }).is_err() {
                        break;
                    }
                }
            });
        }

        // 新バージョン確認
        if self.config.check_updates {
            let http = self.http.clone();
            let tx = self.event_tx.clone();
            self.spawn_session_task(async move {
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
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::UpdateAvailable(version),
                    });
                }
            });
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
            if matches!(id, ShapeId::Channels) {
                self.channel_creators_by_key.clear();
            }
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
        let session_gen = self.session_gen;

        let (shape_tx, mut shape_rx) = mpsc::unbounded_channel();
        if let Some(rt) = &self.runtime {
            let _guard = rt.enter();
            let stream_handle =
                electric::spawn(http, &base_url, credentials, shape_id, shape_tx).abort_handle();
            let forward_handle = rt.spawn(async move {
                struct AbortOnDrop(AbortHandle);
                impl Drop for AbortOnDrop {
                    fn drop(&mut self) {
                        self.0.abort();
                    }
                }
                let _abort_guard = AbortOnDrop(stream_handle);
                while let Some(ev) = shape_rx.recv().await {
                    if tx
                        .send(DesktopEvent::Shape {
                            session_gen,
                            event: ev,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
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
                    if tx
                        .send(DesktopEvent::Shape {
                            session_gen,
                            event: ev,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            });
            self.subscriptions.insert(id, forward_handle);
        }
    }

    pub fn on_shape(&mut self, event: ShapeEvent) {
        match event {
            ShapeEvent::Batch(id, batch) => {
                if matches!(id, ShapeId::Channels) {
                    if batch.reset {
                        self.channel_creators_by_key.clear();
                    }
                    for change in &batch.changes {
                        match change {
                            Change::Upsert { key, row } => {
                                if let Some(ch_id) = row.get("id").and_then(|v| v.as_str()) {
                                    let creator = row
                                        .get("created_user_id")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.is_empty())
                                        .map(str::to_string);
                                    self.channel_creators_by_key
                                        .insert(key.clone(), (ch_id.to_string(), creator));
                                }
                            }
                            Change::Delete { key } => {
                                self.channel_creators_by_key.remove(key);
                            }
                        }
                    }
                }
                self.store.apply(&id, batch);
                if self.store.is_synced(&id) {
                    self.sync_errors.remove(&id);
                }
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
                        if self.current_channel.as_deref() == Some(ch.as_str()) {
                            self.channel_cache.messages_dirty = true;
                        }
                        if self.store.is_synced(&ShapeId::Messages(ch.clone())) {
                            self.reconcile_channel_targets(ch);
                        }
                        if self.current_channel.as_deref() == Some(ch.as_str()) {
                            if self.timeline_at_bottom {
                                self.pending_scroll_to_bottom = true;
                                if self.window_active
                                    && let Some(latest) = self.store.latest_root(ch)
                                {
                                    self.newest_shown = Some((ch.clone(), latest.id.clone()));
                                }
                            }
                            if self.thread.is_some() && self.thread_at_bottom {
                                self.pending_thread_scroll_to_bottom = true;
                            }
                        }
                    }
                    ShapeId::Reactions(ch)
                        if self.current_channel.as_deref() == Some(ch.as_str()) =>
                    {
                        self.channel_cache.reactions_dirty = true;
                    }
                    _ => {}
                }
                self.sync_unread_with_latest();
            }
            ShapeEvent::Failed(id, message) => {
                if message == HttpError::Unauthorized.to_string() {
                    self.preserve_drafts_for_unauthorized();
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
                let err_msg = format!("同期が止まりました ({id:?}): {message}");
                self.sync_errors.insert(id, err_msg.clone());
                self.notify(err_msg);
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
            self.sync_errors.remove(&shape_id);
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
    /// 入力中または退避済みだった下書きは失われないよう `failed_drafts` へ退避する。
    fn evict_channel(&mut self, channel_id: &str) {
        self.unsubscribe_channel(channel_id);
        self.unread.remove(channel_id);
        self.last_read_marked.remove(channel_id);
        self.read_mark_in_flight.remove(channel_id);
        self.read_mark_backoff_until.remove(channel_id);
        let evicted_saved: Vec<FailedDraft> = self
            .saved_drafts
            .iter()
            .filter(|((ch, _), content)| ch == channel_id && !content.trim().is_empty())
            .map(|((ch, target), content)| FailedDraft {
                channel_id: ch.clone(),
                target: target.clone(),
                content: content.clone(),
            })
            .collect();
        self.saved_drafts.retain(|(ch, _), _| ch != channel_id);
        for draft in evicted_saved {
            self.push_failed_draft(draft);
        }
        if self
            .newest_shown
            .as_ref()
            .is_some_and(|(ch, _)| ch == channel_id)
        {
            self.newest_shown = None;
        }
        if self.current_channel.as_deref() == Some(channel_id) {
            if !self.composer_draft.trim().is_empty() {
                let draft = FailedDraft {
                    channel_id: channel_id.to_string(),
                    target: self.target.clone(),
                    content: self.composer_draft.clone(),
                };
                self.push_failed_draft(draft);
            }
            self.stop_typing();
            self.typing.clear();
            self.current_channel = None;
            self.channel_cache = ChannelViewCache::default();
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
            self.timeline_anchor_first_id = None;
            self.thread_at_bottom = true;
            self.pending_thread_scroll_to_bottom = false;
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
                if self.composer_draft.trim().is_empty() {
                    self.switch_composer_context(Some(channel_id), Target::Channel);
                } else {
                    self.target = Target::Channel;
                    if let Some(saved_ch_draft) = self
                        .saved_drafts
                        .remove(&(channel_id.to_string(), Target::Channel))
                        && !saved_ch_draft.trim().is_empty()
                    {
                        self.push_failed_draft(FailedDraft {
                            channel_id: channel_id.to_string(),
                            target: Target::Channel,
                            content: saved_ch_draft,
                        });
                    }
                }
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
                self.preserve_drafts_for_unauthorized();
                self.reset_to_login(Some(
                    "認証が切れました。再度ログインしてください".to_string(),
                ));
                true
            }
            AppEvent::SubmissionSucceeded { submission_id } => {
                self.in_flight_submissions
                    .retain(|(id, _)| *id != submission_id);
                false
            }
            AppEvent::RestoreDraft {
                submission_id,
                channel_id,
                target,
                content,
            } => {
                self.in_flight_submissions
                    .retain(|(id, _)| *id != submission_id);
                let is_archived = self
                    .store
                    .channel(&channel_id)
                    .is_some_and(|c| c.archive_at.is_some());
                if self.current_channel.as_deref() == Some(channel_id.as_str())
                    && self.composer_draft.is_empty()
                    && !matches!(self.target, Target::Edit(_))
                    && !is_archived
                {
                    self.composer_draft = content.clone();
                    self.pending_composer_text = Some(content);
                    self.target = target;
                } else {
                    self.push_failed_draft(FailedDraft {
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
            AppEvent::ReadMarkSucceeded {
                channel_id,
                message_id,
            } => {
                if self.read_mark_in_flight.get(&channel_id) == Some(&message_id) {
                    self.read_mark_in_flight.remove(&channel_id);
                }
                self.read_mark_backoff_until.remove(&channel_id);
                false
            }
            AppEvent::ReadMarkFailed {
                channel_id,
                message_id,
            } => {
                if self.read_mark_in_flight.get(&channel_id) == Some(&message_id) {
                    self.read_mark_in_flight.remove(&channel_id);
                }
                self.read_mark_backoff_until.insert(
                    channel_id.clone(),
                    (message_id.clone(), Instant::now() + READ_MARK_BACKOFF),
                );
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
        if self.read_mark_in_flight.get(&channel_id) == Some(&latest)
            || self
                .read_mark_backoff_until
                .get(&channel_id)
                .is_some_and(|(msg_id, until)| msg_id == &latest && Instant::now() < *until)
        {
            return unread_removed;
        }
        self.last_read_marked
            .insert(channel_id.clone(), latest.clone());
        if let Some(api) = self.api.clone() {
            let tx = self.event_tx.clone();
            let session_gen = self.session_gen;
            let ch_for_task = channel_id.clone();
            let msg_for_task = latest.clone();
            if self.spawn_session_task(async move {
                match api.mark_read(&ch_for_task, &msg_for_task).await {
                    Ok(_) => {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::ReadMarkSucceeded {
                                channel_id: ch_for_task,
                                message_id: msg_for_task,
                            },
                        });
                    }
                    Err(err) => {
                        if matches!(
                            err.downcast_ref::<HttpError>(),
                            Some(HttpError::Unauthorized)
                        ) {
                            let _ = tx.send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::Unauthorized,
                            });
                        } else {
                            let _ = tx.send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::ReadMarkFailed {
                                    channel_id: ch_for_task,
                                    message_id: msg_for_task,
                                },
                            });
                        }
                    }
                }
            }) {
                self.read_mark_in_flight.insert(channel_id, latest);
            }
        }
        unread_removed
    }

    pub fn poll_read_states(&mut self) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        self.spawn_session_task(async move {
            if let Ok(states) = api.read_states().await {
                let _ = tx.send(DesktopEvent::App {
                    session_gen,
                    event: AppEvent::ReadStates(states),
                });
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
        if self
            .store
            .channel(&channel_id)
            .is_none_or(|c| c.archive_at.is_some())
        {
            self.typing.clear();
            return;
        }
        let mut scopes = vec![None];
        if let Some(thread) = &self.thread {
            scopes.push(Some(thread.root_id.clone()));
        }
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        if self.spawn_session_task(async move {
            for parent_id in scopes {
                if let Ok(users) = api.typing(&channel_id, parent_id.as_deref()).await {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::Typing {
                            channel_id: channel_id.clone(),
                            parent_id,
                            users,
                        },
                    });
                }
            }
            let _ = tx.send(DesktopEvent::App {
                session_gen,
                event: AppEvent::TypingPollFinished,
            });
        }) {
            self.typing_poll_in_flight = true;
        }
    }

    pub fn on_composer_changed(&mut self, text: String) {
        if self
            .current_channel
            .as_deref()
            .and_then(|id| self.store.channel(id))
            .is_some_and(|c| c.archive_at.is_some())
        {
            return;
        }
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
        if !self.store.is_member(&channel_id, &me.id)
            || self
                .store
                .channel(&channel_id)
                .is_none_or(|c| c.archive_at.is_some())
        {
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

    fn send_typing(&mut self, channel_id: String, parent_id: Option<String>, active: bool) {
        let Some(api) = self.api.clone() else {
            return;
        };
        let session_id = self.session_id.clone();
        let fut = async move {
            let _ = api
                .set_typing(&channel_id, parent_id.as_deref(), &session_id, active)
                .await;
        };
        if active {
            self.spawn_session_task(fut);
        } else {
            self.typing_stop_tasks.retain(|h| !h.is_finished());
            if let Some(handle) = self.spawn_tokio(fut) {
                self.typing_stop_tasks.push(handle);
            }
        }
    }

    /// API 呼び出しをバックグラウンドで実行し、エラー時は通知または再ログインへ遷移する。
    fn spawn_api<F, T>(&mut self, label: &'static str, future: F)
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        self.spawn_session_task(async move {
            if let Err(err) = future.await {
                if matches!(
                    err.downcast_ref::<HttpError>(),
                    Some(HttpError::Unauthorized)
                ) {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::Unauthorized,
                    });
                } else {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::Notice(format!("{label}に失敗しました: {err}")),
                    });
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

    /// 指定チャンネルの作成者ユーザー ID を返す。
    pub fn channel_creator_id(&self, channel_id: &str) -> Option<&str> {
        self.channel_creators_by_key
            .values()
            .find_map(|(id, creator)| (id == channel_id).then_some(creator.as_deref()).flatten())
    }

    /// 指定チャンネルへ参加可能か判定する。
    pub fn can_join_channel(&self, channel: &Channel) -> bool {
        let Some(me) = self.me.as_ref() else {
            return false;
        };
        !channel.is_dm && channel.archive_at.is_none() && !self.store.is_member(&channel.id, &me.id)
    }

    /// 指定チャンネルから退出可能か判定する（DM 参加者は退出可能、通常チャンネルの作成者は退出不可）。
    pub fn can_leave_channel(&self, channel: &Channel) -> bool {
        let Some(me) = self.me.as_ref() else {
            return false;
        };
        if !self.store.is_member(&channel.id, &me.id) {
            return false;
        }
        if channel.is_dm {
            return true;
        }
        self.channel_creator_id(&channel.id) != Some(me.id.as_str())
    }

    /// 現在開いているチャンネル・スレッドのメッセージ・リアクション集計キャッシュを最新状態に保つ。
    pub fn ensure_channel_cache(&mut self) {
        let me_id = self.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let thread_root_id = self.thread.as_ref().map(|t| t.root_id.clone());
        let Some(channel_id) = self.current_channel.clone() else {
            if self.channel_cache.channel_id.is_some() {
                self.channel_cache = ChannelViewCache::default();
            }
            return;
        };
        if self.channel_cache.channel_id.as_deref() != Some(channel_id.as_str()) {
            self.channel_cache.channel_id = Some(channel_id.clone());
            self.channel_cache.messages_dirty = true;
            self.channel_cache.reactions_dirty = true;
        }
        if self.channel_cache.me_id != me_id {
            self.channel_cache.me_id = me_id.clone();
            self.channel_cache.reactions_dirty = true;
        }
        let thread_changed = self.channel_cache.thread_root_id != thread_root_id;
        if self.channel_cache.messages_dirty {
            self.channel_cache.root_messages = Arc::new(
                self.store
                    .root_messages(&channel_id)
                    .into_iter()
                    .cloned()
                    .collect(),
            );
            self.channel_cache.reply_counts = Arc::new(self.store.reply_counts(&channel_id));
        }
        if self.channel_cache.messages_dirty || thread_changed {
            self.channel_cache.thread_root_id = thread_root_id.clone();
            if let Some(root_id) = &thread_root_id {
                self.channel_cache.thread_root_message =
                    self.store.message(&channel_id, root_id).cloned();
                self.channel_cache.thread_replies = Arc::new(
                    self.store
                        .replies(&channel_id, root_id)
                        .into_iter()
                        .cloned()
                        .collect(),
                );
            } else {
                self.channel_cache.thread_root_message = None;
                self.channel_cache.thread_replies = Arc::default();
            }
            self.channel_cache.messages_dirty = false;
        }
        if self.channel_cache.reactions_dirty {
            self.channel_cache.reactions_by_message =
                Arc::new(self.store.reaction_groups_by_message(&channel_id, &me_id));
            self.channel_cache.reactions_dirty = false;
        }
    }

    /// タイムラインで現在描画すべき `root_messages` のスライス範囲 `(start..end)` を返す。
    /// 上へスクロールしている間は `timeline_anchor_first_id` を先頭に保ち、上限件数を固定する。
    pub fn visible_timeline_bounds(&self, roots: &[Message]) -> (usize, usize) {
        let total = roots.len();
        let limit = self
            .timeline_render_limit
            .clamp(1, MAX_TIMELINE_RENDER_LIMIT);
        if !self.timeline_at_bottom
            && let Some(anchor_id) = self.timeline_anchor_first_id.as_deref()
            && let Some(start) = roots.iter().position(|m| m.id == anchor_id)
        {
            let end = (start + limit).min(total);
            return (start, end);
        }
        let start = total.saturating_sub(limit);
        (start, total)
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
            if matches!(self.target, Target::Edit(_)) {
                self.notify(
                    "メッセージを編集中です。保存するかキャンセルしてからチャンネルを切り替えてください",
                );
                return;
            }
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
            self.timeline_anchor_first_id = None;
        } else if self.current_channel.is_none() {
            self.switch_composer_context(Some(&channel_id), Target::Channel);
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
            self.timeline_anchor_first_id = None;
        }
        self.subscribe(ShapeId::Messages(channel_id.clone()));
        self.subscribe(ShapeId::Reactions(channel_id.clone()));
        self.current_channel = Some(channel_id.clone());
        self.timeline_at_bottom = true;
        self.pending_scroll_to_bottom = true;
        self.timeline_anchor_first_id = None;
        self.ensure_channel_cache();

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
        let was_at_bottom = self.timeline_at_bottom;
        self.window_active = window_active;
        self.timeline_at_bottom = at_bottom;
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if was_at_bottom && !at_bottom && self.timeline_anchor_first_id.is_none() {
            let roots = self.store.root_messages(&channel_id);
            let total = roots.len();
            let start = total.saturating_sub(self.timeline_render_limit.max(1));
            self.timeline_anchor_first_id = roots.get(start).map(|m| m.id.clone());
        } else if at_bottom {
            self.timeline_anchor_first_id = None;
        }
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

    /// スレッドペインのスクロール位置が最下部にあるかどうかを更新する。
    pub fn update_thread_scroll_state(&mut self, at_bottom: bool) {
        self.thread_at_bottom = at_bottom;
    }

    /// タイムラインのさらに古いメッセージを `TIMELINE_LOAD_STEP` 件分追加で表示する。
    pub fn load_older_messages(&mut self) {
        if let Some(channel_id) = self.current_channel.clone()
            && !self.timeline_at_bottom
            && let Some(anchor_id) = self.timeline_anchor_first_id.clone()
        {
            let roots = self.store.root_messages(&channel_id);
            if let Some(pos) = roots.iter().position(|m| m.id == anchor_id) {
                let new_start = pos.saturating_sub(TIMELINE_LOAD_STEP);
                self.timeline_anchor_first_id = roots.get(new_start).map(|m| m.id.clone());
                self.timeline_render_limit = self
                    .timeline_render_limit
                    .saturating_add(pos - new_start)
                    .min(MAX_TIMELINE_RENDER_LIMIT);
                return;
            }
        }
        self.timeline_render_limit = self
            .timeline_render_limit
            .saturating_add(TIMELINE_LOAD_STEP)
            .min(MAX_TIMELINE_RENDER_LIMIT);
    }

    /// タイムラインを最新メッセージ（最下部）へ移動する。
    pub fn jump_to_latest_messages(&mut self) {
        self.timeline_anchor_first_id = None;
        self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
        self.pending_scroll_to_bottom = true;
        self.update_timeline_view_state(self.window_active, true);
    }

    /// チャンネルへの参加・退出を実行する。
    pub fn join_or_leave(&mut self, channel_id: String, join: bool) {
        let Some(channel) = self.store.channel(&channel_id) else {
            return;
        };
        if join && !self.can_join_channel(channel) {
            return;
        }
        if !join && !self.can_leave_channel(channel) {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        if join {
            self.spawn_api("参加", async move { api.join_channel(&channel_id).await });
        } else {
            self.spawn_api("退出", async move { api.leave_channel(&channel_id).await });
        }
    }

    /// スレッドペインを開き、編集中およびアーカイブ済みチャンネルでなければ送信先をスレッド返信に切り替える。
    pub fn open_thread(&mut self, root_id: String) {
        self.thread = Some(ThreadState {
            root_id: root_id.clone(),
            selected: None,
        });
        self.thread_at_bottom = true;
        self.pending_thread_scroll_to_bottom = true;
        let is_archived = self
            .current_channel
            .as_deref()
            .and_then(|id| self.store.channel(id))
            .is_some_and(|c| c.archive_at.is_some());
        if !matches!(self.target, Target::Edit(_)) && !is_archived {
            let current = self.current_channel.clone();
            self.switch_composer_context(current.as_deref(), Target::Reply(root_id));
        }
        self.ensure_channel_cache();
        self.poll_typing();
    }

    /// スレッドペインを閉じ、送信先がスレッド返信ならチャンネルに戻す。
    pub fn close_thread(&mut self) {
        self.thread = None;
        if matches!(self.target, Target::Reply(_)) {
            let current = self.current_channel.clone();
            self.switch_composer_context(current.as_deref(), Target::Channel);
        }
        self.ensure_channel_cache();
    }

    /// 送信先を指定したスレッドへの返信に切り替える（編集中またはアーカイブ済みの場合は変更しない）。
    pub fn focus_thread_reply(&mut self, root_id: String) {
        if self
            .current_channel
            .as_deref()
            .and_then(|id| self.store.channel(id))
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルには返信できません");
            return;
        }
        if matches!(self.target, Target::Edit(_)) {
            self.notify(
                "メッセージを編集中です。編集を完了またはキャンセルしてから返信してください",
            );
            return;
        }
        self.thread_at_bottom = true;
        self.pending_thread_scroll_to_bottom = true;
        let current = self.current_channel.clone();
        self.switch_composer_context(current.as_deref(), Target::Reply(root_id));
    }

    /// 送信先を編集または返信からキャンセルして通常投稿（編集中かつスレッド表示中ならスレッド返信）へ戻す。
    pub fn cancel_target(&mut self) {
        let is_archived = self
            .current_channel
            .as_deref()
            .and_then(|id| self.store.channel(id))
            .is_some_and(|c| c.archive_at.is_some());
        let next_target = match &self.target {
            Target::Edit(_) => match &self.thread {
                Some(thread) if !is_archived => Target::Reply(thread.root_id.clone()),
                _ => Target::Channel,
            },
            Target::Reply(_) | Target::Channel => Target::Channel,
        };
        let current = self.current_channel.clone();
        self.switch_composer_context(current.as_deref(), next_target);
    }

    /// 自分のメッセージの編集を開始する。
    pub fn start_edit(&mut self, message_id: String) {
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルのメッセージは編集できません");
            return;
        }
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
        if matches!(self.target, Target::Edit(_)) {
            self.notify("メッセージを編集中です。保存するかキャンセルしてから編集してください");
            return;
        }
        if !self.composer_draft.trim().is_empty() {
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
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルのメッセージは削除できません");
            return;
        }
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
        if matches!(self.target, Target::Edit(_)) {
            self.notify(
                "メッセージを編集中です。保存するかキャンセルしてから下書きを戻してください",
            );
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
        let draft = self.failed_drafts[index].clone();
        let me_id = self.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let target_ch_writable = self.store.channel(&draft.channel_id).is_some_and(|ch| {
            ch.deleted_at.is_none()
                && ch.archive_at.is_none()
                && (!ch.is_private || self.store.is_member(&ch.id, &me_id))
        });

        if target_ch_writable {
            self.failed_drafts.remove(index);
            if self.current_channel.as_deref() != Some(draft.channel_id.as_str()) {
                self.open_channel(draft.channel_id.clone());
            }
            let next_target = if self
                .store
                .is_synced(&ShapeId::Messages(draft.channel_id.clone()))
            {
                match &draft.target {
                    Target::Reply(root_id)
                        if self.store.message(&draft.channel_id, root_id).is_none() =>
                    {
                        Target::Channel
                    }
                    Target::Edit(msg_id)
                        if self
                            .store
                            .message(&draft.channel_id, msg_id)
                            .is_none_or(|m| m.user_id != me_id) =>
                    {
                        Target::Channel
                    }
                    other => other.clone(),
                }
            } else {
                draft.target
            };
            self.composer_draft = draft.content.clone();
            self.pending_composer_text = Some(draft.content);
            self.target = next_target;
        } else {
            let current_ch_writable = self
                .current_channel
                .as_deref()
                .and_then(|id| self.store.channel(id))
                .is_some_and(|ch| {
                    ch.deleted_at.is_none()
                        && ch.archive_at.is_none()
                        && (!ch.is_private || self.store.is_member(&ch.id, &me_id))
                });
            if current_ch_writable {
                self.failed_drafts.remove(index);
                self.composer_draft = draft.content.clone();
                self.pending_composer_text = Some(draft.content);
                self.target = Target::Channel;
                self.notify(
                    "元のチャンネルにアクセスできないため、現在のチャンネルの入力欄に下書きを復元しました",
                );
            } else {
                self.notify(
                    "元のチャンネルにアクセスできません。送信可能なチャンネルを開いてから下書きを復元してください",
                );
            }
        }
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

        self.next_submission_id = self.next_submission_id.wrapping_add(1);
        let submission_id = self.next_submission_id;
        self.in_flight_submissions.push((
            submission_id,
            FailedDraft {
                channel_id: channel_id.clone(),
                target: target.clone(),
                content: content.clone(),
            },
        ));

        let session_gen = self.session_gen;
        self.spawn_session_task(async move {
            let id = uuid::Uuid::new_v4().to_string();
            let result = match &target {
                Target::Channel => api.send_message(&id, &channel_id, &content, None).await,
                Target::Reply(root_id) => {
                    api.send_message(&id, &channel_id, &content, Some(root_id))
                        .await
                }
                Target::Edit(message_id) => api.edit_message(message_id, &content).await,
            };
            match result {
                Ok(_) => {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::SubmissionSucceeded { submission_id },
                    });
                }
                Err(err) => {
                    let is_unauthorized = matches!(
                        err.downcast_ref::<HttpError>(),
                        Some(HttpError::Unauthorized)
                    );
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::RestoreDraft {
                            submission_id,
                            channel_id,
                            target,
                            content,
                        },
                    });
                    if is_unauthorized {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Unauthorized,
                        });
                    } else {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Notice(format!("送信に失敗しました: {err}")),
                        });
                    }
                }
            }
        });
        true
    }

    /// 指定メッセージに対する絵文字リアクションをトグルする。
    pub fn toggle_reaction(&mut self, message_id: &str, emoji: &str) {
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルではリアクションできません");
            return;
        }
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

        // 編集中に他メッセージの start_edit や open_channel を呼んでも拒否され編集内容が保護される
        state.on_composer_changed("編集途中の内容".into());
        state.open_channel("c2".into());
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert_eq!(state.target, Target::Edit("r1".into()));
        assert_eq!(state.composer_draft, "編集途中の内容");

        state.cancel_target();
        assert_eq!(state.target, Target::Channel);
        assert!(state.composer_draft.is_empty());

        state.request_delete("r1".into());
        assert_eq!(state.confirm_delete_message.as_deref(), Some("r1"));
        state.cancel_delete();
        assert!(state.confirm_delete_message.is_none());

        // 失敗した下書きの保存と復元: 非公開で未参加の c2 宛て下書きを復元すると、現在書き込み可能な c1 にフォールバック復元される
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: 1,
            channel_id: "c2".into(),
            target: Target::Channel,
            content: "未参加c2宛ての本文".into(),
        });
        assert_eq!(state.failed_drafts.len(), 1);
        state.restore_failed_draft();
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert_eq!(state.composer_draft, "未参加c2宛ての本文");
        assert!(state.failed_drafts.is_empty());
        state.on_composer_changed(String::new());

        // c2 のメンバーになった後は c2 を開いて復元できる
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![(
                "cu2",
                json!({ "user_id": "u1", "channel_id": "c2" }),
            )]),
        ));
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: 2,
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

        // (e) mark_read API が一時失敗したときは last_read_marked が巻き戻り、バックオフ中は再送せず、
        //     バックオフ経過後に再送できる
        state.on_app_event(AppEvent::ReadMarkFailed {
            channel_id: "c1".into(),
            message_id: "m4".into(),
        });
        assert!(!state.last_read_marked.contains_key("c1"));
        assert!(state.read_mark_backoff_until.contains_key("c1"));
        state.mark_current_read();
        assert!(!state.last_read_marked.contains_key("c1"));

        state.read_mark_backoff_until.clear();
        state.mark_current_read();
        assert_eq!(
            state.last_read_marked.get("c1").map(String::as_str),
            Some("m4")
        );
        state.on_app_event(AppEvent::ReadMarkSucceeded {
            channel_id: "c1".into(),
            message_id: "m4".into(),
        });
        assert!(!state.read_mark_in_flight.contains_key("c1"));

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

        // スレッド返信の下書きを入力してから cancel_target (Esc) を押すと、スレッド表示のまま通常投稿へ戻りチャンネル用の下書きが復元される
        state.on_composer_changed("スレッド返信の下書き".into());
        state.cancel_target();
        assert!(state.thread.is_some());
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "チャンネル本文の下書き");

        // もう一度 focus_thread_reply を呼ぶと、スレッド返信の下書きが復元される
        state.focus_thread_reply("m1".into());
        assert_eq!(state.target, Target::Reply("m1".into()));
        assert_eq!(state.composer_draft, "スレッド返信の下書き");

        // メッセージ編集中に focus_thread_reply を呼んでも編集内容は消えない
        state.on_composer_changed(String::new());
        state.start_edit("r1".into());
        state.on_composer_changed("編集中テキスト".into());
        state.focus_thread_reply("m1".into());
        assert_eq!(state.target, Target::Edit("r1".into()));
        assert_eq!(state.composer_draft, "編集中テキスト");
        state.cancel_target();
        assert_eq!(state.target, Target::Reply("m1".into()));
        state.on_composer_changed("スレッド返信の下書き".into());

        // スレッド返信 B を入力中にルート m1 が削除されると、B は入力欄に残り、退避していたチャンネル下書き A は failed_drafts に移る
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
        assert_eq!(state.composer_draft, "スレッド返信の下書き");
        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "チャンネル本文の下書き");

        // 送信中 (in_flight_submissions) の本文がある状態で認証切れ (Unauthorized) になった場合も、
        // 同じユーザー (u1) で再ログインしたときに送信中本文を含めて failed_drafts に復元される
        state.in_flight_submissions.push((
            99,
            FailedDraft {
                channel_id: "c1".into(),
                target: Target::Channel,
                content: "送信途中のメッセージ".into(),
            },
        ));
        let old_gen = state.session_gen;
        state.on_app_event(AppEvent::Unauthorized);
        assert_ne!(state.session_gen, old_gen);
        assert!(state.failed_drafts.is_empty());
        assert!(state.in_flight_submissions.is_empty());

        // 古い世代のイベントや古い Auth イベントは無視される
        assert!(!state.on_desktop_event(DesktopEvent::App {
            session_gen: old_gen,
            event: AppEvent::RestoreDraft {
                submission_id: 99,
                channel_id: "c1".into(),
                target: Target::Channel,
                content: "前セッションの遅延本文".into(),
            },
        }));
        assert!(!state.on_desktop_event(DesktopEvent::Auth {
            session_gen: old_gen,
            event: AuthEvent::LoginRequired {
                error: Some("古い認証エラー".into()),
            },
        }));

        // 同じユーザー u1 で再ログインすると退避されていた下書きと送信中だった本文が failed_drafts に戻る
        state.connect_session(
            Credentials::Dev("u1".into()),
            Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
        );
        assert_eq!(state.failed_drafts.len(), 3);
        assert!(
            state
                .failed_drafts
                .iter()
                .any(|d| d.content == "送信途中のメッセージ")
        );

        // もう一度 Unauthorized になったあと、別ユーザー u2 でログインした場合は前のユーザーの下書きは引き継がれない
        state.on_app_event(AppEvent::Unauthorized);
        state.connect_session(
            Credentials::Dev("u2".into()),
            Me {
                id: "u2".into(),
                user_name: "bob".into(),
                display_name: "Bob".into(),
            },
        );
        assert!(state.failed_drafts.is_empty());
        assert!(state.saved_drafts.is_empty());
    }

    #[tokio::test]
    async fn anchors_timeline_window_on_new_message_while_scrolled_up() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.timeline_render_limit = 1;
        state.update_timeline_scroll_state(false);
        assert_eq!(state.timeline_anchor_first_id.as_deref(), Some("m1"));

        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m2",
                json!({ "id": "m2", "user_id": "u2", "channel_id": "c1",
                        "content": "新着ルートメッセージ", "created_at": "2026-11-01 03:02:00" }),
            )]),
        ));
        state.ensure_channel_cache();
        // 上へスクロールしている間に新着が来ても、描画上限は増え続けず、アンカーにより先頭メッセージ m1 が維持される
        assert_eq!(state.timeline_render_limit, 1);
        let (start, end) = state.visible_timeline_bounds(&state.channel_cache.root_messages);
        assert_eq!(start..end, 0..1);
        assert_eq!(state.channel_cache.root_messages[start].id, "m1");

        // 最下部へ戻るとアンカーが解除され、最新メッセージ m2 が表示対象に含まれる
        state.jump_to_latest_messages();
        let (start2, end2) = state.visible_timeline_bounds(&state.channel_cache.root_messages);
        assert_eq!(start2..end2, 0..2);
        assert_eq!(state.channel_cache.root_messages[end2 - 1].id, "m2");
    }

    #[tokio::test]
    async fn tracks_sync_errors_per_shape_and_channel_leave_permissions() {
        let mut state = connected_state();
        // Users の購読が失敗したあと Channels の Batch が届いても Users の同期エラーは消えない
        state.on_shape(ShapeEvent::Failed(
            ShapeId::Users,
            "500 Internal Server Error".into(),
        ));
        assert!(state.sync_error().is_some());

        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![
                (
                    "c1",
                    json!({ "id": "c1", "channel_name": "general", "is_private": "f", "created_user_id": "u1" }),
                ),
                (
                    "c2",
                    json!({ "id": "c2", "channel_name": "secret", "is_private": "t", "created_user_id": "u2" }),
                ),
                (
                    "dm1",
                    json!({ "id": "dm1", "channel_name": "dm:u1:u2", "is_private": "t", "is_dm": "t", "created_user_id": "u1" }),
                ),
            ]),
        ));
        assert!(state.sync_error().is_some());

        // Users 自身が同期完了するとエラーが消える
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Users,
            batch(vec![(
                "u1",
                json!({ "id": "u1", "user_name": "alice", "display_name": "Alice" }),
            )]),
        ));
        assert!(state.sync_error().is_none());

        // 退出許可判定: 通常チャンネル作成者 (c1) は退出不可、通常チャンネル非作成者メンバー (c2) と DM 参加者 (dm1) は退出可能
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![
                ("cu1", json!({ "user_id": "u1", "channel_id": "c1" })),
                ("cu2", json!({ "user_id": "u1", "channel_id": "c2" })),
                ("cudm", json!({ "user_id": "u1", "channel_id": "dm1" })),
            ]),
        ));
        assert!(!state.can_leave_channel(state.store.channel("c1").unwrap()));
        assert!(state.can_leave_channel(state.store.channel("c2").unwrap()));
        assert!(state.can_leave_channel(state.store.channel("dm1").unwrap()));
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
