//! デスクトップ版の状態管理 (`AppState`) と Electric Shape / REST API の非同期連携。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, atomic::AtomicU64},
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
        rows::{Channel, FromRow, Message},
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

/// アップロードを拒否する危険な拡張子 (React 版の DISALLOWED_UPLOAD_EXTENSIONS と同一)。
pub const DISALLOWED_UPLOAD_EXTENSIONS: &[&str] = &[
    "exe", "com", "scr", "msi", "dll", "so", "dylib", "bat", "cmd", "sh",
];

/// ファイル名が危険な拡張子を持つかを判定する。
pub fn is_disallowed_extension(file_name: &str) -> bool {
    let lower = file_name.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    DISALLOWED_UPLOAD_EXTENSIONS.contains(&ext)
}

/// S3 署名付き URL の `X-Amz-Expires` から有効期限秒数を抽出する。
pub fn parse_url_expiry_secs(url: &str) -> u64 {
    if let Some(pos) = url.find("X-Amz-Expires=") {
        let after = &url[pos + "X-Amz-Expires=".len()..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(secs) = num_str.parse::<u64>() {
            return secs.saturating_sub(30).max(60);
        }
    }
    240
}

/// キャッシュされた添付ファイル URL と有効期限。
#[derive(Debug, Clone)]
pub struct CachedAttachmentUrl {
    pub url: String,
    pub fetched_at: Instant,
    pub ttl_secs: u64,
}

/// OS ネイティブ通知を送信する。
pub fn notify_desktop(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .appname("zoo2")
        .summary(title)
        .body(body)
        .show();
}

/// 拡張子から簡易 MIME タイプを判定する。
pub fn guess_content_type(file_name: &str) -> &'static str {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".pdf") {
        "application/pdf"
    } else if lower.ends_with(".txt") {
        "text/plain"
    } else if lower.ends_with(".md") {
        "text/markdown"
    } else if lower.ends_with(".json") {
        "application/json"
    } else if lower.ends_with(".zip") {
        "application/zip"
    } else {
        "application/octet-stream"
    }
}

/// 画像 MIME タイプかどうかを判定する。
pub fn is_image_content_type(content_type: &str) -> bool {
    let base = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

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
    pub attachments: Vec<PendingAttachment>,
    pub message_id: Option<String>,
}

/// 送信待ちの添付ファイル。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAttachment {
    pub local_id: String,
    pub channel_id: String,
    pub session_gen: u64,
    pub file_name: String,
    pub file_size: i64,
    pub content_type: String,
    pub attachment_id: Option<String>,
    pub is_uploading: bool,
    pub error: Option<String>,
}

/// メンション補完候補。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionCandidate {
    pub name: String,
    pub display_name: String,
    pub is_special: bool,
}

/// メンション補完状態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MentionAutocompleteState {
    pub query: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub candidates: Vec<MentionCandidate>,
    pub selected_index: usize,
}

/// チャンネル作成モーダル状態。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CreateChannelModal {
    pub request_id: u64,
    pub channel_name: String,
    pub description: String,
    pub is_private: bool,
    pub error: Option<String>,
    pub in_flight: bool,
}

/// チャンネル内メッセージ検索状態。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MessageSearchState {
    pub query: String,
    pub matches: Vec<String>,
    pub current_match_index: usize,
}

/// クイックチャンネルスイッチャー状態 (Cmd+K)。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuickSwitcherState {
    pub query: String,
    pub selected_index: usize,
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
        sent_attachment_ids: Vec<String>,
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
    AttachmentUploadFinished {
        local_id: String,
        attachment_id: String,
    },
    AttachmentUploadFailed {
        local_id: String,
        error: String,
    },
    ChannelCreated {
        request_id: u64,
        channel_id: String,
    },
    ChannelCreateFailed {
        request_id: u64,
        error: String,
    },
    AttachmentUrlResolved {
        attachment_id: String,
        download_url: String,
    },
    AttachmentUrlFailed {
        attachment_id: String,
    },
    RefreshAttachmentUrls,
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
    /// 実行中のデバイスログインでトークン保存が完了したマネージャ。
    /// 承認完了後にキャンセルされた場合、このマネージャが保存したトークンのみを削除・失効するために保持する。
    pub pending_device_login_manager:
        Arc<std::sync::Mutex<Option<Arc<login::DesktopTokenManager>>>>,

    pub credentials: Option<Credentials>,
    pub token_manager: Option<Arc<login::DesktopTokenManager>>,
    pub api: Option<ApiClient>,
    pub me: Option<Me>,
    pub me_is_admin: bool,
    pub session_id: String,
    /// セッション世代。ログアウトや再ログイン時にインクリメントし、古いタスクから届いたイベントを破棄する。
    pub session_gen: u64,
    pub active_session_gen: Arc<AtomicU64>,

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
    /// `(チャンネル ID, 送信先)` ごとの再送用メッセージ ID。
    pub saved_draft_message_ids: HashMap<(String, Target), String>,
    /// `AppState` 側から入力欄 (`TextInput`) へ反映すべき文字列とキャレット位置。
    pub pending_composer_text: Option<(String, Option<usize>)>,
    /// 復元された下書きの再送用メッセージ ID。
    pub pending_composer_message_id: Option<String>,
    pub failed_drafts: Vec<FailedDraft>,
    /// 現在送信リクエスト実行中の下書き (`(submission_id, FailedDraft)`)。
    pub in_flight_submissions: Vec<(u64, FailedDraft)>,
    pub next_submission_id: u64,
    pub next_channel_create_id: u64,
    /// 認証切れ (`Unauthorized`) でログイン画面に戻った際に、同じユーザーの再ログイン時だけ復元する下書き。
    pub preserved_unauthorized_drafts: Option<(String, Vec<FailedDraft>)>,

    /// `Channels` シェイプのキーごとの `(チャンネル ID, 作成者ユーザー ID)`。
    pub channel_creators_by_key: HashMap<String, (String, Option<String>)>,
    /// `Channels` シェイプのキーごとの `(チャンネル ID, posting_policy)`。
    pub channel_posting_policies_by_key: HashMap<String, (String, String)>,
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

    /// 送信待ち・アップロード中の添付ファイル一覧。
    pub pending_attachments: Vec<PendingAttachment>,
    /// 添付ファイル ID -> キャッシュ情報 (download_url, 取得日時, TTL) のキャッシュ。
    pub attachment_download_urls: HashMap<String, CachedAttachmentUrl>,
    /// 取得中の添付ファイル ID 一覧。
    pub fetching_attachment_urls: HashSet<String>,
    /// 取得に失敗した添付ファイル ID -> 失敗時刻（バックオフ用）。
    pub failed_attachment_urls: HashMap<String, Instant>,
    /// ファイルドラッグオーバー中フラグ。
    pub is_dragging_files: bool,
    /// 絵文字ピッカーの開閉状態。
    pub emoji_picker_open: bool,
    /// 絵文字ピッカーの検索クエリ。
    pub emoji_picker_query: String,
    /// メンション補完状態。
    pub mention_autocomplete: Option<MentionAutocompleteState>,
    /// チャンネル作成モーダル状態。
    pub create_channel_modal: Option<CreateChannelModal>,
    /// チャンネル内メッセージ検索状態。
    pub message_search: Option<MessageSearchState>,
    /// クイックチャンネルスイッチャー状態 (Cmd+K)。
    pub quick_switcher: Option<QuickSwitcherState>,
    /// テーマモード (Dark / Light)。
    pub theme_mode: crate::theme::ThemeMode,
    /// OS 通知有効フラグ。
    pub notifications_enabled: bool,
    /// 検索移動時などに指定位置までスクロールするためのメッセージ ID。
    pub pending_scroll_to_message_id: Option<String>,
    /// モーダルや検索バーを閉じた後に composer へフォーカスを復帰させるフラグ。
    pub pending_focus_composer: bool,
    /// URL 取得完了時に自動でブラウザで開く添付ファイル ID 一覧。
    pub pending_open_attachments: HashSet<String>,
    /// `view` 側でブラウザを開くための保留中 URL。
    pub pending_open_url: Option<String>,
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
            pending_device_login_manager: Arc::new(std::sync::Mutex::new(None)),
            credentials: None,
            token_manager: None,
            api: None,
            me: None,
            me_is_admin: false,
            session_id: uuid::Uuid::new_v4().to_string(),
            session_gen: 0,
            active_session_gen: Arc::new(AtomicU64::new(0)),
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
            saved_draft_message_ids: HashMap::new(),
            pending_composer_text: None,
            pending_composer_message_id: None,
            failed_drafts: Vec::new(),
            in_flight_submissions: Vec::new(),
            next_submission_id: 0,
            next_channel_create_id: 0,
            preserved_unauthorized_drafts: None,
            channel_creators_by_key: HashMap::new(),
            channel_posting_policies_by_key: HashMap::new(),
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
            pending_attachments: Vec::new(),
            attachment_download_urls: HashMap::new(),
            fetching_attachment_urls: HashSet::new(),
            failed_attachment_urls: HashMap::new(),
            is_dragging_files: false,
            emoji_picker_open: false,
            emoji_picker_query: String::new(),
            mention_autocomplete: None,
            create_channel_modal: None,
            message_search: None,
            quick_switcher: None,
            theme_mode: crate::theme::ThemeMode::Dark,
            notifications_enabled: true,
            pending_scroll_to_message_id: None,
            pending_focus_composer: false,
            pending_open_attachments: HashSet::new(),
            pending_open_url: None,
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

    fn bump_session_gen(&mut self) -> u64 {
        let next = login::advance_session_gen(&self.active_session_gen);
        self.session_gen = next;
        next
    }

    /// 起動時または再試行時に初期認証チェックを開始する。
    pub fn start_initial_auth(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        let session_gen = self.bump_session_gen();
        let active_session_gen = self.active_session_gen.clone();
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
            login::connect_initial(http, config, session_gen, active_session_gen, auth_tx).await;
            let _ = forward.await;
        });
    }

    /// OIDC デバイスコード認証を開始する。
    pub fn start_device_login(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        let session_gen = self.bump_session_gen();
        let active_session_gen = self.active_session_gen.clone();
        let saved_manager = Arc::new(std::sync::Mutex::new(None));
        self.pending_device_login_manager = saved_manager.clone();
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
            login::run_device_login(
                http,
                config,
                session_gen,
                active_session_gen,
                saved_manager,
                auth_tx,
            )
            .await;
            let _ = forward.await;
        });
    }

    /// デバイスコード認証の待機をキャンセルする。
    /// 世代番号を進め、このログイン試行が承認後にトークンを保存していた場合のみ削除・失効して次回起動時の誤接続を防ぐ。
    /// 承認前キャンセル時は、以前から存在していた正常なトークンファイルを誤削除しない。
    pub fn cancel_device_login(&mut self) {
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.bump_session_gen();
        if let Some(manager) = self
            .pending_device_login_manager
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            let delete_res = manager.logout_and_delete();
            if let Ok(Some(refresh_token)) = delete_res
                && let AuthConfig::Oidc(oidc) = self.config.auth.clone()
            {
                let http = self.http.clone();
                self.spawn_tokio(async move {
                    login::revoke_refresh_token(http, oidc, refresh_token).await;
                });
            }
        }
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
        self.pending_device_login_manager
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
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
        if draft.content.trim().is_empty() && draft.attachments.is_empty() {
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
            if (!in_flight.content.trim().is_empty() || !in_flight.attachments.is_empty())
                && !drafts.iter().any(|d| d == in_flight)
            {
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
                    attachments: Vec::new(),
                    message_id: self
                        .saved_draft_message_ids
                        .get(&(ch_id.clone(), target.clone()))
                        .cloned(),
                });
            }
        }
        let current_atts: Vec<PendingAttachment> = self
            .current_channel_attachments()
            .into_iter()
            .filter(|a| a.attachment_id.is_some() && !a.is_uploading && a.error.is_none())
            .cloned()
            .collect();
        let has_content = !self.composer_draft.trim().is_empty();
        let has_attachments = !current_atts.is_empty();
        if (has_content || has_attachments)
            && let Some(ch_id) = self.current_channel.clone()
            && !drafts.iter().any(|d| {
                d.channel_id == ch_id
                    && d.target == self.target
                    && d.content == self.composer_draft
                    && d.attachments == current_atts
            })
        {
            drafts.push(FailedDraft {
                channel_id: ch_id,
                target: self.target.clone(),
                content: self.composer_draft.clone(),
                attachments: current_atts,
                message_id: self.pending_composer_message_id.clone(),
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
        if let Some(tm) = self.token_manager.take() {
            tm.disable_persistence();
        }
        self.abort_all_tasks();
        self.store = Store::default();
        self.credentials = None;
        self.api = None;
        self.me = None;
        self.me_is_admin = false;
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
        self.saved_draft_message_ids.clear();
        self.pending_composer_text = Some((String::new(), None));
        self.pending_composer_message_id = None;
        self.failed_drafts.clear();
        self.in_flight_submissions.clear();
        self.channel_creators_by_key.clear();
        self.channel_posting_policies_by_key.clear();
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
        self.pending_attachments.clear();
        self.attachment_download_urls.clear();
        self.fetching_attachment_urls.clear();
        self.failed_attachment_urls.clear();
        self.pending_open_attachments.clear();
        self.pending_open_url = None;
        self.pending_scroll_to_message_id = None;
        self.pending_focus_composer = false;
        self.emoji_picker_open = false;
        self.emoji_picker_query.clear();
        self.mention_autocomplete = None;
        self.create_channel_modal = None;
        self.message_search = None;
        self.quick_switcher = None;
    }

    fn reset_to_login(&mut self, error: Option<String>) {
        self.bump_session_gen();
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
                ShapeId::Attachments(ch.clone()),
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
                is_admin,
            } => {
                if let Some(tm) = &token_manager {
                    tm.bind_to_session_gen(self.session_gen);
                    if let Err(err) = tm.enable_persistence_and_save() {
                        self.login_phase = Some(LoginPhase::Ready {
                            error: Some(format!("トークンの保存に失敗しました: {err:#}")),
                        });
                        return;
                    }
                }
                self.connect_session(credentials, me);
                self.me_is_admin = is_admin;
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
                self.pending_device_login_manager
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
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
            self.subscribe(ShapeId::Reactions(ch.clone()));
            self.subscribe(ShapeId::Attachments(ch));
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

        // 添付ファイル URL の定期リフレッシュ (期限切れ間近の URL を再取得)
        {
            let tx = self.event_tx.clone();
            self.spawn_session_task(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    if tx
                        .send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::RefreshAttachmentUrls,
                        })
                        .is_err()
                    {
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
                self.channel_posting_policies_by_key.clear();
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
                        self.channel_posting_policies_by_key.clear();
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
                                    let policy = row
                                        .get("posting_policy")
                                        .and_then(|v| v.as_str())
                                        .filter(|s| !s.is_empty())
                                        .unwrap_or("members")
                                        .to_string();
                                    self.channel_posting_policies_by_key
                                        .insert(key.clone(), (ch_id.to_string(), policy));
                                }
                            }
                            Change::Delete { key } => {
                                self.channel_creators_by_key.remove(key);
                                self.channel_posting_policies_by_key.remove(key);
                            }
                        }
                    }
                }
                let mut pending_notifications = Vec::new();
                if self.notifications_enabled
                    && let ShapeId::Messages(ch) = &id
                    && self.store.is_synced(&id)
                    && !batch.reset
                {
                    let is_active_view =
                        self.window_active && self.current_channel.as_deref() == Some(ch.as_str());
                    if !is_active_view {
                        for change in &batch.changes {
                            if let electric::Change::Upsert { row, .. } = change
                                && let Ok(msg) = Message::from_row(row)
                            {
                                if self.store.message(ch, &msg.id).is_some()
                                    || msg.deleted_at.is_some()
                                {
                                    continue;
                                }
                                let is_mine = self.me.as_ref().is_some_and(|m| m.id == msg.user_id);
                                if !is_mine {
                                    let me_user_name = self
                                        .me
                                        .as_ref()
                                        .map(|m| m.user_name.as_str())
                                        .unwrap_or("");
                                    let is_dm = self.store.channel(ch).is_some_and(|c| c.is_dm);
                                    let has_mention = (!me_user_name.is_empty()
                                        && msg.content.contains(&format!("@{me_user_name}")))
                                        || msg.content.contains("@channel")
                                        || msg.content.contains("@here");
                                    if is_dm || has_mention {
                                        pending_notifications.push((
                                            ch.clone(),
                                            msg.user_id,
                                            msg.content,
                                            is_dm,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }

                self.store.apply(&id, batch);

                for (ch, sender_id, content, is_dm) in pending_notifications {
                    let sender_name = self.store.display_name(&sender_id);
                    let channel_name = self
                        .store
                        .channel(&ch)
                        .map(|c| c.channel_name.clone())
                        .unwrap_or_else(|| "channel".to_string());
                    let title = if is_dm {
                        format!("{sender_name} からのDM")
                    } else {
                        format!("{sender_name} (#{channel_name})")
                    };
                    let body = if content.chars().count() > 80 {
                        let truncated: String = content.chars().take(80).collect();
                        format!("{truncated}...")
                    } else {
                        content
                    };
                    std::thread::spawn(move || {
                        notify_desktop(&title, &body);
                    });
                }
                if self.store.is_synced(&id) {
                    self.sync_errors.remove(&id);
                }
                match &id {
                    ShapeId::Channels | ShapeId::ChannelUsers => {
                        if self.store.is_synced(&ShapeId::Channels)
                            && self.store.is_synced(&ShapeId::ChannelUsers)
                            && let Some(current) = self.current_channel.clone()
                        {
                            let status = self
                                .store
                                .channel(&current)
                                .map(|ch| (ch.deleted_at.is_some(), ch.archive_at.is_some()));
                            match status {
                                None | Some((true, _)) => {
                                    self.evict_channel(&current);
                                    self.notify("このチャンネルへのアクセス権がなくなりました");
                                }
                                Some((false, true)) => {
                                    self.reconcile_archived_or_restricted_channel(&current, true);
                                }
                                Some((false, false)) if !self.can_post_in_channel(&current) => {
                                    self.reconcile_archived_or_restricted_channel(&current, false);
                                }
                                Some((false, false)) => {}
                            }
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
                    ShapeId::Attachments(ch)
                        if self.current_channel.as_deref() == Some(ch.as_str()) =>
                    {
                        self.channel_cache.messages_dirty = true;
                        let att_ids: Vec<String> = self
                            .store
                            .channel_attachments(ch)
                            .into_iter()
                            .map(|a| a.id.clone())
                            .collect();
                        for id in att_ids {
                            self.ensure_attachment_url(id);
                        }
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
                if let ShapeId::Messages(ch) | ShapeId::Reactions(ch) | ShapeId::Attachments(ch) =
                    &id
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

    /// 特定のチャンネルのメッセージ・リアクション・添付ファイル購読を停止し、キャッシュされた行を破棄する。
    fn unsubscribe_channel(&mut self, channel_id: &str) {
        for shape_id in [
            ShapeId::Messages(channel_id.to_string()),
            ShapeId::Reactions(channel_id.to_string()),
            ShapeId::Attachments(channel_id.to_string()),
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
                attachments: Vec::new(),
                message_id: self
                    .saved_draft_message_ids
                    .get(&(ch.clone(), target.clone()))
                    .cloned(),
            })
            .collect();
        self.saved_drafts.retain(|(ch, _), _| ch != channel_id);
        self.saved_draft_message_ids
            .retain(|(ch, _), _| ch != channel_id);
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
            let s_gen = self.session_gen;
            let ch = channel_id.to_string();
            let (current_atts, remaining_atts): (Vec<_>, Vec<_>) = self
                .pending_attachments
                .drain(..)
                .partition(|a| a.channel_id == ch && a.session_gen == s_gen);
            self.pending_attachments = remaining_atts;
            let valid_atts: Vec<PendingAttachment> = current_atts
                .into_iter()
                .filter(|a| a.attachment_id.is_some() && !a.is_uploading && a.error.is_none())
                .collect();
            if !self.composer_draft.trim().is_empty() || !valid_atts.is_empty() {
                let draft = FailedDraft {
                    channel_id: channel_id.to_string(),
                    target: self.target.clone(),
                    content: self.composer_draft.clone(),
                    attachments: valid_atts,
                    message_id: self.pending_composer_message_id.take(),
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
            self.pending_composer_text = Some((String::new(), None));
            self.target = Target::Channel;
            self.timeline_at_bottom = true;
            self.pending_scroll_to_bottom = false;
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
            self.timeline_anchor_first_id = None;
            self.thread_at_bottom = true;
            self.pending_thread_scroll_to_bottom = false;
        }
    }

    /// チャンネルがアーカイブまたは投稿制限状態になったときに、編集・入力中の下書きを `failed_drafts` へ退避し、
    /// 編集ロックや返信ターゲットを解除して別チャンネルへの切り替えや閲覧を妨げないようにする。
    fn reconcile_archived_or_restricted_channel(&mut self, channel_id: &str, is_archived: bool) {
        let evicted_saved: Vec<FailedDraft> = self
            .saved_drafts
            .iter()
            .filter(|((ch, _), content)| ch == channel_id && !content.trim().is_empty())
            .map(|((ch, target), content)| FailedDraft {
                channel_id: ch.clone(),
                target: target.clone(),
                content: content.clone(),
                attachments: Vec::new(),
                message_id: self
                    .saved_draft_message_ids
                    .get(&(ch.clone(), target.clone()))
                    .cloned(),
            })
            .collect();
        self.saved_drafts.retain(|(ch, _), _| ch != channel_id);
        self.saved_draft_message_ids
            .retain(|(ch, _), _| ch != channel_id);
        for draft in evicted_saved {
            self.push_failed_draft(draft);
        }
        if self.current_channel.as_deref() != Some(channel_id) {
            return;
        }
        if is_archived {
            let was_editing = matches!(self.target, Target::Edit(_));
            let s_gen = self.session_gen;
            let ch = channel_id.to_string();
            let (current_atts, remaining_atts): (Vec<_>, Vec<_>) = self
                .pending_attachments
                .drain(..)
                .partition(|a| a.channel_id == ch && a.session_gen == s_gen);
            self.pending_attachments = remaining_atts;
            let valid_atts: Vec<PendingAttachment> = current_atts
                .into_iter()
                .filter(|a| a.attachment_id.is_some() && !a.is_uploading && a.error.is_none())
                .collect();
            let had_content = !self.composer_draft.trim().is_empty() || !valid_atts.is_empty();
            if had_content {
                let draft = FailedDraft {
                    channel_id: channel_id.to_string(),
                    target: self.target.clone(),
                    content: std::mem::take(&mut self.composer_draft),
                    attachments: valid_atts,
                    message_id: self.pending_composer_message_id.take(),
                };
                self.push_failed_draft(draft);
            } else {
                self.composer_draft.clear();
            }
            self.pending_composer_text = Some((String::new(), None));
            self.stop_typing();
            self.typing.clear();
            self.active_reaction_message = None;
            self.confirm_delete_message = None;
            self.target = Target::Channel;
            if was_editing || had_content {
                self.notify(
                    "このチャンネルがアーカイブされたため、編集・入力中の本文を下書きに退避しました",
                );
            }
        } else if !matches!(self.target, Target::Edit(_)) {
            let s_gen = self.session_gen;
            let ch = channel_id.to_string();
            let (current_atts, remaining_atts): (Vec<_>, Vec<_>) = self
                .pending_attachments
                .drain(..)
                .partition(|a| a.channel_id == ch && a.session_gen == s_gen);
            self.pending_attachments = remaining_atts;
            let valid_atts: Vec<PendingAttachment> = current_atts
                .into_iter()
                .filter(|a| a.attachment_id.is_some() && !a.is_uploading && a.error.is_none())
                .collect();
            let had_content = !self.composer_draft.trim().is_empty() || !valid_atts.is_empty();
            if had_content {
                let draft = FailedDraft {
                    channel_id: channel_id.to_string(),
                    target: self.target.clone(),
                    content: std::mem::take(&mut self.composer_draft),
                    attachments: valid_atts,
                    message_id: self.pending_composer_message_id.take(),
                };
                self.push_failed_draft(draft);
                self.pending_composer_text = Some((String::new(), None));
                self.notify(
                    "このチャンネルの投稿権限が変更されたため、入力中の本文を下書きに退避しました",
                );
            }
            self.stop_typing();
            self.target = Target::Channel;
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
                        let saved_msg_id = self
                            .saved_draft_message_ids
                            .remove(&(channel_id.to_string(), Target::Channel));
                        self.push_failed_draft(FailedDraft {
                            channel_id: channel_id.to_string(),
                            target: Target::Channel,
                            content: saved_ch_draft,
                            attachments: Vec::new(),
                            message_id: saved_msg_id,
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
                Some(thread) if self.can_post_in_channel(channel_id) => {
                    Target::Reply(thread.root_id.clone())
                }
                _ => Target::Channel,
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
            AppEvent::SubmissionSucceeded {
                submission_id,
                sent_attachment_ids,
            } => {
                self.in_flight_submissions
                    .retain(|(id, _)| *id != submission_id);
                if !sent_attachment_ids.is_empty() {
                    self.pending_attachments.retain(|a| match &a.attachment_id {
                        Some(att_id) => !sent_attachment_ids.contains(att_id),
                        None => true,
                    });
                }
                true
            }
            AppEvent::RestoreDraft {
                submission_id,
                channel_id,
                target,
                content,
            } => {
                let in_flight = self
                    .in_flight_submissions
                    .iter()
                    .position(|(id, _)| *id == submission_id)
                    .map(|idx| self.in_flight_submissions.remove(idx).1);
                let is_archived = self
                    .store
                    .channel(&channel_id)
                    .is_some_and(|c| c.archive_at.is_some());
                let can_accept_target = !is_archived
                    && (matches!(target, Target::Edit(_)) || self.can_post_in_channel(&channel_id));
                if self.current_channel.as_deref() == Some(channel_id.as_str())
                    && self.composer_draft.is_empty()
                    && self.current_channel_attachments().is_empty()
                    && !matches!(self.target, Target::Edit(_))
                    && can_accept_target
                    && !self
                        .saved_drafts
                        .contains_key(&(channel_id.clone(), target.clone()))
                {
                    if self.target != target {
                        self.switch_composer_context(Some(&channel_id), target);
                    }
                    self.composer_draft = content.clone();
                    self.pending_composer_text = Some((content, None));
                    if let Some(draft) = in_flight {
                        self.pending_composer_message_id = draft.message_id;
                        for mut att in draft.attachments {
                            att.session_gen = self.session_gen;
                            self.pending_attachments.push(att);
                        }
                    }
                } else {
                    if let Some(draft) = in_flight {
                        self.push_failed_draft(draft);
                    } else {
                        self.push_failed_draft(FailedDraft {
                            channel_id,
                            target,
                            content,
                            attachments: Vec::new(),
                            message_id: None,
                        });
                    }
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
            AppEvent::AttachmentUploadFinished {
                local_id,
                attachment_id,
            } => {
                if let Some(att) = self
                    .pending_attachments
                    .iter_mut()
                    .find(|a| a.local_id == local_id)
                {
                    att.attachment_id = Some(attachment_id);
                    att.is_uploading = false;
                }
                true
            }
            AppEvent::AttachmentUploadFailed { local_id, error } => {
                if let Some(att) = self
                    .pending_attachments
                    .iter_mut()
                    .find(|a| a.local_id == local_id)
                {
                    att.error = Some(error.clone());
                    att.is_uploading = false;
                }
                self.notify(format!("ファイルのアップロードに失敗しました: {error}"));
                true
            }
            AppEvent::ChannelCreated {
                request_id,
                channel_id,
            } => {
                let matches_current = self
                    .create_channel_modal
                    .as_ref()
                    .is_some_and(|m| m.request_id == request_id);
                if matches_current {
                    self.create_channel_modal = None;
                    self.pending_focus_composer = true;
                    self.open_channel(channel_id);
                    self.notify("チャンネルを作成しました");
                } else if self.create_channel_modal.is_none()
                    && self.next_channel_create_id == request_id
                {
                    self.open_channel(channel_id);
                    self.notify("チャンネルを作成しました");
                }
                true
            }
            AppEvent::ChannelCreateFailed { request_id, error } => {
                let matches_current = self
                    .create_channel_modal
                    .as_ref()
                    .is_some_and(|m| m.request_id == request_id);
                if matches_current {
                    if let Some(modal) = &mut self.create_channel_modal {
                        modal.in_flight = false;
                        modal.error = Some(error.clone());
                    }
                    self.notify(format!("チャンネル作成に失敗しました: {error}"));
                } else if self.create_channel_modal.is_none()
                    && self.next_channel_create_id == request_id
                {
                    self.notify(format!("チャンネル作成に失敗しました: {error}"));
                }
                true
            }
            AppEvent::AttachmentUrlResolved {
                attachment_id,
                download_url,
            } => {
                self.fetching_attachment_urls.remove(&attachment_id);
                self.failed_attachment_urls.remove(&attachment_id);
                if self.pending_open_attachments.remove(&attachment_id) {
                    self.pending_open_url = Some(download_url.clone());
                }
                let ttl_secs = parse_url_expiry_secs(&download_url);
                self.attachment_download_urls.insert(
                    attachment_id,
                    CachedAttachmentUrl {
                        url: download_url,
                        fetched_at: Instant::now(),
                        ttl_secs,
                    },
                );
                true
            }
            AppEvent::AttachmentUrlFailed { attachment_id } => {
                self.fetching_attachment_urls.remove(&attachment_id);
                self.failed_attachment_urls
                    .insert(attachment_id.clone(), Instant::now());
                self.pending_open_attachments.remove(&attachment_id);
                true
            }
            AppEvent::RefreshAttachmentUrls => {
                if let Some(ch) = self.current_channel.clone() {
                    let att_ids: Vec<String> = self
                        .store
                        .channel_attachments(&ch)
                        .into_iter()
                        .map(|a| a.id.clone())
                        .collect();
                    for id in att_ids {
                        if let Some(cached) = self.attachment_download_urls.get(&id) {
                            if cached.fetched_at.elapsed().as_secs() + 60 >= cached.ttl_secs {
                                self.ensure_attachment_url(id);
                            }
                        } else {
                            self.ensure_attachment_url(id);
                        }
                    }
                }
                false
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
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            return;
        }
        if !matches!(self.target, Target::Edit(_)) && !self.can_post_in_channel(&channel_id) {
            return;
        }
        let non_empty = !text.trim().is_empty();
        if self.composer_draft != text {
            self.pending_composer_message_id = None;
        }
        self.composer_draft = text;
        if non_empty {
            self.touch_typing();
        } else {
            self.stop_typing();
            self.pending_composer_message_id = None;
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
        if !self.store.is_member(&channel_id, &me.id) || !self.can_post_in_channel(&channel_id) {
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

    /// 指定チャンネルの `posting_policy` (`"members"` または `"managers"`) を返す。
    pub fn channel_posting_policy(&self, channel_id: &str) -> &str {
        self.channel_posting_policies_by_key
            .values()
            .find_map(|(id, policy)| (id == channel_id).then_some(policy.as_str()))
            .unwrap_or("members")
    }

    /// 指定チャンネルへ新規メッセージや返信を投稿可能か判定する。
    pub fn can_post_in_channel(&self, channel_id: &str) -> bool {
        let Some(me) = self.me.as_ref() else {
            return false;
        };
        // Channels または ChannelUsers が再同期中（reset 受信〜初期同期完了）の間は、
        // 投稿制限やメンバーシップ欠落と誤判定せず、入力欄の表示と本文入力を維持する。
        if !self.store.is_synced(&ShapeId::Channels)
            || !self.store.is_synced(&ShapeId::ChannelUsers)
        {
            return true;
        }
        let Some(ch) = self.store.channel(channel_id) else {
            return false;
        };
        if ch.deleted_at.is_some() || ch.archive_at.is_some() {
            return false;
        }
        if ch.is_private && !self.store.is_member(channel_id, &me.id) {
            return false;
        }
        if self.channel_posting_policy(channel_id) == "managers" {
            return self.me_is_admin || self.channel_creator_id(channel_id) == Some(me.id.as_str());
        }
        true
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
                self.saved_drafts.remove(&(ch.clone(), self.target.clone()));
                self.saved_draft_message_ids
                    .remove(&(ch, self.target.clone()));
            } else {
                self.saved_drafts.insert(
                    (ch.clone(), self.target.clone()),
                    self.composer_draft.clone(),
                );
                if let Some(msg_id) = self.pending_composer_message_id.take() {
                    self.saved_draft_message_ids
                        .insert((ch, self.target.clone()), msg_id);
                } else {
                    self.saved_draft_message_ids
                        .remove(&(ch, self.target.clone()));
                }
            }
        }
        self.target = next_target.clone();
        let next_draft = match next_channel {
            Some(ch) if !matches!(next_target, Target::Edit(_)) => self
                .saved_drafts
                .remove(&(ch.to_string(), next_target.clone()))
                .unwrap_or_default(),
            _ => String::new(),
        };
        self.pending_composer_message_id = match next_channel {
            Some(ch) if !matches!(next_target, Target::Edit(_)) => self
                .saved_draft_message_ids
                .remove(&(ch.to_string(), next_target)),
            _ => None,
        };
        if self.composer_draft != next_draft || self.pending_composer_text.is_some() {
            self.composer_draft = next_draft.clone();
            self.pending_composer_text = Some((next_draft, None));
        }
    }

    /// チャンネルを開き、メッセージとリアクションのシェイプ購読を開始する。
    pub fn open_channel(&mut self, channel_id: String) {
        if let Some(old_ch) = self.current_channel.clone()
            && old_ch != channel_id
        {
            let old_is_archived = self
                .store
                .channel(&old_ch)
                .is_some_and(|c| c.archive_at.is_some());
            if old_is_archived {
                self.reconcile_archived_or_restricted_channel(&old_ch, true);
            }
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
            self.close_message_search();
            self.mention_autocomplete = None;
            self.switch_composer_context(Some(&channel_id), Target::Channel);
            // 別のチャンネルへ切り替えるときは前のチャンネルの Messages / Reactions 購読とキャッシュを破棄し、
            // バックグラウンドタスクとメモリがチャンネル数に応じて増え続けるのを防ぐ。
            self.unsubscribe_channel(&old_ch);
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
            self.timeline_anchor_first_id = None;
        } else if self.current_channel.is_none() {
            self.close_message_search();
            self.mention_autocomplete = None;
            self.switch_composer_context(Some(&channel_id), Target::Channel);
            self.timeline_render_limit = INITIAL_TIMELINE_LIMIT;
            self.timeline_anchor_first_id = None;
        }
        self.subscribe(ShapeId::Messages(channel_id.clone()));
        self.subscribe(ShapeId::Reactions(channel_id.clone()));
        self.subscribe(ShapeId::Attachments(channel_id.clone()));
        self.current_channel = Some(channel_id.clone());
        let att_ids: Vec<String> = self
            .store
            .channel_attachments(&channel_id)
            .into_iter()
            .map(|a| a.id.clone())
            .collect();
        for id in att_ids {
            self.ensure_attachment_url(id);
        }
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
        if self.pending_scroll_to_message_id.is_none() {
            if was_at_bottom && !at_bottom && self.timeline_anchor_first_id.is_none() {
                let roots = self.store.root_messages(&channel_id);
                let total = roots.len();
                let start = total.saturating_sub(self.timeline_render_limit.max(1));
                self.timeline_anchor_first_id = roots.get(start).map(|m| m.id.clone());
            } else if at_bottom {
                self.timeline_anchor_first_id = None;
            }
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

    /// スレッドペインを開き、編集中および投稿不可チャンネルでなければ送信先をスレッド返信に切り替える。
    pub fn open_thread(&mut self, root_id: String) {
        self.thread = Some(ThreadState {
            root_id: root_id.clone(),
            selected: None,
        });
        self.thread_at_bottom = true;
        self.pending_thread_scroll_to_bottom = true;
        let can_post = self
            .current_channel
            .as_deref()
            .is_some_and(|id| self.can_post_in_channel(id));
        if !matches!(self.target, Target::Edit(_)) && can_post {
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

    /// 送信先を指定したスレッドへの返信に切り替える（編集中または投稿不可の場合は変更しない）。
    pub fn focus_thread_reply(&mut self, root_id: String) {
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルには返信できません");
            return;
        }
        if !self.can_post_in_channel(&channel_id) {
            self.notify("このチャンネルは管理者のみ投稿できます");
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
        self.switch_composer_context(Some(&channel_id), Target::Reply(root_id));
    }

    /// 送信先を編集または返信からキャンセルして通常投稿（編集中かつスレッド表示中ならスレッド返信）へ戻す。
    pub fn cancel_target(&mut self) {
        let can_post = self
            .current_channel
            .as_deref()
            .is_some_and(|id| self.can_post_in_channel(id));
        let next_target = match &self.target {
            Target::Edit(_) => match &self.thread {
                Some(thread) if can_post => Target::Reply(thread.root_id.clone()),
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
        self.pending_composer_text = Some((content, None));
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
        let ch_accessible = self.store.channel(&draft.channel_id).is_some_and(|ch| {
            ch.deleted_at.is_none()
                && ch.archive_at.is_none()
                && (!ch.is_private || self.store.is_member(&ch.id, &me_id))
        });
        let resolved_target = if self
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
            draft.target.clone()
        };
        let target_ch_writable = ch_accessible
            && (matches!(resolved_target, Target::Edit(_))
                || self.can_post_in_channel(&draft.channel_id));

        if target_ch_writable {
            self.failed_drafts.remove(index);
            if self.current_channel.as_deref() != Some(draft.channel_id.as_str()) {
                self.open_channel(draft.channel_id.clone());
            }
            if let Target::Reply(ref root_id) = resolved_target
                && self.thread.as_ref().map(|t| &t.root_id) != Some(root_id)
            {
                self.thread = Some(ThreadState {
                    root_id: root_id.clone(),
                    selected: None,
                });
            }
            if self.target != resolved_target {
                self.switch_composer_context(Some(&draft.channel_id), resolved_target.clone());
            }
            let current_atts: Vec<PendingAttachment> = self
                .current_channel_attachments()
                .into_iter()
                .filter(|a| a.attachment_id.is_some() && !a.is_uploading && a.error.is_none())
                .cloned()
                .collect();
            if !self.composer_draft.trim().is_empty() || !current_atts.is_empty() {
                let displaced = FailedDraft {
                    channel_id: draft.channel_id.clone(),
                    target: self.target.clone(),
                    content: std::mem::take(&mut self.composer_draft),
                    attachments: current_atts,
                    message_id: self.pending_composer_message_id.take(),
                };
                self.push_failed_draft(displaced);
            }
            let s_gen = self.session_gen;
            let d_ch = draft.channel_id.clone();
            self.pending_attachments
                .retain(|a| !(a.channel_id == d_ch && a.session_gen == s_gen));
            self.composer_draft = draft.content.clone();
            self.pending_composer_text = Some((draft.content, None));
            self.pending_composer_message_id = draft.message_id.clone();
            for mut att in draft.attachments {
                att.channel_id = draft.channel_id.clone();
                att.session_gen = self.session_gen;
                self.pending_attachments.push(att);
            }
            self.ensure_channel_cache();
        } else {
            let current_ch = self
                .current_channel
                .clone()
                .filter(|id| self.can_post_in_channel(id));
            if let Some(current_ch) = current_ch {
                self.failed_drafts.remove(index);
                if self.target != Target::Channel {
                    self.switch_composer_context(Some(&current_ch), Target::Channel);
                }
                let current_atts: Vec<PendingAttachment> = self
                    .current_channel_attachments()
                    .into_iter()
                    .filter(|a| a.attachment_id.is_some() && !a.is_uploading && a.error.is_none())
                    .cloned()
                    .collect();
                if !self.composer_draft.trim().is_empty() || !current_atts.is_empty() {
                    let displaced = FailedDraft {
                        channel_id: current_ch.clone(),
                        target: Target::Channel,
                        content: std::mem::take(&mut self.composer_draft),
                        attachments: current_atts,
                        message_id: self.pending_composer_message_id.take(),
                    };
                    self.push_failed_draft(displaced);
                }
                let s_gen = self.session_gen;
                let c_ch = current_ch.clone();
                self.pending_attachments
                    .retain(|a| !(a.channel_id == c_ch && a.session_gen == s_gen));
                self.composer_draft = draft.content.clone();
                self.pending_composer_text = Some((draft.content, None));
                // 送信先チャンネルが異なるため、元の送信 ID は引き継がず新規採番へリセットする
                self.pending_composer_message_id = None;
                // 添付ファイルは元チャンネルと異なるため引き継がない（DB 整合性維持のため）
                if !draft.attachments.is_empty() {
                    self.notify(
                        "元のチャンネルにアクセスできないため下書きを復元しました（添付ファイルはチャンネルが異なるため除外されました。再添付してください）",
                    );
                } else {
                    self.notify(
                        "元のチャンネルにアクセスできないため、現在のチャンネルの入力欄に下書きを復元しました",
                    );
                }
            } else {
                self.notify(
                    "元のチャンネルにアクセスできません。送信可能なチャンネルを開いてから下書きを復元してください",
                );
            }
        }
    }

    /// 入力欄のメッセージを送信または編集する。送信を開始した場合は `true` を返す。
    pub fn submit_message(&mut self, content: String) -> bool {
        let Some(channel_id) = self.current_channel.clone() else {
            self.notify("先にチャンネルを開いてください");
            return false;
        };
        // 送信中の同一ターゲットへの重複送信を防止する
        if self
            .in_flight_submissions
            .iter()
            .any(|(_, d)| d.channel_id == channel_id && d.target == self.target)
        {
            self.notify("メッセージを送信中です");
            return false;
        }
        let cur_attachments: Vec<&PendingAttachment> = self
            .pending_attachments
            .iter()
            .filter(|a| a.channel_id == channel_id && a.session_gen == self.session_gen)
            .collect();
        if content.trim().is_empty() && cur_attachments.is_empty() {
            return false;
        }
        if cur_attachments.iter().any(|a| a.is_uploading) {
            self.notify("添付ファイルをアップロード中です。完了をお待ちください");
            return false;
        }
        if cur_attachments.iter().any(|a| a.error.is_some()) {
            self.notify(
                "アップロードに失敗した添付ファイルがあります。削除するか再試行してください",
            );
            return false;
        }

        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルには投稿できません");
            return false;
        }
        if !matches!(self.target, Target::Edit(_)) && !self.can_post_in_channel(&channel_id) {
            self.notify("このチャンネルは管理者のみ投稿できます");
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
        self.saved_draft_message_ids
            .remove(&(channel_id.clone(), target.clone()));
        self.stop_typing();

        let (sent_attachments, remaining_attachments): (Vec<_>, Vec<_>) =
            if !matches!(target, Target::Edit(_)) {
                let s_gen = self.session_gen;
                let ch = channel_id.clone();
                self.pending_attachments
                    .drain(..)
                    .partition(|a| a.channel_id == ch && a.session_gen == s_gen)
            } else {
                (Vec::new(), std::mem::take(&mut self.pending_attachments))
            };
        self.pending_attachments = remaining_attachments;

        let attachment_ids: Vec<String> = sent_attachments
            .iter()
            .filter_map(|a| a.attachment_id.clone())
            .collect();
        let attachment_ids_opt = if attachment_ids.is_empty() {
            None
        } else {
            Some(attachment_ids)
        };

        if matches!(target, Target::Edit(_)) {
            let next_target = match &self.thread {
                Some(thread) if self.can_post_in_channel(&channel_id) => {
                    Target::Reply(thread.root_id.clone())
                }
                _ => Target::Channel,
            };
            self.switch_composer_context(Some(&channel_id), next_target);
        }

        let message_id = match &target {
            Target::Edit(_) => uuid::Uuid::new_v4().to_string(),
            _ => self
                .pending_composer_message_id
                .take()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        };

        self.next_submission_id = self.next_submission_id.wrapping_add(1);
        let submission_id = self.next_submission_id;
        self.in_flight_submissions.push((
            submission_id,
            FailedDraft {
                channel_id: channel_id.clone(),
                target: target.clone(),
                content: content.clone(),
                attachments: sent_attachments,
                message_id: Some(message_id.clone()),
            },
        ));

        let sent_attachment_ids = attachment_ids_opt.clone().unwrap_or_default();
        let session_gen = self.session_gen;
        self.spawn_session_task(async move {
            let id = message_id;
            let send_future = async {
                match &target {
                    Target::Channel => {
                        api.send_message_with_attachments(
                            &id,
                            &channel_id,
                            &content,
                            None,
                            attachment_ids_opt,
                        )
                        .await
                    }
                    Target::Reply(root_id) => {
                        api.send_message_with_attachments(
                            &id,
                            &channel_id,
                            &content,
                            Some(root_id),
                            attachment_ids_opt,
                        )
                        .await
                    }
                    Target::Edit(message_id) => api.edit_message(message_id, &content).await,
                }
            };
            let (result, is_timeout) =
                match tokio::time::timeout(std::time::Duration::from_secs(30), send_future).await {
                    Ok(res) => (res, false),
                    Err(_) => (
                        Err(anyhow::anyhow!("送信がタイムアウトしました (30秒)")),
                        true,
                    ),
                };
            match result {
                Ok(_) => {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::SubmissionSucceeded {
                            submission_id,
                            sent_attachment_ids,
                        },
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
                    } else if is_timeout {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Notice(
                                "送信がタイムアウトしました (30秒)。サーバーで作成済みの可能性があるため、タイムラインを確認してください".to_string(),
                            ),
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

    pub fn current_channel_attachments(&self) -> Vec<&PendingAttachment> {
        let Some(ch) = &self.current_channel else {
            return Vec::new();
        };
        self.pending_attachments
            .iter()
            .filter(|a| a.channel_id == *ch && a.session_gen == self.session_gen)
            .collect()
    }

    pub fn attachment_download_url(&self, attachment_id: &str) -> Option<String> {
        self.attachment_download_urls
            .get(attachment_id)
            .filter(|c| c.fetched_at.elapsed().as_secs() < c.ttl_secs)
            .map(|c| c.url.clone())
    }

    /// 添付ファイル URL の取得または事前再取得が必要かどうかを判定する。
    pub fn attachment_url_needs_refresh(&self, attachment_id: &str) -> bool {
        if self.fetching_attachment_urls.contains(attachment_id) {
            return false;
        }
        if let Some(failed_at) = self.failed_attachment_urls.get(attachment_id)
            && failed_at.elapsed().as_secs() < 30
        {
            return false;
        }
        match self.attachment_download_urls.get(attachment_id) {
            Some(c) => c.fetched_at.elapsed().as_secs() + 60 >= c.ttl_secs,
            None => true,
        }
    }

    pub fn ensure_attachment_url(&mut self, attachment_id: String) {
        if !self.attachment_url_needs_refresh(&attachment_id) {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        self.fetching_attachment_urls.insert(attachment_id.clone());
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        let att_id = attachment_id.clone();
        self.spawn_session_task(async move {
            match api.get_attachment_url(&att_id).await {
                Ok(res) => {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::AttachmentUrlResolved {
                            attachment_id: att_id,
                            download_url: res.download_url,
                        },
                    });
                }
                Err(err) => {
                    eprintln!("添付ファイル URL の解決に失敗しました: {err:#}");
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::AttachmentUrlFailed {
                            attachment_id: att_id,
                        },
                    });
                }
            }
        });
    }

    /// 添付ファイル URL を取得する。未取得または期限切れ間近（残り60秒以内）の場合は再取得をトリガーする。
    pub fn get_or_fetch_attachment_url(&mut self, attachment_id: &str) -> Option<String> {
        if self.attachment_url_needs_refresh(attachment_id) {
            self.ensure_attachment_url(attachment_id.to_string());
        }
        self.attachment_download_url(attachment_id)
    }

    /// 添付ファイル URL を開くか、取得中であれば解決完了時に開くキューへ登録する。
    pub fn open_or_queue_attachment(&mut self, attachment_id: &str) -> Option<String> {
        // 有効期限が 30 秒以上残っている場合のみ即座に開く
        if let Some(c) = self.attachment_download_urls.get(attachment_id)
            && c.fetched_at.elapsed().as_secs() + 30 < c.ttl_secs
        {
            return Some(c.url.clone());
        }
        // 明示的なクリック操作のため、失敗バックオフを解除して再取得を開始する
        self.failed_attachment_urls.remove(attachment_id);
        self.ensure_attachment_url(attachment_id.to_string());
        self.pending_open_attachments
            .insert(attachment_id.to_string());
        None
    }

    // --- 添付ファイル ---

    /// ローカルファイルパスから添付ファイルを追加する。
    pub fn add_attachment_from_path(&mut self, path: std::path::PathBuf) {
        if matches!(self.target, Target::Edit(_)) {
            self.notify("メッセージの編集中はファイルを添付できません");
            return;
        }
        if self.current_channel_attachments().len() >= 10 {
            self.notify("添付できるファイルは最大10件までです");
            return;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            self.notify("ファイル情報を取得できませんでした");
            return;
        };
        if !metadata.is_file() {
            self.notify("通常のファイルのみ添付できます");
            return;
        }
        let file_size = metadata.len() as i64;
        if file_size == 0 {
            self.notify("空のファイルは添付できません");
            return;
        }
        if file_size > 50 * 1024 * 1024 {
            self.notify("ファイルサイズが上限(50MB)を超えています");
            return;
        }
        let Some(channel_id) = self.current_channel.clone() else {
            self.notify("チャンネルを開いてからファイルを添付してください");
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let local_id = uuid::Uuid::new_v4().to_string();
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        if is_disallowed_extension(&file_name) {
            self.notify("セキュリティ上の理由により、このファイル形式はアップロードできません");
            return;
        }
        let content_type = guess_content_type(&file_name).to_string();

        self.pending_composer_message_id = None;
        self.pending_attachments.push(PendingAttachment {
            local_id: local_id.clone(),
            channel_id: channel_id.clone(),
            session_gen: self.session_gen,
            file_name: file_name.clone(),
            file_size,
            content_type: content_type.clone(),
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        let session_gen = self.session_gen;
        let tx = self.event_tx.clone();
        self.spawn_session_task(async move {
            let read_res = tokio::task::spawn_blocking(move || std::fs::read(&path)).await;
            let bytes = match read_res {
                Ok(Ok(bytes)) => bytes,
                _ => {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::AttachmentUploadFailed {
                            local_id,
                            error: "ファイルを読み込めませんでした".into(),
                        },
                    });
                    return;
                }
            };
            if bytes.len() > 50 * 1024 * 1024 {
                let _ = tx.send(DesktopEvent::App {
                    session_gen,
                    event: AppEvent::AttachmentUploadFailed {
                        local_id,
                        error: "ファイルサイズが上限(50MB)を超えています".into(),
                    },
                });
                return;
            }
            let file_size = bytes.len() as i64;
            let presign_res = api
                .presign_attachment(&channel_id, &file_name, file_size, &content_type)
                .await;
            match presign_res {
                Ok(presign) => {
                    let upload_res = api
                        .upload_attachment_bytes(&presign.upload_url, &content_type, bytes)
                        .await;
                    match upload_res {
                        Ok(()) => {
                            let _ = tx.send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::AttachmentUploadFinished {
                                    local_id,
                                    attachment_id: presign.attachment_id,
                                },
                            });
                        }
                        Err(err) => {
                            let _ = tx.send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::AttachmentUploadFailed {
                                    local_id,
                                    error: err.to_string(),
                                },
                            });
                        }
                    }
                }
                Err(err) => {
                    let is_unauthorized = matches!(
                        err.downcast_ref::<HttpError>(),
                        Some(HttpError::Unauthorized)
                    );
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::AttachmentUploadFailed {
                            local_id,
                            error: err.to_string(),
                        },
                    });
                    if is_unauthorized {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Unauthorized,
                        });
                    }
                }
            }
        });
    }

    /// バイナリデータから添付ファイルを追加し、非同期アップロードを開始する。
    pub fn add_attachment_from_bytes(
        &mut self,
        file_name: String,
        content_type: String,
        bytes: Vec<u8>,
    ) {
        if matches!(self.target, Target::Edit(_)) {
            self.notify("メッセージの編集中はファイルを添付できません");
            return;
        }
        if self.current_channel_attachments().len() >= 10 {
            self.notify("添付できるファイルは最大10件までです");
            return;
        }
        let Some(channel_id) = self.current_channel.clone() else {
            self.notify("チャンネルを開いてからファイルを添付してください");
            return;
        };
        let Some(api) = self.api.clone() else {
            return;
        };
        let local_id = uuid::Uuid::new_v4().to_string();
        let file_size = bytes.len() as i64;
        if file_size == 0 {
            self.notify("空のファイルは添付できません");
            return;
        }
        if file_size > 50 * 1024 * 1024 {
            self.notify("ファイルサイズが上限(50MB)を超えています");
            return;
        }
        self.pending_composer_message_id = None;
        self.pending_attachments.push(PendingAttachment {
            local_id: local_id.clone(),
            channel_id: channel_id.clone(),
            session_gen: self.session_gen,
            file_name: file_name.clone(),
            file_size,
            content_type: content_type.clone(),
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        let session_gen = self.session_gen;
        let tx = self.event_tx.clone();
        self.spawn_session_task(async move {
            let presign_res = api
                .presign_attachment(&channel_id, &file_name, file_size, &content_type)
                .await;
            match presign_res {
                Ok(presign) => {
                    let upload_res = api
                        .upload_attachment_bytes(&presign.upload_url, &content_type, bytes)
                        .await;
                    match upload_res {
                        Ok(()) => {
                            let _ = tx.send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::AttachmentUploadFinished {
                                    local_id,
                                    attachment_id: presign.attachment_id,
                                },
                            });
                        }
                        Err(err) => {
                            let _ = tx.send(DesktopEvent::App {
                                session_gen,
                                event: AppEvent::AttachmentUploadFailed {
                                    local_id,
                                    error: err.to_string(),
                                },
                            });
                        }
                    }
                }
                Err(err) => {
                    let is_unauthorized = matches!(
                        err.downcast_ref::<HttpError>(),
                        Some(HttpError::Unauthorized)
                    );
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::AttachmentUploadFailed {
                            local_id,
                            error: err.to_string(),
                        },
                    });
                    if is_unauthorized {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Unauthorized,
                        });
                    }
                }
            }
        });
    }

    /// アップロード待ち・完了済みの添付ファイルを削除する。
    pub fn remove_pending_attachment(&mut self, local_id: &str) {
        self.pending_attachments.retain(|a| a.local_id != local_id);
        self.pending_composer_message_id = None;
    }

    // --- メンション補完 ---

    /// 入力文字列とキャレット位置からメンション候補を算出する。
    pub fn update_mention_autocomplete(&mut self, text: &str, caret: usize) {
        let caret = caret.min(text.len());
        let prefix = &text[..caret];
        let Some(pos) = prefix.rfind('@') else {
            self.mention_autocomplete = None;
            return;
        };
        // @ の前が先頭、または空白・括弧・句読点であるか確認
        if pos > 0
            && let Some(c) = prefix[..pos].chars().last()
            && !c.is_whitespace()
            && !matches!(c, '(' | '[' | '{' | '「' | '『' | '（' | '、' | '。')
        {
            self.mention_autocomplete = None;
            return;
        }
        let query = &prefix[pos + 1..];
        // 空白や改行が含まれていたらキャンセル
        if query.contains(char::is_whitespace) {
            self.mention_autocomplete = None;
            return;
        }

        let query_lower = query.to_lowercase();
        let mut candidates = Vec::new();
        // @channel, @here
        if "channel".starts_with(&query_lower) {
            candidates.push(MentionCandidate {
                name: "channel".into(),
                display_name: "@channel (全員へ通知)".into(),
                is_special: true,
            });
        }
        if "here".starts_with(&query_lower) {
            candidates.push(MentionCandidate {
                name: "here".into(),
                display_name: "@here (オンライン中全員)".into(),
                is_special: true,
            });
        }

        // チャンネルメンバー
        if let Some(ch) = &self.current_channel {
            for member in self.store.channel_members(ch) {
                if member.user_name.to_lowercase().contains(&query_lower)
                    || member.display_name.to_lowercase().contains(&query_lower)
                {
                    candidates.push(MentionCandidate {
                        name: member.user_name.clone(),
                        display_name: format!("{} (@{})", member.display_name, member.user_name),
                        is_special: false,
                    });
                    if candidates.len() >= 8 {
                        break;
                    }
                }
            }
        }

        if candidates.is_empty() {
            self.mention_autocomplete = None;
        } else {
            self.mention_autocomplete = Some(MentionAutocompleteState {
                query: query.to_string(),
                start_byte: pos,
                end_byte: caret,
                candidates,
                selected_index: 0,
            });
        }
    }

    pub fn mention_select_next(&mut self) {
        if let Some(state) = &mut self.mention_autocomplete
            && !state.candidates.is_empty()
        {
            state.selected_index = (state.selected_index + 1) % state.candidates.len();
        }
    }

    pub fn mention_select_prev(&mut self) {
        if let Some(state) = &mut self.mention_autocomplete
            && !state.candidates.is_empty()
        {
            state.selected_index =
                (state.selected_index + state.candidates.len() - 1) % state.candidates.len();
        }
    }

    /// 選択中のメンション候補を確定し、置換後の新しい文字列とキャレット位置を返す。
    pub fn mention_confirm(&mut self) -> Option<(String, usize)> {
        let state = self.mention_autocomplete.take()?;
        let candidate = state.candidates.get(state.selected_index)?;
        let insert_text = format!("@{} ", candidate.name);
        let before = &self.composer_draft[..state.start_byte.min(self.composer_draft.len())];
        let after = if state.end_byte < self.composer_draft.len() {
            &self.composer_draft[state.end_byte..]
        } else {
            ""
        };
        let new_text = format!("{before}{insert_text}{after}");
        let new_caret = before.len() + insert_text.len();
        self.pending_composer_message_id = None;
        self.composer_draft = new_text.clone();
        self.pending_composer_text = Some((new_text.clone(), Some(new_caret)));
        Some((new_text, new_caret))
    }

    pub fn close_mention_autocomplete(&mut self) {
        self.mention_autocomplete = None;
    }

    // --- 絵文字ピッカー ---

    pub fn open_emoji_picker(&mut self) {
        self.emoji_picker_open = true;
        self.emoji_picker_query.clear();
    }

    pub fn close_emoji_picker(&mut self) {
        self.emoji_picker_open = false;
        self.emoji_picker_query.clear();
        self.pending_focus_composer = true;
    }

    pub fn insert_emoji(&mut self, emoji: &str, range: std::ops::Range<usize>) {
        let start = range.start.min(self.composer_draft.len());
        let end = range.end.min(self.composer_draft.len());
        let (start, end) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        let before = &self.composer_draft[..start];
        let after = &self.composer_draft[end..];
        let new_text = format!("{before}{emoji}{after}");
        let new_caret = before.len() + emoji.len();
        self.pending_composer_message_id = None;
        self.composer_draft = new_text.clone();
        self.pending_composer_text = Some((new_text, Some(new_caret)));
        self.close_emoji_picker();
    }

    // --- チャンネル作成モーダル ---

    pub fn open_create_channel_modal(&mut self) {
        self.create_channel_modal = Some(CreateChannelModal::default());
    }

    pub fn close_create_channel_modal(&mut self) {
        self.create_channel_modal = None;
        self.pending_focus_composer = true;
    }

    pub fn submit_create_channel(&mut self) {
        let Some(modal) = &mut self.create_channel_modal else {
            return;
        };
        if modal.in_flight {
            return;
        }
        let name = modal.channel_name.trim().to_string();
        if name.is_empty() {
            modal.error = Some("チャンネル名を入力してください".to_string());
            return;
        }
        if name.chars().count() > 80 {
            modal.error = Some("チャンネル名は80文字以内で入力してください".to_string());
            return;
        }
        let desc = if modal.description.trim().is_empty() {
            None
        } else {
            Some(modal.description.trim().to_string())
        };
        let is_private = modal.is_private;
        self.next_channel_create_id = self.next_channel_create_id.wrapping_add(1);
        let request_id = self.next_channel_create_id;
        modal.request_id = request_id;
        modal.in_flight = true;
        modal.error = None;

        let Some(api) = self.api.clone() else {
            return;
        };
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;

        self.spawn_session_task(async move {
            let res = api.create_channel(&name, desc.as_deref(), is_private).await;
            match res {
                Ok((channel, _)) => {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::ChannelCreated {
                            request_id,
                            channel_id: channel.id,
                        },
                    });
                }
                Err(err) => {
                    let _ = tx.send(DesktopEvent::App {
                        session_gen,
                        event: AppEvent::ChannelCreateFailed {
                            request_id,
                            error: err.to_string(),
                        },
                    });
                }
            }
        });
    }

    // --- メッセージ検索 (チャンネル内) ---

    pub fn open_message_search(&mut self) {
        self.message_search = Some(MessageSearchState::default());
    }

    pub fn close_message_search(&mut self) {
        self.message_search = None;
        self.pending_focus_composer = true;
    }

    pub fn update_message_search_query(&mut self, query: String) {
        let query_lower = query.trim().to_lowercase();
        let mut matches = Vec::new();
        if !query_lower.is_empty()
            && let Some(ch) = &self.current_channel
        {
            let channel_msgs = self.store.channel_messages(ch);
            for msg in channel_msgs {
                if msg.content.to_lowercase().contains(&query_lower) {
                    matches.push(msg.id.clone());
                }
            }
        }
        self.message_search = Some(MessageSearchState {
            query,
            matches,
            current_match_index: 0,
        });
        self.scroll_to_current_search_match();
    }

    pub fn search_select_next(&mut self) {
        if let Some(ms) = &mut self.message_search
            && !ms.matches.is_empty()
        {
            ms.current_match_index = (ms.current_match_index + 1) % ms.matches.len();
            self.scroll_to_current_search_match();
        }
    }

    pub fn search_select_prev(&mut self) {
        if let Some(ms) = &mut self.message_search
            && !ms.matches.is_empty()
        {
            ms.current_match_index =
                (ms.current_match_index + ms.matches.len() - 1) % ms.matches.len();
            self.scroll_to_current_search_match();
        }
    }

    fn scroll_to_current_search_match(&mut self) {
        let Some(ms) = &self.message_search else {
            return;
        };
        let Some(matched_id) = ms.matches.get(ms.current_match_index).cloned() else {
            return;
        };
        let Some(channel_id) = self.current_channel.clone() else {
            return;
        };
        if let Some(msg) = self.store.message(&channel_id, &matched_id).cloned() {
            if let Some(parent_id) = msg.parent_id {
                let thread_already_open =
                    self.thread.as_ref().is_some_and(|t| t.root_id == parent_id);
                if !thread_already_open {
                    self.open_thread(parent_id.clone());
                }
                if let Some(thread) = &mut self.thread {
                    thread.selected = Some(matched_id.clone());
                }
                self.pending_thread_scroll_to_bottom = false;
                self.thread_at_bottom = false;

                let roots = self.store.root_messages(&channel_id);
                if let Some(pos) = roots.iter().position(|m| m.id == parent_id) {
                    let anchor_pos = pos.saturating_sub(10);
                    self.timeline_anchor_first_id = roots.get(anchor_pos).map(|m| m.id.clone());
                } else {
                    self.timeline_anchor_first_id = Some(parent_id);
                }
            } else {
                let roots = self.store.root_messages(&channel_id);
                if let Some(pos) = roots.iter().position(|m| m.id == matched_id) {
                    let anchor_pos = pos.saturating_sub(10);
                    self.timeline_anchor_first_id = roots.get(anchor_pos).map(|m| m.id.clone());
                } else {
                    self.timeline_anchor_first_id = Some(matched_id.clone());
                }
            }
            self.timeline_at_bottom = false;
            self.timeline_render_limit = MAX_TIMELINE_RENDER_LIMIT;
            self.pending_scroll_to_message_id = Some(matched_id);
            self.ensure_channel_cache();
        }
    }

    // --- クイックチャンネルスイッチャー (Cmd+K) ---

    pub fn open_quick_switcher(&mut self) {
        self.quick_switcher = Some(QuickSwitcherState::default());
    }

    pub fn close_quick_switcher(&mut self) {
        self.quick_switcher = None;
        self.pending_focus_composer = true;
    }

    pub fn update_quick_switcher_query(&mut self, query: String) {
        if let Some(qs) = &mut self.quick_switcher {
            qs.query = query;
            qs.selected_index = 0;
        }
    }

    pub fn quick_switcher_select_next(&mut self) {
        let count = self.quick_switcher_candidates().len();
        if let Some(qs) = &mut self.quick_switcher
            && count > 0
        {
            qs.selected_index = (qs.selected_index + 1) % count;
        }
    }

    pub fn quick_switcher_select_prev(&mut self) {
        let count = self.quick_switcher_candidates().len();
        if let Some(qs) = &mut self.quick_switcher
            && count > 0
        {
            qs.selected_index = (qs.selected_index + count - 1) % count;
        }
    }

    pub fn quick_switcher_candidates(&self) -> Vec<(&Channel, String)> {
        let query = self
            .quick_switcher
            .as_ref()
            .map(|qs| qs.query.trim().to_lowercase())
            .unwrap_or_default();
        let me_id = self.me.as_ref().map(|m| m.id.as_str()).unwrap_or("");
        self.store
            .channels()
            .into_iter()
            .filter_map(|ch| {
                let label = self.store.channel_label(ch, me_id);
                if query.is_empty() || label.to_lowercase().contains(&query) {
                    Some((ch, label))
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn quick_switcher_confirm(&mut self) {
        let candidates = self.quick_switcher_candidates();
        let index = self
            .quick_switcher
            .as_ref()
            .map(|qs| qs.selected_index)
            .unwrap_or(0);
        if let Some((ch, _)) = candidates.get(index) {
            let ch_id = ch.id.clone();
            self.close_quick_switcher();
            self.open_channel(ch_id);
        } else {
            self.close_quick_switcher();
        }
    }

    // --- テーマ切り替え ---

    pub fn toggle_theme(&mut self) {
        self.theme_mode = self.theme_mode.toggle();
        self.notify(format!(
            "テーマを{}モードに切り替えました",
            self.theme_mode.label()
        ));
    }

    pub fn theme(&self) -> crate::theme::Theme {
        crate::theme::Theme::from_mode(self.theme_mode)
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
            attachments: Vec::new(),
            message_id: None,
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
        assert_eq!(
            state
                .pending_composer_text
                .as_ref()
                .map(|(s, _)| s.as_str()),
            Some("")
        );
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
        assert_eq!(
            state
                .pending_composer_text
                .as_ref()
                .map(|(s, _)| s.as_str()),
            Some("スレッド返信")
        );

        // 編集中に他メッセージの start_edit や open_channel を呼んでも拒否され編集内容が保護される
        state.on_composer_changed("編集途中の内容".into());
        state.open_channel("c2".into());
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert_eq!(state.target, Target::Edit("r1".into()));
        assert_eq!(state.composer_draft, "編集途中の内容");

        // 編集中にチャンネル c1 がアーカイブされた場合、編集内容は failed_drafts へ退避されて Target::Edit が解除され、別チャンネルへ移動できる
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "f", "archive_at": "2026-11-01 03:30:00" }),
            )]),
        ));
        assert_eq!(state.target, Target::Channel);
        assert!(state.composer_draft.is_empty());
        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "編集途中の内容");
        state.open_channel("c2".into());
        assert_eq!(state.current_channel.as_deref(), Some("c2"));
        // c1 のアーカイブを解除して元の状態に戻す
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "f" }),
            )]),
        ));
        state.open_channel("c1".into());
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
        state.failed_drafts.clear();

        state.request_delete("r1".into());
        assert_eq!(state.confirm_delete_message.as_deref(), Some("r1"));
        state.cancel_delete();
        assert!(state.confirm_delete_message.is_none());

        // 失敗した下書きの保存と復元:
        // c1 の通常投稿に下書き X を書いたあとスレッド返信を開き（X は saved_drafts[(c1, Channel)] に退避）、
        // 未参加の c2 宛て失敗下書きをフォールバック復元しても X は消えずに failed_drafts に保護される
        state.on_composer_changed("c1の退避下書きX".into());
        state.open_thread("m1".into());
        assert_eq!(state.target, Target::Reply("m1".into()));
        assert!(state.composer_draft.is_empty());
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: 1,
            channel_id: "c2".into(),
            target: Target::Channel,
            content: "未参加c2宛ての本文".into(),
        });
        assert_eq!(state.failed_drafts.len(), 1);
        state.restore_failed_draft();
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "未参加c2宛ての本文");
        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "c1の退避下書きX");
        state.failed_drafts.clear();
        state.on_composer_changed(String::new());
        state.close_thread();

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
                attachments: Vec::new(),
                message_id: None,
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

        // posting_policy が managers のチャンネル (c2, 作成者 u2) では、非管理者の u1 は新規投稿できないが、
        // 作成者である c1 や me_is_admin = true のときは投稿できる
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![
                (
                    "c1",
                    json!({ "id": "c1", "channel_name": "general", "is_private": "f", "created_user_id": "u1", "posting_policy": "managers" }),
                ),
                (
                    "c2",
                    json!({ "id": "c2", "channel_name": "secret", "is_private": "t", "created_user_id": "u2", "posting_policy": "managers" }),
                ),
            ]),
        ));
        assert!(state.can_post_in_channel("c1"));
        assert!(!state.can_post_in_channel("c2"));
        state.open_channel("c2".into());
        assert!(!state.submit_message("制限チャンネルへの投稿".into()));
        state.me_is_admin = true;
        assert!(state.can_post_in_channel("c2"));
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

    static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TempEnvGuard {
        key: &'static str,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TempEnvGuard {
        fn set(key: &'static str, val: &str) -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: 単体テスト実行用の環境変数一時変更。TEST_ENV_LOCK でテスト間直列化する。
            unsafe { std::env::set_var(key, val) };
            Self { key, _lock: lock }
        }
    }

    impl Drop for TempEnvGuard {
        fn drop(&mut self) {
            // SAFETY: 単体テスト終了後の環境変数復元。TEST_ENV_LOCK 保有下で実行。
            unsafe { std::env::remove_var(self.key) };
        }
    }

    #[tokio::test]
    async fn preserves_token_manager_persistence_after_connected_event_and_cleans_up_on_cancel() {
        use zoo2_client_core::auth::token::{self, StoredTokens};

        let dir = std::env::temp_dir().join(format!(
            "zoo2-desktop-app-token-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let token_path = token::token_path(&dir);
        let _env_guard = TempEnvGuard::set("ZOO2_DESKTOP_TOKEN_PATH", token_path.to_str().unwrap());

        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "zoo2-desktop".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let mut state = AppState::from_config(&config);
        assert_eq!(state.session_gen, 0);

        let active_gen = state.active_session_gen.clone();
        let tokens = StoredTokens {
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: "https://api.example".into(),
            access_token: "access-initial".into(),
            refresh_token: Some("refresh-initial".into()),
            expires_at: token::now() + 300,
        };
        let tm = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc.clone(),
            token_path.clone(),
            tokens,
            state.session_gen,
            active_gen,
        ));

        // 接続イベント (Connected) を受け取ると、セッション世代を維持したままトークンが保存され、
        // 接続後もトークンマネージャの save_if_active() による更新保存が可能であること
        state.on_auth_event(AuthEvent::Connected {
            credentials: Credentials::Bearer(tm.clone()),
            token_manager: Some(tm.clone()),
            me: Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
            is_admin: false,
        });

        assert!(state.login_phase.is_none());
        assert!(token_path.exists());
        assert!(state.token_manager.is_some());

        // 接続後、バックグラウンドでの refresh 発生時 (save_if_active) も現行セッション世代として正常に保存できること
        assert!(tm.save_if_active().unwrap());

        // 1. 以前から保存されていたトークンがある状態でデバイスログインを開始し、
        // 承認前に Esc 等でキャンセルされた場合、以前の正常なトークンファイルは削除されないこと
        state.login_phase = Some(LoginPhase::AwaitingApproval {
            verification_uri: "https://example.com/device".into(),
            user_code: "USER-PRE".into(),
        });
        assert!(token_path.exists());
        let old_gen = state.session_gen;
        state.cancel_device_login();
        assert_ne!(state.session_gen, old_gen);
        // 承認前キャンセルなので既存トークンは削除されず残る
        assert!(token_path.exists());
        assert_eq!(state.login_phase, Some(LoginPhase::Ready { error: None }));

        // 2. 承認後に今回のログイン試行が TOKEN_FILE_LOCK 下でトークンを保存・スロット記録した状態でキャンセルされた場合、
        // 今回保存されたトークンファイルが削除され、世代が進んで次回起動時の誤接続を防ぐこと
        tm.bind_to_session_gen(state.session_gen);
        assert!(
            tm.save_if_active_and_record(&state.pending_device_login_manager)
                .unwrap()
        );
        assert!(token_path.exists());
        assert!(state.pending_device_login_manager.lock().unwrap().is_some());
        state.login_phase = Some(LoginPhase::AwaitingApproval {
            verification_uri: "https://example.com/device".into(),
            user_code: "USER-POST".into(),
        });
        let old_gen = state.session_gen;
        state.cancel_device_login();
        assert_ne!(state.session_gen, old_gen);
        // 今回のログインが保存したトークンが削除される
        assert!(!token_path.exists());
        assert_eq!(state.login_phase, Some(LoginPhase::Ready { error: None }));

        // 3. キャンセル (bump_session_gen) 後に遅れて save_if_active_and_record が試みられた場合は、
        // 世代不一致でファイル保存もスロット記録も行われないこと
        assert!(
            !tm.save_if_active_and_record(&state.pending_device_login_manager)
                .unwrap()
        );
        assert!(!token_path.exists());
        assert!(state.pending_device_login_manager.lock().unwrap().is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn preserves_drafts_and_composer_during_shape_re_sync() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.composer_draft = "同期中に入力した本文".into();

        // Channels が 409 must-refetch 等で reset バッチを受信して同期未完了の間は、
        // can_post_in_channel は true を返し、入力欄や下書きを保持する
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            Batch {
                reset: true,
                changes: vec![],
                up_to_date: false,
            },
        ));
        assert!(!state.store.is_synced(&ShapeId::Channels));
        assert!(state.can_post_in_channel("c1"));
        assert_eq!(state.composer_draft, "同期中に入力した本文");
        assert!(state.failed_drafts.is_empty());

        // Channels が未同期の間に ChannelUsers のバッチが届いても、
        // 投稿制限やアーカイブと誤認して下書きを退避しない
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![(
                "cu1",
                json!({ "user_id": "u1", "channel_id": "c1" }),
            )]),
        ));
        assert!(state.can_post_in_channel("c1"));
        assert_eq!(state.composer_draft, "同期中に入力した本文");
        assert!(state.failed_drafts.is_empty());

        // Channels の同期が完了した後は通常の投稿判定が適用される
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "f" }),
            )]),
        ));
        assert!(state.store.is_synced(&ShapeId::Channels));
        assert!(state.store.is_synced(&ShapeId::ChannelUsers));
        assert!(state.can_post_in_channel("c1"));
        assert_eq!(state.composer_draft, "同期中に入力した本文");
    }

    #[test]
    fn manages_pending_attachments() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        assert!(state.pending_attachments.is_empty());

        state.add_attachment_from_bytes("photo.png".into(), "image/png".into(), vec![1, 2, 3, 4]);
        assert_eq!(state.pending_attachments.len(), 1);
        assert_eq!(state.pending_attachments[0].file_name, "photo.png");
        assert_eq!(state.pending_attachments[0].content_type, "image/png");

        let id = state.pending_attachments[0].local_id.clone();
        state.remove_pending_attachment(&id);
        assert!(state.pending_attachments.is_empty());
    }

    #[test]
    fn supports_mention_autocomplete() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 新メンバー u3 (charlie) を投入
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Users,
            batch(vec![(
                "u3",
                json!({
                    "id": "u3",
                    "user_name": "charlie",
                    "display_name": "Charlie Brown",
                    "role": "normal",
                    "deleted": false
                }),
            )]),
        ));
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![(
                "cu3",
                json!({ "user_id": "u3", "channel_id": "c1" }),
            )]),
        ));

        // @ch で補完開始 (@channel と @charlie の2件が候補になる)
        state.composer_draft = "Hello @ch".into();
        state.update_mention_autocomplete("Hello @ch", 9);
        assert!(state.mention_autocomplete.is_some());
        let ac = state.mention_autocomplete.as_ref().unwrap();
        assert_eq!(ac.candidates.len(), 2);
        assert_eq!(ac.candidates[0].name, "channel");
        assert_eq!(ac.candidates[1].name, "charlie");

        // 次の候補 (charlie) を選択して確定
        state.mention_select_next();
        let confirmed = state.mention_confirm();
        assert_eq!(confirmed, Some(("Hello @charlie ".to_string(), 15)));
        assert!(state.mention_autocomplete.is_none());
    }

    #[test]
    fn manages_create_channel_modal() {
        let mut state = connected_state();
        assert!(state.create_channel_modal.is_none());

        state.open_create_channel_modal();
        state.create_channel_modal.as_mut().unwrap().is_private = true;
        assert!(state.create_channel_modal.as_ref().unwrap().is_private);

        // 空文字バリデーション
        state.submit_create_channel();
        assert_eq!(
            state
                .create_channel_modal
                .as_ref()
                .unwrap()
                .error
                .as_deref(),
            Some("チャンネル名を入力してください")
        );

        // 81文字 (日本語) はエラー
        state.create_channel_modal.as_mut().unwrap().channel_name = "あ".repeat(81);
        state.submit_create_channel();
        assert_eq!(
            state
                .create_channel_modal
                .as_ref()
                .unwrap()
                .error
                .as_deref(),
            Some("チャンネル名は80文字以内で入力してください")
        );

        // 80文字 (日本語、UTF-8で240バイト) は80文字制限をパスして送信に進む
        state.create_channel_modal.as_mut().unwrap().channel_name = "あ".repeat(80);
        state.submit_create_channel();
        assert!(state.create_channel_modal.as_ref().unwrap().in_flight);

        state.close_create_channel_modal();
        assert!(state.create_channel_modal.is_none());

        // チャンネル作成試行 A
        state.open_create_channel_modal();
        state.create_channel_modal.as_mut().unwrap().channel_name = "channel-a".into();
        state.submit_create_channel();
        let req_id_a = state.create_channel_modal.as_ref().unwrap().request_id;
        assert!(req_id_a > 0);

        // 作成中にモーダルを閉じて、別のチャンネル B の入力を開始する
        state.close_create_channel_modal();
        state.open_create_channel_modal();
        state.create_channel_modal.as_mut().unwrap().channel_name = "channel-b".into();

        // 過去の試行 A の失敗イベントが届いても、モーダル B にエラーが混入しないこと
        state.on_app_event(AppEvent::ChannelCreateFailed {
            request_id: req_id_a,
            error: "Failed A".into(),
        });
        assert_eq!(
            state.create_channel_modal.as_ref().unwrap().channel_name,
            "channel-b"
        );
        assert!(state.create_channel_modal.as_ref().unwrap().error.is_none());

        // 過去の試行 A の作成完了イベントが届いても、モーダル B は閉じられず入力内容が保護されること
        state.on_app_event(AppEvent::ChannelCreated {
            request_id: req_id_a,
            channel_id: "c_new_a".into(),
        });
        assert!(state.create_channel_modal.is_some());
        assert_eq!(
            state.create_channel_modal.as_ref().unwrap().channel_name,
            "channel-b"
        );

        // チャンネル B を送信し、試行 B の作成完了イベントを受け取るとモーダルが閉じて遷移すること
        state.submit_create_channel();
        let req_id_b = state.create_channel_modal.as_ref().unwrap().request_id;
        assert_ne!(req_id_a, req_id_b);
        assert_eq!(state.next_channel_create_id, req_id_b);

        state.on_app_event(AppEvent::ChannelCreated {
            request_id: req_id_b,
            channel_id: "c_new_b".into(),
        });
        assert!(state.create_channel_modal.is_none());
        assert_eq!(state.current_channel.as_deref(), Some("c_new_b"));

        // 試行 B の成功後に、古い試行 A の完了・失敗イベントが届いても無視され、B の滞在と通知が維持されること
        state.on_app_event(AppEvent::ChannelCreated {
            request_id: req_id_a,
            channel_id: "c_new_a".into(),
        });
        assert_eq!(state.current_channel.as_deref(), Some("c_new_b"));
        state.on_app_event(AppEvent::ChannelCreateFailed {
            request_id: req_id_a,
            error: "Failed A".into(),
        });
        assert_eq!(state.current_channel.as_deref(), Some("c_new_b"));

        // チャンネル C を送信し、モーダルを閉じた後に最新試行 C の成功イベントが届いた場合、チャンネル遷移と通知が行われること
        state.open_create_channel_modal();
        state.create_channel_modal.as_mut().unwrap().channel_name = "channel-c".into();
        state.submit_create_channel();
        let req_id_c = state.create_channel_modal.as_ref().unwrap().request_id;
        assert_eq!(state.next_channel_create_id, req_id_c);
        state.close_create_channel_modal();
        assert!(state.create_channel_modal.is_none());

        state.on_app_event(AppEvent::ChannelCreated {
            request_id: req_id_c,
            channel_id: "c_new_c".into(),
        });
        assert_eq!(state.current_channel.as_deref(), Some("c_new_c"));
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("チャンネルを作成しました")
        );

        // チャンネル D を送信し、モーダルを閉じた後に最新試行 D の失敗イベントが届いた場合、Notice 通知が行われること
        state.open_create_channel_modal();
        state.create_channel_modal.as_mut().unwrap().channel_name = "channel-d".into();
        state.submit_create_channel();
        let req_id_d = state.create_channel_modal.as_ref().unwrap().request_id;
        state.close_create_channel_modal();
        assert!(state.create_channel_modal.is_none());

        state.on_app_event(AppEvent::ChannelCreateFailed {
            request_id: req_id_d,
            error: "Failed closed modal".into(),
        });
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("チャンネル作成に失敗しました: Failed closed modal")
        );
    }

    #[test]
    fn searches_messages_and_navigates_matches() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "search_m1",
                    json!({
                        "id": "search_m1",
                        "channel_id": "c1",
                        "user_id": "u1",
                        "content": "Rust and GPUI are awesome",
                        "created_at": "2026-11-01 03:02:00"
                    }),
                ),
                (
                    "search_m2",
                    json!({
                        "id": "search_m2",
                        "channel_id": "c1",
                        "user_id": "u1",
                        "content": "Testing search functionality",
                        "created_at": "2026-11-01 03:03:00"
                    }),
                ),
                (
                    "search_reply",
                    json!({
                        "id": "search_reply",
                        "channel_id": "c1",
                        "parent_id": "search_m1",
                        "user_id": "u1",
                        "content": "A threaded reply mentioning GPUI",
                        "created_at": "2026-11-01 03:04:00"
                    }),
                ),
            ]),
        ));

        state.open_message_search();
        assert!(state.message_search.is_some());

        // ルートメッセージだけでなくスレッド返信も網羅して検索できる
        state.update_message_search_query("GPUI".into());
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches.len(), 2);
        assert_eq!(ms.matches[0], "search_m1");
        assert_eq!(ms.matches[1], "search_reply");

        state.search_select_next();
        assert_eq!(
            state.message_search.as_ref().unwrap().current_match_index,
            1
        );
        // スレッド返信の一致へ移動した際、スレッドペインが開き、最下部スクロールフラグは立たず、一致返信が選択される
        assert_eq!(
            state.thread.as_ref().map(|t| t.root_id.as_str()),
            Some("search_m1")
        );
        assert_eq!(
            state.thread.as_ref().and_then(|t| t.selected.as_deref()),
            Some("search_reply")
        );
        assert!(!state.pending_thread_scroll_to_bottom);
        assert_eq!(
            state.pending_scroll_to_message_id.as_deref(),
            Some("search_reply")
        );

        state.search_select_prev();
        assert_eq!(
            state.message_search.as_ref().unwrap().current_match_index,
            0
        );
        assert_eq!(
            state.pending_scroll_to_message_id.as_deref(),
            Some("search_m1")
        );

        state.close_message_search();
        assert!(state.message_search.is_none());
    }

    #[test]
    fn inserts_emoji_at_caret_position() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.on_composer_changed("Hello world".into());
        // "Hello " の後ろ (cursor = 6) に "😀" を挿入 (選択範囲なし)
        state.insert_emoji("😀", 6..6);
        assert_eq!(
            state.pending_composer_text,
            Some(("Hello 😀world".to_string(), Some(6 + "😀".len())))
        );
        assert_eq!(state.composer_draft, "Hello 😀world");

        // 選択範囲 ("world") を置換して "🎉" を挿入
        let world_start = 6 + "😀".len();
        let world_end = world_start + "world".len();
        state.insert_emoji("🎉", world_start..world_end);
        assert_eq!(state.composer_draft, "Hello 😀🎉");
    }

    #[test]
    fn isolates_attachments_by_channel_and_preserves_on_failed_submit() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "local_1".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "test.png".into(),
            file_size: 1024,
            content_type: "image/png".into(),
            attachment_id: Some("att_1".into()),
            is_uploading: false,
            error: None,
        });
        assert_eq!(state.current_channel_attachments().len(), 1);

        // 別チャンネルを開くと c1 の添付は見えない
        state.open_channel("c2".into());
        assert_eq!(state.current_channel_attachments().len(), 0);

        // c1 に戻ると見える
        state.open_channel("c1".into());
        assert_eq!(state.current_channel_attachments().len(), 1);

        // ログアウト / セッションリセットでクリアされる
        state.clear_session_state();
        assert_eq!(state.pending_attachments.len(), 0);
    }

    #[test]
    fn validates_disallowed_extensions_and_url_expiry() {
        assert!(is_disallowed_extension("program.exe"));
        assert!(is_disallowed_extension("script.sh"));
        assert!(is_disallowed_extension("batch.bat"));
        assert!(is_disallowed_extension("library.dll"));
        assert!(!is_disallowed_extension("image.png"));
        assert!(!is_disallowed_extension("document.pdf"));

        // X-Amz-Expires の解析
        let url =
            "https://s3.amazonaws.com/bucket/key?X-Amz-Date=20261108T000000Z&X-Amz-Expires=300";
        assert_eq!(parse_url_expiry_secs(url), 270);

        // クエリがない場合はデフォルト 240 秒
        assert_eq!(
            parse_url_expiry_secs("https://s3.amazonaws.com/bucket/key"),
            240
        );
    }

    #[test]
    fn prevents_duplicate_channel_creation() {
        let mut state = connected_state();
        state.open_create_channel_modal();
        assert!(state.create_channel_modal.is_some());
        state.create_channel_modal.as_mut().unwrap().in_flight = true;
        state.create_channel_modal.as_mut().unwrap().channel_name = "test-chan".into();

        // in_flight の場合は送信をスキップ
        state.submit_create_channel();
        assert_eq!(state.background_tasks.len(), 0);
    }

    #[test]
    fn retains_unrelated_attachments_on_submission_succeeded() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_1".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "sent.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_sent".into()),
            is_uploading: false,
            error: None,
        });

        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_2".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "next.png".into(),
            file_size: 200,
            content_type: "image/png".into(),
            attachment_id: Some("att_next".into()),
            is_uploading: false,
            error: None,
        });

        // 送信中 submission をシミュレート
        state.in_flight_submissions.push((
            1,
            FailedDraft {
                channel_id: "c1".into(),
                target: Target::Channel,
                content: "送信メッセージ".into(),
                attachments: Vec::new(),
                message_id: None,
            },
        ));

        // SubmissionSucceeded で att_sent のみを送信完了通知
        state.on_app_event(AppEvent::SubmissionSucceeded {
            submission_id: 1,
            sent_attachment_ids: vec!["att_sent".into()],
        });

        // att_sent は削除され、後から追加された att_next は保持される
        assert_eq!(state.pending_attachments.len(), 1);
        assert_eq!(state.pending_attachments[0].local_id, "loc_2");
    }

    #[test]
    fn quick_switcher_and_theme_toggle() {
        let mut state = connected_state();
        assert_eq!(state.theme_mode, crate::theme::ThemeMode::Dark);

        // テーマ切り替え
        state.toggle_theme();
        assert_eq!(state.theme_mode, crate::theme::ThemeMode::Light);
        state.toggle_theme();
        assert_eq!(state.theme_mode, crate::theme::ThemeMode::Dark);

        // クイックスイッチャー
        state.open_quick_switcher();
        assert!(state.quick_switcher.is_some());

        state.update_quick_switcher_query("gen".into());
        assert!(!state.quick_switcher_candidates().is_empty());

        state.quick_switcher_select_next();
        state.quick_switcher_select_prev();
        state.quick_switcher_confirm();
        assert!(state.quick_switcher.is_none());
    }

    #[test]
    fn prevents_duplicate_submission_and_preserves_attachments() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_1".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "test.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_1".into()),
            is_uploading: false,
            error: None,
        });

        // 1回目の送信 (in_flight に入る)
        assert!(state.submit_message("Hello".into()));
        assert_eq!(state.in_flight_submissions.len(), 1);
        // pending_attachments から送信中添付が取り出されている
        assert!(state.pending_attachments.is_empty());

        // 送信中に2回目の送信を試行しても拒否される
        assert!(!state.submit_message("Hello again".into()));

        // 送信失敗時の RestoreDraft で下書き本文と添付ファイルが復元される
        let sub_id = state.in_flight_submissions[0].0;
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: sub_id,
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "Hello".into(),
        });
        assert_eq!(state.composer_draft, "Hello");
        assert_eq!(state.pending_attachments.len(), 1);
        assert_eq!(state.pending_attachments[0].local_id, "loc_1");
    }

    #[test]
    fn rejects_attachment_while_editing_and_resets_search_on_channel_switch() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 編集中はファイル添付が拒否される
        state.target = Target::Edit("m1".into());
        let temp_dir = std::env::temp_dir();
        let dummy_file = temp_dir.join("zoo2_test_dummy.txt");
        let _ = std::fs::write(&dummy_file, "hello");
        state.add_attachment_from_path(dummy_file.clone());
        let _ = std::fs::remove_file(&dummy_file);
        assert!(state.pending_attachments.is_empty());
        assert_eq!(
            state.notice.as_ref().map(|(msg, _)| msg.as_str()),
            Some("メッセージの編集中はファイルを添付できません")
        );

        // 検索とメンション補完を開いた状態で別チャンネルへ移動
        state.target = Target::Channel;
        state.open_message_search();
        state.update_message_search_query("test".into());
        assert!(state.message_search.is_some());
        state.mention_autocomplete = Some(MentionAutocompleteState {
            query: "alice".into(),
            start_byte: 0,
            end_byte: 6,
            candidates: vec![],
            selected_index: 0,
        });

        // チャンネル切り替えで検索とメンション補完がリセットされる
        state.open_channel("c2".into());
        assert!(state.message_search.is_none());
        assert!(state.mention_autocomplete.is_none());
    }

    #[test]
    fn queues_attachment_open_and_resolves_url() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 未解決の添付ファイルをクリック
        let res = state.open_or_queue_attachment("att_99");
        assert!(res.is_none());
        assert!(state.pending_open_attachments.contains("att_99"));

        // URL 解決イベントで pending_open_url に格納される
        state.on_app_event(AppEvent::AttachmentUrlResolved {
            attachment_id: "att_99".into(),
            download_url: "https://example.com/download/att_99?X-Amz-Expires=300".into(),
        });
        assert!(!state.pending_open_attachments.contains("att_99"));
        assert_eq!(
            state.pending_open_url,
            Some("https://example.com/download/att_99?X-Amz-Expires=300".into())
        );
    }

    #[test]
    fn preserves_draft_with_only_attachments_on_unauthorized_and_excludes_uploading() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        // 本文は空
        state.composer_draft = "   ".into();
        // 完了済み添付
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_ready".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "ready.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_ready".into()),
            is_uploading: false,
            error: None,
        });
        // アップロード中の添付
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_uploading".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "uploading.png".into(),
            file_size: 200,
            content_type: "image/png".into(),
            attachment_id: None,
            is_uploading: true,
            error: None,
        });
        // エラーの添付
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_error".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "error.png".into(),
            file_size: 300,
            content_type: "image/png".into(),
            attachment_id: None,
            is_uploading: false,
            error: Some("fail".into()),
        });

        // 認証切れイベントで退避
        state.on_app_event(AppEvent::Unauthorized);

        let preserved = state.preserved_unauthorized_drafts.as_ref().unwrap();
        assert_eq!(preserved.1.len(), 1);
        let draft = &preserved.1[0];
        // 完了済みの添付 1 件だけが退避され、アップロード中とエラーは除外されていること
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].local_id, "loc_ready");
    }

    #[test]
    fn restore_failed_draft_to_different_channel_excludes_attachments() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        // 存在しないチャンネル c_deleted への下書き（本文と添付）
        state.failed_drafts.push(FailedDraft {
            channel_id: "c_deleted".into(),
            target: Target::Channel,
            content: "下書き本文".into(),
            attachments: vec![PendingAttachment {
                local_id: "loc_1".into(),
                channel_id: "c_deleted".into(),
                session_gen: state.session_gen,
                file_name: "att.png".into(),
                file_size: 500,
                content_type: "image/png".into(),
                attachment_id: Some("att_1".into()),
                is_uploading: false,
                error: None,
            }],
            message_id: Some("old_id".into()),
        });

        // 復元を実行
        state.restore_failed_draft();

        // 本文は現在のチャンネル c1 に復元される
        assert_eq!(state.composer_draft, "下書き本文");
        // 別チャンネルのため添付は引き継がれず空になっていること
        assert!(state.pending_attachments.is_empty());
        // 別チャンネルへの復元のため送信 ID は引き継がれずリセットされること
        assert!(state.pending_composer_message_id.is_none());
        // 除外された旨の通知が出ていること
        assert!(
            state
                .notice
                .as_ref()
                .unwrap()
                .0
                .contains("添付ファイルはチャンネルが異なるため除外されました")
        );
    }

    #[test]
    fn attachment_url_refresh_logic_and_failure_backoff() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // キャッシュがない状態 -> リフレッシュ必要
        assert!(state.attachment_url_needs_refresh("att_x"));

        // キャッシュあり（残り200秒） -> リフレッシュ不要
        state.attachment_download_urls.insert(
            "att_x".into(),
            CachedAttachmentUrl {
                url: "https://example.com/att_x".into(),
                fetched_at: Instant::now(),
                ttl_secs: 300,
            },
        );
        assert!(!state.attachment_url_needs_refresh("att_x"));

        // キャッシュあり（残り50秒: 60秒未満） -> 事前リフレッシュ必要
        state.attachment_download_urls.insert(
            "att_expiring".into(),
            CachedAttachmentUrl {
                url: "https://example.com/att_expiring".into(),
                fetched_at: Instant::now() - std::time::Duration::from_secs(250),
                ttl_secs: 300,
            },
        );
        assert!(state.attachment_url_needs_refresh("att_expiring"));

        // 失敗イベントを受け取るとバックオフ期間（30秒）に入り、リフレッシュ要求を抑制する
        state.on_app_event(AppEvent::AttachmentUrlFailed {
            attachment_id: "att_fail".into(),
        });
        assert!(!state.attachment_url_needs_refresh("att_fail"));

        // バックオフ中であっても、非画像ピルのクリック (open_or_queue_attachment) でバックオフが解除され
        // 再取得が開始されてキューに登録される
        let open_res = state.open_or_queue_attachment("att_fail");
        assert!(open_res.is_none());
        assert!(!state.failed_attachment_urls.contains_key("att_fail"));
        assert!(state.fetching_attachment_urls.contains("att_fail"));
        assert!(state.pending_open_attachments.contains("att_fail"));
    }

    #[test]
    fn duplicate_submission_notifies_and_add_attachment_from_bytes_guards_edit() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 編集中に設定
        state.target = Target::Edit("m1".into());
        // 編集中は add_attachment_from_bytes が拒否される
        state.add_attachment_from_bytes("test.png".into(), "image/png".into(), vec![1, 2, 3]);
        assert!(state.pending_attachments.is_empty());
        assert_eq!(
            state.notice.as_ref().unwrap().0,
            "メッセージの編集中はファイルを添付できません"
        );

        // チャンネル送信に戻す
        state.target = Target::Channel;
        // 1回目送信
        assert!(state.submit_message("Hello 1".into()));
        // 2回目重複送信 -> 拒否され「メッセージを送信中です」と通知される
        assert!(!state.submit_message("Hello 2".into()));
        assert_eq!(state.notice.as_ref().unwrap().0, "メッセージを送信中です");
    }

    #[test]
    fn preserves_message_id_across_failed_draft_restore_and_resend() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 初回送信
        assert!(state.submit_message("初回送信メッセージ".into()));
        assert_eq!(state.in_flight_submissions.len(), 1);
        let first_submission_id = state.in_flight_submissions[0].0;
        let first_msg_id = state.in_flight_submissions[0]
            .1
            .message_id
            .clone()
            .expect("初回送信時に message_id が採番されていること");

        // 送信失敗 / タイムアウトによる RestoreDraft イベントを受信
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: first_submission_id,
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "初回送信メッセージ".into(),
        });

        // 下書き本文とともに message_id が復元されていること
        assert_eq!(state.composer_draft, "初回送信メッセージ");
        assert_eq!(
            state.pending_composer_message_id.as_deref(),
            Some(first_msg_id.as_str())
        );

        // 2回目の再送時、新しい UUID ではなく初回と同じ message_id が再利用されること
        assert!(state.submit_message("初回送信メッセージ".into()));
        assert_eq!(state.in_flight_submissions.len(), 1);
        let resend_submission_id = state.in_flight_submissions[0].0;
        let resend_msg_id = state.in_flight_submissions[0]
            .1
            .message_id
            .clone()
            .expect("再送時にも message_id が保持されていること");
        assert_eq!(resend_msg_id, first_msg_id);

        // サーバー側で重複 ID の再送が受理され、成功イベントを受信した場合
        state.on_app_event(AppEvent::SubmissionSucceeded {
            submission_id: resend_submission_id,
            sent_attachment_ids: Vec::new(),
        });
        assert!(state.in_flight_submissions.is_empty());
        assert!(state.composer_draft.is_empty());
        assert!(state.pending_composer_message_id.is_none());

        // 復元後に本文を変更した場合、再送用 ID がリセットされて新規採番されること
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: 999,
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "本文そのまま".into(),
        });
        state.pending_composer_message_id = Some("same-id".into());
        // 本文を編集
        state.on_composer_changed("本文を書き換えた".into());
        assert!(state.pending_composer_message_id.is_none());
        assert!(state.submit_message("本文を書き換えた".into()));
        let edited_submission_msg_id = state.in_flight_submissions[0].1.message_id.clone().unwrap();
        assert_ne!(edited_submission_msg_id, "same-id");
        state.in_flight_submissions.clear();

        // 復元後に添付を追加した場合も、再送用 ID がリセットされること
        state.pending_composer_message_id = Some("same-id".into());
        state.add_attachment_from_bytes("sample.png".into(), "image/png".into(), vec![1, 2]);
        assert!(state.pending_composer_message_id.is_none());

        // 復元後に添付を削除した場合も、再送用 ID がリセットされること
        let local_id = state.pending_attachments[0].local_id.clone();
        state.pending_composer_message_id = Some("same-id".into());
        state.remove_pending_attachment(&local_id);
        assert!(state.pending_composer_message_id.is_none());

        // メンション確定でも再送用 ID がリセットされること
        state.pending_composer_message_id = Some("same-id".into());
        state.composer_draft = "Hello @".into();
        state.update_mention_autocomplete("Hello @", 7);
        state.mention_confirm();
        assert!(state.pending_composer_message_id.is_none());

        // 絵文字挿入でも再送用 ID がリセットされること
        state.pending_composer_message_id = Some("same-id".into());
        state.insert_emoji("🎉", 0..0);
        assert!(state.pending_composer_message_id.is_none());

        // 空ファイル (0バイト) の追加は拒否されること
        state.add_attachment_from_bytes("empty.png".into(), "image/png".into(), vec![]);
        assert!(state.pending_attachments.is_empty());
        assert_eq!(
            state.notice.as_ref().unwrap().0,
            "空のファイルは添付できません"
        );
    }

    #[test]
    fn normalizes_image_content_types() {
        assert!(is_image_content_type("image/png"));
        assert!(is_image_content_type("image/jpeg"));
        assert!(is_image_content_type("image/gif"));
        assert!(is_image_content_type("image/webp"));
        assert!(is_image_content_type("Image/PNG"));
        assert!(is_image_content_type("image/png; charset=binary"));
        assert!(is_image_content_type(" IMAGE/JPEG ; foo=bar "));
        assert!(!is_image_content_type("application/pdf"));
        assert!(!is_image_content_type("text/plain"));
    }

    #[test]
    fn does_not_mix_failed_attachments_into_subsequent_attachment_only_draft() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 添付付き投稿 1 を送信中へセット
        let att1 = PendingAttachment {
            local_id: "local_1".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "file1.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_1".into()),
            is_uploading: false,
            error: None,
        };
        state.in_flight_submissions.push((
            1,
            FailedDraft {
                channel_id: "c1".into(),
                target: Target::Channel,
                content: "First post".into(),
                attachments: vec![att1.clone()],
                message_id: Some("msg_1".into()),
            },
        ));

        // 次の投稿として、本文は空のまま別の添付 2 を追加中
        let att2 = PendingAttachment {
            local_id: "local_2".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "file2.png".into(),
            file_size: 200,
            content_type: "image/png".into(),
            attachment_id: Some("att_2".into()),
            is_uploading: false,
            error: None,
        };
        state.pending_attachments.push(att2.clone());
        assert_eq!(state.composer_draft, "");
        assert_eq!(state.current_channel_attachments().len(), 1);

        // 投稿 1 の送信が失敗して RestoreDraft が届く
        state.on_app_event(AppEvent::RestoreDraft {
            submission_id: 1,
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "First post".into(),
        });

        // 現在の入力欄（添付 2 のみ）に失敗した添付 1 や本文が混ざらないこと
        assert_eq!(state.composer_draft, "");
        assert_eq!(state.pending_attachments.len(), 1);
        assert_eq!(state.pending_attachments[0].local_id, "local_2");

        // 失敗した投稿 1 は独立した failed_drafts に退避されていること
        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "First post");
        assert_eq!(state.failed_drafts[0].attachments.len(), 1);
        assert_eq!(state.failed_drafts[0].attachments[0].local_id, "local_1");
    }

    #[test]
    fn preserves_different_attachments_with_same_content_on_unauthorized() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        let att1 = PendingAttachment {
            local_id: "local_1".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "file1.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_1".into()),
            is_uploading: false,
            error: None,
        };
        // 送信中の下書き
        state.in_flight_submissions.push((
            1,
            FailedDraft {
                channel_id: "c1".into(),
                target: Target::Channel,
                content: "Same message".into(),
                attachments: vec![att1],
                message_id: Some("msg_1".into()),
            },
        ));

        // 同じ本文で別の添付 2 を入力欄にセット
        let att2 = PendingAttachment {
            local_id: "local_2".into(),
            channel_id: "c1".into(),
            session_gen: state.session_gen,
            file_name: "file2.png".into(),
            file_size: 200,
            content_type: "image/png".into(),
            attachment_id: Some("att_2".into()),
            is_uploading: false,
            error: None,
        };
        state.composer_draft = "Same message".into();
        state.pending_attachments.push(att2);

        // 認証切れ発生
        state.on_app_event(AppEvent::Unauthorized);

        // preserved_unauthorized_drafts に両方の下書きが退避されていること
        let preserved = state.preserved_unauthorized_drafts.as_ref().unwrap();
        assert_eq!(preserved.1.len(), 2);
        assert_eq!(preserved.1[0].attachments[0].local_id, "local_1");
        assert_eq!(preserved.1[1].attachments[0].local_id, "local_2");
    }
}
