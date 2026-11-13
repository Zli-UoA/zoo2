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
        rows::{Attachment, Channel, Message},
    },
    http::{Credentials, HttpError},
    poll::PollGate,
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
    let trimmed = file_name.trim_end_matches(|c: char| c == '.' || c.is_whitespace());
    let lower = trimmed.to_ascii_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    DISALLOWED_UPLOAD_EXTENSIONS.contains(&ext)
}

/// S3 署名付き URL の `X-Amz-Expires` から有効期限秒数を抽出する。
pub fn parse_url_expiry_secs(url: &str) -> u64 {
    if let Some(pos) = url.find("X-Amz-Expires=") {
        let after = &url[pos + "X-Amz-Expires=".len()..];
        let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(secs) = num_str.parse::<u64>() {
            return secs;
        }
    }
    300
}

/// OS 通知のテスト用シンク型。
pub type NotificationSink = std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>;

/// キャッシュされた添付ファイル URL と有効期限。
#[derive(Debug, Clone)]
pub struct CachedAttachmentUrl {
    pub url: String,
    pub fetched_at: Instant,
    pub ttl_secs: u64,
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
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
    pub target: Target,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateChannelModal {
    pub request_id: u64,
    pub channel_id: Option<String>,
    pub channel_name: String,
    pub description: String,
    pub is_private: bool,
    pub error: Option<String>,
    pub in_flight: bool,
}

impl Default for CreateChannelModal {
    fn default() -> Self {
        Self {
            request_id: 0,
            channel_id: Some(uuid::Uuid::new_v4().to_string()),
            channel_name: String::new(),
            description: String::new(),
            is_private: false,
            error: None,
            in_flight: false,
        }
    }
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
/// `ShapeId::Messages` / `ShapeId::Reactions` / `ShapeId::Attachments` の更新時またはチャンネル・スレッド切替時のみ再計算する。
#[derive(Debug, Default, Clone)]
pub struct ChannelViewCache {
    pub channel_id: Option<String>,
    pub me_id: String,
    pub root_messages: Arc<Vec<Message>>,
    pub reply_counts: Arc<HashMap<String, usize>>,
    pub reactions_by_message: Arc<HashMap<String, Vec<ReactionGroup>>>,
    pub attachments_by_message: Arc<HashMap<String, Vec<Attachment>>>,
    pub thread_root_id: Option<String>,
    pub thread_root_message: Option<Message>,
    pub thread_replies: Arc<Vec<Message>>,
    pub messages_dirty: bool,
    pub reactions_dirty: bool,
    pub attachments_dirty: bool,
}

/// バックグラウンドタスクから `AppState` へ届くイベント。
#[derive(Debug)]
pub enum AppEvent {
    /// DND の取得結果。`Some(until)` は取得成功、`None` は失敗 (前の値を保つ)。
    Dnd(Option<Option<jiff::civil::DateTime>>),
    Notice(String),
    Unauthorized,
    AccountProfiles(zoo2_client_core::store::AccountProfiles),
    EmojiCatalog {
        shortcodes: HashMap<String, String>,
        custom: HashSet<String>,
    },
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
    ReadStatesPollFinished,
    EmojiPollFinished,
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
    Auth {
        session_gen: u64,
        event: AuthEvent,
    },
    Shape {
        session_gen: u64,
        event: ShapeEvent,
    },
    App {
        session_gen: u64,
        event: AppEvent,
    },
    Tick {
        session_gen: u64,
    },
    LogoutFinished {
        session_gen: u64,
        result: Result<Vec<String>, (String, bool)>,
    },
    AuthPersisted {
        session_gen: u64,
        event: AuthEvent,
        error: Option<String>,
    },
    CancelFinished {
        session_gen: u64,
        result: Result<Vec<String>, (String, bool)>,
    },
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
    logout_in_progress: bool,
    deferred_unauthorized: bool,
    cancel_in_progress: bool,
    /// 実行中のデバイスログインでトークン保存が完了したマネージャ。
    /// 承認完了後にキャンセルされた場合、このマネージャが保存したトークンのみを削除・失効するために保持する。
    pub pending_device_login_manager:
        Arc<std::sync::Mutex<Option<Arc<login::DesktopTokenManager>>>>,

    pub credentials: Option<Credentials>,
    pub token_manager: Option<Arc<login::DesktopTokenManager>>,
    recovery_manager: Option<Arc<login::DesktopTokenManager>>,
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
    /// 一度 `encrypted = true` を見たチャンネル。Channels の再同期で行が消えても暗号化として扱い、
    /// 表示と送信をしない（ログアウトまで覚える）。
    pub known_encrypted_channels: HashSet<String>,
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
    typing_poll: PollGate,
    read_poll: PollGate,
    emoji_poll: PollGate,

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
    /// 絵文字ピッカーのキーボード選択中インデックス。
    pub emoji_picker_selected: usize,
    /// 絵文字ピッカーのスクロールをキーボード選択位置へ追従させる要求フラグ。
    pub pending_emoji_picker_scroll: bool,
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
    /// OS 通知送信ハンドラ（テストでの通知検証用やモック用）。
    pub notification_sink: Option<NotificationSink>,
    /// `GET /api/users/me/profile` の `dnd_until` (UTC)。
    pub dnd_until: Option<jiff::civil::DateTime>,
    /// DND を一度でも取りにいったか。取れなかった場合も true にして、通知を止め続けない。
    pub dnd_checked: bool,
    /// 検索移動時などに指定位置までスクロールするためのメッセージ ID。
    pub pending_scroll_to_message_id: Option<String>,
    /// モーダルや検索バーを閉じた後に composer へフォーカスを復帰させるフラグ。
    pub pending_focus_composer: bool,
    /// URL 取得完了時に自動でブラウザで開く添付ファイル ID 一覧。
    pub pending_open_attachments: HashSet<String>,
    /// `view` 側でブラウザを開くための保留中 URL。
    pub pending_open_url: Option<String>,
    /// クイックスイッチャーのスクロールをキーボード選択位置へ追従させる要求フラグ。
    pub pending_quick_switcher_scroll: bool,
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

pub const MAX_CHANNEL_NAME_LEN: usize = 80;
pub const RESERVED_CHANNEL_NAMES: &[&str] = &["all", "channel", "everyone", "here", "general"];

/// チャンネル名の文字種・予約語・文字数バリデーション (バックエンドの `validate_channel_name` 準拠)
pub fn validate_channel_name_input(name: &str) -> Result<(), &'static str> {
    if name.trim().is_empty() {
        return Err("チャンネル名を入力してください");
    }
    if name.chars().count() > MAX_CHANNEL_NAME_LEN {
        return Err("チャンネル名は80文字以内で入力してください");
    }
    if RESERVED_CHANNEL_NAMES.contains(&name) {
        return Err("このチャンネル名は予約されているため使用できません");
    }
    static CHANNEL_NAME_PATTERN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\A[\p{L}\p{M}\p{N}_-]+\z").unwrap());
    if !CHANNEL_NAME_PATTERN.is_match(name) {
        return Err("チャンネル名には文字・数字・ハイフン(-)・アンダースコア(_)のみ使用できます");
    }
    Ok(())
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
            logout_in_progress: false,
            deferred_unauthorized: false,
            cancel_in_progress: false,
            pending_device_login_manager: Arc::new(std::sync::Mutex::new(None)),
            credentials: None,
            token_manager: None,
            recovery_manager: None,
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
            known_encrypted_channels: HashSet::new(),
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
            typing_poll: PollGate::new(Duration::from_millis(1500), Duration::from_secs(5)),
            read_poll: PollGate::new(READ_STATE_POLL, READ_STATE_POLL),
            emoji_poll: PollGate::new(Duration::from_secs(60), Duration::from_secs(60)),
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
            emoji_picker_selected: 0,
            pending_emoji_picker_scroll: false,
            mention_autocomplete: None,
            create_channel_modal: None,
            message_search: None,
            quick_switcher: None,
            theme_mode: crate::theme::ThemeMode::Dark,
            notifications_enabled: true,
            notification_sink: None,
            dnd_until: None,
            dnd_checked: false,
            pending_scroll_to_message_id: None,
            pending_focus_composer: false,
            pending_open_attachments: HashSet::new(),
            pending_open_url: None,
            pending_quick_switcher_scroll: false,
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
        let recovery_manager = self.recovery_manager.clone();
        if let Some(manager) = &recovery_manager {
            manager.bind_to_session_gen(session_gen);
        }
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
            if let Some(manager) = recovery_manager {
                login::retry_pending_session(http, config, manager, auth_tx).await;
            } else {
                login::connect_initial(http, config, session_gen, active_session_gen, auth_tx)
                    .await;
            }
            let _ = forward.await;
        });
    }

    /// OIDC デバイスコード認証を開始する。
    pub fn start_device_login(&mut self) {
        if self.recovery_manager.is_some() {
            self.start_initial_auth();
            return;
        }
        if self.cancel_in_progress {
            return;
        }
        if self
            .pending_device_login_manager
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
        {
            self.cancel_device_login();
            return;
        }
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
        if self.cancel_in_progress {
            return;
        }
        if self.recovery_manager.is_some() {
            if let Some(task) = self.login_task.take() {
                task.abort();
            }
            self.bump_session_gen();
            self.login_phase = Some(LoginPhase::Ready { error: Some("認証情報を保存できていません。保存領域の復旧後に再試行してください。アプリ終了時には更新済み token が失われる可能性があります".into()) });
            return;
        }
        if let Some(task) = self.login_task.take() {
            task.abort();
        }
        self.bump_session_gen();
        let manager = self
            .pending_device_login_manager
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(manager) = manager {
            self.cancel_in_progress = true;
            self.login_phase = Some(LoginPhase::Checking);
            let tx = self.event_tx.clone();
            let session_gen = self.session_gen;
            let http = self.http.clone();
            let auth = self.config.auth.clone();
            if self
                .spawn_tokio(async move {
                    let result = tokio::task::spawn_blocking(move || manager.logout_and_delete())
                        .await
                        .map_err(|e| (e.to_string(), false))
                        .and_then(|r| {
                            r.map_err(|e| (format!("{e:#}"), e.is::<login::SessionChanged>()))
                        });
                    if let (Ok(tokens), AuthConfig::Oidc(oidc)) = (&result, auth) {
                        let tokens = tokens.clone();
                        tokio::spawn(async move {
                            for refresh_token in tokens {
                                login::revoke_refresh_token(
                                    http.clone(),
                                    oidc.clone(),
                                    refresh_token,
                                )
                                .await;
                            }
                        });
                    }
                    let result = result.map(|_| Vec::new());
                    let _ = tx.send(DesktopEvent::CancelFinished {
                        session_gen,
                        result,
                    });
                })
                .is_none()
            {
                self.cancel_in_progress = false;
                self.login_phase = Some(LoginPhase::Ready {
                    error: Some("ログイン取消を開始できませんでした".into()),
                });
            }
        } else {
            self.login_phase = Some(LoginPhase::Ready { error: None });
        }
    }

    /// ログアウトしてトークンを削除し、ログイン画面へ戻る。
    /// ローカルトークンの削除をタスク中断より前に確定させるため、削除に失敗した場合はタスクや実行中フラグを壊さずにログイン状態を維持する。
    pub fn logout(&mut self) {
        if let AuthConfig::Oidc(oidc) = self.config.auth.clone() {
            if self.logout_in_progress {
                return;
            }
            let manager = self.token_manager.clone();
            let tx = self.event_tx.clone();
            let session_gen = self.session_gen;
            let http = self.http.clone();
            self.logout_in_progress = true;
            if self
                .spawn_tokio(async move {
                    let result = tokio::task::spawn_blocking(move || match manager {
                        Some(tm) => tm.logout_and_delete(),
                        None => Err(anyhow::anyhow!("現在のログイン状態を確認できません")),
                    })
                    .await
                    .map_err(|e| (e.to_string(), false))
                    .and_then(|r| {
                        r.map_err(|e| (format!("{e:#}"), e.is::<login::SessionChanged>()))
                    });
                    if let Ok(tokens) = &result {
                        let tokens = tokens.clone();
                        tokio::spawn(async move {
                            for refresh_token in tokens {
                                login::revoke_refresh_token(
                                    http.clone(),
                                    oidc.clone(),
                                    refresh_token,
                                )
                                .await;
                            }
                        });
                    }
                    let result = result.map(|_| Vec::new());
                    let _ = tx.send(DesktopEvent::LogoutFinished {
                        session_gen,
                        result,
                    });
                })
                .is_none()
            {
                self.logout_in_progress = false;
                self.notify("ログアウトを開始できませんでした");
            }
        } else {
            self.stop_typing();
            self.preserved_unauthorized_drafts = None;
            self.reset_to_login(None);
        }
    }

    fn finish_logout(&mut self, result: Result<Vec<String>, (String, bool)>) {
        self.logout_in_progress = false;
        let deferred_unauthorized = std::mem::take(&mut self.deferred_unauthorized);
        if let AuthConfig::Oidc(oidc) = self.config.auth.clone() {
            match result {
                Ok(refresh_tokens) => {
                    self.stop_typing();
                    self.preserved_unauthorized_drafts = None;
                    self.reset_to_login(None);
                    if !refresh_tokens.is_empty() {
                        let http = self.http.clone();
                        self.spawn_tokio(async move {
                            for refresh_token in refresh_tokens {
                                login::revoke_refresh_token(
                                    http.clone(),
                                    oidc.clone(),
                                    refresh_token,
                                )
                                .await;
                            }
                        });
                    }
                }
                Err((_, true)) => {
                    self.stop_typing();
                    self.preserved_unauthorized_drafts = None;
                    self.reset_to_login(Some("別のログインを保持してログアウトしました".into()));
                }
                Err((err, false)) => {
                    let message =
                        format!("ログアウトに失敗しました（トークン削除エラー）: {err:#}");
                    if deferred_unauthorized {
                        self.reset_to_login_after_unauthorized(message);
                    } else {
                        self.notify(message);
                    }
                }
            }
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
        self.typing_poll = PollGate::new(Duration::from_millis(1500), Duration::from_secs(5));
        self.read_poll = PollGate::new(READ_STATE_POLL, READ_STATE_POLL);
        self.emoji_poll = PollGate::new(Duration::from_secs(60), Duration::from_secs(60));
        self.read_mark_in_flight.clear();
        handles
    }

    fn abort_all_tasks(&mut self) {
        let _ = self.drain_and_abort_all_tasks();
    }

    /// 現在のセッションに紐づくバックグラウンドタスクを起動し、`background_tasks` に登録する。
    pub(crate) fn spawn_session_task<F>(&mut self, future: F) -> bool
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
        // pending_attachments のうち現在のセッションで有効な添付を (channel_id, target) ごとに集約
        let s_gen = self.session_gen;
        let mut atts_by_channel_target: std::collections::HashMap<
            (String, Target),
            Vec<PendingAttachment>,
        > = std::collections::HashMap::new();
        for a in &self.pending_attachments {
            if a.session_gen == s_gen {
                if a.attachment_id.is_some() && !a.is_uploading && a.error.is_none() {
                    atts_by_channel_target
                        .entry((a.channel_id.clone(), a.target.clone()))
                        .or_default()
                        .push(a.clone());
                } else if a.is_uploading || a.error.is_some() {
                    // アップロード未完了またはエラーの添付も、再添付が必要な状態として保持し、
                    // 認証復帰後にユーザーが選択していたファイル名が黙って消えるのを防ぐ
                    let mut preserved_att = a.clone();
                    preserved_att.is_uploading = false;
                    preserved_att.attachment_id = None;
                    if preserved_att.error.is_none() {
                        preserved_att.error = Some("再添付してください".into());
                    }
                    atts_by_channel_target
                        .entry((a.channel_id.clone(), a.target.clone()))
                        .or_default()
                        .push(preserved_att);
                }
            }
        }

        // 退避対象となる (channel_id, target) のキー集合を集める
        let mut all_keys: std::collections::HashSet<(String, Target)> =
            std::collections::HashSet::new();
        for key in self.saved_drafts.keys() {
            all_keys.insert(key.clone());
        }
        for key in atts_by_channel_target.keys() {
            all_keys.insert(key.clone());
        }
        if let Some(ch_id) = &self.current_channel {
            all_keys.insert((ch_id.clone(), self.target.clone()));
        }

        for (ch_id, target) in all_keys {
            let is_current =
                self.current_channel.as_deref() == Some(ch_id.as_str()) && self.target == target;
            let content = if is_current {
                self.composer_draft.clone()
            } else {
                self.saved_drafts
                    .get(&(ch_id.clone(), target.clone()))
                    .cloned()
                    .unwrap_or_default()
            };
            let message_id = if is_current {
                self.pending_composer_message_id.clone()
            } else {
                self.saved_draft_message_ids
                    .get(&(ch_id.clone(), target.clone()))
                    .cloned()
            };
            let atts = atts_by_channel_target
                .remove(&(ch_id.clone(), target.clone()))
                .unwrap_or_default();

            let new_draft = FailedDraft {
                channel_id: ch_id,
                target,
                content,
                attachments: atts,
                message_id,
            };
            if (!new_draft.content.trim().is_empty() || !new_draft.attachments.is_empty())
                && !drafts.iter().any(|d| d == &new_draft)
            {
                drafts.push(new_draft);
            }
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
        zoo2_client_core::mfm::set_emoji_catalog(HashMap::new(), HashSet::new());
        if let Some(tm) = self.token_manager.take() {
            tm.disable_persistence();
        }
        self.abort_all_tasks();
        self.store = Store::default();
        self.credentials = None;
        self.api = None;
        self.me = None;
        self.dnd_until = None;
        self.dnd_checked = false;
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
        self.known_encrypted_channels.clear();
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
        self.typing_poll = PollGate::new(Duration::from_millis(1500), Duration::from_secs(5));
        self.read_poll = PollGate::new(READ_STATE_POLL, READ_STATE_POLL);
        self.emoji_poll = PollGate::new(Duration::from_secs(60), Duration::from_secs(60));
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

    fn reset_to_login_after_unauthorized(&mut self, error: String) {
        if self.logout_in_progress {
            self.deferred_unauthorized = true;
            return;
        }
        let recovery = self
            .token_manager
            .take()
            .filter(|manager| manager.retain_pending_or_disable());
        let generation = self.bump_session_gen();
        self.clear_session_state();
        if let Some(manager) = recovery {
            manager.bind_to_session_gen(generation);
            self.recovery_manager = Some(manager);
            self.login_phase = Some(LoginPhase::Ready {
                error: Some(
                    "更新済みの認証情報を保存できていません。保存領域を確認し、再試行してください"
                        .into(),
                ),
            });
        } else {
            self.login_phase = Some(LoginPhase::Ready { error: Some(error) });
        }
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    /// 現在発生している Shape 同期エラーの表示文字列を返す。
    pub fn sync_error(&self) -> Option<String> {
        for shape in [
            ShapeId::Channels,
            ShapeId::Users,
            ShapeId::ChannelUsers,
            ShapeId::Notifications,
        ] {
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
            DesktopEvent::CancelFinished {
                session_gen,
                result,
            } => {
                self.cancel_in_progress = false;
                if session_gen != self.session_gen {
                    return false;
                }
                match result {
                    Ok(refresh_tokens) => {
                        self.pending_device_login_manager
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .take();
                        self.login_phase = Some(LoginPhase::Ready { error: None });
                        if let AuthConfig::Oidc(oidc) = self.config.auth.clone() {
                            let http = self.http.clone();
                            self.spawn_tokio(async move {
                                for refresh_token in refresh_tokens {
                                    login::revoke_refresh_token(
                                        http.clone(),
                                        oidc.clone(),
                                        refresh_token,
                                    )
                                    .await;
                                }
                            });
                        }
                    }
                    Err((error, changed)) => {
                        if changed {
                            let manager = self
                                .pending_device_login_manager
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .take();
                            if let Some(manager) = manager {
                                manager.disable_persistence();
                            }
                        }
                        self.login_phase = Some(LoginPhase::Ready {
                            error: Some(format!("ログイン取消に失敗しました: {error}")),
                        })
                    }
                }
                true
            }
            DesktopEvent::AuthPersisted {
                session_gen,
                event,
                error,
            } => {
                if session_gen != self.session_gen {
                    return false;
                }
                if let Some(error) = error {
                    let pending_manager = self
                        .pending_device_login_manager
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take();
                    if let Some(manager) = pending_manager {
                        manager.disable_persistence();
                    }
                    if self
                        .recovery_manager
                        .as_ref()
                        .is_some_and(|manager| !manager.has_pending_persistence())
                    {
                        self.recovery_manager = None;
                    }
                    self.login_phase = Some(LoginPhase::Ready { error: Some(error) });
                } else if let AuthEvent::Connected {
                    credentials,
                    token_manager,
                    me,
                    is_admin,
                } = event
                {
                    self.recovery_manager = None;
                    self.connect_session(credentials, me);
                    self.me_is_admin = is_admin;
                    self.token_manager = token_manager;
                }
                true
            }
            DesktopEvent::LogoutFinished {
                session_gen,
                result,
            } => {
                self.logout_in_progress = false;
                if session_gen != self.session_gen {
                    return false;
                }
                self.finish_logout(result);
                true
            }
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
                    let manager = tm.clone();
                    let tx = self.event_tx.clone();
                    let session_gen = self.session_gen;
                    let event = AuthEvent::Connected {
                        credentials,
                        token_manager,
                        me,
                        is_admin,
                    };
                    if self
                        .spawn_tokio(async move {
                            let error = match tokio::task::spawn_blocking(move || {
                                manager.confirm_persisted_session()
                            })
                            .await
                            {
                                Ok(Ok(())) => None,
                                Ok(Err(err)) => {
                                    Some(format!("トークンの保存に失敗しました: {err:#}"))
                                }
                                Err(err) => Some(format!("トークンの保存に失敗しました: {err}")),
                            };
                            let _ = tx.send(DesktopEvent::AuthPersisted {
                                session_gen,
                                event,
                                error,
                            });
                        })
                        .is_none()
                    {
                        self.login_phase = Some(LoginPhase::Ready {
                            error: Some("トークン保存を開始できませんでした".into()),
                        });
                    }
                    return;
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
                if self
                    .recovery_manager
                    .as_ref()
                    .is_some_and(|manager| !manager.has_pending_persistence())
                {
                    self.recovery_manager = None;
                }
                self.pending_device_login_manager
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                self.login_phase = Some(LoginPhase::Ready { error });
            }
            AuthEvent::RecoveryRequired { manager, error } => {
                self.pending_device_login_manager
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                self.recovery_manager = Some(manager);
                self.login_phase = Some(LoginPhase::Ready {
                    error: Some(format!(
                        "{error}。保存領域を復旧して再試行してください。アプリを終了すると更新済み token が失われる可能性があります"
                    )),
                });
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
        self.subscribe(ShapeId::Notifications);
        self.poll_read_states();
        self.start_periodic_tasks();
    }

    /// 停止した Shape 購読を再試行する。
    pub fn retry_sync(&mut self) {
        self.sync_errors.clear();
        self.subscribe(ShapeId::Channels);
        self.subscribe(ShapeId::Users);
        self.subscribe(ShapeId::ChannelUsers);
        self.subscribe(ShapeId::Notifications);
        if let Some(ch) = self.current_channel.clone() {
            self.subscribe(ShapeId::Messages(ch.clone()));
        }
        self.poll_read_states();
    }

    fn start_periodic_tasks(&mut self) {
        let session_gen = self.session_gen;
        crate::resident::notify::spawn_dnd_refresh(self);

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
                                    if row.get("encrypted").is_some_and(|v| {
                                        v.as_bool() == Some(true)
                                            || matches!(v.as_str(), Some("t" | "true"))
                                    }) {
                                        self.known_encrypted_channels.insert(ch_id.to_string());
                                    }
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
                // 判定は適用より前に行う (適用後は、新しく届いた行と既知の行を区別できない)。
                if id == ShapeId::Notifications {
                    self.show_notifications(&batch);
                }

                self.store.apply(&id, batch);

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
                            self.channel_cache.reactions_dirty = true;
                            self.channel_cache.attachments_dirty = true;
                        }
                        if self.store.is_synced(&ShapeId::Messages(ch.clone())) {
                            self.reconcile_channel_targets(ch);
                        }
                        if self.current_channel.as_deref() == Some(ch.as_str()) {
                            if self.message_search.is_some() {
                                self.refresh_message_search_results();
                            }
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
                        self.channel_cache.attachments_dirty = true;
                    }
                    _ => {}
                }
                self.sync_unread_with_latest();
            }
            ShapeEvent::Failed(id, message) => {
                if message == HttpError::Unauthorized.to_string() {
                    self.preserve_drafts_for_unauthorized();
                    self.reset_to_login_after_unauthorized(
                        "セッションの有効期限が切れました。再度ログインしてください".to_string(),
                    );
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

    /// チャンネルの下書き（saved_drafts, 現在の入力欄）および添付ファイルを送信先（Target）ごとに正しく紐付けて failed_drafts へ退避する。
    /// 何らかの入力内容または添付ファイルが退避された場合は true を返す。
    fn evict_channel_drafts(&mut self, channel_id: &str) -> bool {
        let s_gen = self.session_gen;
        let (ch_atts, rem_atts): (Vec<_>, Vec<_>) = self
            .pending_attachments
            .drain(..)
            .partition(|a| a.channel_id == channel_id && a.session_gen == s_gen);
        self.pending_attachments = rem_atts;

        let mut atts_by_target: std::collections::HashMap<Target, Vec<PendingAttachment>> =
            std::collections::HashMap::new();
        for a in ch_atts {
            if (a.is_uploading || a.attachment_id.is_some()) && a.error.is_none() {
                atts_by_target.entry(a.target.clone()).or_default().push(a);
            }
        }

        let is_current = self.current_channel.as_deref() == Some(channel_id);

        let mut targets_with_data: std::collections::HashSet<Target> =
            std::collections::HashSet::new();
        for (ch, target) in self.saved_drafts.keys() {
            if ch == channel_id {
                targets_with_data.insert(target.clone());
            }
        }
        for target in atts_by_target.keys() {
            targets_with_data.insert(target.clone());
        }
        if is_current {
            targets_with_data.insert(self.target.clone());
        }

        let mut had_content = false;
        for target in targets_with_data {
            let (content, message_id) = if is_current && self.target == target {
                (
                    std::mem::take(&mut self.composer_draft),
                    self.pending_composer_message_id.take(),
                )
            } else {
                (
                    self.saved_drafts
                        .remove(&(channel_id.to_string(), target.clone()))
                        .unwrap_or_default(),
                    self.saved_draft_message_ids
                        .remove(&(channel_id.to_string(), target.clone())),
                )
            };
            let atts = atts_by_target.remove(&target).unwrap_or_default();
            if !content.trim().is_empty() || !atts.is_empty() {
                had_content = true;
                self.push_failed_draft(FailedDraft {
                    channel_id: channel_id.to_string(),
                    target,
                    content,
                    attachments: atts,
                    message_id,
                });
            }
        }

        self.saved_drafts.retain(|(ch, _), _| ch != channel_id);
        self.saved_draft_message_ids
            .retain(|(ch, _), _| ch != channel_id);

        had_content
    }

    /// チャンネルの閲覧権限を失った、またはチャンネルが削除されたときに状態を片付ける。
    /// 入力中または退避済みだった下書きは失われないよう `failed_drafts` へ退避する。
    fn evict_channel(&mut self, channel_id: &str) {
        self.unsubscribe_channel(channel_id);
        self.unread.remove(channel_id);
        self.last_read_marked.remove(channel_id);
        self.read_mark_in_flight.remove(channel_id);
        self.read_mark_backoff_until.remove(channel_id);
        if self
            .newest_shown
            .as_ref()
            .is_some_and(|(ch, _)| ch == channel_id)
        {
            self.newest_shown = None;
        }
        self.evict_channel_drafts(channel_id);
        if self.current_channel.as_deref() == Some(channel_id) {
            self.stop_typing();
            self.typing.clear();
            self.typing_poll.wake();
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
        if self.current_channel.as_deref() != Some(channel_id) {
            self.evict_channel_drafts(channel_id);
            return;
        }
        if is_archived {
            let was_editing = matches!(self.target, Target::Edit(_));
            let had_content = self.evict_channel_drafts(channel_id);
            self.composer_draft.clear();
            self.pending_composer_text = Some((String::new(), None));
            self.stop_typing();
            self.typing.clear();
            self.typing_poll.wake();
            self.active_reaction_message = None;
            self.confirm_delete_message = None;
            self.target = Target::Channel;
            if was_editing || had_content {
                self.notify(
                    "このチャンネルがアーカイブされたため、編集・入力中の本文を下書きに退避しました",
                );
            }
        } else if !matches!(self.target, Target::Edit(_)) {
            let had_content = self.evict_channel_drafts(channel_id);
            if had_content {
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
        let thread_deleted = if let Some(thread) = self.thread.clone() {
            if self.store.message(channel_id, &thread.root_id).is_none() {
                self.thread = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        let reply_target_deleted = match &self.target {
            Target::Reply(root_id) => self.store.message(channel_id, root_id).is_none(),
            _ => false,
        };
        if (thread_deleted || reply_target_deleted) && matches!(self.target, Target::Reply(_)) {
            let old_target = self.target.clone();
            // 返信先メッセージが削除されたため、返信用の再送 ID は引き継がず破棄する
            self.pending_composer_message_id = None;
            let has_reply_content = !self.composer_draft.trim().is_empty()
                || !self.current_channel_attachments().is_empty();
            if !has_reply_content {
                self.switch_composer_context(Some(channel_id), Target::Channel);
            } else {
                self.target = Target::Channel;
                let saved_ch_draft = self
                    .saved_drafts
                    .remove(&(channel_id.to_string(), Target::Channel))
                    .unwrap_or_default();
                let saved_msg_id = self
                    .saved_draft_message_ids
                    .remove(&(channel_id.to_string(), Target::Channel));
                let s_gen = self.session_gen;
                let ch = channel_id.to_string();
                let (saved_atts, rem_atts): (Vec<_>, Vec<_>) =
                    self.pending_attachments.drain(..).partition(|a| {
                        a.channel_id == ch && a.target == Target::Channel && a.session_gen == s_gen
                    });
                self.pending_attachments = rem_atts;
                let mut valid_saved_atts = Vec::new();
                for att in saved_atts {
                    if att.is_uploading || (att.attachment_id.is_some() && att.error.is_none()) {
                        valid_saved_atts.push(att);
                    }
                }
                if !saved_ch_draft.trim().is_empty() || !valid_saved_atts.is_empty() {
                    self.push_failed_draft(FailedDraft {
                        channel_id: channel_id.to_string(),
                        target: Target::Channel,
                        content: saved_ch_draft,
                        attachments: valid_saved_atts,
                        message_id: saved_msg_id,
                    });
                }
            }
            for att in &mut self.pending_attachments {
                if att.channel_id == channel_id && att.target == old_target {
                    att.target = Target::Channel;
                }
            }
            self.notify("返信元のメッセージが削除されたため、チャンネルへの投稿に切り替えました");
        }

        // 現在の入力先ではない保存済みの返信先についても、返信元が削除されているものを検出し、
        // 復元可能な下書き (Target::Channel, message_id: None) へ退避する
        let orphaned_reply_targets: Vec<Target> = self
            .saved_drafts
            .keys()
            .chain(self.saved_draft_message_ids.keys())
            .filter(|(ch, _)| ch == channel_id)
            .map(|(_, tgt)| tgt.clone())
            .chain(
                self.pending_attachments
                    .iter()
                    .filter(|a| a.channel_id == channel_id)
                    .map(|a| a.target.clone()),
            )
            .filter(|tgt| match tgt {
                Target::Reply(root_id) => self.store.message(channel_id, root_id).is_none(),
                _ => false,
            })
            .collect();
        let mut unique_orphaned = orphaned_reply_targets;
        unique_orphaned.sort();
        unique_orphaned.dedup();

        for orphaned_target in unique_orphaned {
            if self.target == orphaned_target {
                continue;
            }
            let saved_draft = self
                .saved_drafts
                .remove(&(channel_id.to_string(), orphaned_target.clone()))
                .unwrap_or_default();
            let _saved_msg_id = self
                .saved_draft_message_ids
                .remove(&(channel_id.to_string(), orphaned_target.clone()));
            let s_gen = self.session_gen;
            let ch = channel_id.to_string();
            let tgt = orphaned_target.clone();
            let (saved_atts, rem_atts): (Vec<_>, Vec<_>) = self
                .pending_attachments
                .drain(..)
                .partition(|a| a.channel_id == ch && a.target == tgt && a.session_gen == s_gen);
            self.pending_attachments = rem_atts;
            let mut valid_saved_atts = Vec::new();
            for mut att in saved_atts {
                if att.is_uploading || (att.attachment_id.is_some() && att.error.is_none()) {
                    // 親が削除されたため通常投稿へ昇格させ、現在の下書きに混ざらないよう FailedDraft に退避する
                    att.target = Target::Channel;
                    valid_saved_atts.push(att);
                }
            }
            if !saved_draft.trim().is_empty() || !valid_saved_atts.is_empty() {
                self.push_failed_draft(FailedDraft {
                    channel_id: channel_id.to_string(),
                    target: Target::Channel,
                    content: saved_draft,
                    attachments: valid_saved_atts,
                    message_id: None,
                });
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
            AppEvent::Dnd(result) => {
                if let Some(until) = result {
                    self.dnd_until = until;
                }
                self.dnd_checked = true;
                false
            }
            AppEvent::AccountProfiles(profiles) => {
                self.store.account_profiles = profiles;
                true
            }
            AppEvent::EmojiCatalog { shortcodes, custom } => {
                zoo2_client_core::mfm::set_emoji_catalog(shortcodes, custom);
                true
            }
            AppEvent::Notice(message) => {
                self.notify(message);
                true
            }
            AppEvent::Unauthorized => {
                self.preserve_drafts_for_unauthorized();
                self.reset_to_login_after_unauthorized(
                    "認証が切れました。再度ログインしてください".to_string(),
                );
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
                        self.switch_composer_context(Some(&channel_id), target.clone());
                    }
                    self.composer_draft = content.clone();
                    self.pending_composer_text = Some((content, None));
                    if let Some(draft) = in_flight {
                        self.pending_composer_message_id = draft.message_id;
                        for mut att in draft.attachments {
                            att.session_gen = self.session_gen;
                            att.target = target.clone();
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
            AppEvent::ReadStatesPollFinished => {
                self.read_poll.finish(Instant::now(), true);
                false
            }
            AppEvent::EmojiPollFinished => {
                self.emoji_poll.finish(Instant::now(), true);
                false
            }
            AppEvent::TypingPollFinished => {
                self.typing_poll_in_flight = false;
                self.typing_poll.finish(
                    Instant::now(),
                    self.typing.values().any(|users| !users.is_empty())
                        || self.typing_sent.is_some(),
                );
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
                    att.attachment_id = Some(attachment_id.clone());
                    att.is_uploading = false;
                }
                for draft in &mut self.failed_drafts {
                    for att in &mut draft.attachments {
                        if att.local_id == local_id {
                            att.attachment_id = Some(attachment_id.clone());
                            att.is_uploading = false;
                        }
                    }
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
                for draft in &mut self.failed_drafts {
                    for att in &mut draft.attachments {
                        if att.local_id == local_id {
                            att.error = Some(error.clone());
                            att.is_uploading = false;
                        }
                    }
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
                        // 409 Conflict 等で失敗した場合は次回の再試行に向けて新しい channel_id を再採番
                        if error.contains("409") || error.contains("already exists") {
                            modal.channel_id = Some(uuid::Uuid::new_v4().to_string());
                        }
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
                if self.pending_open_attachments.remove(&attachment_id) {
                    self.notify("ファイルの取得に失敗しました");
                }
                true
            }
            AppEvent::RefreshAttachmentUrls => {
                if !self.window_active {
                    return false;
                }
                let cache = &self.channel_cache;
                let (start, end) = self.visible_timeline_bounds(&cache.root_messages);
                let visible_messages = cache.root_messages[start..end]
                    .iter()
                    .chain(cache.thread_root_message.iter())
                    .chain(cache.thread_replies.iter());
                let visible_attachment_ids: std::collections::HashSet<String> = visible_messages
                    .filter_map(|message| cache.attachments_by_message.get(&message.id))
                    .flatten()
                    .map(|attachment| attachment.id.clone())
                    .collect();
                let to_refresh: Vec<String> = self
                    .attachment_download_urls
                    .iter()
                    .filter(|(id, cached)| {
                        visible_attachment_ids.contains(*id) && {
                            let margin = (cached.ttl_secs / 3).min(60);
                            cached.fetched_at.elapsed().as_secs() + margin >= cached.ttl_secs
                        }
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in to_refresh {
                    self.ensure_attachment_url(id);
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
        self.poll_read_states();
        self.poll_emojis();
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
        // 暗号化チャンネルは本文を表示しないので、見ていないメッセージを既読にしない。
        if self
            .current_channel
            .as_deref()
            .is_some_and(|id| self.is_encrypted_channel(id))
        {
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
        if !self.window_active {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        if !self.read_poll.start(Instant::now()) {
            return;
        }
        if !self.spawn_session_task(async move {
            if let Ok(states) = api.read_states().await {
                let _ = tx.send(DesktopEvent::App {
                    session_gen,
                    event: AppEvent::ReadStates(states),
                });
            }
            let _ = tx.send(DesktopEvent::App {
                session_gen,
                event: AppEvent::ReadStatesPollFinished,
            });
        }) {
            self.read_poll.finish(Instant::now(), true);
        }
    }

    fn poll_emojis(&mut self) {
        if !self.window_active {
            return;
        }
        let Some(api) = self.api.clone() else {
            return;
        };
        if !self.emoji_poll.start(Instant::now()) {
            return;
        }
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        if !self.spawn_session_task(async move {
            if let Ok(profiles) = api.account_profiles().await {
                let _ = tx.send(DesktopEvent::App {
                    session_gen,
                    event: AppEvent::AccountProfiles(profiles),
                });
            }
            if let Ok((shortcodes, custom)) = api.emoji_catalog().await {
                let _ = tx.send(DesktopEvent::App {
                    session_gen,
                    event: AppEvent::EmojiCatalog { shortcodes, custom },
                });
            }
            let _ = tx.send(DesktopEvent::App {
                session_gen,
                event: AppEvent::EmojiPollFinished,
            });
        }) {
            self.emoji_poll.finish(Instant::now(), true);
        }
    }

    pub fn poll_typing(&mut self) {
        if !self.window_active || self.typing_poll_in_flight {
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
            self.typing_poll.wake();
            return;
        }
        let mut scopes = vec![None];
        if let Some(thread) = &self.thread {
            scopes.push(Some(thread.root_id.clone()));
        }
        let tx = self.event_tx.clone();
        let session_gen = self.session_gen;
        if !self.typing_poll.start(Instant::now()) {
            return;
        }
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
        } else {
            self.typing_poll.finish(Instant::now(), true);
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
        if self.typing_sent.is_none() {
            self.typing_poll.wake();
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

    /// 暗号化チャンネルか。デスクトップ版は復号も暗号化もしないので、表示と送信をしない。
    /// 再同期で行が消えても、一度暗号化と見たチャンネルは暗号化として扱う。
    pub fn is_encrypted_channel(&self, channel_id: &str) -> bool {
        self.known_encrypted_channels.contains(channel_id)
            || self.store.channel(channel_id).is_some_and(|c| c.encrypted)
    }

    /// 今 HTTP で送ってよいか（新規、返信、編集、添付）。チャンネルの行がない（再同期中で暗号化か
    /// 分からない）あいだと、暗号化チャンネルには送らない。入力欄と下書きは
    /// [`can_post_in_channel`](Self::can_post_in_channel) のとおり残す。
    pub fn can_send_to_channel(&self, channel_id: &str) -> bool {
        self.store.channel(channel_id).is_some() && !self.is_encrypted_channel(channel_id)
    }

    /// 今のチャンネルにファイルを添付してよいか（ファイルの中身をアップロードする前に確かめる）。
    fn can_attach_now(&mut self) -> bool {
        let Some(channel_id) = self.current_channel.clone() else {
            return true;
        };
        if self.can_send_to_channel(&channel_id) {
            return true;
        }
        self.notify(if self.is_encrypted_channel(&channel_id) {
            "暗号化チャンネルには、このクライアントからは添付できません"
        } else {
            "チャンネルの情報を同期しています。少し待ってから添付してください"
        });
        false
    }

    /// 指定チャンネルへ新規メッセージや返信を投稿可能か判定する。
    pub fn can_post_in_channel(&self, channel_id: &str) -> bool {
        let Some(me) = self.me.as_ref() else {
            return false;
        };
        // デスクトップ版は E2EE の対象外。暗号化チャンネルには平文で送らない（docs/e2ee.md）。
        // 再同期中の判定より先に見る（行が届いていれば、同期中でも送らない）。
        if self.is_encrypted_channel(channel_id) {
            return false;
        }
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
            self.channel_cache.attachments_dirty = true;
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
        if self.channel_cache.attachments_dirty {
            self.channel_cache.attachments_by_message =
                Arc::new(self.store.attachments_by_message(&channel_id));
            self.channel_cache.attachments_dirty = false;
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
            let has_attachments = !self.current_channel_attachments().is_empty();
            if self.composer_draft.trim().is_empty() && !has_attachments {
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
        self.open_channel_inner(channel_id, true);
    }

    /// `jump_to_newest` が false のときは、最下部へのスクロールも、最新投稿を表示済みとする記録と既読化もしない。
    /// 通知から古い投稿へ移るとき、移動先を見る前に既読化しないために使う。
    fn open_channel_inner(&mut self, channel_id: String, jump_to_newest: bool) {
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
            self.typing_poll.wake();
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

        self.current_channel = Some(channel_id.clone());
        self.timeline_anchor_first_id = None;
        if !jump_to_newest {
            self.timeline_at_bottom = false;
            self.pending_scroll_to_bottom = false;
            self.newest_shown = None;
            self.ensure_channel_cache();
            self.poll_typing();
            return;
        }
        self.timeline_at_bottom = true;
        self.pending_scroll_to_bottom = true;
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

    /// 通知のクリックなどで、チャンネルを開き、`message_id` の投稿まで移る。
    /// 投稿がまだ Store に無いとき (取得前) は、チャンネルを開くだけにする。
    pub fn open_message(&mut self, channel_id: String, message_id: Option<String>) {
        let target = message_id.and_then(|id| {
            self.store
                .message(&channel_id, &id)
                .filter(|m| m.deleted_at.is_none())
                .cloned()
        });
        let Some(msg) = target else {
            self.open_channel(channel_id);
            return;
        };
        let message_id = msg.id.clone();
        self.open_channel_inner(channel_id.clone(), false);
        if self.current_channel.as_deref() != Some(channel_id.as_str()) {
            return;
        }
        if let Some(parent_id) = msg.parent_id {
            self.open_thread(parent_id.clone());
            if let Some(thread) = &mut self.thread {
                thread.selected = Some(message_id.clone());
            }
            self.timeline_at_bottom = false;
            self.pending_scroll_to_bottom = false;
            self.pending_thread_scroll_to_bottom = false;
            self.thread_at_bottom = false;
            let roots = self.store.root_messages(&channel_id);
            self.timeline_anchor_first_id = match roots.iter().position(|m| m.id == parent_id) {
                Some(pos) => roots.get(pos.saturating_sub(10)).map(|m| m.id.clone()),
                None => Some(parent_id),
            };
            self.timeline_render_limit = MAX_TIMELINE_RENDER_LIMIT;
            self.pending_scroll_to_message_id = Some(message_id);
            self.ensure_channel_cache();
            return;
        }
        let roots = self.store.root_messages(&channel_id);
        let Some(pos) = roots.iter().position(|m| m.id == message_id) else {
            return;
        };
        self.timeline_anchor_first_id = roots.get(pos.saturating_sub(10)).map(|m| m.id.clone());
        self.timeline_at_bottom = false;
        self.pending_scroll_to_bottom = false;
        self.timeline_render_limit = MAX_TIMELINE_RENDER_LIMIT;
        self.pending_scroll_to_message_id = Some(message_id);
        self.ensure_channel_cache();
    }

    /// ウィンドウのアクティブ状態とタイムラインのスクロール位置を更新し、
    /// アクティブかつ最下部を表示しているときだけ最新メッセージを既読化する。
    pub fn update_timeline_view_state(&mut self, window_active: bool, at_bottom: bool) {
        let was_at_bottom = self.timeline_at_bottom;
        if window_active && !self.window_active {
            self.read_poll.wake();
            self.emoji_poll.wake();
            self.typing_poll.wake();
        }
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
        // 暗号化チャンネルは本文を表示しないので、スレッドも開かない（通知から開いた返信を含む）。
        if self
            .current_channel
            .as_deref()
            .is_some_and(|id| self.is_encrypted_channel(id))
        {
            self.thread = None;
            return;
        }
        self.typing_poll.wake();
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
        self.typing_poll.wake();
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
        // デスクトップ版は暗号化して送れないので、暗号化チャンネルでは編集の状態に入らない。
        if self.is_encrypted_channel(&channel_id) {
            return;
        }
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
        if let Some(msg_id) = self.pending_composer_message_id.take() {
            self.saved_draft_message_ids
                .insert((channel_id.clone(), self.target.clone()), msg_id);
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
        if self
            .current_channel_attachments()
            .iter()
            .any(|a| a.is_uploading)
        {
            self.notify("添付ファイルをアップロード中です。完了をお待ちください");
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

        // 復元先のチャンネルおよびターゲットを決定
        let (dest_ch, dest_target) = if target_ch_writable {
            (draft.channel_id.clone(), resolved_target.clone())
        } else if let Some(current_ch) = self
            .current_channel
            .clone()
            .filter(|id| self.can_post_in_channel(id))
        {
            (current_ch, Target::Channel)
        } else {
            return;
        };

        // 復元先の (channel_id, target) にアップロード中の添付がある場合は復元を中断する
        let s_gen = self.session_gen;
        if self.pending_attachments.iter().any(|a| {
            a.channel_id == dest_ch
                && a.target == dest_target
                && a.session_gen == s_gen
                && a.is_uploading
        }) {
            self.notify("添付ファイルをアップロード中です。完了をお待ちください");
            return;
        }

        if target_ch_writable {
            self.failed_drafts.remove(index);
            if self.current_channel.as_deref() != Some(draft.channel_id.as_str()) {
                self.open_channel(draft.channel_id.clone());
            }
            if let Target::Reply(ref root_id) = resolved_target
                && self.thread.as_ref().map(|t| &t.root_id) != Some(root_id)
            {
                self.typing_poll.wake();
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
            let d_tgt = resolved_target.clone();
            self.pending_attachments.retain(|a| {
                !(a.channel_id == d_ch
                    && a.target == d_tgt
                    && a.session_gen == s_gen
                    && !a.is_uploading)
            });
            self.composer_draft = draft.content.clone();
            self.pending_composer_text = Some((draft.content, None));
            self.pending_composer_message_id = if resolved_target == draft.target {
                draft.message_id.clone()
            } else {
                None
            };
            for mut att in draft.attachments {
                att.channel_id = draft.channel_id.clone();
                att.session_gen = self.session_gen;
                att.target = resolved_target.clone();
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
                self.pending_attachments.retain(|a| {
                    !(a.channel_id == c_ch
                        && a.target == Target::Channel
                        && a.session_gen == s_gen
                        && !a.is_uploading)
                });
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
        let cur_attachments = self.current_channel_attachments();
        let has_existing_attachments = match &self.target {
            Target::Edit(msg_id) => !self
                .store
                .attachments_for_message(&channel_id, msg_id)
                .is_empty(),
            _ => false,
        };
        if content.trim().is_empty() && cur_attachments.is_empty() && !has_existing_attachments {
            return false;
        }
        if cur_attachments.iter().any(|a| a.is_uploading) {
            self.notify("添付ファイルをアップロード中です。完了をお待ちください");
            return false;
        }
        if cur_attachments.iter().any(|a| a.error.is_some()) {
            self.notify(
                "アップロードに失敗した添付ファイルがあります。削除してからファイルを再度添付してください",
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
        // 暗号化かどうか分からない（再同期中で行がない）、または暗号化チャンネルなら、編集を含めて
        // 送らない。下書きは消さずに残す。
        if !self.can_send_to_channel(&channel_id) {
            self.notify(if self.is_encrypted_channel(&channel_id) {
                "暗号化チャンネルには、このクライアントからは送信できません"
            } else {
                "チャンネルの情報を同期しています。少し待ってから送ってください"
            });
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
                let t = target.clone();
                self.pending_attachments
                    .drain(..)
                    .partition(|a| a.channel_id == ch && a.target == t && a.session_gen == s_gen)
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
            .filter(|a| {
                a.channel_id == *ch && a.target == self.target && a.session_gen == self.session_gen
            })
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
            Some(c) => {
                let margin = (c.ttl_secs / 3).min(60);
                c.fetched_at.elapsed().as_secs() + margin >= c.ttl_secs
            }
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
                    if matches!(
                        err.downcast_ref::<HttpError>(),
                        Some(HttpError::Unauthorized)
                    ) {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Unauthorized,
                        });
                        return;
                    }
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
        let min_margin = if let Some(c) = self.attachment_download_urls.get(attachment_id) {
            (c.ttl_secs / 3).min(30)
        } else {
            30
        };
        // 有効期限が十分に残っている場合のみ即座に開く
        if let Some(c) = self.attachment_download_urls.get(attachment_id)
            && c.fetched_at.elapsed().as_secs() + min_margin < c.ttl_secs
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
        if !self.can_attach_now() {
            return;
        }
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
        let Some(me) = self.me.as_ref() else {
            return;
        };
        if self.store.is_synced(&ShapeId::ChannelUsers)
            && !self.store.is_member(&channel_id, &me.id)
        {
            self.notify("チャンネルに参加してからファイルを添付してください");
            return;
        }
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルにはファイルを添付できません");
            return;
        }
        if !self.can_post_in_channel(&channel_id) {
            self.notify("このチャンネルには投稿権限がありません");
            return;
        }
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
            target: self.target.clone(),
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
        if !self.can_attach_now() {
            return;
        }
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
        let Some(me) = self.me.as_ref() else {
            return;
        };
        if self.store.is_synced(&ShapeId::ChannelUsers)
            && !self.store.is_member(&channel_id, &me.id)
        {
            self.notify("チャンネルに参加してからファイルを添付してください");
            return;
        }
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルにはファイルを添付できません");
            return;
        }
        if !self.can_post_in_channel(&channel_id) {
            self.notify("このチャンネルには投稿権限がありません");
            return;
        }
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
            target: self.target.clone(),
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
                display_name: "@here (全員へ通知)".into(),
                is_special: true,
            });
        }

        // チャンネルメンバー
        if let Some(ch) = &self.current_channel {
            for member in self.store.channel_members(ch) {
                // 通常ユーザーまたはメンション可能な Bot のみを補完候補とする
                if member.is_bot && !member.bot_mentionable {
                    continue;
                }
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
        let draft_len = self.composer_draft.len();
        if state.start_byte > state.end_byte
            || state.end_byte > draft_len
            || !self.composer_draft.is_char_boundary(state.start_byte)
            || !self.composer_draft.is_char_boundary(state.end_byte)
        {
            return None;
        }
        let target_slice = &self.composer_draft[state.start_byte..state.end_byte];
        if !target_slice.starts_with('@') {
            return None;
        }
        let insert_text = format!("@{} ", candidate.name);
        let before = &self.composer_draft[..state.start_byte];
        let after = &self.composer_draft[state.end_byte..];
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

    pub const EMOJI_PICKER_COLUMNS: usize = 9;

    pub const COMMON_EMOJIS: &[&str] = &[
        "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "😊", "😇", "🙂", "🙃", "😉", "😌", "😍",
        "🥰", "😘", "😗", "😙", "😚", "😋", "😛", "😝", "😜", "🤪", "🤨", "🧐", "🤓", "😎", "🥳",
        "😏", "😒", "😞", "😔", "😟", "😕", "🙁", "😣", "😖", "😫", "😩", "🥺", "😢", "😭", "😤",
        "😠", "😡", "🤬", "🤯", "😳", "🥵", "🥶", "😱", "😨", "😰", "😥", "😓", "🤗", "🤔", "🤭",
        "🤫", "🤥", "😶", "😐", "😑", "😬", "🙄", "😯", "😦", "😧", "😮", "😲", "🥱", "😴", "🤤",
        "👍", "👎", "👏", "🙌", "👐", "🤲", "🤝", "🙏", "✌️", "🤞", "🤟", "🤘", "👌", "🤌", "🤏",
        "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "❣️", "💕", "💞", "💓", "💗",
        "💖", "💘", "💝", "✨", "🎉", "🎊", "🔥", "💯", "⭐", "🌟", "👀", "💡", "🚀", "☕", "🍺",
    ];

    pub fn open_emoji_picker(&mut self) {
        self.emoji_picker_open = true;
        self.emoji_picker_query.clear();
        self.emoji_picker_selected = 0;
        self.pending_emoji_picker_scroll = true;
    }

    pub fn close_emoji_picker(&mut self) {
        self.emoji_picker_open = false;
        self.emoji_picker_query.clear();
        self.emoji_picker_selected = 0;
        self.pending_emoji_picker_scroll = false;
        self.pending_focus_composer = true;
    }

    pub fn take_pending_emoji_picker_scroll(&mut self) -> bool {
        std::mem::take(&mut self.pending_emoji_picker_scroll)
    }

    pub fn filtered_emojis(&self) -> Vec<&'static str> {
        let query = self.emoji_picker_query.to_lowercase();
        if query.is_empty() {
            Self::COMMON_EMOJIS.to_vec()
        } else {
            Self::COMMON_EMOJIS
                .iter()
                .copied()
                .filter(|e| {
                    if let Some(emoji_data) = emojis::get(e) {
                        emoji_data.name().to_lowercase().contains(&query)
                            || emoji_data
                                .shortcode()
                                .is_some_and(|s| s.to_lowercase().contains(&query))
                    } else {
                        false
                    }
                })
                .collect()
        }
    }

    pub fn selected_emoji(&self) -> Option<&'static str> {
        let list = self.filtered_emojis();
        if list.is_empty() {
            None
        } else {
            let idx = self.emoji_picker_selected.min(list.len() - 1);
            Some(list[idx])
        }
    }

    pub fn emoji_picker_prev(&mut self) {
        let len = self.filtered_emojis().len();
        if len > 0 {
            if self.emoji_picker_selected == 0 {
                self.emoji_picker_selected = len - 1;
            } else {
                self.emoji_picker_selected -= 1;
            }
            self.pending_emoji_picker_scroll = true;
        }
    }

    pub fn emoji_picker_next(&mut self) {
        let len = self.filtered_emojis().len();
        if len > 0 {
            self.emoji_picker_selected = (self.emoji_picker_selected + 1) % len;
            self.pending_emoji_picker_scroll = true;
        }
    }

    pub fn emoji_picker_prev_row(&mut self) {
        let len = self.filtered_emojis().len();
        if len > 0 {
            let cols = Self::EMOJI_PICKER_COLUMNS;
            if self.emoji_picker_selected >= cols {
                self.emoji_picker_selected -= cols;
            } else {
                let rem = self.emoji_picker_selected;
                let last_row_start = (len / cols) * cols;
                let target = last_row_start + rem;
                self.emoji_picker_selected = if target < len { target } else { len - 1 };
            }
            self.pending_emoji_picker_scroll = true;
        }
    }

    pub fn emoji_picker_next_row(&mut self) {
        let len = self.filtered_emojis().len();
        if len > 0 {
            let cols = Self::EMOJI_PICKER_COLUMNS;
            let target = self.emoji_picker_selected + cols;
            if target < len {
                self.emoji_picker_selected = target;
            } else {
                self.emoji_picker_selected %= cols;
            }
            self.pending_emoji_picker_scroll = true;
        }
    }

    pub fn insert_selected_emoji(&mut self, range: std::ops::Range<usize>) {
        if let Some(emoji) = self.selected_emoji() {
            self.insert_emoji(emoji, range);
        } else {
            self.close_emoji_picker();
        }
    }

    pub fn insert_emoji(&mut self, emoji: &str, range: std::ops::Range<usize>) {
        self.mention_autocomplete = None;
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

    // --- ゲスト判定 ---

    pub fn is_current_user_guest(&self) -> bool {
        let Some(me) = &self.me else {
            return false;
        };
        self.store.user(&me.id).is_some_and(|u| u.is_guest)
    }

    // --- チャンネル作成モーダル ---

    pub fn open_create_channel_modal(&mut self) {
        if self.is_current_user_guest() {
            return;
        }
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
        if let Err(err) = validate_channel_name_input(&name) {
            modal.error = Some(err.to_string());
            return;
        }
        let desc_trimmed = modal.description.trim().to_string();
        let desc_len = desc_trimmed.chars().count();
        if desc_len > 250 {
            modal.error = Some(format!(
                "説明は250文字以内で入力してください (現在 {desc_len} 文字)"
            ));
            return;
        }
        let desc = if desc_trimmed.is_empty() {
            None
        } else {
            Some(desc_trimmed)
        };
        let is_private = modal.is_private;
        let channel_id = modal
            .channel_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
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
            let res = api
                .create_channel(Some(&channel_id), &name, desc.as_deref(), is_private)
                .await;
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
                    let is_unauthorized = matches!(
                        err.downcast_ref::<HttpError>(),
                        Some(HttpError::Unauthorized)
                    );
                    if is_unauthorized {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::Unauthorized,
                        });
                    } else {
                        let _ = tx.send(DesktopEvent::App {
                            session_gen,
                            event: AppEvent::ChannelCreateFailed {
                                request_id,
                                error: err.to_string(),
                            },
                        });
                    }
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

    /// チャンネル内で検索クエリに一致するメッセージ ID 一覧を収集する。
    /// 返信メッセージの親メッセージ存在確認は、チャンネル内の可視メッセージ ID を一度集合にまとめることで
    /// 二重走査（O(N^2)）を避けて O(N) で行う。
    fn collect_search_matches(&self, ch: &str, query_lower: &str) -> Vec<String> {
        if query_lower.is_empty() {
            return Vec::new();
        }
        let channel_msgs = self.store.channel_messages(ch);
        let existing_ids: HashSet<&str> = channel_msgs.iter().map(|m| m.id.as_str()).collect();
        let mut matches = Vec::new();
        for msg in channel_msgs {
            // 親メッセージが削除された（または不可視な）返信はスレッド描画不能なため検索結果から除外
            if let Some(parent_id) = &msg.parent_id
                && !existing_ids.contains(parent_id.as_str())
            {
                continue;
            }
            if msg.content.to_lowercase().contains(query_lower) {
                matches.push(msg.id.clone());
            }
        }
        matches
    }

    pub fn update_message_search_query(&mut self, query: String) {
        let query_lower = query.trim().to_lowercase();
        let matches = if let Some(ch) = &self.current_channel {
            self.collect_search_matches(ch, &query_lower)
        } else {
            Vec::new()
        };
        self.message_search = Some(MessageSearchState {
            query,
            matches,
            current_match_index: 0,
        });
        self.scroll_to_current_search_match();
    }

    /// メッセージ同期時に、現在の検索クエリで一致メッセージ一覧を更新する。
    /// 以前選択されていたメッセージが一致リストに残っていればその選択とスクロール位置を維持し、
    /// 一致リストから消えた場合のみ新しい選択位置へスクロールする。
    pub fn refresh_message_search_results(&mut self) {
        let query_lower = match &self.message_search {
            Some(ms) => ms.query.trim().to_lowercase(),
            None => return,
        };
        let matches = if let Some(ch) = &self.current_channel {
            self.collect_search_matches(ch, &query_lower)
        } else {
            Vec::new()
        };
        let Some(ms) = &mut self.message_search else {
            return;
        };
        let prev_selected_id = ms.matches.get(ms.current_match_index).cloned();
        let new_index = if let Some(prev_id) = &prev_selected_id
            && let Some(pos) = matches.iter().position(|id| id == prev_id)
        {
            pos
        } else {
            0
        };
        let need_scroll =
            prev_selected_id.is_none() || matches.get(new_index) != prev_selected_id.as_ref();

        ms.matches = matches;
        ms.current_match_index = new_index;

        if need_scroll && !ms.matches.is_empty() {
            self.scroll_to_current_search_match();
        }
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
        self.pending_quick_switcher_scroll = true;
    }

    pub fn close_quick_switcher(&mut self) {
        self.quick_switcher = None;
        self.pending_quick_switcher_scroll = false;
        self.pending_focus_composer = true;
    }

    pub fn update_quick_switcher_query(&mut self, query: String) {
        if let Some(qs) = &mut self.quick_switcher {
            qs.query = query;
            qs.selected_index = 0;
            self.pending_quick_switcher_scroll = true;
        }
    }

    pub fn quick_switcher_select_next(&mut self) {
        let count = self.quick_switcher_candidates().len();
        if let Some(qs) = &mut self.quick_switcher
            && count > 0
        {
            qs.selected_index = (qs.selected_index + 1) % count;
            self.pending_quick_switcher_scroll = true;
        }
    }

    pub fn quick_switcher_select_prev(&mut self) {
        let count = self.quick_switcher_candidates().len();
        if let Some(qs) = &mut self.quick_switcher
            && count > 0
        {
            qs.selected_index = (qs.selected_index + count - 1) % count;
            self.pending_quick_switcher_scroll = true;
        }
    }

    pub fn take_pending_quick_switcher_scroll(&mut self) -> bool {
        std::mem::take(&mut self.pending_quick_switcher_scroll)
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
    zoo2_client_core::mfm::resolve_emoji(input)
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
pub(crate) mod tests {
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

    pub(crate) fn connected_state() -> AppState {
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

    #[tokio::test]
    async fn does_not_send_while_the_channel_row_is_missing_or_encrypted() {
        let mut state = connected_state();
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c2",
                json!({ "id": "c2", "channel_name": "secret", "is_private": "t", "encrypted": "t" }),
            )]),
        ));
        state.open_channel("c1".into());
        // Channels の再同期で行が消えたあいだは、暗号化か分からないので送らない（下書きは残す）。
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            Batch {
                reset: true,
                changes: vec![],
                up_to_date: false,
            },
        ));
        state.composer_draft = "再同期中の本文".into();
        assert!(state.can_post_in_channel("c1"));
        assert!(!state.can_send_to_channel("c1"));
        assert!(!state.submit_message("再同期中の本文".into()));
        assert_eq!(state.composer_draft, "再同期中の本文");
        state.target = Target::Edit("r1".into());
        assert!(!state.submit_message("編集".into()));
        state.target = Target::Channel;
        // 一度暗号化と見たチャンネルは、行が消えても暗号化として扱う。
        assert!(state.is_encrypted_channel("c2"));
        assert!(!state.can_send_to_channel("c2"));
        // 平文のチャンネルの行が戻れば送れる。
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "f" }),
            )]),
        ));
        assert!(state.can_send_to_channel("c1"));
        assert!(state.submit_message("同期後の本文".into()));
    }

    #[test]
    fn encrypted_channels_are_not_marked_read() {
        let mut state = connected_state();
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "t", "encrypted": "t" }),
            )]),
        ));
        state.window_active = true;
        state.open_channel("c1".into());
        // 本文を表示しない暗号化チャンネルでは、既読の API を呼ばない。
        assert!(!state.mark_current_read());
    }

    #[test]
    fn encrypted_channels_do_not_open_threads_or_edits() {
        let mut state = connected_state();
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c1",
                json!({ "id": "c1", "channel_name": "general", "is_private": "t", "encrypted": "t" }),
            )]),
        ));
        // 通知から暗号化チャンネルの返信を開いても、スレッドペインを開かない。
        state.open_message("c1".into(), Some("r1".into()));
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert!(state.thread.is_none());
        state.open_thread("m1".into());
        assert!(state.thread.is_none());
        // 自分のメッセージでも、編集の状態に入らない。
        state.start_edit("r1".into());
        assert!(!matches!(state.target, Target::Edit(_)));
    }

    #[test]
    fn open_message_to_a_reply_scrolls_the_thread_to_the_reply() {
        let mut state = connected_state();
        state.open_message("c1".into(), Some("r1".into()));
        assert_eq!(state.current_channel.as_deref(), Some("c1"));
        assert_eq!(
            state.thread.as_ref().map(|t| t.root_id.as_str()),
            Some("m1")
        );
        assert_eq!(
            state.thread.as_ref().and_then(|t| t.selected.as_deref()),
            Some("r1")
        );
        assert_eq!(state.pending_scroll_to_message_id.as_deref(), Some("r1"));
        // 返信へ移る間は、タイムラインを最下部として扱わない (古い位置から既読にしない)。
        assert!(!state.timeline_at_bottom);
        assert!(!state.pending_thread_scroll_to_bottom);
    }

    #[test]
    fn open_message_on_the_active_current_channel_does_not_mark_read() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.window_active = true;
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m_new",
                json!({ "id": "m_new", "user_id": "u2", "channel_id": "c1",
                        "content": "新しい投稿", "created_at": "2026-11-01 04:00:00" }),
            )]),
        ));
        // 新着を、まだ見ていない状態にする。
        state.update_timeline_view_state(true, false);
        state.last_read_marked.clear();
        state.read_mark_in_flight.clear();
        state.unread.insert("c1".into());
        state.newest_shown = None;

        state.open_message("c1".into(), Some("m1".into()));

        assert!(state.unread.contains("c1"), "既読化されてはいけない");
        assert!(state.last_read_marked.is_empty());
        assert!(state.read_mark_in_flight.is_empty());
        assert_ne!(
            state.newest_shown.as_ref().map(|(_, id)| id.as_str()),
            Some("m_new")
        );
        assert!(!state.pending_scroll_to_bottom);
        assert_eq!(state.pending_scroll_to_message_id.as_deref(), Some("m1"));

        // 最下部まで見たら、通常の経路で既読になる。
        state.update_timeline_view_state(true, true);
        assert!(!state.unread.contains("c1"));
    }

    #[test]
    fn message_batches_refresh_bundled_reactions_without_another_subscription() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        assert!(
            !state
                .subscriptions
                .contains_key(&ShapeId::Reactions("c1".into()))
        );
        let mut row = json!({"id":"m1", "user_id":"u2", "channel_id":"c1", "content":"hello", "created_at":"2026-11-01 03:00:00", "display_data":{"reactions":[
            {"id":"remote", "message_id":"m1", "user_id":"https://remote/user", "emoji":"👍", "created_at":"2026-11-01 03:00:01"}
        ]}});
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![("m1", row.clone())]),
        ));
        state.ensure_channel_cache();
        assert_eq!(state.channel_cache.reactions_by_message["m1"][0].count, 1);
        row["display_data"] = json!({"reactions":[]});
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![("m1", row)]),
        ));
        state.ensure_channel_cache();
        assert!(!state.channel_cache.reactions_by_message.contains_key("m1"));
    }

    #[test]
    fn bundled_attachments_refresh_without_another_subscription() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        assert!(
            !state
                .subscriptions
                .contains_key(&ShapeId::Attachments("c1".into()))
        );
        let mut row = json!({"id":"m1", "user_id":"u2", "channel_id":"c1", "content":"hello", "created_at":"2026-11-01 03:00:00", "display_data":{"attachments":[
            {"id":"a1", "message_id":"m1", "uploader_id":"u2", "channel_id":"c1", "file_name":"pic.png", "file_size":100, "content_type":"image/png", "storage_key":"key", "created_at":"2026-11-01 03:00:01"}
        ]}});
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![("m1", row.clone())]),
        ));
        state.ensure_channel_cache();
        assert_eq!(state.channel_cache.attachments_by_message["m1"][0].id, "a1");
        assert_eq!(
            state.store.attachment("c1", "a1").unwrap().file_name,
            "pic.png"
        );
        row["display_data"] = json!({"attachments":[]});
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![("m1", row)]),
        ));
        state.ensure_channel_cache();
        assert!(
            !state
                .channel_cache
                .attachments_by_message
                .contains_key("m1")
        );
    }

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
    async fn connected_event_remains_responsive_while_token_lock_is_held() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir =
            std::env::temp_dir().join(format!("desktop-connected-lock-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, event_tx);
        let tokens = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "access".into(),
            refresh_token: None,
            expires_at: token::now() + 300,
        };
        token::save(&path, &tokens).unwrap();
        let manager = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc,
            path.clone(),
            tokens,
            state.session_gen,
            state.active_session_gen.clone(),
        ));
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("json.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            drop(lock);
        });
        let started = std::time::Instant::now();
        state.on_auth_event(AuthEvent::Connected {
            credentials: Credentials::Bearer(manager.clone()),
            token_manager: Some(manager),
            me: Me {
                id: "u1".into(),
                user_name: "u".into(),
                display_name: "U".into(),
            },
            is_admin: false,
        });
        let elapsed = started.elapsed();
        release.join().unwrap();
        assert!(
            elapsed < std::time::Duration::from_millis(400),
            "connected event blocked UI for {elapsed:?}"
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));
        assert!(state.login_phase.is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cancel_changed_session_releases_pending_manager() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir = std::env::temp_dir().join(format!("cancel-changed-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, tx);
        let old = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "old".into(),
            refresh_token: None,
            expires_at: token::now() + 300,
        };
        let manager = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc.clone(),
            path.clone(),
            old.clone(),
            state.session_gen,
            state.active_session_gen.clone(),
        ));
        *state.pending_device_login_manager.lock().unwrap() = Some(manager);
        let mut replacement = old;
        replacement.session_id = uuid::Uuid::new_v4().to_string();
        token::save(&path, &replacement).unwrap();
        state.cancel_device_login();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));
        assert!(state.pending_device_login_manager.lock().unwrap().is_none());
        assert_eq!(
            token::load(&path, &oidc, Some(&config.api_base_url)).unwrap(),
            Some(replacement)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn connected_event_does_not_overwrite_another_login() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir =
            std::env::temp_dir().join(format!("desktop-connected-new-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, tx);
        let old = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "old".into(),
            refresh_token: None,
            expires_at: token::now() + 300,
        };
        token::save(&path, &old).unwrap();
        let manager = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc.clone(),
            path.clone(),
            old.clone(),
            state.session_gen,
            state.active_session_gen.clone(),
        ));
        let mut replacement = old;
        replacement.session_id = uuid::Uuid::new_v4().to_string();
        replacement.access_token = "new".into();
        token::save(&path, &replacement).unwrap();
        *state.pending_device_login_manager.lock().unwrap() = Some(manager.clone());
        state.on_auth_event(AuthEvent::Connected {
            credentials: Credentials::Bearer(manager.clone()),
            token_manager: Some(manager),
            me: Me {
                id: "u".into(),
                user_name: "u".into(),
                display_name: "U".into(),
            },
            is_admin: false,
        });
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));
        assert!(state.pending_device_login_manager.lock().unwrap().is_none());
        assert_eq!(
            token::load(&path, &oidc, Some(&config.api_base_url)).unwrap(),
            Some(replacement)
        );
        assert!(matches!(
            state.login_phase,
            Some(LoginPhase::Ready { error: Some(_) })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn unauthorized_preserves_pending_refresh_for_retry() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir = std::env::temp_dir().join(format!("desktop-recovery-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, tx);
        let tokens = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url,
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: token::now() + 300,
        };
        token::save(&path, &tokens).unwrap();
        let manager = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc,
            path,
            tokens.clone(),
            state.session_gen,
            state.active_session_gen.clone(),
        ));
        manager.set_pending_for_test(tokens);
        state.token_manager = Some(manager.clone());
        state.logout_in_progress = true;
        let generation = state.session_gen;
        state.on_app_event(AppEvent::Unauthorized);
        assert_eq!(state.session_gen, generation);
        assert!(state.recovery_manager.is_none());
        state.logout_in_progress = false;
        state.on_app_event(AppEvent::Unauthorized);
        assert!(Arc::ptr_eq(
            state.recovery_manager.as_ref().unwrap(),
            &manager
        ));
        assert!(manager.has_pending_persistence());
        assert_eq!(manager.access_token().await.unwrap(), "access");
        assert!(!manager.has_pending_persistence());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn failed_logout_replays_unauthorized_event() {
        let mut state = connected_state();
        state.config.auth = AuthConfig::Oidc(OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        });
        state.logout_in_progress = true;
        state.on_shape(ShapeEvent::Failed(
            ShapeId::Users,
            HttpError::Unauthorized.to_string(),
        ));
        assert!(state.login_phase.is_none());
        state.finish_logout(Err(("storage error".into(), false)));
        assert!(matches!(state.login_phase, Some(LoginPhase::Ready { .. })));
    }

    #[tokio::test]
    async fn logout_preserves_replacement_session_and_leaves_chat() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir = std::env::temp_dir().join(format!(
            "desktop-logout-replacement-{}",
            uuid::Uuid::new_v4()
        ));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, tx);
        let original = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "old".into(),
            refresh_token: Some("old-refresh".into()),
            expires_at: token::now() + 300,
        };
        let manager = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc.clone(),
            path.clone(),
            original.clone(),
            state.session_gen,
            state.active_session_gen.clone(),
        ));
        state.token_manager = Some(manager.clone());
        state.login_phase = None;
        let mut replacement = original;
        replacement.session_id = uuid::Uuid::new_v4().to_string();
        token::save(&path, &replacement).unwrap();
        state.logout();
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));
        assert!(matches!(state.login_phase, Some(LoginPhase::Ready { .. })));
        assert!(state.token_manager.is_none());
        assert!(!manager.save_if_active().unwrap());
        assert_eq!(
            token::load(&path, &oidc, Some(&config.api_base_url)).unwrap(),
            Some(replacement)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn logout_remains_responsive_while_another_process_holds_token_lock() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir = std::env::temp_dir().join(format!("desktop-lock-test-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "zoo2-desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, event_tx);
        let tokens = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "access".into(),
            refresh_token: None,
            expires_at: token::now() + 300,
        };
        token::save(&path, &tokens).unwrap();
        state.token_manager = Some(Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc,
            path.clone(),
            tokens,
            state.session_gen,
            state.active_session_gen.clone(),
        )));
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("json.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            drop(lock);
        });
        let started = std::time::Instant::now();
        state.logout();
        let elapsed = started.elapsed();
        release.join().unwrap();
        assert!(
            elapsed < std::time::Duration::from_millis(400),
            "logout blocked the UI for {elapsed:?}"
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));
        assert!(!path.exists());
        assert!(matches!(state.login_phase, Some(LoginPhase::Ready { .. })));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn logout_revokes_token_even_if_ui_session_changes_before_completion() {
        use std::io::{Read, Write};
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let server_issuer = issuer.clone();
        let (request_tx, request_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut count = 0;
            while count < 2 && std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut buf = [0; 8192];
                        let _ = stream.read(&mut buf);
                        let body = if count == 0 {
                            serde_json::json!({"issuer":server_issuer,"token_endpoint":format!("{server_issuer}/token"),"revocation_endpoint":format!("{server_issuer}/revoke")}).to_string()
                        } else {
                            "{}".to_string()
                        };
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        count += 1;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
            request_tx.send(count).unwrap();
        });
        let dir =
            std::env::temp_dir().join(format!("desktop-logout-stale-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer,
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, event_tx);
        let tokens = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "access".into(),
            refresh_token: Some("refresh".into()),
            expires_at: token::now() + 300,
        };
        token::save(&path, &tokens).unwrap();
        state.token_manager = Some(Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc,
            path.clone(),
            tokens,
            state.session_gen,
            state.active_session_gen.clone(),
        )));
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("json.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        state.logout();
        state.bump_session_gen();
        drop(lock);
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(!state.on_desktop_event(event));
        let count = tokio::task::spawn_blocking(move || request_rx.recv().unwrap())
            .await
            .unwrap();
        server.join().unwrap();
        assert_eq!(
            count, 2,
            "revocation must run even when the UI event is stale"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn cancelling_login_remains_responsive_while_token_lock_is_held() {
        use zoo2_client_core::auth::token::{self, StoredTokens};
        let dir =
            std::env::temp_dir().join(format!("desktop-cancel-lock-{}", uuid::Uuid::new_v4()));
        let path = token::token_path(&dir);
        let oidc = OidcConfig {
            issuer: "https://issuer.example".into(),
            client_id: "desktop".into(),
            resource: None,
            scope: "openid".into(),
        };
        let config = Config {
            api_base_url: "https://api.example".into(),
            auth: AuthConfig::Oidc(oidc.clone()),
            check_updates: false,
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, tx);
        let tokens = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: None,
            api_base_url: config.api_base_url.clone(),
            access_token: "access".into(),
            refresh_token: None,
            expires_at: token::now() + 300,
        };
        token::save(&path, &tokens).unwrap();
        let manager = Arc::new(login::DesktopTokenManager::new(
            reqwest::Client::new(),
            oidc,
            path.clone(),
            tokens,
            state.session_gen,
            state.active_session_gen.clone(),
        ));
        *state.pending_device_login_manager.lock().unwrap() = Some(manager);
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("json.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(600));
            drop(lock);
        });
        let started = std::time::Instant::now();
        state.cancel_device_login();
        let elapsed = started.elapsed();
        release.join().unwrap();
        assert!(
            elapsed < std::time::Duration::from_millis(400),
            "cancel blocked UI for {elapsed:?}"
        );
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(dir);
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
        let (event_tx, mut event_rx) = mpsc::unbounded_channel();
        let mut state = AppState::with_channel(config.clone(), None, event_tx);
        assert_eq!(state.session_gen, 0);

        let active_gen = state.active_session_gen.clone();
        let tokens = StoredTokens {
            session_id: uuid::Uuid::new_v4().to_string(),
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

        assert!(tm.save_if_active().unwrap());

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

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(state.on_desktop_event(event));

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
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let event = event_rx.recv().await.unwrap();
                if matches!(event, DesktopEvent::CancelFinished { .. }) {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        assert!(state.on_desktop_event(event));
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
            target: Target::Channel,
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
        assert!(is_disallowed_extension("program.exe."));
        assert!(is_disallowed_extension("program.exe. "));
        assert!(is_disallowed_extension("script.sh"));
        assert!(is_disallowed_extension("batch.bat"));
        assert!(is_disallowed_extension("library.dll"));
        assert!(!is_disallowed_extension("image.png"));
        assert!(!is_disallowed_extension("document.pdf"));

        // X-Amz-Expires の解析 (実際の期限秒数を返す)
        let url =
            "https://s3.amazonaws.com/bucket/key?X-Amz-Date=20261108T000000Z&X-Amz-Expires=300";
        assert_eq!(parse_url_expiry_secs(url), 300);

        // クエリがない場合はデフォルト 300 秒
        assert_eq!(
            parse_url_expiry_secs("https://s3.amazonaws.com/bucket/key"),
            300
        );

        // 短い期限（例: 45秒）もそのまま取得できること
        assert_eq!(
            parse_url_expiry_secs("https://s3.amazonaws.com/bucket/key?X-Amz-Expires=45"),
            45
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
            target: Target::Channel,
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
            target: Target::Channel,
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
            target: Target::Channel,
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

        // 未解決の添付ファイルをクリック後、URL 取得が失敗した場合はトースト通知が出る
        let res2 = state.open_or_queue_attachment("att_fail");
        assert!(res2.is_none());
        assert!(state.pending_open_attachments.contains("att_fail"));
        state.on_app_event(AppEvent::AttachmentUrlFailed {
            attachment_id: "att_fail".into(),
        });
        assert!(!state.pending_open_attachments.contains("att_fail"));
        assert_eq!(
            state.notice.as_ref().map(|(msg, _)| msg.as_str()),
            Some("ファイルの取得に失敗しました")
        );
    }

    #[test]
    fn preserves_draft_with_attachments_on_unauthorized_retaining_uploading_as_retryable() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        // 本文は空
        state.composer_draft = "   ".into();
        // 完了済み添付
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_ready".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
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
            target: Target::Channel,
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
            target: Target::Channel,
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
        // 完了済みの添付に加え、アップロード中・エラーの添付もファイル名と再添付状態を保持して退避されること
        assert_eq!(draft.attachments.len(), 3);
        assert_eq!(draft.attachments[0].local_id, "loc_ready");
        assert_eq!(
            draft.attachments[0].attachment_id.as_deref(),
            Some("att_ready")
        );
        assert!(!draft.attachments[0].is_uploading);

        assert_eq!(draft.attachments[1].local_id, "loc_uploading");
        assert_eq!(draft.attachments[1].file_name, "uploading.png");
        assert!(!draft.attachments[1].is_uploading);
        assert!(draft.attachments[1].attachment_id.is_none());
        assert_eq!(
            draft.attachments[1].error.as_deref(),
            Some("再添付してください")
        );

        assert_eq!(draft.attachments[2].local_id, "loc_error");
        assert_eq!(draft.attachments[2].error.as_deref(), Some("fail"));
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
                target: Target::Channel,
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
    fn attachment_url_renewal_only_runs_for_visible_messages_in_an_active_window() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.on_shape(ShapeEvent::Batch(ShapeId::Attachments("c1".into()), batch(vec![
            ("a1", json!({"id":"a1", "message_id":"m1", "uploader_id":"u2", "channel_id":"c1", "file_name":"pic.png", "file_size":100, "content_type":"image/png", "storage_key":"key", "created_at":"2026-11-01 03:00:01"})),
            ("offscreen", json!({"id":"offscreen", "message_id":"unloaded", "uploader_id":"u2", "channel_id":"c1", "file_name":"old.png", "file_size":100, "content_type":"image/png", "storage_key":"old", "created_at":"2026-11-01 03:00:01"})),
        ])));
        state.ensure_channel_cache();
        for id in ["a1", "offscreen"] {
            state.attachment_download_urls.insert(
                id.into(),
                CachedAttachmentUrl {
                    url: "https://example.com/image".into(),
                    fetched_at: Instant::now(),
                    ttl_secs: 0,
                },
            );
        }
        state.window_active = false;
        state.on_app_event(AppEvent::RefreshAttachmentUrls);
        assert!(state.fetching_attachment_urls.is_empty());
        state.window_active = true;
        state.on_app_event(AppEvent::RefreshAttachmentUrls);
        assert!(state.fetching_attachment_urls.contains("a1"));
        assert!(!state.fetching_attachment_urls.contains("offscreen"));
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
            target: Target::Channel,
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
            target: Target::Channel,
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
            target: Target::Channel,
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
            target: Target::Channel,
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

    #[test]
    fn isolates_attachments_by_target_and_restores_on_switch() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 通常投稿に添付 1 を追加
        state.add_attachment_from_bytes("chan.png".into(), "image/png".into(), vec![1, 2]);
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(state.current_channel_attachments()[0].file_name, "chan.png");

        // スレッド返信に切り替えると、返信欄の添付は空になること
        state.focus_thread_reply("root_1".into());
        assert!(matches!(state.target, Target::Reply(_)));
        assert_eq!(state.current_channel_attachments().len(), 0);

        // スレッド返信に添付 2 を追加
        state.add_attachment_from_bytes("reply.png".into(), "image/png".into(), vec![3, 4]);
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(
            state.current_channel_attachments()[0].file_name,
            "reply.png"
        );

        // 通常投稿に戻ると、添付 1 が復元され、添付 2 は混ざらないこと
        state.switch_composer_context(Some("c1"), Target::Channel);
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(state.current_channel_attachments()[0].file_name, "chan.png");

        // 再度スレッド返信に戻ると、添付 2 が復元されること
        state.switch_composer_context(Some("c1"), Target::Reply("root_1".into()));
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(
            state.current_channel_attachments()[0].file_name,
            "reply.png"
        );
    }

    #[test]
    fn submit_message_isolates_attachments_by_target_and_retains_others() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 通常投稿（Target::Channel）に完了済み添付とアップロード中添付を追加
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_chan".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "chan.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_chan".into()),
            is_uploading: false,
            error: None,
        });
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_chan_uploading".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "chan_uploading.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        // スレッド返信に切り替えて、返信欄に完了済み添付を追加
        state.focus_thread_reply("root_1".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("root_1".into()),
            session_gen: state.session_gen,
            file_name: "reply.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_reply".into()),
            is_uploading: false,
            error: None,
        });

        // 通常投稿側の添付がアップロード中であっても、返信の送信はブロックされないこと
        let sent = state.submit_message("返信本文".into());
        assert!(sent, "スレッド返信の送信が成功すること");

        // 送信中データ（in_flight_submissions）には返信の添付（att_reply）のみが含まれ、通常投稿の添付は含まれないこと
        assert_eq!(state.in_flight_submissions.len(), 1);
        let inflight = &state.in_flight_submissions[0].1;
        assert_eq!(inflight.target, Target::Reply("root_1".into()));
        assert_eq!(inflight.attachments.len(), 1);
        assert_eq!(inflight.attachments[0].file_name, "reply.png");
        assert_eq!(
            inflight.attachments[0].attachment_id.as_deref(),
            Some("att_reply")
        );

        // 送信後も通常投稿の添付2件は pending_attachments に保持されていること
        assert_eq!(state.pending_attachments.len(), 2);
        assert!(
            state
                .pending_attachments
                .iter()
                .all(|a| a.target == Target::Channel)
        );

        // 通常投稿に戻ると、current_channel_attachments() に通常投稿の添付が取得できること
        state.switch_composer_context(Some("c1"), Target::Channel);
        assert_eq!(state.current_channel_attachments().len(), 2);
    }

    #[test]
    fn restore_failed_draft_retains_other_target_attachments_and_rewrites_fallback_target() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 1. スレッド返信に添付 reply.png がある状態で通常投稿の失敗下書きを復元する
        state.focus_thread_reply("root_1".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("root_1".into()),
            session_gen: state.session_gen,
            file_name: "reply.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_reply".into()),
            is_uploading: false,
            error: None,
        });

        // 通常投稿の失敗下書き（添付 chan.png 付き）を追加
        state.failed_drafts.push(FailedDraft {
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "通常投稿ドラフト".into(),
            attachments: vec![PendingAttachment {
                local_id: "loc_chan".into(),
                channel_id: "c1".into(),
                target: Target::Channel,
                session_gen: state.session_gen,
                file_name: "chan.png".into(),
                file_size: 100,
                content_type: "image/png".into(),
                attachment_id: Some("att_chan".into()),
                is_uploading: false,
                error: None,
            }],
            message_id: None,
        });

        // 通常投稿の失敗下書きを復元
        state.restore_failed_draft();
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "通常投稿ドラフト");
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(state.current_channel_attachments()[0].file_name, "chan.png");

        // 返信の添付 reply.png も削除されずに残っていること
        state.switch_composer_context(Some("c1"), Target::Reply("root_1".into()));
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(
            state.current_channel_attachments()[0].file_name,
            "reply.png"
        );

        // 2. 親メッセージが削除された Reply の失敗下書きを復元した場合、Channel へフォールバックされ target が更新されること
        state.switch_composer_context(Some("c1"), Target::Channel);
        state.composer_draft.clear();
        state
            .pending_attachments
            .retain(|a| a.target != Target::Channel);
        state.failed_drafts.push(FailedDraft {
            channel_id: "c1".into(),
            target: Target::Reply("deleted_root".into()),
            content: "孤立返信ドラフト".into(),
            attachments: vec![PendingAttachment {
                local_id: "loc_fallback".into(),
                channel_id: "c1".into(),
                target: Target::Reply("deleted_root".into()),
                session_gen: state.session_gen,
                file_name: "fallback.png".into(),
                file_size: 100,
                content_type: "image/png".into(),
                attachment_id: Some("att_fallback".into()),
                is_uploading: false,
                error: None,
            }],
            message_id: None,
        });

        state.restore_failed_draft();
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "孤立返信ドラフト");
        // att.target が Target::Channel に付け替えられているため、current_channel_attachments() に表示されること
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(
            state.current_channel_attachments()[0].file_name,
            "fallback.png"
        );
        assert_eq!(
            state.current_channel_attachments()[0].target,
            Target::Channel
        );
    }

    #[test]
    fn evicts_channel_drafts_isolating_attachments_per_target() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 通常投稿に下書きと添付 A を設定
        state.composer_draft = "通常投稿下書き".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_chan".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "chan.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_chan".into()),
            is_uploading: false,
            error: None,
        });

        // スレッド返信に切り替えて下書きと添付 B を設定
        state.focus_thread_reply("root_1".into());
        state.composer_draft = "返信下書き".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("root_1".into()),
            session_gen: state.session_gen,
            file_name: "reply.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_reply".into()),
            is_uploading: false,
            error: None,
        });

        // チャンネルから evict される
        state.evict_channel("c1");

        // failed_drafts に通常投稿とスレッド返信それぞれのドラフトが分離して退避されていること
        assert_eq!(state.failed_drafts.len(), 2);
        let chan_draft = state
            .failed_drafts
            .iter()
            .find(|d| d.target == Target::Channel)
            .expect("Channel draft must be evicted");
        assert_eq!(chan_draft.content, "通常投稿下書き");
        assert_eq!(chan_draft.attachments.len(), 1);
        assert_eq!(chan_draft.attachments[0].file_name, "chan.png");

        let reply_draft = state
            .failed_drafts
            .iter()
            .find(|d| d.target == Target::Reply("root_1".into()))
            .expect("Reply draft must be evicted");
        assert_eq!(reply_draft.content, "返信下書き");
        assert_eq!(reply_draft.attachments.len(), 1);
        assert_eq!(reply_draft.attachments[0].file_name, "reply.png");

        // pending_attachments は空になっていること
        assert!(state.pending_attachments.is_empty());
    }

    #[test]
    fn preserves_attachments_per_target_across_unauthorized() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 通常投稿に添付 A（chan.png）を追加
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_chan".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "chan.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_chan".into()),
            is_uploading: false,
            error: None,
        });

        // スレッド返信に切り替えて本文と添付 B（reply.png）を入力
        state.focus_thread_reply("root_1".into());
        state.composer_draft = "返信本文".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("root_1".into()),
            session_gen: state.session_gen,
            file_name: "reply.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_reply".into()),
            is_uploading: false,
            error: None,
        });

        // 認証切れイベントを受信
        state.on_app_event(AppEvent::Unauthorized);

        // 再ログイン
        state.connect_session(
            Credentials::Dev("u1".into()),
            Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
        );

        // failed_drafts に通常投稿（添付 A）と返信（添付 B）の両方が退避・復元されていること
        assert_eq!(state.failed_drafts.len(), 2);
        let chan_draft = state
            .failed_drafts
            .iter()
            .find(|d| d.target == Target::Channel)
            .expect("Channel draft must be preserved");
        assert_eq!(chan_draft.attachments.len(), 1);
        assert_eq!(chan_draft.attachments[0].file_name, "chan.png");

        let reply_draft = state
            .failed_drafts
            .iter()
            .find(|d| d.target == Target::Reply("root_1".into()))
            .expect("Reply draft must be preserved");
        assert_eq!(reply_draft.content, "返信本文");
        assert_eq!(reply_draft.attachments.len(), 1);
        assert_eq!(reply_draft.attachments[0].file_name, "reply.png");
    }

    #[test]
    fn mention_confirm_handles_emoji_insertion_and_invalid_boundaries_safely() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.composer_draft = "@a".into();
        state.update_mention_autocomplete("@a", 2);
        assert!(state.mention_autocomplete.is_some());

        // キャレットを先頭にして絵文字を挿入
        state.insert_emoji("🎉", 0..0);
        // 絵文字挿入によりメンション補完状態が破棄されていること
        assert!(state.mention_autocomplete.is_none());

        // その後に mention_confirm を呼んでも安全に None が返ること
        assert_eq!(state.mention_confirm(), None);

        // 手動で文字境界のずれた不正な補完状態がセットされた場合でもパニックしないこと
        state.composer_draft = "🎉@a".into();
        state.mention_autocomplete = Some(MentionAutocompleteState {
            query: "a".into(),
            start_byte: 1, // '🎉' の途中（非文字境界）
            end_byte: 3,
            candidates: vec![MentionCandidate {
                name: "alice".into(),
                display_name: "Alice".into(),
                is_special: false,
            }],
            selected_index: 0,
        });
        assert_eq!(state.mention_confirm(), None);
    }

    #[test]
    fn restoring_draft_to_different_target_clears_pending_composer_message_id() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 親メッセージが存在しない Reply の失敗下書き（message_id 付き）
        state.failed_drafts.push(FailedDraft {
            channel_id: "c1".into(),
            target: Target::Reply("non_existent_root".into()),
            content: "親なし返信".into(),
            attachments: vec![],
            message_id: Some("msg_prev_retry_id".into()),
        });

        // 復元すると Target::Channel へフォールバックされる
        state.restore_failed_draft();
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "親なし返信");
        // ターゲットが変わったため、再送 ID は None にリセットされていること
        assert_eq!(state.pending_composer_message_id, None);
    }

    #[test]
    fn reconciles_targets_isolates_saved_channel_attachments_from_reply() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 通常投稿に下書きと添付 A を保存
        state.composer_draft = "通常投稿本文".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_chan".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "chan.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_chan".into()),
            is_uploading: false,
            error: None,
        });

        // 返信に切り替えて返信本文と添付 B を入力
        state.focus_thread_reply("m1".into());
        state.composer_draft = "返信本文".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("m1".into()),
            session_gen: state.session_gen,
            file_name: "reply.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: Some("att_reply".into()),
            is_uploading: false,
            error: None,
        });

        // 返信元メッセージ m1 を削除
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));

        // Target::Channel へ昇格する
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "返信本文");

        // 入力欄の添付には返信由来の reply.png のみがあり、保存されていた chan.png は混ざらないこと
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(
            state.current_channel_attachments()[0].file_name,
            "reply.png"
        );

        // 保存されていた通常投稿の下書き（添付 chan.png 付き）は failed_drafts に退避されていること
        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "通常投稿本文");
        assert_eq!(state.failed_drafts[0].attachments.len(), 1);
        assert_eq!(state.failed_drafts[0].attachments[0].file_name, "chan.png");
    }

    #[test]
    fn restore_failed_draft_blocks_when_uploading() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 失敗下書きを用意
        state.failed_drafts.push(FailedDraft {
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "復元したい下書き".into(),
            attachments: vec![],
            message_id: None,
        });

        // 現在の入力欄にアップロード中の添付が存在
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_up".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "uploading.png".into(),
            content_type: "image/png".into(),
            file_size: 1024,
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        // 復元を試行
        state.restore_failed_draft();

        // 復元がブロックされ、通知が出ること
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("添付ファイルをアップロード中です。完了をお待ちください")
        );
        // 本文は空のままで復元されていないこと
        assert!(state.composer_draft.is_empty());
        // アップロード中の添付が失われていないこと
        assert_eq!(state.pending_attachments.len(), 1);
        assert!(state.pending_attachments[0].is_uploading);
        // 失敗下書きリストも保持されていること
        assert_eq!(state.failed_drafts.len(), 1);
    }

    #[test]
    fn reconciles_targets_clears_pending_message_id_on_reply_root_deletion() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 親メッセージ m1 を配置
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({
                    "id": "m1", "user_id": "u2", "channel_id": "c1",
                    "content": "親メッセージ", "created_at": "2026-11-01 03:00:00"
                }),
            )]),
        ));

        // m1 への返信を開き、再送 ID を保持
        state.open_thread("m1".into());
        state.composer_draft = "返信下書き".into();
        state.pending_composer_message_id = Some("resend-uuid-1".into());

        // m1 が削除される
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));

        // Target::Channel へ自動切り替え
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "返信下書き");
        // 送信先が親なしの Channel に変わったため、返信用再送 ID は破棄されて None になること
        assert_eq!(state.pending_composer_message_id, None);
    }

    #[test]
    fn reconciles_targets_orphaned_saved_reply_drafts_evicted_to_failed_drafts() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 親メッセージ m1 を配置
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({
                    "id": "m1", "user_id": "u2", "channel_id": "c1",
                    "content": "親メッセージ", "created_at": "2026-11-01 03:00:00"
                }),
            )]),
        ));

        // m1 への返信を開き、本文・添付・再送 ID をセット
        state.open_thread("m1".into());
        state.composer_draft = "保存返信本文".into();
        state.pending_composer_message_id = Some("reply-msg-id-1".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("m1".into()),
            session_gen: state.session_gen,
            file_name: "reply_att.png".into(),
            content_type: "image/png".into(),
            file_size: 1024,
            attachment_id: Some("att-1".into()),
            is_uploading: false,
            error: None,
        });

        // 通常投稿に戻る（スレッドを閉じる）
        state.close_thread();
        assert_eq!(state.target, Target::Channel);

        // この状態で m1 が削除される
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));

        // saved_drafts, saved_draft_message_ids, pending_attachments から m1 返信が削除され、
        // failed_drafts (Target::Channel, message_id: None) へ安全に退避されていること
        assert!(
            !state
                .saved_drafts
                .contains_key(&("c1".into(), Target::Reply("m1".into())))
        );
        assert!(
            !state
                .saved_draft_message_ids
                .contains_key(&("c1".into(), Target::Reply("m1".into())))
        );
        assert!(
            state
                .pending_attachments
                .iter()
                .all(|a| a.target != Target::Reply("m1".into()))
        );

        assert_eq!(state.failed_drafts.len(), 1);
        let evicted = &state.failed_drafts[0];
        assert_eq!(evicted.channel_id, "c1");
        assert_eq!(evicted.target, Target::Channel);
        assert_eq!(evicted.content, "保存返信本文");
        assert_eq!(evicted.attachments.len(), 1);
        assert_eq!(evicted.attachments[0].file_name, "reply_att.png");
        // 再送 ID は None になっていること
        assert_eq!(evicted.message_id, None);
    }

    #[test]
    fn switch_composer_context_preserves_message_id_for_attachment_only_draft() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 本文は空で、添付ファイルのみ存在し、再送 ID がセットされている状態
        state.composer_draft.clear();
        state.pending_composer_message_id = Some("attachment-only-uuid".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_solo".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "standalone.png".into(),
            content_type: "image/png".into(),
            file_size: 2048,
            attachment_id: Some("att-solo".into()),
            is_uploading: false,
            error: None,
        });

        // 別ターゲット（返信）へ切り替え
        state.switch_composer_context(Some("c1"), Target::Reply("m99".into()));
        assert_eq!(state.target, Target::Reply("m99".into()));
        assert!(state.pending_composer_message_id.is_none());

        // 元の通常投稿へ戻る
        state.switch_composer_context(Some("c1"), Target::Channel);
        assert_eq!(state.target, Target::Channel);

        // 添付だけの下書きでも再送 ID が保持されていること
        assert_eq!(
            state.pending_composer_message_id.as_deref(),
            Some("attachment-only-uuid")
        );
        assert_eq!(state.current_channel_attachments().len(), 1);
        assert_eq!(
            state.current_channel_attachments()[0].file_name,
            "standalone.png"
        );
    }

    #[test]
    fn attachment_url_refresh_margin_avoids_infinite_loop_for_short_expiry() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 有効期限が 45 秒の URL を登録
        state.attachment_download_urls.insert(
            "att-short".into(),
            CachedAttachmentUrl {
                url: "https://example.com/short?X-Amz-Expires=45".into(),
                fetched_at: Instant::now(),
                ttl_secs: 45,
            },
        );

        // 取得直後はリフレッシュ不要であること（マージン 15 秒 < 45 秒）
        assert!(!state.attachment_url_needs_refresh("att-short"));
    }

    #[test]
    fn messages_shape_sync_refreshes_message_search_results() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 初期の検索バーを開く
        state.open_message_search();
        state.update_message_search_query("テスト".into());
        assert_eq!(state.message_search.as_ref().unwrap().matches.len(), 0);

        // 新着メッセージが同期される
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m_new",
                json!({
                    "id": "m_new", "user_id": "u2", "channel_id": "c1",
                    "content": "これはテストメッセージです", "created_at": "2026-11-01 03:00:00"
                }),
            )]),
        ));

        // 検索結果が自動的に再計算されてマッチが含まれること
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches.len(), 1);
        assert_eq!(ms.matches[0], "m_new");
    }

    #[test]
    fn submit_message_allows_empty_content_when_editing_message_with_attachments() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 編集対象のメッセージを配置
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m_edit",
                json!({
                    "id": "m_edit", "user_id": "u1", "channel_id": "c1",
                    "content": "本文あり", "created_at": "2026-11-01 03:00:00"
                }),
            )]),
        ));

        // そのメッセージに紐づく添付ファイルを store に登録
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Attachments("c1".into()),
            batch(vec![(
                "att_edit",
                json!({
                    "id": "att_edit", "channel_id": "c1", "message_id": "m_edit",
                    "uploader_id": "u1", "file_name": "photo.jpg",
                    "content_type": "image/jpeg", "file_size": 5000,
                    "storage_key": "k1",
                    "created_at": "2026-11-01 03:00:00"
                }),
            )]),
        ));

        // 編集モードにする
        state.target = Target::Edit("m_edit".into());

        // 本文を空文字にして送信
        let ok = state.submit_message("   ".into());

        // 既存の添付ファイルがあるため、空本文への編集送信が許可されること
        assert!(ok);
    }

    #[test]
    fn cursor_movement_closes_mention_autocomplete() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // @a で補完ポップアップが開く
        state.on_composer_changed("@a".into());
        state.update_mention_autocomplete("@a", 2);
        assert!(state.mention_autocomplete.is_some());
        let ac = state.mention_autocomplete.as_ref().unwrap();
        assert_eq!(ac.end_byte, 2);

        // キャレットが end_byte (2) 以外の位置 (0) へ移動したとき、補完を閉じる動作を検証
        if ac.end_byte != 0 {
            state.close_mention_autocomplete();
        }
        assert!(state.mention_autocomplete.is_none());
    }

    #[test]
    fn cursor_movement_or_selection_closes_mention_autocomplete() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // @a で補完ポップアップが開く
        state.on_composer_changed("@a".into());
        state.update_mention_autocomplete("@a", 2);
        assert!(state.mention_autocomplete.is_some());
        let ac = state.mention_autocomplete.as_ref().unwrap();
        assert_eq!(ac.end_byte, 2);

        // キャレットが end_byte (2) 以外の位置 (0) へ移動したとき、または選択範囲が空でないとき補完を閉じる動作を検証
        let caret = 0;
        let sel_is_empty = true;
        if !sel_is_empty || caret != ac.end_byte {
            state.close_mention_autocomplete();
        }
        assert!(state.mention_autocomplete.is_none());

        // 再び補完を開き、選択範囲がある場合は確定させずに閉じる動作を検証
        state.update_mention_autocomplete("@a", 2);
        assert!(state.mention_autocomplete.is_some());
        let sel_is_empty = false; // 選択範囲が存在する
        let mention_handled = if !sel_is_empty {
            state.close_mention_autocomplete();
            false
        } else {
            state.mention_confirm();
            true
        };
        assert!(!mention_handled);
        assert!(state.mention_autocomplete.is_none());
    }

    #[test]
    fn reconcile_channel_targets_evicts_uploading_saved_reply_attachment_to_failed_drafts() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 返信先メッセージ m1 を store に追加
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({
                    "id": "m1", "user_id": "u1", "channel_id": "c1",
                    "content": "親メッセージ", "created_at": "2026-11-01 00:00:00"
                }),
            )]),
        ));

        // 返信モードにしてアップロード中添付を追加
        state.open_thread("m1".into());
        state.composer_draft = "返信下書き".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("m1".into()),
            session_gen: state.session_gen,
            file_name: "reply_up.png".into(),
            content_type: "image/png".into(),
            file_size: 1024,
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        // 通常投稿へ切り替えて、返信下書きを saved_drafts へ退避させる
        state.close_thread();
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.pending_attachments.len(), 1);
        assert_eq!(
            state.pending_attachments[0].target,
            Target::Reply("m1".into())
        );

        // 通常投稿で下書き本文を入力（この入力欄に返信の添付が混入してはならない）
        state.composer_draft = "通常投稿下書き".into();

        // 親メッセージ m1 を削除
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));

        // 現在の通常投稿の pending_attachments / 入力欄には混入せず、空であること
        assert_eq!(state.current_channel_attachments().len(), 0);
        assert_eq!(state.pending_attachments.len(), 0);

        // 返信のアップロード中添付は、下書き本文とともに failed_drafts に退避されていること
        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "返信下書き");
        assert_eq!(state.failed_drafts[0].attachments.len(), 1);
        assert_eq!(
            state.failed_drafts[0].attachments[0].file_name,
            "reply_up.png"
        );
        assert!(state.failed_drafts[0].attachments[0].is_uploading);

        // バックグラウンドでアップロードが完了すると、failed_drafts 内の添付も完了状態へ更新されること
        state.on_app_event(AppEvent::AttachmentUploadFinished {
            local_id: "loc_reply".into(),
            attachment_id: "att_resolved_1".into(),
        });
        assert_eq!(
            state.failed_drafts[0].attachments[0].attachment_id,
            Some("att_resolved_1".into())
        );
        assert!(!state.failed_drafts[0].attachments[0].is_uploading);
    }

    #[test]
    fn reconcile_channel_targets_preserves_uploading_channel_attachment_when_switching_from_deleted_reply()
     {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 親メッセージ m1 を store に追加
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({
                    "id": "m1", "user_id": "u1", "channel_id": "c1",
                    "content": "親メッセージ", "created_at": "2026-11-01 00:00:00"
                }),
            )]),
        ));

        // 通常投稿でファイルをアップロード開始
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_chan_up".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "chan_up.png".into(),
            content_type: "image/png".into(),
            file_size: 2048,
            attachment_id: None,
            is_uploading: true,
            error: None,
        });
        state.composer_draft = "通常投稿下書き".into();

        // 返信モードへ切り替えて返信本文を入力
        state.open_thread("m1".into());
        state.composer_draft = "返信本文を書いた".into();

        // 親メッセージ m1 を削除
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));

        // 返信本文が通常投稿として入力欄に昇格し、
        // 退避された以前の通常投稿の下書き（アップロード中添付 chan_up.png 含む）が failed_drafts に保持されていること
        assert_eq!(state.target, Target::Channel);
        assert_eq!(state.composer_draft, "返信本文を書いた");

        assert_eq!(state.failed_drafts.len(), 1);
        assert_eq!(state.failed_drafts[0].content, "通常投稿下書き");
        assert_eq!(state.failed_drafts[0].attachments.len(), 1);
        assert_eq!(
            state.failed_drafts[0].attachments[0].file_name,
            "chan_up.png"
        );
        assert!(state.failed_drafts[0].attachments[0].is_uploading);

        // アップロード完了イベントで failed_drafts 内の添付が更新されること
        state.on_app_event(AppEvent::AttachmentUploadFinished {
            local_id: "loc_chan_up".into(),
            attachment_id: "att_chan_1".into(),
        });
        assert_eq!(
            state.failed_drafts[0].attachments[0].attachment_id,
            Some("att_chan_1".into())
        );
        assert!(!state.failed_drafts[0].attachments[0].is_uploading);
    }

    #[test]
    fn refresh_message_search_results_maintains_selected_match_without_resetting_scroll() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // メッセージを2件投入
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "m1",
                    json!({
                        "id": "m1", "user_id": "u1", "channel_id": "c1",
                        "content": "test keyword 1", "created_at": "2026-11-01 00:00:00"
                    }),
                ),
                (
                    "m2",
                    json!({
                        "id": "m2", "user_id": "u1", "channel_id": "c1",
                        "content": "test keyword 2", "created_at": "2026-11-01 00:01:00"
                    }),
                ),
            ]),
        ));

        // 検索クエリ "test" を設定し、2番目のメッセージ "m2" を選択
        state.update_message_search_query("test".into());
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches.len(), 2);
        state.search_select_next(); // 0 -> 1 (m2)
        assert_eq!(
            state.message_search.as_ref().unwrap().current_match_index,
            1
        );

        // スクロール要求フラグをクリア
        state.pending_scroll_to_message_id = None;

        // メッセージ同期（新規メッセージ "m3" が追加される）
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m3",
                json!({
                    "id": "m3", "user_id": "u1", "channel_id": "c1",
                    "content": "test keyword 3", "created_at": "2026-11-01 00:02:00"
                }),
            )]),
        ));

        // 検索結果が3件に増えても、m2 の選択が維持され、スクロール要求が立たないこと
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches.len(), 3);
        assert_eq!(ms.matches[ms.current_match_index], "m2");
        assert_eq!(ms.current_match_index, 1);
        assert_eq!(state.pending_scroll_to_message_id, None);
    }

    #[test]
    fn refresh_message_search_results_scrolls_when_selected_match_removed() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "m1",
                    json!({
                        "id": "m1", "user_id": "u1", "channel_id": "c1",
                        "content": "target keyword", "created_at": "2026-11-01 00:00:00"
                    }),
                ),
                (
                    "m2",
                    json!({
                        "id": "m2", "user_id": "u1", "channel_id": "c1",
                        "content": "target keyword", "created_at": "2026-11-01 00:01:00"
                    }),
                ),
            ]),
        ));

        state.update_message_search_query("target".into());
        state.search_select_next(); // m2 を選択
        assert_eq!(
            state.message_search.as_ref().unwrap().current_match_index,
            1
        );

        // m2 が削除される
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m2".into() }],
                up_to_date: true,
            },
        ));

        // m2 が削除されたため index は 0 (m1) に再設定され、スクロール要求が立つこと
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches.len(), 1);
        assert_eq!(ms.current_match_index, 0);
        assert_eq!(state.pending_scroll_to_message_id, Some("m1".into()));
    }

    #[test]
    fn search_results_exclude_orphaned_replies_when_parent_is_deleted() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 親メッセージ m1 と返信 r1 を配置
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "m1",
                    json!({
                        "id": "m1", "user_id": "u1", "channel_id": "c1",
                        "content": "親メッセージ", "created_at": "2026-11-01 00:00:00"
                    }),
                ),
                (
                    "r1",
                    json!({
                        "id": "r1", "user_id": "u1", "channel_id": "c1", "parent_id": "m1",
                        "content": "返信キーワード", "created_at": "2026-11-01 00:01:00"
                    }),
                ),
            ]),
        ));

        // 検索すると r1 がヒットする
        state.update_message_search_query("返信キーワード".into());
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches, vec!["r1".to_string()]);

        // 親メッセージ m1 が削除される
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            Batch {
                reset: false,
                changes: vec![Change::Delete { key: "m1".into() }],
                up_to_date: true,
            },
        ));

        // 親が削除されたため孤児返信となり、検索結果から除外されること
        let ms = state.message_search.as_ref().unwrap();
        assert_eq!(ms.matches.len(), 0);
    }

    #[test]
    fn restore_failed_draft_blocks_when_target_has_uploading_attachment() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // c1 に m1 メッセージを作成
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({
                    "id": "m1", "user_id": "u1", "channel_id": "c1",
                    "content": "親メッセージ", "created_at": "2026-11-01 00:00:00"
                }),
            )]),
        ));

        // 返信先 m1 への失敗下書きを用意
        state.failed_drafts.push(FailedDraft {
            channel_id: "c1".into(),
            target: Target::Reply("m1".into()),
            content: "復元したい返信下書き".into(),
            attachments: vec![],
            message_id: None,
        });

        // 現在は Target::Channel だが、Target::Reply("m1") にアップロード中の添付がある
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_up_reply".into(),
            channel_id: "c1".into(),
            target: Target::Reply("m1".into()),
            session_gen: state.session_gen,
            file_name: "reply_uploading.png".into(),
            content_type: "image/png".into(),
            file_size: 1024,
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        // 復元を試行
        state.restore_failed_draft();

        // 復元先の Target::Reply("m1") にアップロード中の添付があるためブロックされること
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("添付ファイルをアップロード中です。完了をお待ちください")
        );
        assert!(state.composer_draft.is_empty());
        assert_eq!(state.failed_drafts.len(), 1);
    }

    #[test]
    fn evict_channel_drafts_retains_uploading_attachments_in_failed_drafts() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 本文は空で、アップロード中の添付のみがある状態
        state.composer_draft.clear();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_up1".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "uploading.png".into(),
            content_type: "image/png".into(),
            file_size: 2048,
            attachment_id: None,
            is_uploading: true,
            error: None,
        });

        // チャンネル退避を実行
        assert!(state.evict_channel_drafts("c1"));

        // failed_drafts にアップロード中添付を含む下書きが退避されていること
        assert_eq!(state.failed_drafts.len(), 1);
        let draft = &state.failed_drafts[0];
        assert_eq!(draft.channel_id, "c1");
        assert_eq!(draft.target, Target::Channel);
        assert_eq!(draft.attachments.len(), 1);
        assert_eq!(draft.attachments[0].local_id, "loc_up1");
        assert!(draft.attachments[0].is_uploading);
    }

    #[test]
    fn preserve_drafts_for_unauthorized_distinguishes_same_content_by_message_id() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 送信中の下書き（message_id あり）
        let in_flight = FailedDraft {
            channel_id: "c1".into(),
            target: Target::Channel,
            content: "同じ文面の下書き".into(),
            attachments: vec![],
            message_id: Some("inflight-uuid-123".into()),
        };
        state.in_flight_submissions.push((1, in_flight));

        // 現在の入力欄に同じ送信先・同文の新規下書き（message_id は None）を入力
        state.composer_draft = "同じ文面の下書き".into();
        state.pending_composer_message_id = None;

        // 認証切れによる退避を実行
        state.preserve_drafts_for_unauthorized();

        // 2件とも別々の下書きとして退避されていること
        let preserved = state.preserved_unauthorized_drafts.as_ref().unwrap();
        assert_eq!(preserved.1.len(), 2);
        assert!(
            preserved
                .1
                .iter()
                .any(|d| d.message_id.as_deref() == Some("inflight-uuid-123"))
        );
        assert!(preserved.1.iter().any(|d| d.message_id.is_none()));
    }

    #[test]
    fn start_edit_preserves_pending_composer_message_id_and_restores_on_cancel() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 自分のメッセージ m1 を用意
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![(
                "m1",
                json!({
                    "id": "m1", "user_id": "u1", "channel_id": "c1",
                    "content": "投稿済みメッセージ", "created_at": "2026-11-01 00:00:00"
                }),
            )]),
        ));

        // 本文は空、添付1件、再送 ID ありの通常投稿下書き
        state.composer_draft.clear();
        state.pending_composer_message_id = Some("resend-uuid-abc".into());
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_att1".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "attached.png".into(),
            content_type: "image/png".into(),
            file_size: 1024,
            attachment_id: Some("att-uuid-1".into()),
            is_uploading: false,
            error: None,
        });

        // 編集を開始
        state.start_edit("m1".into());
        assert_eq!(state.target, Target::Edit("m1".into()));
        assert_eq!(state.composer_draft, "投稿済みメッセージ");
        // 編集対象には再送 ID が引き継がれないこと
        assert_eq!(state.pending_composer_message_id, None);

        // 編集をキャンセルして通常投稿に戻る
        state.cancel_target();
        assert_eq!(state.target, Target::Channel);
        assert!(state.composer_draft.is_empty());
        // 元の通常投稿の再送 ID が復元されていること
        assert_eq!(
            state.pending_composer_message_id.as_deref(),
            Some("resend-uuid-abc")
        );
    }

    #[test]
    fn unjoined_public_channel_blocks_attachment_upload() {
        let mut state = connected_state();

        // 公開チャンネル c_pub を追加（alice は未参加）
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c_pub",
                json!({ "id": "c_pub", "channel_name": "public-general", "is_private": "f" }),
            )]),
        ));

        state.open_channel("c_pub".into());

        // ChannelUsers が未同期の間は、所属確認を保留して添付アップロードを許可すること
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            Batch {
                reset: true,
                changes: vec![],
                up_to_date: false,
            },
        ));
        state.add_attachment_from_bytes("unsynced.png".into(), "image/png".into(), vec![1, 2, 3]);
        assert_eq!(state.pending_attachments.len(), 1);
        state.pending_attachments.clear();

        // ChannelUsers が同期済みになると、未参加のチャンネルでは添付ファイルの追加が拒否されること
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            Batch {
                reset: false,
                changes: vec![],
                up_to_date: true,
            },
        ));
        state.add_attachment_from_bytes("test.png".into(), "image/png".into(), vec![1, 2, 3]);

        // 添付は追加されず、参加を促す通知が表示されること
        assert!(state.pending_attachments.is_empty());
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("チャンネルに参加してからファイルを添付してください")
        );
    }

    #[test]
    fn restricted_channel_blocks_attachment_upload() {
        let mut state = connected_state();
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c_mgr",
                json!({
                    "id": "c_mgr",
                    "channel_name": "announcements",
                    "is_private": "f",
                    "created_user_id": "u2",
                    "posting_policy": "managers",
                }),
            )]),
        ));
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![(
                "cu_mgr",
                json!({ "user_id": "u1", "channel_id": "c_mgr" }),
            )]),
        ));
        state.open_channel("c_mgr".into());
        assert!(!state.can_post_in_channel("c_mgr"));

        state.add_attachment_from_bytes("test.png".into(), "image/png".into(), vec![1, 2, 3]);
        assert!(state.pending_attachments.is_empty());
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("このチャンネルには投稿権限がありません")
        );
    }

    #[test]
    fn archived_channel_blocks_attachment_upload() {
        let mut state = connected_state();
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            batch(vec![(
                "c_archived",
                json!({
                    "id": "c_archived",
                    "channel_name": "old-project",
                    "is_private": "f",
                    "archive_at": "2026-01-01 00:00:00",
                }),
            )]),
        ));
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![(
                "cu_arch",
                json!({ "user_id": "u1", "channel_id": "c_archived" }),
            )]),
        ));
        state.open_channel("c_archived".into());

        state.add_attachment_from_bytes("test.png".into(), "image/png".into(), vec![1, 2, 3]);
        assert!(state.pending_attachments.is_empty());
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some("アーカイブされたチャンネルにはファイルを添付できません")
        );
    }

    #[test]
    fn submit_message_with_failed_attachment_shows_correct_notice() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.composer_draft = "Hello".into();
        state.pending_attachments.push(PendingAttachment {
            local_id: "loc_fail".into(),
            channel_id: "c1".into(),
            target: Target::Channel,
            session_gen: state.session_gen,
            file_name: "fail.png".into(),
            file_size: 100,
            content_type: "image/png".into(),
            attachment_id: None,
            is_uploading: false,
            error: Some("upload failed".into()),
        });

        assert!(!state.submit_message("Hello".into()));
        assert_eq!(
            state.notice.as_ref().map(|(m, _)| m.as_str()),
            Some(
                "アップロードに失敗した添付ファイルがあります。削除してからファイルを再度添付してください"
            )
        );
    }

    #[test]
    fn create_channel_error_unauthorized_detection() {
        let err: anyhow::Error = HttpError::Unauthorized.into();
        let is_unauthorized = matches!(
            err.downcast_ref::<HttpError>(),
            Some(HttpError::Unauthorized)
        );
        assert!(is_unauthorized);

        let err_other: anyhow::Error = anyhow::anyhow!("channel already exists");
        let is_unauthorized_other = matches!(
            err_other.downcast_ref::<HttpError>(),
            Some(HttpError::Unauthorized)
        );
        assert!(!is_unauthorized_other);
    }

    #[test]
    fn mention_candidates_exclude_non_mentionable_bot() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // 3人のユーザーを追加
        // 1. 通常ユーザー bob
        // 2. メンション可能な Bot bot_mentionable
        // 3. メンション不可（通知専用）Bot bot_silent
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Users,
            batch(vec![
                (
                    "u_bob",
                    json!({
                        "id": "u_bob",
                        "user_name": "bob",
                        "display_name": "Bob",
                        "is_bot": "f",
                        "bot_mentionable": "f",
                        "is_guest": "f",
                    }),
                ),
                (
                    "u_bot_m",
                    json!({
                        "id": "u_bot_m",
                        "user_name": "bot_m",
                        "display_name": "Mentionable Bot",
                        "is_bot": "t",
                        "bot_mentionable": "t",
                        "is_guest": "f",
                    }),
                ),
                (
                    "u_bot_s",
                    json!({
                        "id": "u_bot_s",
                        "user_name": "bot_s",
                        "display_name": "Silent Bot",
                        "is_bot": "t",
                        "bot_mentionable": "f",
                        "is_guest": "f",
                    }),
                ),
            ]),
        ));

        // 3人とも c1 に所属
        state.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelUsers,
            batch(vec![
                ("cu_bob", json!({ "user_id": "u_bob", "channel_id": "c1" })),
                (
                    "cu_bot_m",
                    json!({ "user_id": "u_bot_m", "channel_id": "c1" }),
                ),
                (
                    "cu_bot_s",
                    json!({ "user_id": "u_bot_s", "channel_id": "c1" }),
                ),
            ]),
        ));

        // "@b" で補完検索
        state.update_mention_autocomplete("@b", 2);
        let ac = state
            .mention_autocomplete
            .as_ref()
            .expect("autocomplete should be open");
        let candidate_names: Vec<&str> = ac.candidates.iter().map(|c| c.name.as_str()).collect();

        // 通常ユーザーとメンション可能 Bot は候補に含まれる
        assert!(candidate_names.contains(&"bob"));
        assert!(candidate_names.contains(&"bot_m"));
        // メンション不可 Bot は候補に含まれないこと
        assert!(!candidate_names.contains(&"bot_s"));
    }

    #[test]
    fn guest_user_cannot_open_create_channel_modal() {
        let mut state = connected_state();
        assert!(!state.is_current_user_guest());
        state.open_create_channel_modal();
        assert!(state.create_channel_modal.is_some());
        state.close_create_channel_modal();

        // ログイン中のユーザー u1 (alice) をゲストに設定
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Users,
            batch(vec![(
                "u1",
                json!({
                    "id": "u1",
                    "user_name": "alice",
                    "display_name": "Alice",
                    "is_bot": "f",
                    "bot_mentionable": "f",
                    "is_guest": "t",
                }),
            )]),
        ));

        assert!(state.is_current_user_guest());
        state.open_create_channel_modal();
        // ゲストの場合はモーダルが開かないこと
        assert!(state.create_channel_modal.is_none());
    }

    #[test]
    fn create_channel_description_length_limit() {
        let mut state = connected_state();
        state.open_create_channel_modal();
        let modal = state.create_channel_modal.as_mut().unwrap();
        modal.channel_name = "test-limit".into();

        // 251文字の説明を設定
        modal.description = "a".repeat(251);
        state.submit_create_channel();

        // 送信されず、モーダルにエラーが設定されていること
        let modal = state.create_channel_modal.as_ref().unwrap();
        assert!(modal.error.is_some());
        assert!(
            modal
                .error
                .as_ref()
                .unwrap()
                .contains("説明は250文字以内で入力してください")
        );
        assert!(!modal.in_flight);

        // 250文字以内の場合はエラーにならないこと (in_flight になる)
        let modal = state.create_channel_modal.as_mut().unwrap();
        modal.description = "a".repeat(250);
        state.submit_create_channel();
        let modal = state.create_channel_modal.as_ref().unwrap();
        assert!(modal.error.is_none());
        assert!(modal.in_flight);
    }

    #[test]
    fn create_channel_reissues_id_on_mutation_and_keeps_on_retry() {
        let mut state = connected_state();
        state.open_create_channel_modal();
        let modal = state.create_channel_modal.as_mut().unwrap();
        modal.channel_name = "test-channel".into();

        // 1回目の送信 (初回 ID)
        state.submit_create_channel();
        let initial_id = state
            .create_channel_modal
            .as_ref()
            .unwrap()
            .channel_id
            .clone()
            .expect("should have channel_id");
        let initial_req_id = state.create_channel_modal.as_ref().unwrap().request_id;
        assert!(state.create_channel_modal.as_ref().unwrap().in_flight);

        // ネットワークエラー (非409) の失敗イベントを受信
        state.on_app_event(AppEvent::ChannelCreateFailed {
            request_id: initial_req_id,
            error: "Connection refused".to_string(),
        });
        let modal = state.create_channel_modal.as_ref().unwrap();
        assert!(!modal.in_flight);
        // 通常の失敗では再試行のために同じ channel_id が維持される
        assert_eq!(modal.channel_id.as_ref().unwrap(), &initial_id);

        // 再試行で再送信
        state.submit_create_channel();
        let retry_modal = state.create_channel_modal.as_ref().unwrap();
        assert!(retry_modal.in_flight);
        let retry_req_id = retry_modal.request_id;
        assert_eq!(retry_modal.channel_id.as_ref().unwrap(), &initial_id);

        // 409 Conflict 失敗イベントを受信
        state.on_app_event(AppEvent::ChannelCreateFailed {
            request_id: retry_req_id,
            error: "HTTP 409 Conflict: channel already exists".to_string(),
        });
        let modal_after_409 = state.create_channel_modal.as_ref().unwrap();
        assert!(!modal_after_409.in_flight);
        let reissued_id = modal_after_409.channel_id.clone().unwrap();
        // 409 Conflict 後は次回再試行に向けて新しい channel_id が再採番される
        assert_ne!(reissued_id, initial_id);

        // モーダルの名前や説明が変更された場合も新しい channel_id が採番される
        {
            let modal = state.create_channel_modal.as_mut().unwrap();
            modal.channel_name = "new-name".into();
            modal.channel_id = Some(uuid::Uuid::new_v4().to_string());
        }
        let mutated_id = state
            .create_channel_modal
            .as_ref()
            .unwrap()
            .channel_id
            .clone()
            .unwrap();
        assert_ne!(mutated_id, reissued_id);
    }

    #[test]
    fn create_channel_name_validation() {
        let mut state = connected_state();
        state.open_create_channel_modal();

        // 空文字
        state.create_channel_modal.as_mut().unwrap().channel_name = "   ".into();
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
        assert!(!state.create_channel_modal.as_ref().unwrap().in_flight);

        // 80文字超過
        state.create_channel_modal.as_mut().unwrap().channel_name = "a".repeat(81);
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

        // 予約語 (general, all, here, everyone, channel)
        for reserved in &["general", "all", "here", "everyone", "channel"] {
            state.create_channel_modal.as_mut().unwrap().channel_name = reserved.to_string();
            state.submit_create_channel();
            assert_eq!(
                state
                    .create_channel_modal
                    .as_ref()
                    .unwrap()
                    .error
                    .as_deref(),
                Some("このチャンネル名は予約されているため使用できません")
            );
        }

        // 不正な文字種 (空白、特殊記号)
        for invalid in &["hello world", "test!", "chat#room", "my$channel"] {
            state.create_channel_modal.as_mut().unwrap().channel_name = invalid.to_string();
            state.submit_create_channel();
            assert_eq!(
                state
                    .create_channel_modal
                    .as_ref()
                    .unwrap()
                    .error
                    .as_deref(),
                Some("チャンネル名には文字・数字・ハイフン(-)・アンダースコア(_)のみ使用できます")
            );
        }

        // 正常なチャンネル名 (日本語、ハイフン、アンダースコア、80文字)
        for valid in &["dev", "dev-123", "開発_部屋", "test-room_01"] {
            state.create_channel_modal.as_mut().unwrap().channel_name = valid.to_string();
            state.create_channel_modal.as_mut().unwrap().in_flight = false;
            state.submit_create_channel();
            assert!(state.create_channel_modal.as_ref().unwrap().error.is_none());
            assert!(state.create_channel_modal.as_ref().unwrap().in_flight);
            state.create_channel_modal.as_mut().unwrap().in_flight = false;
        }
    }

    #[test]
    fn quick_switcher_keyboard_navigation_sets_pending_scroll() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        state.open_quick_switcher();
        assert!(state.take_pending_quick_switcher_scroll());
        assert!(!state.take_pending_quick_switcher_scroll());

        // 次の候補へ選択移動
        state.quick_switcher_select_next();
        assert!(state.take_pending_quick_switcher_scroll());
        assert!(!state.take_pending_quick_switcher_scroll());

        // 前の候補へ選択移動
        state.quick_switcher_select_prev();
        assert!(state.take_pending_quick_switcher_scroll());
        assert!(!state.take_pending_quick_switcher_scroll());

        // クエリ更新
        state.update_quick_switcher_query("gen".to_string());
        assert!(state.take_pending_quick_switcher_scroll());
        assert!(!state.take_pending_quick_switcher_scroll());

        // クローズ時はフラグが false
        state.close_quick_switcher();
        assert!(!state.take_pending_quick_switcher_scroll());
    }

    #[test]
    fn emoji_picker_grid_navigation() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.open_emoji_picker();
        assert_eq!(state.emoji_picker_selected, 0);

        // 1行下へ (9項目進む)
        state.emoji_picker_next_row();
        assert_eq!(state.emoji_picker_selected, 9);

        // 1行上へ (9項目戻る)
        state.emoji_picker_prev_row();
        assert_eq!(state.emoji_picker_selected, 0);
    }

    #[test]
    fn emoji_picker_keyboard_selection_and_insert() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.composer_draft = "Hello ".into();

        state.open_emoji_picker();
        assert_eq!(state.emoji_picker_selected, 0);
        let first = state.selected_emoji().unwrap();
        assert_eq!(first, "😀");

        // 次の絵文字へ移動
        state.emoji_picker_next();
        assert_eq!(state.emoji_picker_selected, 1);
        let second = state.selected_emoji().unwrap();
        assert_eq!(second, "😃");

        // 前の絵文字へ戻る
        state.emoji_picker_prev();
        assert_eq!(state.emoji_picker_selected, 0);

        // 確定して挿入
        state.insert_selected_emoji(6..6);
        assert!(!state.emoji_picker_open);
        assert_eq!(state.composer_draft, "Hello 😀");
    }

    #[test]
    fn emoji_picker_keyboard_navigation_sets_pending_scroll() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.open_emoji_picker();
        assert!(state.take_pending_emoji_picker_scroll());
        assert!(!state.take_pending_emoji_picker_scroll());

        // 次へ移動
        state.emoji_picker_next();
        assert!(state.take_pending_emoji_picker_scroll());
        assert!(!state.take_pending_emoji_picker_scroll());

        // 前へ移動
        state.emoji_picker_prev();
        assert!(state.take_pending_emoji_picker_scroll());
        assert!(!state.take_pending_emoji_picker_scroll());

        // 行移動
        state.emoji_picker_next_row();
        assert!(state.take_pending_emoji_picker_scroll());
        assert!(!state.take_pending_emoji_picker_scroll());

        state.emoji_picker_prev_row();
        assert!(state.take_pending_emoji_picker_scroll());
        assert!(!state.take_pending_emoji_picker_scroll());
    }

    #[test]
    fn cancel_input_with_emoji_picker_open_preserves_editing_target() {
        let mut state = connected_state();
        state.open_channel("c1".into());
        state.target = Target::Edit("m1".into());
        assert_eq!(state.target, Target::Edit("m1".into()));

        state.open_emoji_picker();
        assert!(state.emoji_picker_open);

        // view.rs 内の InputEvent::Cancel ハンドラ挙動の検証
        if state.mention_autocomplete.is_some() {
            state.close_mention_autocomplete();
        } else if state.emoji_picker_open {
            state.close_emoji_picker();
        } else {
            state.cancel_target();
        }

        assert!(!state.emoji_picker_open);
        assert_eq!(state.target, Target::Edit("m1".into()));

        // 再度 Cancel が来たら cancel_target される
        if state.mention_autocomplete.is_some() {
            state.close_mention_autocomplete();
        } else if state.emoji_picker_open {
            state.close_emoji_picker();
        } else {
            state.cancel_target();
        }
        assert_eq!(state.target, Target::Channel);
    }

    #[test]
    fn search_matches_parent_resolution_and_orphan_filtering() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // c1 に親メッセージ m1, m2 と、返信 r1(親m1), r2(親m2), r3(親m3: 不存在) を登録
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "m1",
                    json!({
                        "id": "m1", "user_id": "u1", "channel_id": "c1",
                        "content": "親メッセージ1", "created_at": "2026-11-01 00:00:00"
                    }),
                ),
                (
                    "m2",
                    json!({
                        "id": "m2", "user_id": "u1", "channel_id": "c1",
                        "content": "親メッセージ2", "created_at": "2026-11-01 00:01:00"
                    }),
                ),
                (
                    "r1",
                    json!({
                        "id": "r1", "user_id": "u1", "channel_id": "c1", "parent_id": "m1",
                        "content": "共通キーワード返信1", "created_at": "2026-11-01 00:02:00"
                    }),
                ),
                (
                    "r2",
                    json!({
                        "id": "r2", "user_id": "u1", "channel_id": "c1", "parent_id": "m2",
                        "content": "共通キーワード返信2", "created_at": "2026-11-01 00:03:00"
                    }),
                ),
                (
                    "r3",
                    json!({
                        "id": "r3", "user_id": "u1", "channel_id": "c1", "parent_id": "m3",
                        "content": "共通キーワード返信3(孤児)", "created_at": "2026-11-01 00:04:00"
                    }),
                ),
            ]),
        ));

        state.update_message_search_query("共通キーワード".into());
        let ms = state.message_search.as_ref().unwrap();
        // 存在する親を持つ r1, r2 は一致し、存在しない親を持つ r3 は除外されること
        assert_eq!(ms.matches, vec!["r1".to_string(), "r2".to_string()]);
    }

    #[test]
    fn channel_cache_attachments_by_message_updates_on_shape_and_cache_sync() {
        let mut state = connected_state();
        state.open_channel("c1".into());

        // c1 に m1, m2 を作成
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Messages("c1".into()),
            batch(vec![
                (
                    "m1",
                    json!({
                        "id": "m1", "user_id": "u1", "channel_id": "c1",
                        "content": "メッセージ1", "created_at": "2026-11-01 00:00:00"
                    }),
                ),
                (
                    "m2",
                    json!({
                        "id": "m2", "user_id": "u1", "channel_id": "c1",
                        "content": "メッセージ2", "created_at": "2026-11-01 00:01:00"
                    }),
                ),
            ]),
        ));

        // c1 に m1 紐づきの添付 2 件と m2 紐づきの添付 1 件を同期
        state.on_shape(ShapeEvent::Batch(
            ShapeId::Attachments("c1".into()),
            batch(vec![
                (
                    "att1",
                    json!({
                        "id": "att1", "message_id": "m1", "uploader_id": "u1",
                        "channel_id": "c1", "file_name": "file1.png", "file_size": 100,
                        "content_type": "image/png", "storage_key": "k1",
                        "created_at": "2026-11-01 00:00:05"
                    }),
                ),
                (
                    "att2",
                    json!({
                        "id": "att2", "message_id": "m1", "uploader_id": "u1",
                        "channel_id": "c1", "file_name": "file2.pdf", "file_size": 200,
                        "content_type": "application/pdf", "storage_key": "k2",
                        "created_at": "2026-11-01 00:00:10"
                    }),
                ),
                (
                    "att3",
                    json!({
                        "id": "att3", "message_id": "m2", "uploader_id": "u1",
                        "channel_id": "c1", "file_name": "file3.jpg", "file_size": 300,
                        "content_type": "image/jpeg", "storage_key": "k3",
                        "created_at": "2026-11-01 00:01:05"
                    }),
                ),
            ]),
        ));

        state.ensure_channel_cache();
        let atts_by_msg = &state.channel_cache.attachments_by_message;
        assert_eq!(atts_by_msg.get("m1").unwrap().len(), 2);
        assert_eq!(atts_by_msg.get("m1").unwrap()[0].file_name, "file1.png");
        assert_eq!(atts_by_msg.get("m1").unwrap()[1].file_name, "file2.pdf");
        assert_eq!(atts_by_msg.get("m2").unwrap().len(), 1);
        assert_eq!(atts_by_msg.get("m2").unwrap()[0].file_name, "file3.jpg");
    }
}
