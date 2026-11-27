//! 画面の状態とキー操作。描画は `view.rs`、通信は `api` と `electric` に任せる。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::{sync::mpsc, task::JoinHandle};
use tui_textarea::TextArea;
use zoo2_client_core::poll::PollGate;

use super::e2ee_chat::{
    self, ChannelEncryption, CreateForm, CreateKind, Decryptions, RowKey, Shown,
};
use super::global_search::{self, GlobalSearch};
use crate::{
    api::{ApiClient, Me, NotificationLevel, ReadState, ThreadSummary, TypingUser, stale_revision},
    e2ee::{
        send::{self, ChannelSender, SendContext},
        store::{Decryption, EncryptedRow, RecipientSummary},
    },
    electric::{self, Batch, ShapeEvent, ShapeId},
    http::{ApiError, Credentials},
    store::Store,
};

const NOTICE_TTL: Duration = Duration::from_secs(6);
/// 暗号化したメッセージを、本文の代わりに出すところ（スレッドの一覧など）。
const ENCRYPTED_PLACEHOLDER: &str = "🔒 暗号化されたメッセージ";
/// `on_tick` の間隔。`ui::run` の interval と合わせる。
pub const TICK: Duration = Duration::from_millis(500);
/// 入力中の通知は2秒に1回まで送り、3秒止まったら取り消す。
const TYPING_SEND_INTERVAL: Duration = Duration::from_secs(2);
const TYPING_IDLE: Duration = Duration::from_secs(3);
/// 長いメッセージの中を1回で動かす行数。
const INNER_SCROLL_STEP: usize = 10;

/// 非同期の API 呼び出しから画面に返す結果。
#[derive(Debug)]
pub enum AppEvent {
    Notice(String),
    AccountProfiles(zoo2_client_core::store::AccountProfiles),
    EmojiCatalog {
        shortcodes: HashMap<String, String>,
        custom: HashSet<String>,
    },
    /// 送信に失敗した本文を、送り先と一緒に入力欄へ戻すか、戻せないときは保存しておく。
    RestoreDraft {
        channel_id: String,
        target: Target,
        content: String,
        /// 暗号化したメッセージの編集を始めたときの版（#361）。下書きと一緒に戻す。
        edit_base: Option<u64>,
    },
    ReadStates(Vec<ReadState>),
    Threads {
        requested_at: Instant,
        threads: Vec<ThreadSummary>,
        archived: bool,
        append: bool,
        next_cursor: Option<String>,
    },
    ThreadRequestFinished {
        archived: bool,
    },
    ThreadsFailed {
        requested_at: Instant,
        archived: bool,
        append: bool,
        message: String,
    },
    ThreadsChanged {
        reopened: Option<String>,
    },
    /// 移行中、未移行の子スレッドへの返信がサーバーで最上位へ正規化された。
    ThreadRedirected {
        from: String,
        to: String,
    },
    ThreadLeft(String),
    /// 全文検索の 1 ページ（`generation` が今のものと違えば捨てる）。
    SearchResults {
        generation: u64,
        append: bool,
        page: crate::api::SearchPage,
    },
    SearchFailed {
        generation: u64,
        message: String,
    },
    ReadStatesPollFinished,
    EmojiPollFinished,
    TypingPollFinished,
    /// GitHub Releases に新しい版がある。
    UpdateAvailable(String),
    Typing {
        channel_id: String,
        parent_id: Option<String>,
        users: Vec<TypingUser>,
    },
    /// 暗号化の画面の裏の処理の結果（#360）。
    Encryption(super::encryption::EncryptionEvent),
    /// 端末と検証の画面の裏の処理の結果（#362）。
    Devices(super::devices::DevicesEvent),
    /// 他のユーザーの検証の知らせ（#368）。
    Verification(crate::e2ee::verification::VerificationEvent),
    /// 暗号化したメッセージの復号の結果（メッセージ ID、行の版、結果）と、そのときの暗号ストアの
    /// 受信用セッションの数え（#361）。
    Decrypted {
        channel_id: String,
        results: Vec<(String, RowKey, Decryption)>,
        generation: Option<u64>,
    },
    /// 暗号化チャンネルの配布先を判定し直した（#361）。
    Recipients {
        channel_id: String,
        summary: RecipientSummary,
    },
    /// 暗号ストアに記録した、暗号化チャンネルの ID と、保存できずに記録し直すもの（#361）。
    KnownEncryptedChannels {
        known: Vec<String>,
        failed: Vec<String>,
    },
    /// 暗号化したメッセージの編集が 409 `E2EE_STALE_REVISION` になった（#361）。今の版の本文が
    /// 送ろうとした本文と同じなら、応答をなくしただけなので成功として扱う。
    EditConflict {
        channel_id: String,
        message_id: String,
        current_revision: u64,
        content: String,
        edit_base: Option<u64>,
    },
    /// ピン留めの並びの版（`GET /api/channel-pins` または書き込みの応答）。
    PinsVersion(i64),
    /// チャンネルか DM を作った（#361）。開く。`encrypted` は応答の値。
    Created {
        channel_id: String,
        requested_encryption: bool,
        encrypted: bool,
    },
    /// 通知レベルの変更が終わった。失敗したときは、最後に確定した値まで戻す。
    NotificationLevelFinished {
        channel_id: String,
        /// 送信した値。
        level: NotificationLevel,
        succeeded: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Messages,
    Thread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarItem {
    Channel(String),
    /// サイドバー先頭のピン留め。通常の一覧にも同じチャンネルが出る。
    Pinned(String),
    Thread {
        channel_id: String,
        id: String,
        /// ピン留めの行の下に出すスレッドか。通常の一覧にも同じスレッドが出るので、選択位置を区別する。
        pinned: bool,
    },
    MoreThreads {
        archived: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// 入力欄に書いている。
    Insert,
    /// リアクションの絵文字 (ショートコードか絵文字そのもの) を入力している。
    React {
        message_id: String,
        input: String,
    },
    /// 削除の確認。
    ConfirmDelete {
        message_id: String,
    },
    /// チャンネル名で絞り込む。
    Search(String),
    /// 全文検索（状態は `App::global_search`）。
    GlobalSearch,
    Help,
    /// 暗号化の画面（初回設定と解錠。#360）。
    Encryption,
    /// 端末と検証の画面（端末一覧と失効、他のユーザーの検証、鍵スロット。#362）。
    Devices,
    /// チャンネルか DM を作る（#361）。
    Create(CreateForm),
    /// 暗号化チャンネルの配布先の詳細（#361）。`scroll` 行目から出す。
    Recipients {
        scroll: usize,
    },
}

/// 入力欄の送り先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Channel,
    Reply(String),
    Edit(String),
}

/// 送信に失敗し、入力欄に戻せなかった本文。`R` で入力欄に戻して送り直す。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedDraft {
    pub channel_id: String,
    pub target: Target,
    pub content: String,
    /// 暗号化したメッセージの編集を始めたときの版（#361）。戻したときの競合の確かめに使う。
    pub edit_base: Option<u64>,
}

pub struct ThreadView {
    pub root_id: String,
    pub selected: Option<String>,
    pub scroll: usize,
    /// 選択中のメッセージがペインより長いときの、メッセージ内の表示位置。
    pub inner_scroll: usize,
}

pub struct App {
    pub api: ApiClient,
    http: reqwest::Client,
    api_base_url: String,
    credentials: Credentials,
    pub me: Me,
    pub store: Store,
    /// 一度 `encrypted = true` を見たチャンネル。Channels の再同期で行が消えても暗号化として扱う。
    known_encrypted_channels: HashSet<String>,
    pub(super) subscriptions: HashMap<ShapeId, JoinHandle<()>>,
    shape_tx: mpsc::UnboundedSender<ShapeEvent>,
    app_tx: mpsc::UnboundedSender<AppEvent>,

    pub focus: Focus,
    pub mode: Mode,
    pub sidebar_index: usize,
    pub active_threads: Vec<ThreadSummary>,
    pub archived_threads: Vec<ThreadSummary>,
    /// 遅れた取得結果で、再開後の一覧を上書きしないための取得開始時刻。
    threads_requested_at: [Option<Instant>; 2],
    thread_in_flight: [bool; 2],
    thread_pending: [Option<bool>; 2],
    thread_states: HashMap<String, (bool, Instant)>,
    /// 再同期中は部分スナップショットの件数で一覧を減らさない。
    message_resync_counts: HashMap<String, HashMap<String, usize>>,
    locally_reopened_threads: HashMap<String, jiff::civil::DateTime>,
    pub thread_cursors: [Option<String>; 2],
    thread_has_additional_pages: [bool; 2],
    pub thread_failed_append: [Option<bool>; 2],
    pub current: Option<String>,
    /// 選択中のルートメッセージ。None なら最新を追う。
    pub selected: Option<String>,
    pub scroll: usize,
    /// 選択中のメッセージがペインより長いときの、メッセージ内の表示位置。
    pub inner_scroll: usize,
    pub thread: Option<ThreadView>,
    /// 全文検索の語と結果。閉じても残し、開き直したときに続きから使う。
    pub global_search: GlobalSearch,
    /// 直近の描画でタイムラインに出ていた最も新しいメッセージ (チャンネル, メッセージ)。
    /// 既読はここまでしか進めない。
    pub newest_shown: Option<(String, String)>,
    pub composer: TextArea<'static>,
    pub target: Target,
    /// 送信に失敗した本文。古い順。
    pub failed_drafts: Vec<FailedDraft>,
    pub notice: Option<(String, Instant)>,
    /// 新しい版があれば、その版。ステータス行に出し続ける。
    pub update_available: Option<String>,
    pub should_quit: bool,

    /// 未読のあるチャンネル。
    pub unread: HashSet<String>,
    /// ピン留めの並びの版。書き込みのたびに更新し、409 のときは取り直す。
    pub pin_version: i64,
    /// チャンネルごとに送信中の通知レベル。別チャンネルの操作では消さない。
    notification_level_pending: HashMap<String, NotificationLevel>,
    /// チャンネルごとの未完了の PATCH の数。0 になってはじめて Shape を正本に戻せる。
    notification_level_in_flight: HashMap<String, usize>,
    /// チャンネルごとに最後に選んだ通知レベル。Shape が古くてもこの値から次を計算する。
    notification_level_selected: HashMap<String, NotificationLevel>,
    /// チャンネルごとに最後に確定した通知レベル（成功した値、初期値は Store の値）。
    /// 連続で失敗しても未保存の値を戻り先にしないため、ここを正本にする。
    notification_level_confirmed: HashMap<String, NotificationLevel>,
    /// 同じチャンネルへの PATCH を入力順に直列化するロック。別チャンネルは互いにブロックしない。
    notification_level_locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// チャンネルごとに既読を送った最新メッセージ。同じ位置を何度も送らない。
    marked_read: HashMap<String, String>,
    /// 開いているチャンネルで入力中のユーザー。キーはスレッドの親 (None はタイムライン)。
    pub typing: HashMap<Option<String>, Vec<TypingUser>>,
    typing_session: String,
    /// 入力中を通知している送り先と、最後に送った時刻。
    typing_sent: Option<(String, Option<String>, Instant)>,
    last_keystroke: Option<Instant>,
    ticks: u64,
    terminal_active: bool,
    read_poll: PollGate,
    emoji_poll: PollGate,
    typing_poll: PollGate,
    /// 暗号化の画面と、その裏の処理。
    pub encryption: super::encryption::Encryption,
    /// 端末と検証の画面と、その裏の処理（#362、#368）。
    pub devices: super::devices::Devices,
    /// 検証の期限切れと送り直しを最後に確かめた時刻。
    verification_tick: Option<Instant>,
    /// 暗号化したメッセージの復号の結果（#361）。
    pub decryptions: Decryptions,
    /// 暗号化チャンネルごとの、最後に判定した配布先（送信画面に出す。#361）。
    pub recipients: HashMap<String, RecipientSummary>,
    /// 暗号化チャンネルへの送信の直列化と、知っている epoch（#361）。
    channel_sender: ChannelSender,
    /// 入力欄の暗号化したメッセージの編集を始めたときの版（メッセージ ID と版。#361）。下書きを
    /// 戻すときは、下書きと一緒に保存した版に戻す。
    edit_base: Option<(String, u64)>,
    /// `state.json` の暗号化チャンネルの記録を読めなかった（暗号ストアから読むまで平文で送らない）。
    known_encrypted_unreadable: bool,
    /// 暗号ストアの置き場所（暗号化チャンネルの記録を、その場で書く）。
    crypto_root: Option<std::path::PathBuf>,
    /// 暗号ストアに記録していない、`encrypted = true` を見たチャンネル（#361）。
    unrecorded_encrypted: HashSet<String>,
    /// 暗号ストアの記録を読んだか。
    known_encrypted_loaded: bool,
    /// 記録を読んだ暗号ストア（開き直したら読み直す）。
    known_encrypted_store: Option<usize>,
    known_encrypted_in_flight: bool,
}

impl App {
    pub fn new(
        api: ApiClient,
        http: reqwest::Client,
        api_base_url: String,
        credentials: Credentials,
        me: Me,
        shape_tx: mpsc::UnboundedSender<ShapeEvent>,
        app_tx: mpsc::UnboundedSender<AppEvent>,
    ) -> Self {
        crate::mfm::set_emoji_catalog(HashMap::new(), HashSet::new());
        let encryption =
            super::encryption::Encryption::new(api.clone(), http.clone(), app_tx.clone());
        let devices = super::devices::Devices::new(api.clone(), http.clone(), app_tx.clone());
        let mut app = Self {
            encryption,
            devices,
            verification_tick: None,
            api,
            http,
            api_base_url,
            credentials,
            me,
            store: Store::default(),
            known_encrypted_channels: HashSet::new(),
            subscriptions: HashMap::new(),
            shape_tx,
            app_tx,
            focus: Focus::Sidebar,
            mode: Mode::Normal,
            sidebar_index: 0,
            active_threads: Vec::new(),
            archived_threads: Vec::new(),
            threads_requested_at: [None, None],
            thread_in_flight: [false; 2],
            thread_pending: [None; 2],
            thread_states: HashMap::new(),
            message_resync_counts: HashMap::new(),
            locally_reopened_threads: HashMap::new(),
            thread_cursors: [None, None],
            thread_has_additional_pages: [false; 2],
            thread_failed_append: [None, None],
            global_search: GlobalSearch::default(),
            current: None,
            selected: None,
            scroll: 0,
            inner_scroll: 0,
            thread: None,
            newest_shown: None,
            composer: new_composer(),
            target: Target::Channel,
            failed_drafts: Vec::new(),
            notice: None,
            update_available: None,
            should_quit: false,
            unread: HashSet::new(),
            pin_version: 0,
            notification_level_pending: HashMap::new(),
            notification_level_in_flight: HashMap::new(),
            notification_level_selected: HashMap::new(),
            notification_level_confirmed: HashMap::new(),
            notification_level_locks: HashMap::new(),
            marked_read: HashMap::new(),
            typing: HashMap::new(),
            typing_session: uuid::Uuid::new_v4().to_string(),
            typing_sent: None,
            last_keystroke: None,
            ticks: 0,
            terminal_active: true,
            read_poll: PollGate::new(Duration::from_secs(10), Duration::from_secs(10)),
            emoji_poll: PollGate::new(Duration::from_secs(60), Duration::from_secs(60)),
            typing_poll: PollGate::new(Duration::from_millis(1500), Duration::from_secs(5)),
            decryptions: Decryptions::default(),
            recipients: HashMap::new(),
            channel_sender: ChannelSender::default(),
            edit_base: None,
            known_encrypted_unreadable: false,
            crypto_root: None,
            unrecorded_encrypted: HashSet::new(),
            known_encrypted_loaded: false,
            known_encrypted_store: None,
            known_encrypted_in_flight: false,
        };
        for id in [
            ShapeId::Channels,
            ShapeId::Users,
            ShapeId::ChannelUsers,
            ShapeId::ChannelNotificationSettings,
            ShapeId::UserPinnedChannels,
        ] {
            app.subscribe(id);
        }
        app.refresh_pin_version();
        app
    }

    fn subscribe(&mut self, id: ShapeId) {
        if self
            .subscriptions
            .get(&id)
            .is_some_and(|task| !task.is_finished())
        {
            return;
        }
        if let ShapeId::Messages(channel_id) = &id {
            self.begin_message_resync(channel_id);
        }
        // 止まった購読を作り直すときは最初から取り直すので、止まっていた間の行は捨てる。
        // 物理削除された行は新しいスナップショットに現れないため、捨てないと残り続ける。
        if self.subscriptions.contains_key(&id) {
            self.store.apply(
                &id,
                Batch {
                    reset: true,
                    ..Default::default()
                },
            );
        }
        let task = electric::spawn(
            self.http.clone(),
            &self.api_base_url,
            self.credentials.clone(),
            id.clone(),
            self.shape_tx.clone(),
        );
        self.subscriptions.insert(id, task);
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.notice = Some((message.into(), Instant::now()));
    }

    /// API を裏で呼び、失敗したら通知に出す。成功の結果はシェイプで届く。
    fn spawn_api<T, F>(&self, label: &'static str, call: F)
    where
        F: Future<Output = anyhow::Result<T>> + Send + 'static,
    {
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            if let Err(err) = call.await {
                let _ = tx.send(AppEvent::Notice(format!("{label}に失敗しました: {err}")));
            }
        });
    }

    fn begin_message_resync(&mut self, channel_id: &str) {
        let mut counts = self.store.reply_counts(channel_id);
        // チャンネル切替でStoreが空でも、一覧で確認済みの件数を保持する。
        for thread in self
            .threads()
            .filter(|thread| thread.channel_id == channel_id)
        {
            if thread.reply_count > 0 {
                counts
                    .entry(thread.id.clone())
                    .or_insert(thread.reply_count as usize);
            }
        }
        self.message_resync_counts
            .entry(channel_id.to_owned())
            .or_default()
            .extend(counts);
    }

    pub fn on_shape(&mut self, event: ShapeEvent) {
        let selected = self.sidebar_items().get(self.sidebar_index).cloned();
        let mut completed_resync = false;
        let previous_reply_counts = match &event {
            ShapeEvent::Batch(ShapeId::Messages(channel_id), batch) => {
                if batch.reset
                    || (batch.up_to_date && self.message_resync_counts.contains_key(channel_id))
                {
                    // 同期途中に一覧応答・ローカル補完で追加された行も確定時に判定する。
                    self.begin_message_resync(channel_id);
                }
                if batch.up_to_date && self.message_resync_counts.contains_key(channel_id) {
                    completed_resync = true;
                    self.message_resync_counts.remove(channel_id)
                } else if self.message_resync_counts.contains_key(channel_id) {
                    None
                } else {
                    Some(self.store.reply_counts(channel_id))
                }
            }
            _ => None,
        };
        let changed_messages = match &event {
            ShapeEvent::Batch(ShapeId::Messages(channel_id), batch) => Some((
                channel_id.clone(),
                batch
                    .changes
                    .iter()
                    .filter_map(|change| match change {
                        electric::Change::Upsert { row, .. } => {
                            row.get("id")?.as_str().map(str::to_owned)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
            )),
            _ => None,
        };
        match event {
            ShapeEvent::Batch(id, batch) => {
                let is_channels = matches!(id, ShapeId::Channels);
                let is_notification_settings = matches!(id, ShapeId::ChannelNotificationSettings);
                self.store.apply(&id, batch);
                if is_notification_settings {
                    // 未完了の PATCH がある間は Store が一致しても選択値を消さない。
                    // 連打で選んだ値が Store の古い値に戻ると、一致を理由に解除され、
                    // あとから反映された値を基準に次の値を送ってしまうため。
                    let store = &self.store;
                    let me_id = &self.me.id;
                    let in_flight = &self.notification_level_in_flight;
                    self.notification_level_selected
                        .retain(|channel_id, level| {
                            in_flight.contains_key(channel_id)
                                || store.notification_level(channel_id, me_id) != *level
                        });
                }
                if is_channels {
                    for channel in self.store.channels() {
                        if channel.encrypted
                            && self.known_encrypted_channels.insert(channel.id.clone())
                        {
                            self.record_encrypted(&channel.id);
                            self.unrecorded_encrypted.insert(channel.id.clone());
                        }
                    }
                }
                if matches!(id, ShapeId::Messages(_)) {
                    self.decryptions.dirty = true;
                }
            }
            ShapeEvent::Failed(id, err) => {
                self.notify(format!("同期を停止しました ({id:?}): {err}"));
            }
        }
        let store = &self.store;
        self.locally_reopened_threads
            .retain(|root_id, reopened_at| {
                !store.channels().iter().any(|channel| {
                    store.message(&channel.id, root_id).is_some_and(|root| {
                        root.thread_archive_at.is_some_and(|at| at > *reopened_at)
                    })
                })
            });
        let changed_channel = changed_messages
            .as_ref()
            .map(|(channel_id, _)| channel_id.clone());
        if let Some((channel_id, ids)) = changed_messages {
            for id in ids {
                if let Some(root) = self.store.message(&channel_id, &id) {
                    let relevant = root.thread_archive_at.is_some()
                        || self.thread_states.contains_key(&id)
                        || self.threads().any(|thread| thread.id == id)
                        || self
                            .thread
                            .as_ref()
                            .is_some_and(|thread| thread.root_id == id);
                    if relevant {
                        let archived = root.thread_archive_at.is_some_and(|at| {
                            self.locally_reopened_threads
                                .get(&id)
                                .is_none_or(|reopened| at > *reopened)
                        });
                        let reopened = !archived
                            && self
                                .thread_states
                                .get(&id)
                                .is_some_and(|(was_archived, _)| *was_archived);
                        self.thread_states
                            .insert(id.clone(), (archived, Instant::now()));
                        if reopened {
                            self.ensure_active_thread(&id);
                        }
                    }
                }
            }
        }
        // 返信の同期で件数が変わった親は、一覧GETの成功を待たず補正する。
        if let (Some(before), Some(channel_id)) = (previous_reply_counts, changed_channel) {
            let after = self.store.reply_counts(&channel_id);
            for thread in self
                .active_threads
                .iter_mut()
                .chain(self.archived_threads.iter_mut())
            {
                if thread.channel_id == channel_id
                    && (completed_resync || before.get(&thread.id) != after.get(&thread.id))
                {
                    thread.reply_count = after.get(&thread.id).copied().unwrap_or(0) as u64;
                    self.thread_states
                        .entry(thread.id.clone())
                        .and_modify(|(_, updated)| *updated = Instant::now())
                        .or_insert((thread.thread_archive_at.is_some(), Instant::now()));
                }
            }
            // 最後の返信が消えた行は除外する。返信の到着待ちで0件の補完行は残す。
            self.active_threads.retain(|thread| {
                thread.channel_id != channel_id
                    || !before.contains_key(&thread.id)
                    || after.contains_key(&thread.id)
            });
        }
        self.restore_sidebar_selection(selected);
    }

    pub fn on_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::AccountProfiles(profiles) => {
                self.store.account_profiles = profiles;
            }
            AppEvent::Encryption(event) => self.encryption.on_event(event),
            AppEvent::Devices(event) => self.devices.on_event(event),
            AppEvent::Verification(event) => self.on_verification(event),
            AppEvent::Decrypted {
                channel_id,
                results,
                generation,
            } => {
                self.decryptions.in_flight = false;
                if let Some(generation) = generation {
                    self.decryptions.generations.insert(channel_id, generation);
                }
                for (message_id, row, decryption) in results {
                    self.decryptions.apply(&message_id, row, decryption);
                }
            }
            AppEvent::Recipients {
                channel_id,
                summary,
            } => {
                self.recipients.insert(channel_id, summary);
            }
            AppEvent::KnownEncryptedChannels { known, failed } => {
                self.known_encrypted_in_flight = false;
                self.known_encrypted_loaded = true;
                self.known_encrypted_channels.extend(known);
                self.unrecorded_encrypted.extend(failed);
            }
            AppEvent::EditConflict {
                channel_id,
                message_id,
                current_revision,
                content,
                edit_base,
            } => {
                let applied = self
                    .store
                    .message(&channel_id, &message_id)
                    .and_then(|message| self.decryptions.editable(message))
                    .is_some_and(|(shown, revision)| {
                        revision == current_revision && shown.body == content
                    });
                if !applied {
                    self.notify(
                        "このメッセージはほかの場所で編集されました。読み込み直してから編集してください",
                    );
                    self.on_app_event(AppEvent::RestoreDraft {
                        channel_id,
                        target: Target::Edit(message_id),
                        content,
                        edit_base,
                    });
                }
            }
            AppEvent::Created {
                channel_id,
                requested_encryption,
                encrypted,
            } => {
                if encrypted {
                    if self.known_encrypted_channels.insert(channel_id.clone()) {
                        self.record_encrypted(&channel_id);
                        self.unrecorded_encrypted.insert(channel_id.clone());
                    }
                } else if requested_encryption {
                    // 暗号化を求めたのに平文のチャンネルが返った。このプロセスのあいだは送らない。
                    self.known_encrypted_channels.insert(channel_id.clone());
                    self.notify(e2ee_chat::DOWNGRADED);
                }
                self.focus = Focus::Messages;
                self.open_channel(channel_id);
            }
            AppEvent::Threads {
                requested_at,
                threads,
                archived,
                append,
                next_cursor,
            } => {
                let scope = usize::from(archived);
                if self.threads_requested_at[scope].is_some_and(|last| requested_at < last) {
                    return;
                }
                // 別種別の新しい応答で確定した項目を、古い応答で移動しない。
                let threads: Vec<_> = threads
                    .into_iter()
                    .filter(|thread| {
                        !self
                            .thread_states
                            .get(&thread.id)
                            .is_some_and(|(_, last)| requested_at < *last)
                    })
                    .collect();
                let mut selected = self.sidebar_items().get(self.sidebar_index).cloned();
                if append
                    && matches!(selected, Some(SidebarItem::MoreThreads { archived: scope }) if scope == archived)
                    && let Some(first) = threads.first()
                {
                    selected = Some(SidebarItem::Thread {
                        channel_id: first.channel_id.clone(),
                        id: first.id.clone(),
                        pinned: false,
                    });
                }
                self.thread_has_additional_pages[scope] = append || threads.len() > 100;
                let incoming_ids: HashSet<_> =
                    threads.iter().map(|thread| thread.id.clone()).collect();
                for thread in &threads {
                    if thread.thread_archive_at.is_some() {
                        self.locally_reopened_threads.remove(&thread.id);
                    }
                    self.thread_states.insert(
                        thread.id.clone(),
                        (thread.thread_archive_at.is_some(), requested_at),
                    );
                }
                let (current, other) = if archived {
                    (&mut self.archived_threads, &mut self.active_threads)
                } else {
                    (&mut self.active_threads, &mut self.archived_threads)
                };
                other.retain(|entry| !incoming_ids.contains(&entry.id));
                if append {
                    current.retain(|entry| !incoming_ids.contains(&entry.id));
                } else {
                    current.retain(|entry| {
                        self.thread_states
                            .get(&entry.id)
                            .is_some_and(|(_, updated)| *updated > requested_at)
                    });
                }
                if append {
                    current.extend(threads);
                } else {
                    let mut refreshed = threads;
                    refreshed.append(current);
                    *current = refreshed;
                }
                // Shapeで確認した状態は、購読停止や一覧からの除外でも保持する。
                self.thread_cursors[scope] = next_cursor;
                self.thread_failed_append[scope] = None;
                self.threads_requested_at[scope] = Some(requested_at);
                self.restore_sidebar_selection(selected);
            }
            AppEvent::ThreadRequestFinished { archived } => {
                let scope = usize::from(archived);
                self.thread_in_flight[scope] = false;
                if let Some(append) = self.thread_pending[scope].take() {
                    self.fetch_thread_page(archived, append);
                }
            }
            AppEvent::ThreadsFailed {
                requested_at,
                archived,
                append,
                message,
            } => {
                let scope = usize::from(archived);
                if self.threads_requested_at[scope].is_some_and(|last| requested_at < last) {
                    return;
                }
                self.threads_requested_at[scope] = Some(requested_at);
                self.thread_failed_append[scope] = Some(append);
                self.notify(format!(
                    "スレッド一覧を取得できませんでした: {message}。一覧の再試行を選んでください"
                ));
            }
            AppEvent::SearchResults {
                generation,
                append,
                page,
            } => self.global_search.apply(generation, append, page),
            AppEvent::SearchFailed {
                generation,
                message,
            } => self.global_search.fail(generation, message),
            AppEvent::ThreadLeft(id) => {
                // 退出の応答を待つあいだにカーソルを動かしていても、同じ行に残す（行が消えて添字がずれるため）。
                let selected = self.selected_sidebar_item();
                self.active_threads.retain(|thread| thread.id != id);
                self.archived_threads.retain(|thread| thread.id != id);
                self.restore_sidebar_selection(selected);
                self.threads_requested_at = [Some(Instant::now()); 2];
                self.refresh_threads();
            }
            AppEvent::ThreadRedirected { from, to } => {
                // 一覧にない `from` の一時の行が消える（`to` が一覧にあれば、その行が代わりになる）と、
                // 添字がずれるので、カーソルは同じ行に残す。
                let selected = self.selected_sidebar_item();
                if let Some(thread) = self.thread.as_mut().filter(|t| t.root_id == from) {
                    thread.root_id = to.clone();
                    thread.selected = None;
                    thread.scroll = 0;
                    thread.inner_scroll = 0;
                }
                if self.target == Target::Reply(from) {
                    self.target = Target::Reply(to);
                }
                self.restore_sidebar_selection(selected);
            }
            AppEvent::ThreadsChanged { reopened } => {
                // 再Openしたスレッドの行が増えて添字がずれるので、カーソルは同じ行に残す。
                let selected = self.selected_sidebar_item();
                if let Some(id) = reopened {
                    self.locally_reopened_threads.insert(
                        id.clone(),
                        jiff::Timestamp::now()
                            .to_zoned(jiff::tz::TimeZone::UTC)
                            .datetime(),
                    );
                    self.thread_states
                        .insert(id.clone(), (false, Instant::now()));
                    self.ensure_active_thread(&id);
                }
                self.restore_sidebar_selection(selected);
                // 書き込み前の一覧で、確定済みの状態を上書きしない。
                self.threads_requested_at = [Some(Instant::now()); 2];
                self.refresh_threads();
            }
            AppEvent::EmojiCatalog { shortcodes, custom } => {
                crate::mfm::set_emoji_catalog(shortcodes, custom)
            }
            AppEvent::NotificationLevelFinished {
                channel_id,
                level,
                succeeded,
            } => {
                if self.notification_level_pending.get(&channel_id) == Some(&level) {
                    self.notification_level_pending.remove(&channel_id);
                }
                if let Some(count) = self.notification_level_in_flight.get_mut(&channel_id) {
                    if *count <= 1 {
                        self.notification_level_in_flight.remove(&channel_id);
                    } else {
                        *count -= 1;
                    }
                }
                if succeeded {
                    // 成功した値だけがサーバーに保存された。Shape が遅れていても失敗時の戻り先はこれ。
                    self.notification_level_confirmed
                        .insert(channel_id.clone(), level);
                } else if self.notification_level_selected.get(&channel_id) == Some(&level) {
                    // 失敗した値のあとに別の操作がなければ、最後に確定した値へ戻す。
                    // 途中の未保存の値を戻すと、連続失敗のあと実際の設定からずれてしまう。
                    let confirmed = self
                        .notification_level_confirmed
                        .get(&channel_id)
                        .copied()
                        .unwrap_or_else(|| self.store.notification_level(&channel_id, &self.me.id));
                    self.notification_level_selected
                        .insert(channel_id.clone(), confirmed);
                }
                // すべての PATCH が終わったら、Store と一致する選択値を解除する。
                // Shape が先に届いて解除できなかった場合でもここで正本に戻せる。
                if !self.notification_level_in_flight.contains_key(&channel_id)
                    && self.notification_level_selected.get(&channel_id)
                        == Some(&self.store.notification_level(&channel_id, &self.me.id))
                {
                    self.notification_level_selected.remove(&channel_id);
                }
            }
            AppEvent::Notice(message) => self.notify(message),
            AppEvent::RestoreDraft {
                channel_id,
                target,
                content,
                edit_base,
            } => {
                // 入力欄に書きかけの本文があるか別のチャンネルにいるときは、上書きせずに取っておく。
                if self.current.as_deref() == Some(channel_id.as_str()) && self.composer.is_empty()
                {
                    self.composer = composer_with(&content);
                    self.set_edit_base(&target, edit_base);
                    self.target = target;
                } else {
                    self.failed_drafts.push(FailedDraft {
                        channel_id,
                        target,
                        content,
                        edit_base,
                    });
                    self.notify(format!(
                        "送れなかった本文を保存しました ({}件)。R で入力欄に戻せます",
                        self.failed_drafts.len()
                    ));
                }
            }
            AppEvent::ReadStatesPollFinished => self.read_poll.finish(Instant::now(), true),
            AppEvent::EmojiPollFinished => self.emoji_poll.finish(Instant::now(), true),
            AppEvent::PinsVersion(version) => self.pin_version = version,
            AppEvent::TypingPollFinished => self.typing_poll.finish(
                Instant::now(),
                self.typing.values().any(|users| !users.is_empty()) || self.typing_sent.is_some(),
            ),
            AppEvent::ReadStates(states) => {
                self.unread = states
                    .into_iter()
                    .filter(|state| state.has_unread)
                    .map(|state| state.channel_id)
                    .collect();
                // 開いているチャンネルは既読を送り直す。
                if let Some(channel_id) = &self.current
                    && self.unread.contains(channel_id)
                {
                    self.marked_read.remove(channel_id);
                }
                self.mark_current_read();
            }
            AppEvent::UpdateAvailable(version) => self.update_available = Some(version),
            AppEvent::Typing {
                channel_id,
                parent_id,
                users,
            } => {
                if self.terminal_active && self.current.as_deref() == Some(channel_id.as_str()) {
                    self.typing.insert(parent_id, users);
                }
            }
        }
    }

    /// 端末の管理のタスクと OIDC の設定を、暗号化の画面につなぐ（#360）。
    pub fn attach_encryption(&mut self, context: super::encryption::E2eeContext) {
        // 暗号ストアを開く前から、記録した暗号化チャンネルを暗号化として扱う（開くまでの平文の送信を止める）。
        // 記録を読めなかったときは、暗号ストアから読めるまで平文で送らない（記録がないものとして送ると、
        // 暗号化チャンネルの格下げを見逃すため）。
        self.crypto_root = context.crypto_root.clone();
        if let Some(root) = &context.crypto_root {
            match crate::e2ee::store::read_encrypted_channels(root, &self.me.id) {
                Ok(ids) => self.known_encrypted_channels.extend(ids),
                Err(_) => self.known_encrypted_unreadable = true,
            }
        }
        let mut context = context;
        if let Some(events) = context.events.take() {
            self.devices.attach(
                context.oidc.clone(),
                context.stores.clone(),
                Some(events.device_limit),
            );
            // 端末の管理のタスクが受け取った検証のメッセージを、検証の処理に渡す。
            if let Some(sas) = self.devices.sas.clone() {
                let mut verifications = events.verifications;
                tokio::spawn(async move {
                    while let Some(messages) = verifications.recv().await {
                        sas.handle_messages(messages);
                    }
                });
            }
        } else {
            self.devices
                .attach(context.oidc.clone(), context.stores.clone(), None);
        }
        self.encryption.attach(context);
    }

    /// 他のユーザーの検証の知らせ。届いた要求は前面に出す。入力の途中などでは、キー操作を奪わないよう、
    /// ステータス行で知らせて `D` で開いてもらう。
    fn on_verification(&mut self, event: crate::e2ee::verification::VerificationEvent) {
        let fresh = self.devices.on_verification(event);
        if fresh && !matches!(self.mode, Mode::Normal | Mode::Devices) {
            self.devices.sas_open = None;
            self.notify("検証の要求が届いています（D で開く）");
        }
    }

    /// 暗号化チャンネル（削除されていないもの）を共有する相手（Web の `useEncryptedPeers`）。
    pub fn encrypted_peers(&self) -> Vec<String> {
        let me = self.me.id.as_str();
        let mut peers: Vec<String> = self
            .store
            .channels()
            .into_iter()
            .filter(|channel| channel.encrypted && self.store.is_member(&channel.id, me))
            .flat_map(|channel| self.store.channel_members(&channel.id))
            .map(|user| user.id.clone())
            .filter(|id| id != me)
            .collect();
        peers.sort();
        peers.dedup();
        peers
    }

    pub fn on_tick(&mut self) {
        self.encryption.on_tick(self.mode == Mode::Encryption);
        if self
            .verification_tick
            .is_none_or(|at| at.elapsed() >= crate::e2ee::verification::TICK_INTERVAL)
        {
            self.verification_tick = Some(Instant::now());
            self.devices.on_tick();
        }
        self.sync_known_encrypted();
        self.request_decryptions();
        if self.terminal_active {
            for archived in [false, true] {
                let scope = usize::from(archived);
                if !self.thread_in_flight[scope]
                    && let Some(append) = self.thread_pending[scope].take()
                {
                    self.fetch_thread_page(archived, append);
                }
            }
        }
        if self.terminal_active && !self.thread_in_flight[0] && self.ticks.is_multiple_of(120) {
            self.refresh_threads();
        }
        if self.terminal_active && self.emoji_poll.start(Instant::now()) {
            let api = self.api.clone();
            let tx = self.app_tx.clone();
            tokio::spawn(async move {
                let profiles = api.account_profiles().await.unwrap_or_default();
                let _ = tx.send(AppEvent::AccountProfiles(profiles));
                if let Ok((shortcodes, custom)) = api.emoji_catalog().await {
                    let _ = tx.send(AppEvent::EmojiCatalog { shortcodes, custom });
                }
                let _ = tx.send(AppEvent::EmojiPollFinished);
            });
        }
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > NOTICE_TTL)
        {
            self.notice = None;
        }
        self.refresh_read_states();
        self.poll_typing();
        if self
            .last_keystroke
            .is_some_and(|at| at.elapsed() > TYPING_IDLE)
        {
            self.stop_typing();
        }
        self.mark_current_read();
        self.ticks += 1;
    }

    fn refresh_read_states(&mut self) {
        if !self.terminal_active || !self.read_poll.start(Instant::now()) {
            return;
        }
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            // 一時的な失敗は次の取得で回復するので、通知しない。
            if let Ok(states) = api.read_states().await {
                let _ = tx.send(AppEvent::ReadStates(states));
            }
            let _ = tx.send(AppEvent::ReadStatesPollFinished);
        });
    }

    fn refresh_threads(&mut self) {
        self.fetch_thread_page(false, false);
    }

    fn fetch_thread_page(&mut self, archived: bool, append: bool) {
        let scope = usize::from(archived);
        if self.thread_in_flight[scope] || !self.terminal_active {
            // A refresh supersedes an append; repeat notifications collapse to one request.
            self.thread_pending[scope] =
                Some(self.thread_pending[scope].unwrap_or(append) && append);
            return;
        }
        self.thread_in_flight[scope] = true;
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        let cursor = if append {
            self.thread_cursors[usize::from(archived)].clone()
        } else {
            None
        };
        let rebuild = !append && self.thread_has_additional_pages[usize::from(archived)];
        let through = if rebuild {
            self.thread_cursors[usize::from(archived)].clone()
        } else {
            None
        };
        let requested_at = Instant::now();
        tokio::spawn(async move {
            let result = async {
                let mut page = api
                    .thread_page(archived, cursor.as_deref(), through.as_deref())
                    .await?;
                let mut items = page.items;
                let mut seen_cursors = HashSet::new();
                while let Some(next) = if rebuild {
                    page.next_cursor.take()
                } else {
                    None
                } {
                    anyhow::ensure!(seen_cursors.insert(next.clone()), "Repeated thread cursor");
                    page = api
                        .thread_page(archived, Some(&next), through.as_deref())
                        .await?;
                    items.extend(page.items);
                }
                Ok::<_, anyhow::Error>((items, if rebuild { through } else { page.next_cursor }))
            }
            .await;
            match result {
                Ok((threads, next_cursor)) => {
                    let _ = tx.send(AppEvent::Threads {
                        requested_at,
                        threads,
                        archived,
                        append,
                        next_cursor,
                    });
                }
                Err(error) => {
                    let _ = tx.send(AppEvent::ThreadsFailed {
                        requested_at,
                        archived,
                        append,
                        message: error.to_string(),
                    });
                }
            }
            let _ = tx.send(AppEvent::ThreadRequestFinished { archived });
        });
    }

    /// 再開成功・購読で確認した再開を、一覧の再取得を待たず反映する。
    fn ensure_active_thread(&mut self, id: &str) {
        // Reopening alone is not joining. Only the participant-filtered API
        // may add a persistent entry; the viewed Open thread has its own row.
        if let Some(thread) = self
            .active_threads
            .iter_mut()
            .find(|thread| thread.id == id)
        {
            thread.thread_archive_at = None;
        }
    }

    pub fn threads(&self) -> impl Iterator<Item = &ThreadSummary> {
        self.active_threads
            .iter()
            .chain(self.archived_threads.iter())
    }

    pub fn thread_is_archived(&self, channel_id: &str, root_id: &str) -> bool {
        if let Some(reopened_at) = self.locally_reopened_threads.get(root_id) {
            return self
                .store
                .message(channel_id, root_id)
                .and_then(|root| root.thread_archive_at)
                .is_some_and(|archived_at| archived_at > *reopened_at);
        }
        if let Some((archived, _)) = self.thread_states.get(root_id) {
            return *archived;
        }
        self.threads()
            .any(|thread| thread.id == root_id && thread.thread_archive_at.is_some())
            || self
                .store
                .message(channel_id, root_id)
                .is_some_and(|root| root.thread_archive_at.is_some())
    }

    /// 一覧外の一時表示にも、描画と検索で同じ本文を使う。
    pub fn thread_content(&self, channel_id: &str, id: &str) -> Option<&str> {
        // 暗号化したスレッドの親は、復号できた本文を出す（一覧の本文はサーバーにないので空）。
        if let Some(root) = self.store.message(channel_id, id)
            && root.ciphertext.is_some()
        {
            return Some(match self.decryptions.shown(root, true) {
                Shown::Decrypted { message, .. } => message.body.as_str(),
                _ => ENCRYPTED_PLACEHOLDER,
            });
        }
        if self.known_encrypted_channels.contains(channel_id)
            && self
                .threads()
                .any(|thread| thread.id == id && thread.content.is_empty())
        {
            return Some(ENCRYPTED_PLACEHOLDER);
        }
        self.threads()
            .find(|thread| thread.id == id && thread.channel_id == channel_id)
            .map(|thread| thread.content.as_str())
            .or_else(|| {
                self.store
                    .message(channel_id, id)
                    .map(|root| root.content.as_str())
            })
    }

    fn can_change_message(&mut self, message_id: &str) -> bool {
        let Some(channel_id) = self.current.as_deref() else {
            return false;
        };
        let root_id = self
            .store
            .message(channel_id, message_id)
            .and_then(|message| message.parent_id.as_deref())
            .unwrap_or(message_id);
        let archived = self.thread_is_archived(channel_id, root_id)
            || (self.focus == Focus::Thread
                && self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| self.thread_is_archived(channel_id, &thread.root_id)));
        if archived {
            self.notify("アーカイブ済みスレッドの変更は再開後に行ってください");
        }
        !archived
    }

    /// チャンネルの下に出すスレッドの行。通常の一覧とピン留めで、同じスレッドを同じ並びで出す。
    /// 参加していないチャンネルは、閲覧中の1件だけを出す。
    fn thread_rows(&self, channel_id: &str, pinned: bool) -> Vec<SidebarItem> {
        let joined = self
            .store
            .channel(channel_id)
            .is_some_and(|channel| !channel.is_dm)
            && self.store.is_member(channel_id, &self.me.id);
        let row = |id: &str| SidebarItem::Thread {
            channel_id: channel_id.to_owned(),
            id: id.to_owned(),
            pinned,
        };
        let mut rows: Vec<SidebarItem> = self
            .threads()
            .filter(|thread| {
                joined
                    && thread.channel_id == channel_id
                    && !self.thread_is_archived(channel_id, &thread.id)
            })
            .map(|thread| row(&thread.id))
            .collect();
        if self.current.as_deref() == Some(channel_id)
            && let Some(thread) = &self.thread
        {
            let selected = row(&thread.root_id);
            if !rows.contains(&selected)
                && self.store.message(channel_id, &thread.root_id).is_some()
                && !self.thread_is_archived(channel_id, &thread.root_id)
                && !self.store.replies(channel_id, &thread.root_id).is_empty()
            {
                rows.push(selected);
            }
        }
        rows
    }

    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();
        // ピン留めは先頭に出す。アーカイブ中は通常の一覧だけに残す（アンアーカイブで戻る）。
        // 通常のチャンネルには、通常の一覧と同じスレッドを下に出す（DM には出さない）。
        for pin in self.store.pinned_channels() {
            let visible = self.store.channel(&pin.channel_id).is_some_and(|channel| {
                channel.deleted_at.is_none() && channel.archive_at.is_none()
            });
            if visible {
                items.push(SidebarItem::Pinned(pin.channel_id.clone()));
                let is_dm = self
                    .store
                    .channel(&pin.channel_id)
                    .is_some_and(|channel| channel.is_dm);
                if !is_dm {
                    items.extend(self.thread_rows(&pin.channel_id, true));
                }
            }
        }
        for channel in self.store.channels() {
            items.push(SidebarItem::Channel(channel.id.clone()));
            items.extend(self.thread_rows(&channel.id, false));
        }
        for archived in [false, true] {
            if !archived
                && (self.thread_cursors[usize::from(archived)].is_some()
                    || self.thread_failed_append[usize::from(archived)].is_some())
            {
                items.push(SidebarItem::MoreThreads { archived });
            }
        }
        items
    }

    /// カーソルがある行。一時のスレッドの行が増減する操作の前に控えて、あとで `restore_sidebar_selection` に渡す。
    fn selected_sidebar_item(&self) -> Option<SidebarItem> {
        self.sidebar_items().get(self.sidebar_index).cloned()
    }

    fn restore_sidebar_selection(&mut self, selected: Option<SidebarItem>) {
        let items = self.sidebar_items();
        self.sidebar_index = selected
            .and_then(|selected| items.iter().position(|item| *item == selected))
            .unwrap_or_else(|| self.sidebar_index.min(items.len().saturating_sub(1)));
    }

    fn reopen_thread(&self) {
        let id = match self.focus {
            Focus::Sidebar => match self.sidebar_items().get(self.sidebar_index) {
                Some(SidebarItem::Thread { id, .. }) => Some(id.clone()),
                _ => None,
            },
            Focus::Thread => self.thread.as_ref().map(|thread| thread.root_id.clone()),
            _ => None,
        };
        let Some(id) = id else { return };
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        self.spawn_api("スレッドの再開", async move {
            api.unarchive_thread(&id).await?;
            // POST の成否だけを再開結果とし、一覧取得は画面側で再試行する。
            let _ = tx.send(AppEvent::ThreadsChanged { reopened: Some(id) });
            Ok(())
        });
    }

    /// 開いているチャンネルの最新のルートメッセージまで既読にする。参加していないチャンネルは
    /// backend が既読を持たないので送らない。
    fn mark_current_read(&mut self) {
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        if !self.terminal_active
            || !self.store.is_synced(&ShapeId::Messages(channel_id.clone()))
            || !self.store.is_member(&channel_id, &self.me.id)
        {
            return;
        }
        let Some(latest) = self.store.latest_root(&channel_id).map(|m| m.id.clone()) else {
            return;
        };
        // 古いメッセージを読み返している間は、画面に出ていない新着を既読にしない。
        // backend は既読位置を戻さないので、表示してから送る。
        if self.newest_shown.as_ref() != Some(&(channel_id.clone(), latest.clone())) {
            return;
        }
        self.unread.remove(&channel_id);
        if self.marked_read.get(&channel_id) == Some(&latest) {
            return;
        }
        self.marked_read.insert(channel_id.clone(), latest.clone());
        let api = self.api.clone();
        tokio::spawn(async move {
            let _ = api.mark_read(&channel_id, &latest).await;
        });
    }

    /// タイムラインと、開いているスレッドの入力中ユーザーを取得する。
    fn poll_typing(&mut self) {
        if !self.terminal_active {
            return;
        }
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        if !self.store.is_member(&channel_id, &self.me.id) {
            return;
        }
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|channel| channel.archive_at.is_some() || channel.deleted_at.is_some())
        {
            self.typing.clear();
            return;
        }
        let mut scopes = vec![None];
        if let Some(thread) = &self.thread {
            scopes.push(Some(thread.root_id.clone()));
        }
        if !self.typing_poll.start(Instant::now()) {
            return;
        }
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            for parent_id in scopes {
                if let Ok(users) = api.typing(&channel_id, parent_id.as_deref()).await {
                    let _ = tx.send(AppEvent::Typing {
                        channel_id: channel_id.clone(),
                        parent_id,
                        users,
                    });
                }
            }
            let _ = tx.send(AppEvent::TypingPollFinished);
        });
    }

    /// 文字を入力したときに呼ぶ。送り先が変わったら、前の送り先の入力中を取り消す。
    fn touch_typing(&mut self) {
        let (Some(channel_id), parent_id) = (
            self.current.clone(),
            match &self.target {
                Target::Channel => None,
                Target::Reply(root_id) => Some(root_id.clone()),
                Target::Edit(_) => return,
            },
        ) else {
            return;
        };
        self.last_keystroke = Some(Instant::now());
        if let Some((sent_channel, sent_parent, at)) = &self.typing_sent {
            let same = *sent_channel == channel_id && *sent_parent == parent_id;
            if same && at.elapsed() < TYPING_SEND_INTERVAL {
                return;
            }
            if !same {
                self.stop_typing();
                self.last_keystroke = Some(Instant::now());
            }
        }
        if self.typing_sent.is_none() {
            self.typing_poll.wake();
        }
        self.send_typing(&channel_id, parent_id.clone(), true);
        self.typing_sent = Some((channel_id, parent_id, Instant::now()));
    }

    fn stop_typing(&mut self) {
        self.last_keystroke = None;
        if let Some((channel_id, parent_id, _)) = self.typing_sent.take() {
            self.send_typing(&channel_id, parent_id, false);
        }
    }

    fn send_typing(&self, channel_id: &str, parent_id: Option<String>, active: bool) {
        let api = self.api.clone();
        let channel_id = channel_id.to_string();
        let session = self.typing_session.clone();
        tokio::spawn(async move {
            let _ = api
                .set_typing(&channel_id, parent_id.as_deref(), &session, active)
                .await;
        });
    }

    pub fn on_terminal_event(&mut self, event: Event) {
        if matches!(event, Event::FocusLost | Event::FocusGained) {
            self.terminal_active = matches!(event, Event::FocusGained);
            if self.terminal_active {
                self.refresh_threads();
                self.read_poll.wake();
                self.emoji_poll.wake();
                self.typing_poll.wake();
            } else {
                self.stop_typing();
                self.typing.clear();
            }
            return;
        }
        let key = match event {
            Event::Key(key) => key,
            Event::Paste(text) => return self.on_paste(&text),
            _ => return,
        };
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        // 前面に出した他のユーザーの検証は、ほかの画面より先にキーを受け取る。
        if matches!(self.mode, Mode::Normal | Mode::Devices) && self.devices.has_open_verification()
        {
            self.devices.on_verification_key(key);
            return;
        }
        match self.mode.clone() {
            Mode::Normal => self.on_normal_key(key),
            Mode::Insert => self.on_insert_key(key),
            Mode::ConfirmDelete { message_id } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y'))
                    && self.can_change_message(&message_id)
                {
                    // スレッドの親を消したら、スレッドも閉じる。
                    if self
                        .thread
                        .as_ref()
                        .is_some_and(|thread| thread.root_id == message_id)
                    {
                        self.close_thread();
                    }
                    let api = self.api.clone();
                    let tx = self.app_tx.clone();
                    self.spawn_api("削除", async move {
                        api.delete_message(&message_id).await?;
                        let _ = tx.send(AppEvent::ThreadsChanged { reopened: None });
                        Ok(())
                    });
                }
                self.mode = Mode::Normal;
            }
            Mode::Search(query) => self.on_search_key(key, query),
            Mode::GlobalSearch => self.on_global_search_key(key),
            Mode::React { message_id, input } => self.on_react_key(key, message_id, input),
            Mode::Help => self.mode = Mode::Normal,
            Mode::Encryption => {
                let user_id = self.me.id.clone();
                if !self.encryption.on_key(key, &user_id) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Devices => {
                // 読み直し（`r`）では、同期が進んで増えた、暗号化チャンネルを共有する相手も取り直す。
                if key.code == KeyCode::Char('r') {
                    self.devices.set_peers(self.encrypted_peers());
                }
                if !self.devices.on_key(key) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Create(form) => self.on_create_key(key, form),
            Mode::Recipients { scroll } => {
                let last = super::view::recipient_details(self)
                    .map_or(0, |lines| lines.len().saturating_sub(1));
                self.mode = match key.code {
                    KeyCode::Char('j') | KeyCode::Down => Mode::Recipients {
                        scroll: (scroll + 1).min(last),
                    },
                    KeyCode::Char('k') | KeyCode::Up => Mode::Recipients {
                        scroll: scroll.saturating_sub(1),
                    },
                    KeyCode::Char('g') | KeyCode::Home => Mode::Recipients { scroll: 0 },
                    KeyCode::Char('G') | KeyCode::End => Mode::Recipients { scroll: last },
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('V') => Mode::Normal,
                    _ => Mode::Recipients { scroll },
                };
            }
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('E') => {
                self.mode = Mode::Encryption;
                self.encryption.open();
            }
            KeyCode::Char('D') => {
                self.mode = Mode::Devices;
                let peers = self.encrypted_peers();
                self.devices.open(peers);
                // 知らせただけの検証の要求があれば、前面に出す。
                if !self.devices.has_open_verification() {
                    self.devices.sas_open = self
                        .devices
                        .sas_entries
                        .iter()
                        .find(|(_, entry)| !entry.view.finished())
                        .map(|(txn, _)| txn.clone());
                }
            }
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            // ピン留めの並び替えは Alt+↑↓ と [ / ]。通常の選択移動より先に判定する。
            KeyCode::Up if key.modifiers.contains(KeyModifiers::ALT) => self.move_pinned(-1),
            KeyCode::Down if key.modifiers.contains(KeyModifiers::ALT) => self.move_pinned(1),
            KeyCode::Char('[') => self.move_pinned(-1),
            KeyCode::Char(']') => self.move_pinned(1),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_selection(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_selection(isize::MAX),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_inside(true)
            }
            KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.mode = Mode::GlobalSearch
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_inside(false)
            }
            KeyCode::PageDown => self.scroll_inside(true),
            KeyCode::PageUp => self.scroll_inside(false),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => self.back(),
            KeyCode::Char('i') => self.start_compose(),
            KeyCode::Char('e') => self.start_edit(),
            KeyCode::Char('d') => self.confirm_delete(),
            KeyCode::Char('r') => {
                if let Some((_, message_id)) = self.focused_message()
                    && self.can_change_message(&message_id)
                {
                    self.mode = Mode::React {
                        message_id,
                        input: String::new(),
                    };
                }
            }
            KeyCode::Char('p') => self.toggle_pin(),
            // J はピン留め行でもチャンネル参加。並び替えは [ / ] と Alt+↑↓ に分ける。
            KeyCode::Char('J') => self.join_or_leave(true),
            KeyCode::Char('L') => self.join_or_leave(false),
            KeyCode::Char('N') => self.cycle_notification_level(),
            KeyCode::Char('/') => self.mode = Mode::Search(String::new()),
            KeyCode::Char('S') => self.mode = Mode::GlobalSearch,
            KeyCode::Char('R') => self.restore_failed_draft(),
            KeyCode::Char('u') => self.reopen_thread(),
            KeyCode::Char('c') => self.mode = Mode::Create(CreateForm::channel()),
            KeyCode::Char('m') => self.mode = Mode::Create(CreateForm::dm()),
            KeyCode::Char('V') => {
                if self
                    .current
                    .as_ref()
                    .is_some_and(|id| self.recipients.contains_key(id))
                {
                    self.mode = Mode::Recipients { scroll: 0 };
                } else {
                    self.notify("まだ配布先を判定していません。暗号化チャンネルに送ると判定します");
                }
            }
            _ => {}
        }
    }

    /// チャンネルか DM の作成の入力欄（#361）。Tab で公開範囲（暗号化するか）を切り替える。
    fn on_create_key(&mut self, key: KeyEvent, mut form: CreateForm) {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                return;
            }
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                self.create(form);
                return;
            }
            KeyCode::Tab | KeyCode::BackTab => form.toggle(),
            KeyCode::Backspace => {
                form.input.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                form.input.push(c)
            }
            _ => {}
        }
        self.mode = Mode::Create(form);
    }

    /// チャンネルか DM を作り、作れたら開く。暗号化を求めたのに、サーバーが暗号化されていない
    /// チャンネルを返したら、平文では送らない（docs/e2ee.md の「暗号化チャンネル」）。
    fn create(&mut self, form: CreateForm) {
        let encrypted = form.encrypted();
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        match form.kind {
            CreateKind::Channel => {
                let name = form.input.trim().to_owned();
                if name.is_empty() {
                    self.notify("チャンネル名を入れてください");
                    return;
                }
                let private = form.visibility != e2ee_chat::Visibility::Public;
                tokio::spawn(async move {
                    let result = if encrypted {
                        api.create_encrypted_channel(&name, None).await
                    } else {
                        api.create_channel(None, &name, None, private).await
                    };
                    let _ = tx.send(match result {
                        Ok((channel, _)) => AppEvent::Created {
                            channel_id: channel.id,
                            requested_encryption: encrypted,
                            encrypted: channel.encrypted,
                        },
                        Err(err) => AppEvent::Notice(e2ee_chat::create_error(&err)),
                    });
                });
            }
            CreateKind::Dm => {
                let mut ids = Vec::new();
                for name in form.input.split_whitespace() {
                    let name = name.trim_start_matches('@');
                    let found = self
                        .store
                        .users()
                        .find(|user| user.user_name == name)
                        .map(|user| user.id.clone());
                    match found {
                        Some(id) if id == self.me.id => {}
                        Some(id) => ids.push(id),
                        None => {
                            self.notify(format!("ユーザーが見つかりません: @{name}"));
                            return;
                        }
                    }
                }
                ids.sort();
                ids.dedup();
                if ids.is_empty() {
                    self.notify("相手のユーザー名を入れてください");
                    return;
                }
                tokio::spawn(async move {
                    let _ = tx.send(match api.create_dm(&ids, encrypted).await {
                        Ok((channel, _)) => AppEvent::Created {
                            channel_id: channel.id,
                            requested_encryption: encrypted,
                            encrypted: channel.encrypted,
                        },
                        Err(err) => AppEvent::Notice(e2ee_chat::create_error(&err)),
                    });
                });
            }
        }
    }

    fn on_insert_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter
                if !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SHIFT) =>
            {
                self.submit()
            }
            KeyCode::Enter => {
                self.composer.insert_newline();
            }
            _ => {
                if self.composer.input(key) {
                    self.touch_typing();
                }
            }
        }
    }

    /// 貼り付けは改行を含めて入力欄に入れ、送信はしない。入力中でなければ、キー操作と
    /// 取り違えないよう無視する。
    fn on_paste(&mut self, text: &str) {
        if self.mode == Mode::Encryption {
            self.encryption.on_paste(text);
            return;
        }
        if self.mode == Mode::Devices {
            self.devices.on_paste(text);
            return;
        }
        if let Mode::Create(form) = &mut self.mode {
            form.input.extend(text.chars().filter(|c| !c.is_control()));
            return;
        }
        if self.mode == Mode::GlobalSearch {
            // 検索語に足すだけ。検索は Enter で行う。
            self.global_search
                .query
                .extend(text.chars().filter(|c| !c.is_control()));
            return;
        }
        if self.mode != Mode::Insert {
            return;
        }
        // 端末によっては改行を CR だけで送ってくる。
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.composer.insert_str(text) {
            self.touch_typing();
        }
    }

    fn on_global_search_key(&mut self, key: KeyEvent) {
        match self.global_search.on_key(key) {
            global_search::Action::None => {}
            global_search::Action::Close => self.mode = Mode::Normal,
            global_search::Action::Search => self.fetch_search(false),
            global_search::Action::More => self.fetch_search(true),
            global_search::Action::Open => {
                if let Some(target) = self.global_search.destination() {
                    self.mode = Mode::Normal;
                    self.open_search_result(target);
                }
            }
        }
    }

    /// 全文検索の 1 ページを取る。古い取得の結果は番号で捨てる（取り消しはしない）。
    fn fetch_search(&mut self, append: bool) {
        let Some((generation, query, kind, cursor)) = self.global_search.start(append) else {
            return;
        };
        // `from:@名前`・`in:#チャンネル`（複数可）・`after:`・`before:`・`with:me`・`-is:bot` を絞り込みにする
        // （名前は同期済みの一覧で引く）。
        let parsed = global_search::parse_modifiers(&query);
        // 日付は端末の暦の日として、その 0 時で区切る。
        let tz = self
            .global_search
            .time_zone
            .clone()
            .unwrap_or_else(jiff::tz::TimeZone::system);
        let midnight = |day: Option<String>| day.map(|d| global_search::local_midnight(&d, &tz));
        let mut filters = crate::api::SearchFilters {
            after: midnight(parsed.after),
            before: midnight(parsed.before),
            exclude_bots: parsed.no_bots,
            involving_me: parsed.with_me,
            ..Default::default()
        };
        for name in &parsed.from {
            match self
                .store
                .users()
                .find(|user| user.user_name.eq_ignore_ascii_case(name))
            {
                Some(user) => filters.user_ids.push(user.id.clone()),
                None => {
                    self.global_search
                        .fail(generation, format!("@{name} という人は見つかりません"));
                    return;
                }
            }
        }
        for name in &parsed.channels {
            match self
                .store
                .channels()
                .into_iter()
                .find(|channel| !channel.is_dm && channel.channel_name.eq_ignore_ascii_case(name))
            {
                Some(channel) => filters.channel_ids.push(channel.id.clone()),
                None => {
                    self.global_search.fail(
                        generation,
                        format!("#{name} というチャンネルは見つかりません"),
                    );
                    return;
                }
            }
        }
        if filters.user_ids.len() > 20 || filters.channel_ids.len() > 20 {
            self.global_search.fail(
                generation,
                "from: と in: は、それぞれ 20 個まで書けます".into(),
            );
            return;
        }
        if parsed.terms.is_empty() {
            self.global_search.fail(
                generation,
                "絞り込みのほかに、探す語を入れてください".into(),
            );
            return;
        }
        let terms = parsed.terms;
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            let event = match api.search(kind, &terms, &filters, cursor.as_deref()).await {
                Ok(page) => AppEvent::SearchResults {
                    generation,
                    append,
                    page,
                },
                Err(error) => AppEvent::SearchFailed {
                    generation,
                    message: format!("{error:#}"),
                },
            };
            let _ = tx.send(event);
        });
    }

    /// 検索結果のメッセージを開く。スレッドの返信ならスレッドを開いてその返信を選ぶ。
    /// 行がまだ同期されていなくても、選択は届いた時点で効く（描画時に位置を探す）。
    fn open_search_result(&mut self, target: global_search::Destination) {
        self.open_channel(target.channel_id);
        match target.parent_id {
            Some(root_id) => {
                self.thread = Some(ThreadView {
                    root_id,
                    selected: Some(target.message_id),
                    scroll: 0,
                    inner_scroll: 0,
                });
                self.focus = Focus::Thread;
                self.poll_typing();
            }
            None => {
                self.selected = Some(target.message_id);
                // 前に選んでいたメッセージの中のスクロールを持ち越さない（`move_selection` と同じ）。
                self.inner_scroll = 0;
                self.focus = Focus::Messages;
            }
        }
    }

    fn on_search_key(&mut self, key: KeyEvent, mut query: String) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                if let Some(index) = self.search_matches(&query).first().copied() {
                    self.sidebar_index = index;
                    self.activate_sidebar();
                }
            }
            KeyCode::Backspace => {
                query.pop();
                self.mode = Mode::Search(query);
            }
            KeyCode::Char(ch) => {
                query.push(ch);
                self.mode = Mode::Search(query);
            }
            _ => {}
        }
    }

    fn on_react_key(&mut self, key: KeyEvent, message_id: String, mut input: String) {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Normal,
            KeyCode::Enter => {
                self.mode = Mode::Normal;
                match resolve_emoji(&input) {
                    Some(emoji) => self.toggle_reaction(message_id, emoji),
                    None => self.notify(format!("絵文字が見つかりません: {input}")),
                }
            }
            KeyCode::Backspace => {
                input.pop();
                self.mode = Mode::React { message_id, input };
            }
            KeyCode::Char(ch) => {
                input.push(ch);
                self.mode = Mode::React { message_id, input };
            }
            _ => {}
        }
    }

    /// 自分がすでに付けた絵文字なら外し、そうでなければ付ける。
    fn toggle_reaction(&mut self, message_id: String, emoji: String) {
        if !self.can_change_message(&message_id) {
            return;
        }
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        let mine = self
            .store
            .reaction_groups(&channel_id, &message_id, &self.me.id)
            .iter()
            .any(|group| group.emoji == emoji && group.mine);
        let api = self.api.clone();
        if mine {
            self.spawn_api("リアクションの取り消し", async move {
                api.remove_reaction(&message_id, &emoji).await
            });
        } else {
            let id = uuid::Uuid::new_v4().to_string();
            self.spawn_api("リアクション", async move {
                api.add_reaction(&id, &message_id, &emoji).await
            });
        }
    }

    /// サイドバーで選択中のチャンネル。スレッド選択中はその親チャンネル。
    fn selected_sidebar_channel(&self) -> Option<String> {
        match self.focus {
            Focus::Sidebar => self
                .sidebar_items()
                .get(self.sidebar_index)
                .and_then(|item| match item {
                    SidebarItem::Channel(id) | SidebarItem::Pinned(id) => Some(id.clone()),
                    SidebarItem::Thread { channel_id, .. } => Some(channel_id.clone()),
                    SidebarItem::MoreThreads { .. } => None,
                }),
            _ => self.current.clone(),
        }
    }

    /// ピン留めの版を取り直す。起動時と、409 のあとに使う。
    fn refresh_pin_version(&self) {
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            if let Ok(pins) = api.channel_pins().await {
                let _ = tx.send(AppEvent::PinsVersion(pins.version));
            }
        });
    }

    /// 選択中のチャンネルのピン留めを切り替える。409 のときは最新の版で 1 回やり直す。
    fn toggle_pin(&mut self) {
        let Some(channel_id) = self.selected_sidebar_channel() else {
            self.notify("チャンネルを選んでください");
            return;
        };
        let pinned = self.store.is_pinned(&channel_id);
        if !pinned && let Some(channel) = self.store.channel(&channel_id) {
            // アーカイブ中・削除済みは新しくピン留めできない。サーバーと同じ条件を
            // 先に確かめ、押しただけで API を送らない。ピン済みなら解除できる。
            // チャンネル行がまだ届いていない場合は、同期前の有効な操作を誤って
            // 拒否しないよう、サーバーの判定に任せる。
            if channel.deleted_at.is_some() {
                self.notify("削除済みのチャンネルはピン留めできません");
                return;
            }
            if channel.archive_at.is_some() {
                self.notify("アーカイブ中のチャンネルはピン留めできません");
                return;
            }
            // `channel_user` の初回同期前は、未参加と「まだ届いていない」を区別できない。
            // 同期済みのときだけゲストの未参加を理由に止める。
            let is_guest = self
                .store
                .user(&self.me.id)
                .is_some_and(|user| user.is_guest);
            if is_guest
                && self.store.is_synced(&ShapeId::Users)
                && self.store.is_synced(&ShapeId::ChannelUsers)
                && !self.store.is_member(&channel_id, &self.me.id)
            {
                self.notify("ゲストは参加していない公開チャンネルをピン留めできません");
                return;
            }
        }
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        let version = self.pin_version;
        self.spawn_api(
            if pinned {
                "ピン留めの解除"
            } else {
                "ピン留め"
            },
            async move {
                let result = if pinned {
                    api.unpin_channel(&channel_id, version).await
                } else {
                    api.pin_channel(&channel_id, version).await
                };
                match result {
                    Ok(version) => {
                        let _ = tx.send(AppEvent::PinsVersion(version));
                        Ok(())
                    }
                    Err(err) if ApiError::has_code(&err, "CHANNEL_PINS_CONFLICT") => {
                        let latest = api.channel_pins().await?;
                        let version = if pinned {
                            api.unpin_channel(&channel_id, latest.version).await?
                        } else {
                            api.pin_channel(&channel_id, latest.version).await?
                        };
                        let _ = tx.send(AppEvent::PinsVersion(version));
                        Ok(())
                    }
                    Err(err) => Err(err),
                }
            },
        );
    }

    /// 選択中のピン留めを `delta` だけ動かす。409 のときは最新の版で 1 回やり直し、
    /// 相手が先に解除していたら復活させない。
    fn move_pinned(&mut self, delta: isize) {
        let selected = match self.focus {
            Focus::Sidebar => match self.sidebar_items().get(self.sidebar_index) {
                Some(SidebarItem::Pinned(id)) => id.clone(),
                _ => {
                    self.notify("ピン留めを選んでください");
                    return;
                }
            },
            _ => return,
        };
        let ids: Vec<String> = self
            .store
            .pinned_channels()
            .into_iter()
            .filter(|pin| {
                self.store.channel(&pin.channel_id).is_some_and(|channel| {
                    channel.deleted_at.is_none() && channel.archive_at.is_none()
                })
            })
            .map(|pin| pin.channel_id.clone())
            .collect();
        let Some(index) = ids.iter().position(|id| id == &selected) else {
            return;
        };
        let target = (index as isize + delta).clamp(0, ids.len() as isize - 1) as usize;
        if target == index {
            return;
        }
        let mut order = ids;
        order.remove(index);
        order.insert(target, selected.clone());
        let moved = selected.clone();
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        let version = self.pin_version;
        self.spawn_api("ピン留めの並び替え", async move {
            match api.reorder_channel_pins(&order, version).await {
                Ok(version) => {
                    let _ = tx.send(AppEvent::PinsVersion(version));
                    Ok(())
                }
                Err(err) if ApiError::has_code(&err, "CHANNEL_PINS_CONFLICT") => {
                    let latest = api.channel_pins().await?;
                    let _ = tx.send(AppEvent::PinsVersion(latest.version));
                    let latest_active: Vec<String> = latest
                        .pins
                        .iter()
                        .filter(|pin| !pin.archived)
                        .map(|pin| pin.channel_id.clone())
                        .collect();
                    if !latest_active.contains(&moved) {
                        return Err(anyhow::anyhow!(
                            "別の端末でピン留めが解除またはアーカイブされたため、並びを変えませんでした"
                        ));
                    }
                    // ほかの端末が追加したピンは落とさず、最新の有効なピンの上で
                    // 動かした項目だけを、意図した並びの隣を目印に入れ直す。
                    let next = merge_pinned_order(&order, &moved, &latest_active);
                    let version = api.reorder_channel_pins(&next, latest.version).await?;
                    let _ = tx.send(AppEvent::PinsVersion(version));
                    Ok(())
                }
                Err(err) => Err(err),
            }
        });
    }

    /// チャンネル名・スレッド名に `query` を含む行の添字。
    pub fn search_matches(&self, query: &str) -> Vec<usize> {
        let query = query.to_lowercase();
        self.sidebar_items()
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                let label = match item {
                    SidebarItem::Channel(id) | SidebarItem::Pinned(id) => self
                        .store
                        .channel(id)
                        .map(|channel| self.store.channel_label(channel, &self.me.id))
                        .unwrap_or_default(),
                    SidebarItem::MoreThreads { .. } => return false,
                    SidebarItem::Thread { channel_id, id, .. } => self
                        .thread_content(channel_id, id)
                        .unwrap_or_default()
                        .to_owned(),
                };
                label.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn cycle_focus(&mut self, forward: bool) {
        let mut order = vec![Focus::Sidebar, Focus::Messages];
        if self.thread.is_some() {
            order.push(Focus::Thread);
        }
        let index = order
            .iter()
            .position(|focus| *focus == self.focus)
            .unwrap_or(0);
        let next = if forward {
            (index + 1) % order.len()
        } else {
            (index + order.len() - 1) % order.len()
        };
        self.focus = order[next];
    }

    fn move_selection(&mut self, delta: isize) {
        match self.focus {
            Focus::Sidebar => {
                let len = self.sidebar_items().len();
                self.sidebar_index = step(Some(self.sidebar_index), delta, len).unwrap_or(0);
            }
            Focus::Messages => {
                let Some(channel_id) = self.current.clone() else {
                    return;
                };
                let ids: Vec<String> = self
                    .store
                    .root_messages(&channel_id)
                    .iter()
                    .map(|message| message.id.clone())
                    .collect();
                self.selected = move_id(&ids, self.selected.as_deref(), delta);
                self.inner_scroll = 0;
            }
            Focus::Thread => {
                let (Some(channel_id), Some(thread)) = (self.current.clone(), self.thread.as_mut())
                else {
                    return;
                };
                // 親が削除されて描かれないときは、候補を描いている返信だけにする（見えない親を選ばない）。
                let mut ids: Vec<String> = self
                    .store
                    .message(&channel_id, &thread.root_id)
                    .map(|root| root.id.clone())
                    .into_iter()
                    .collect();
                ids.extend(
                    self.store
                        .replies(&channel_id, &thread.root_id)
                        .iter()
                        .map(|message| message.id.clone()),
                );
                thread.selected = move_id(&ids, thread.selected.as_deref(), delta);
                thread.inner_scroll = 0;
            }
        }
    }

    /// ペインより長いメッセージの中を動かす。行き過ぎは描画時に詰める。
    fn scroll_inside(&mut self, down: bool) {
        let inner = match self.focus {
            Focus::Messages if self.selected.is_some() => &mut self.inner_scroll,
            Focus::Thread => match self.thread.as_mut() {
                Some(thread) if thread.selected.is_some() => &mut thread.inner_scroll,
                _ => return,
            },
            _ => return,
        };
        *inner = if down {
            inner.saturating_add(INNER_SCROLL_STEP)
        } else {
            inner.saturating_sub(INNER_SCROLL_STEP)
        };
    }

    fn activate(&mut self) {
        let root_id = match self.focus {
            Focus::Sidebar => {
                self.activate_sidebar();
                return;
            }
            Focus::Messages => self.selected.clone(),
            Focus::Thread => self.thread.as_ref().and_then(|thread| {
                thread
                    .selected
                    .as_ref()
                    .filter(|id| {
                        self.current
                            .as_deref()
                            .is_some_and(|channel| !self.store.replies(channel, id).is_empty())
                    })
                    .cloned()
            }),
        };
        if let Some(root_id) = root_id {
            self.typing_poll.wake();
            // 一覧にないスレッドを開くと、その下に一時の行が増えるので、カーソルは同じ行に残す。
            let selected = self.selected_sidebar_item();
            self.thread = Some(ThreadView {
                root_id,
                selected: None,
                scroll: 0,
                inner_scroll: 0,
            });
            self.restore_sidebar_selection(selected);
            self.focus = Focus::Thread;
        }
    }

    fn activate_sidebar(&mut self) {
        match self.selected_sidebar_item() {
            Some(SidebarItem::Channel(channel_id) | SidebarItem::Pinned(channel_id)) => {
                // カーソルは `open_channel` が、押した行に残す。
                self.open_channel(channel_id);
                self.focus = Focus::Messages;
            }
            Some(SidebarItem::Thread {
                channel_id,
                id,
                pinned,
            }) => {
                self.open_channel(channel_id);
                self.thread = Some(ThreadView {
                    root_id: id.clone(),
                    selected: None,
                    scroll: 0,
                    inner_scroll: 0,
                });
                // 開いた行（ピン留めの下か通常の一覧か）に、カーソルを残す。
                self.restore_sidebar_selection(Some(SidebarItem::Thread {
                    channel_id: self.current.clone().unwrap_or_default(),
                    id: id.clone(),
                    pinned,
                }));
                self.focus = Focus::Thread;
                self.poll_typing();
            }
            Some(SidebarItem::MoreThreads { archived }) => self.fetch_thread_page(
                archived,
                self.thread_failed_append[usize::from(archived)].unwrap_or(true),
            ),
            None => {}
        }
    }

    pub fn open_channel(&mut self, channel_id: String) {
        // 別チャンネルの、一覧にないスレッドを閲覧中は、一時の行（ピン留めの下を含む）が出ている。
        // チャンネルを開くとスレッドが閉じてその行が消え、添字がずれるので、カーソルは同じ行に残す。
        let selected = self.selected_sidebar_item();
        if self.current.as_deref() != Some(channel_id.as_str()) {
            let previous: Vec<_> = self
                .subscriptions
                .keys()
                .filter(|id| matches!(id, ShapeId::Messages(_) | ShapeId::Reactions(_)))
                .cloned()
                .collect();
            for id in previous {
                if let Some(task) = self.subscriptions.remove(&id) {
                    task.abort();
                }
                if let ShapeId::Messages(channel_id) = &id {
                    self.message_resync_counts.remove(channel_id);
                }
                self.store.apply(
                    &id,
                    Batch {
                        reset: true,
                        ..Default::default()
                    },
                );
            }
            self.stop_typing();
            self.typing.clear();
            self.typing_poll.wake();
            self.selected = None;
            self.scroll = 0;
            self.thread = None;
            // 編集中の本文は元のメッセージのものなので、別のチャンネルへは持ち越さない。
            if matches!(self.target, Target::Edit(_)) {
                self.composer = new_composer();
            }
            self.target = Target::Channel;
        }
        self.subscribe(ShapeId::Messages(channel_id.clone()));

        self.current = Some(channel_id);
        self.restore_sidebar_selection(selected);
        self.poll_typing();
    }

    fn back(&mut self) {
        match self.focus {
            Focus::Thread => self.close_thread(),
            Focus::Messages if self.selected.is_some() => self.selected = None,
            Focus::Messages => self.focus = Focus::Sidebar,
            Focus::Sidebar => {}
        }
    }

    /// スレッドを閉じる。返信を書きかけていたら、送り先をチャンネルに戻す。
    fn close_thread(&mut self) {
        self.typing_poll.wake();
        // 一覧にないスレッドを閉じると、その下の一時の行が消えるので、カーソルは同じ行に残す。
        let selected = self.selected_sidebar_item();
        self.thread = None;
        self.restore_sidebar_selection(selected);
        self.focus = Focus::Messages;
        if matches!(self.target, Target::Reply(_)) {
            self.target = Target::Channel;
        }
    }

    /// 送り先はフォーカスで決める。スレッドを見ているときは返信になる。
    fn start_compose(&mut self) {
        let Some(channel_id) = self.current.clone() else {
            self.notify("先にチャンネルを開いてください");
            return;
        };
        if self
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some())
        {
            self.notify("アーカイブされたチャンネルには投稿できません");
            return;
        }
        // 書きかけの編集 (送信に失敗して戻ったものを含む) は、編集のまま続ける。
        if matches!(self.target, Target::Edit(_)) && !self.composer.is_empty() {
            self.mode = Mode::Insert;
            return;
        }
        let target = match (&self.thread, self.focus) {
            (Some(thread), Focus::Thread) => Target::Reply(thread.root_id.clone()),
            _ => Target::Channel,
        };
        if matches!(self.target, Target::Edit(_)) || self.target != target {
            if matches!(self.target, Target::Edit(_)) {
                self.composer = new_composer();
            }
            self.target = target;
        }
        self.mode = Mode::Insert;
    }

    /// 保存した失敗の本文を新しい順に1件、送り先のチャンネルを開いて入力欄に戻す。
    fn restore_failed_draft(&mut self) {
        if self.failed_drafts.is_empty() {
            self.notify("送れなかった本文はありません");
            return;
        }
        if !self.composer.is_empty() {
            self.notify("書きかけの下書きがあります。送信するか消してから戻してください");
            return;
        }
        // 開いているチャンネルのものを優先する。
        let index = self
            .failed_drafts
            .iter()
            .rposition(|draft| self.current.as_deref() == Some(draft.channel_id.as_str()))
            .unwrap_or(self.failed_drafts.len() - 1);
        let draft = self.failed_drafts.remove(index);
        if self.current.as_deref() != Some(draft.channel_id.as_str()) {
            self.open_channel(draft.channel_id);
            self.focus = Focus::Messages;
        }
        self.composer = composer_with(&draft.content);
        self.set_edit_base(&draft.target, draft.edit_base);
        self.target = draft.target;
        self.mode = Mode::Insert;
    }

    /// 戻した下書きの、編集を始めたときの版を入力欄に戻す（ほかの下書きの版で確かめないため）。
    fn set_edit_base(&mut self, target: &Target, edit_base: Option<u64>) {
        self.edit_base = match (target, edit_base) {
            (Target::Edit(message_id), Some(revision)) => Some((message_id.clone(), revision)),
            _ => None,
        };
    }

    /// フォーカス中のペインで選択しているメッセージ。
    pub fn focused_message(&self) -> Option<(String, String)> {
        let channel_id = self.current.clone()?;
        let message_id = match self.focus {
            Focus::Thread => self.thread.as_ref()?.selected.clone()?,
            Focus::Messages => self.selected.clone()?,
            Focus::Sidebar => return None,
        };
        Some((channel_id, message_id))
    }

    fn start_edit(&mut self) {
        let Some((channel_id, message_id)) = self.focused_message() else {
            return;
        };
        if !self.can_change_message(&message_id) {
            return;
        }
        let Some(message) = self.store.message(&channel_id, &message_id) else {
            return;
        };
        if message.user_id != self.me.id {
            self.notify("自分のメッセージだけを編集できます");
            return;
        }
        // 暗号化チャンネルでは、復号できた最新の版の本文を編集する（#361）。暗号化前の平文の
        // メッセージは編集できない（サーバーも 409 `E2EE_LEGACY_IMMUTABLE` で拒否する。#367）。
        let encrypted = self.channel_encryption(&channel_id).treat_as_encrypted();
        let body = match self.decryptions.shown(message, encrypted) {
            Shown::Plain => message.content.clone(),
            Shown::BeforeEncryption => {
                self.notify("暗号化前のメッセージは編集できません");
                return;
            }
            Shown::Decrypted { warning: None, .. } => match self.decryptions.editable(message) {
                Some((decrypted, revision)) => {
                    // 編集を始めた版を覚え、送るときに版が進んでいたら送らない（別の端末の編集を、
                    // 古い版から作った本文で上書きしないため）。
                    self.edit_base = Some((message_id.clone(), revision));
                    decrypted.body.clone()
                }
                None => return,
            },
            Shown::Decrypted { .. } | Shown::Undecryptable(_) => {
                self.notify("復号できていないメッセージは編集できません");
                return;
            }
        };
        // 編集は入力欄を本文で置き換えるので、書きかけの下書きがあれば始めない。
        if !matches!(self.target, Target::Edit(_)) && !self.composer.is_empty() {
            self.notify("書きかけの下書きがあります。送信するか消してから編集してください");
            return;
        }
        self.composer = composer_with(&body);
        self.target = Target::Edit(message_id);
        self.mode = Mode::Insert;
    }

    fn confirm_delete(&mut self) {
        let Some((channel_id, message_id)) = self.focused_message() else {
            return;
        };
        if !self.can_change_message(&message_id) {
            return;
        }
        if self
            .store
            .message(&channel_id, &message_id)
            .is_some_and(|message| message.user_id == self.me.id)
        {
            self.mode = Mode::ConfirmDelete { message_id };
        } else {
            self.notify("自分のメッセージだけを削除できます");
        }
    }

    /// サイドバーでは選択中のチャンネル、それ以外では開いているチャンネルを対象にする。
    fn join_or_leave(&mut self, join: bool) {
        let thread_id = match self.focus {
            Focus::Thread => self.thread.as_ref().map(|thread| thread.root_id.clone()),
            Focus::Sidebar => match self.sidebar_items().get(self.sidebar_index) {
                Some(SidebarItem::Thread { id, .. }) => Some(id.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(id) = thread_id {
            let api = self.api.clone();
            let tx = self.app_tx.clone();
            self.spawn_api("スレッドの参加・退出", async move {
                if join {
                    api.join_thread(&id).await?;
                    let _ = tx.send(AppEvent::ThreadsChanged { reopened: None });
                } else {
                    api.leave_thread(&id).await?;
                    let _ = tx.send(AppEvent::ThreadLeft(id));
                }
                Ok(())
            });
            return;
        }
        let channel_id = match self.focus {
            Focus::Sidebar => self
                .sidebar_items()
                .get(self.sidebar_index)
                .and_then(|item| match item {
                    SidebarItem::Channel(id) | SidebarItem::Pinned(id) => Some(id.clone()),
                    SidebarItem::Thread { channel_id, .. } => Some(channel_id.clone()),
                    SidebarItem::MoreThreads { .. } => None,
                }),
            _ => self.current.clone(),
        };
        let Some(channel_id) = channel_id else {
            return;
        };
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        if join {
            self.spawn_api("参加", async move {
                api.join_channel(&channel_id).await?;
                let _ = tx.send(AppEvent::ThreadsChanged { reopened: None });
                Ok(())
            });
        } else {
            self.spawn_api("退出", async move {
                api.leave_channel(&channel_id).await?;
                let _ = tx.send(AppEvent::ThreadsChanged { reopened: None });
                Ok(())
            });
        }
    }

    fn notification_level_label(level: NotificationLevel) -> &'static str {
        match level {
            NotificationLevel::All => "すべての投稿",
            NotificationLevel::Mentions => "メンション",
            NotificationLevel::Mute => "メンション以外を通知しない",
        }
    }

    fn next_notification_level(level: NotificationLevel) -> NotificationLevel {
        match level {
            NotificationLevel::All => NotificationLevel::Mentions,
            NotificationLevel::Mentions => NotificationLevel::Mute,
            NotificationLevel::Mute => NotificationLevel::All,
        }
    }

    /// 開いているチャンネルの通知レベルを all → mentions → mute の順に変える。
    /// 通知設定の Shape が新しい値を届けるので、書き込みのあとに取得はしない。
    /// Shape が古くても選んだ値から次を計算するため、連続操作で同じ値を送らない。
    fn cycle_notification_level(&mut self) {
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        // 自分の設定行が届いていないと store は既定値を返すだけなので、実際の設定から
        // 循環できない。行が届くまで操作を止め、誤った値を送らないようにする。
        if self
            .store
            .notification_setting(&channel_id, &self.me.id)
            .is_none()
        {
            self.notify("通知設定を読み込み中です。少し待ってからもう一度押してください");
            return;
        }
        let previous = self
            .notification_level_selected
            .get(&channel_id)
            .copied()
            .unwrap_or_else(|| self.store.notification_level(&channel_id, &self.me.id));
        let next = Self::next_notification_level(previous);
        // 初期値は Store の値（届いている設定行）。以降は成功した値だけを確定値にする。
        if !self.notification_level_confirmed.contains_key(&channel_id) {
            let initial = self.store.notification_level(&channel_id, &self.me.id);
            self.notification_level_confirmed
                .insert(channel_id.clone(), initial);
        }
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        // 連打しても入力順に PATCH が届くよう、チャンネルごとにロックを取ってから送る。
        // ロックの取得はタスクの中で行い、キー操作を待たせない（Mutex は待ち順が公平）。
        let lock = self
            .notification_level_locks
            .entry(channel_id.clone())
            .or_default()
            .clone();
        let label = format!("通知を {} にしました", Self::notification_level_label(next));
        *self
            .notification_level_in_flight
            .entry(channel_id.clone())
            .or_default() += 1;
        self.notification_level_pending
            .insert(channel_id.clone(), next);
        self.notification_level_selected
            .insert(channel_id.clone(), next);
        self.spawn_api("通知設定の変更", async move {
            let _guard = lock.lock().await;
            let result = api.set_channel_notification_level(&channel_id, next).await;
            let succeeded = result.is_ok();
            let _ = tx.send(AppEvent::NotificationLevelFinished {
                channel_id: channel_id.clone(),
                level: next,
                succeeded,
            });
            result?;
            let _ = tx.send(AppEvent::Notice(label));
            Ok(())
        });
    }

    /// チャンネルの暗号化の扱い（docs/e2ee.md の「暗号化の格下げを防ぐ」）。一度 `encrypted = true` を
    /// 見た（暗号ストアに記録した）、名前が `edm:`、暗号文のある行がある、のどれかで `encrypted = false`
    /// なら格下げとして送信を止める。
    pub fn channel_encryption(&self, channel_id: &str) -> ChannelEncryption {
        let has_ciphertext = self
            .store
            .visible_messages(channel_id)
            .any(|message| message.ciphertext.is_some());
        ChannelEncryption::of(
            self.store.channel(channel_id),
            self.known_encrypted_channels.contains(channel_id),
            has_ciphertext,
        )
    }

    /// `encrypted = true` を見たチャンネルを、暗号ストアを待たずにその場で記録する（暗号ストアに記録する
    /// 前に終了しても、起動し直したあとに格下げを見つけられるように）。書けなくても、暗号ストアへの記録は
    /// 続ける。
    fn record_encrypted(&self, channel_id: &str) {
        if let Some(root) = &self.crypto_root {
            let _ = crate::e2ee::store::record_encrypted_channels(
                root,
                &self.me.id,
                &[channel_id.to_owned()],
            );
        }
    }

    /// `encrypted = true` を見たチャンネルを暗号ストアに記録し、記録済みのものを読む（開き直したあとも
    /// 格下げを見つけられるように）。暗号ストアが開いていなければ、次の機会にする。
    fn sync_known_encrypted(&mut self) {
        if self.known_encrypted_in_flight {
            return;
        }
        let Some((_, store)) = self.encryption.store() else {
            return;
        };
        // 暗号ストアを開き直した（作り直した）ら、読み直して、今見えている暗号化チャンネルを記録し直す。
        let identity = std::sync::Arc::as_ptr(&store) as usize;
        if self.known_encrypted_store != Some(identity) {
            self.known_encrypted_store = Some(identity);
            self.known_encrypted_loaded = false;
            let visible: Vec<String> = self
                .store
                .channels()
                .into_iter()
                .filter(|channel| channel.encrypted)
                .map(|channel| channel.id.clone())
                .collect();
            self.unrecorded_encrypted.extend(visible);
        }
        if self.known_encrypted_loaded && self.unrecorded_encrypted.is_empty() {
            return;
        }
        let ids: Vec<String> = self.unrecorded_encrypted.drain().collect();
        self.known_encrypted_in_flight = true;
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            let mut store = store.lock().await;
            // 保存できなかったものは、次の機会に記録し直す（記録がないまま起動し直すと、格下げを見つけられない）。
            let failed = ids
                .into_iter()
                .filter(|id| store.remember_encrypted_channel(id).is_err())
                .collect();
            let known = store.known_encrypted_channels().cloned().collect();
            drop(store);
            let _ = tx.send(AppEvent::KnownEncryptedChannels { known, failed });
        });
    }

    /// 開いているチャンネルの暗号化したメッセージを、暗号ストアで復号する（#361）。行が届いたときと、
    /// 鍵を待っているメッセージがあって受信用セッションが増えたときに、1 つずつ行う。
    fn request_decryptions(&mut self) {
        if self.decryptions.in_flight {
            return;
        }
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        let Some((_, store)) = self.encryption.store() else {
            return;
        };
        // 受信用セッションが増えたら（Olm で鍵が届いた、鍵バックアップから取り込んだ）、鍵待ちの行と、
        // 鍵バックアップ由来で復号した行を復号し直す。増えたかはチャンネルごとの数えで決める（別の
        // チャンネルを開いているあいだに鍵が届いても、戻ったときに復号し直す）。
        let generation = store
            .try_lock()
            .ok()
            .map(|store| store.inbound_generation());
        let retry = generation.is_some_and(|generation| {
            self.decryptions.generations.get(&channel_id) != Some(&generation)
        });
        if !self.decryptions.dirty && !retry {
            return;
        }
        // 復号するときは、鍵待ちの行も必ず含める。タスクはロックを取ったあとの数えを記録するので、
        // 鍵待ちの行を試さずに数えだけを進めると、鍵が届いていても復号し直さなくなるため（ロックを取れずに
        // 数えが増えたかを確かめられなかったときも同じ）。鍵待ちの行の復号は手元だけで済む。
        let rows: Vec<(RowKey, EncryptedRow)> = self
            .store
            .visible_messages(&channel_id)
            .filter(|message| self.decryptions.needs(message, true))
            .filter_map(|message| {
                let row = EncryptedRow {
                    channel_id: message.channel_id.clone(),
                    message_id: message.id.clone(),
                    parent_id: message.parent_id.clone(),
                    user_id: message.user_id.clone(),
                    message_kind: message
                        .message_kind
                        .clone()
                        .unwrap_or_else(|| "message".into()),
                    epoch: message.epoch,
                    revision: message.revision,
                    ciphertext: message.ciphertext.clone()?,
                };
                Some((RowKey::of(message), row))
            })
            .collect();
        self.decryptions.dirty = false;
        if rows.is_empty() {
            // 復号し直す行がなければ、数えだけを記録する（tick ごとに確かめ直さないため）。
            if let Some(generation) = generation {
                self.decryptions.generations.insert(channel_id, generation);
            }
            return;
        }
        self.decryptions.in_flight = true;
        let tx = self.app_tx.clone();
        tokio::spawn(async move {
            let mut store = store.lock().await;
            let generation = store.inbound_generation();
            let results = rows
                .into_iter()
                .map(|(key, row)| {
                    let decryption = store.decrypt_message(&row);
                    (row.message_id, key, decryption)
                })
                .collect();
            drop(store);
            let _ = tx.send(AppEvent::Decrypted {
                channel_id,
                results,
                generation: Some(generation),
            });
        });
    }

    fn submit(&mut self) {
        let encryption = self
            .current
            .as_deref()
            .map(|id| self.channel_encryption(id))
            .unwrap_or(ChannelEncryption::Plain);
        // サーバーが暗号化チャンネルを平文と返しても、平文では送らない（下書きは残す）。
        if encryption == ChannelEncryption::Downgraded {
            self.notify(e2ee_chat::DOWNGRADED);
            return;
        }
        // Channels の再同期でチャンネルの行がないあいだは、暗号化か分からないので送らない（下書きは残す）。
        if self
            .current
            .as_deref()
            .is_some_and(|id| self.store.channel(id).is_none())
        {
            self.notify("チャンネルの情報を同期しています。少し待ってから送ってください");
            return;
        }
        if encryption == ChannelEncryption::Encrypted {
            self.submit_encrypted();
            return;
        }
        // 暗号化チャンネルの記録を読めなかったあいだは、暗号ストアから読めるまで平文で送らない。
        if self.known_encrypted_unreadable && !self.known_encrypted_loaded {
            self.notify(
                "暗号化チャンネルの記録を読めないため、確かめられるまで送信を止めています。少し待ってから送ってください",
            );
            return;
        }
        if let Target::Edit(message_id) = self.target.clone()
            && !self.can_change_message(&message_id)
        {
            return;
        }
        let content = self.composer.lines().join("\n");
        if content.trim().is_empty() {
            return;
        }
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        let api = self.api.clone();
        let tx = self.app_tx.clone();
        let target = self.target.clone();
        self.composer = new_composer();
        self.stop_typing();
        // 編集は1回で終わる。新規投稿と返信は続けて書けるよう入力モードのままにする。
        if matches!(target, Target::Edit(_)) {
            self.target = Target::Channel;
            self.mode = Mode::Normal;
        }
        tokio::spawn(async move {
            let id = uuid::Uuid::new_v4().to_string();
            let mut resolved_parent: Option<String> = None;
            let result = match &target {
                Target::Channel => api.send_message(&id, &channel_id, &content, None).await,
                Target::Reply(root_id) => api
                    .send_reply(&id, &channel_id, &content, root_id)
                    .await
                    .map(|(txid, parent)| {
                        resolved_parent = parent.filter(|parent| parent != root_id);
                        txid
                    }),
                Target::Edit(message_id) => api.edit_message(message_id, &content).await,
            };
            if let Err(err) = result {
                let _ = tx.send(AppEvent::Notice(send_failure_notice(&target, &err)));
                let _ = tx.send(AppEvent::RestoreDraft {
                    channel_id,
                    target,
                    content,
                    edit_base: None,
                });
            } else {
                let reopened = match target {
                    Target::Reply(root_id) => {
                        if let Some(parent) = resolved_parent {
                            let _ = tx.send(AppEvent::ThreadRedirected {
                                from: root_id,
                                to: parent.clone(),
                            });
                            Some(parent)
                        } else {
                            Some(root_id)
                        }
                    }
                    _ => None,
                };
                let _ = tx.send(AppEvent::ThreadsChanged { reopened });
            }
        });
    }

    /// 暗号化チャンネルに送る（docs/e2ee.md の「membership_epoch と送信の順序」。#361）。本文は送る直前に
    /// 暗号化し、平文はサーバーに送らない。送れなければ、平文の下書きとして入力欄に戻す。
    fn submit_encrypted(&mut self) {
        if let Target::Edit(message_id) = self.target.clone()
            && !self.can_change_message(&message_id)
        {
            return;
        }
        let content = self.composer.lines().join("\n");
        if content.trim().is_empty() {
            return;
        }
        let Some(channel_id) = self.current.clone() else {
            return;
        };
        // 編集は、始めたときの版を基準にする。そのあとに版が進んでいたら（別の端末で編集された）、
        // 下書きを残したまま知らせ、送らない。
        if let Target::Edit(message_id) = &self.target {
            let current = self
                .store
                .message(&channel_id, message_id)
                .and_then(|message| self.decryptions.editable(message))
                .map(|(_, revision)| revision);
            let base = self
                .edit_base
                .as_ref()
                .filter(|(id, _)| id == message_id)
                .map(|(_, revision)| *revision);
            match (base, current) {
                (Some(base), Some(current)) if base == current => {}
                (None, _) => {
                    self.notify("編集を始めた版が分かりません。e で編集し直してください");
                    return;
                }
                (Some(_), None) => {
                    self.notify("復号できていないメッセージは編集できません");
                    return;
                }
                (Some(_), Some(_)) => {
                    self.notify(
                        "このメッセージはほかの場所で編集されました。読み込み直してから編集してください",
                    );
                    return;
                }
            }
        }
        let Some((_, store)) = self.encryption.store() else {
            self.notify(
                "この端末では、まだ暗号化の準備ができていません。少し待ってから送ってください",
            );
            return;
        };
        let shape_epoch = self
            .store
            .channel(&channel_id)
            .and_then(|channel| channel.membership_epoch);
        let mentions = crate::mentions::encrypted_mentions(
            &content,
            self.store
                .channel_members(&channel_id)
                .into_iter()
                .map(|user| (user.id.as_str(), user.user_name.as_str())),
        );
        let target = self.target.clone();
        let edit_base = match &target {
            Target::Edit(_) => self.edit_base.as_ref().map(|(_, revision)| *revision),
            _ => None,
        };
        // 編集は、復号できた最新の版を基準にする（行が先に進んでいれば 409 になる）。
        let edit = match &target {
            Target::Edit(message_id) => {
                let Some(message) = self.store.message(&channel_id, message_id) else {
                    return;
                };
                let Some((decrypted, row_revision)) = self.decryptions.editable(message) else {
                    self.notify("復号できていないメッセージは編集できません");
                    return;
                };
                Some(send::Edit {
                    channel_id: channel_id.clone(),
                    shape_epoch,
                    message_id: message_id.clone(),
                    parent_id: message.parent_id.clone(),
                    revision: row_revision + 1,
                    body: content.clone(),
                    kind: decrypted.kind.clone(),
                    mentions: mentions.clone(),
                    attachments: decrypted.attachments.clone(),
                })
            }
            _ => None,
        };
        let ctx = SendContext {
            api: self.api.clone(),
            store,
            sender: self.channel_sender.clone(),
        };
        let tx = self.app_tx.clone();
        self.composer = new_composer();
        self.stop_typing();
        if matches!(target, Target::Edit(_)) {
            self.target = Target::Channel;
            self.mode = Mode::Normal;
        }
        tokio::spawn(async move {
            let mut recipients = None;
            let result = match &edit {
                Some(edit) => send::edit_encrypted_message(&ctx, edit, &mut recipients).await,
                None => {
                    let message = send::NewMessage {
                        channel_id: channel_id.clone(),
                        shape_epoch,
                        message_id: uuid::Uuid::new_v4().to_string(),
                        parent_id: match &target {
                            Target::Reply(root_id) => Some(root_id.clone()),
                            _ => None,
                        },
                        body: content.clone(),
                        mentions,
                    };
                    send::send_encrypted_message(&ctx, &message, &mut recipients).await
                }
            };
            // 暗号ストアを手放してから知らせる（端末の管理のタスクが開き直せるように）。
            drop(ctx);
            if let Some(summary) = recipients {
                let _ = tx.send(AppEvent::Recipients {
                    channel_id: channel_id.clone(),
                    summary,
                });
            }
            match result {
                Ok(()) => {
                    let reopened = match target {
                        Target::Reply(root_id) => Some(root_id),
                        _ => None,
                    };
                    let _ = tx.send(AppEvent::ThreadsChanged { reopened });
                }
                Err(err) => {
                    if let (Some(current_revision), Target::Edit(message_id)) =
                        (stale_revision(&err), &target)
                    {
                        let _ = tx.send(AppEvent::EditConflict {
                            channel_id,
                            message_id: message_id.clone(),
                            current_revision,
                            content,
                            edit_base,
                        });
                        return;
                    }
                    let _ = tx.send(AppEvent::Notice(send_failure_notice(&target, &err)));
                    let _ = tx.send(AppEvent::RestoreDraft {
                        channel_id,
                        target,
                        content,
                        edit_base,
                    });
                }
            }
        });
    }
}

/// 送信に失敗したときの通知。参加行があるスレッドへ未参加・退出済みのまま返信すると 403 になるので、
/// 参加（`J`）すれば送れることを伝える。投稿権限がないなど、ほかの失敗は原因をそのまま出す。
fn send_failure_notice(target: &Target, err: &anyhow::Error) -> String {
    if matches!(target, Target::Reply(_)) && zoo2_client_core::http::is_thread_join_required(err) {
        "このスレッドに参加していません。Esc で入力を終え、J で参加してから返信してください（アーカイブ済みなら、先に u で再Openします。下書きは残しています）"
            .to_string()
    } else {
        format!("送信に失敗しました: {err}")
    }
}

/// `thumbsup` や `:thumbsup:` のショートコード、または絵文字そのものを絵文字にする。
fn resolve_emoji(input: &str) -> Option<String> {
    zoo2_client_core::mfm::resolve_emoji(input)
}

fn new_composer() -> TextArea<'static> {
    let mut composer = TextArea::default();
    composer.set_cursor_line_style(ratatui::style::Style::new());
    composer
}

fn composer_with(text: &str) -> TextArea<'static> {
    let mut composer = TextArea::new(text.split('\n').map(str::to_string).collect());
    composer.set_cursor_line_style(ratatui::style::Style::new());
    composer.move_cursor(tui_textarea::CursorMove::Bottom);
    composer.move_cursor(tui_textarea::CursorMove::End);
    composer
}

/// 添字を `delta` だけ動かす。`isize::MIN`/`MAX` は先頭と末尾。
fn step(current: Option<usize>, delta: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let last = len - 1;
    Some(match (current, delta) {
        (_, isize::MIN) => 0,
        (_, isize::MAX) => last,
        // 未選択から動かすときは、最新 (末尾) から始める。
        (None, _) => last,
        (Some(index), delta) => index.saturating_add_signed(delta).min(last),
    })
}

fn move_id(ids: &[String], current: Option<&str>, delta: isize) -> Option<String> {
    let index = current.and_then(|id| ids.iter().position(|candidate| candidate == id));
    step(index, delta, ids.len()).map(|index| ids[index].clone())
}

/// 競合したあとに送り直す並び。ほかの端末が追加したピンは最新の並びのまま残し、
/// 動かした項目だけを、意図した並びで隣にある既存の項目を目印に入れ直す
/// （末尾へ動かしたときは直前の項目の後ろ）。最新の添字をそのまま使うと、
/// 他端末が前へピンを足したぶんだけ位置がずれる。
fn merge_pinned_order(intended: &[String], moved: &str, latest_active: &[String]) -> Vec<String> {
    let mut base: Vec<String> = latest_active
        .iter()
        .filter(|id| id.as_str() != moved)
        .cloned()
        .collect();
    let Some(moved_index) = intended.iter().position(|id| id == moved) else {
        return base;
    };
    // 動かした項目より後ろで、最新にも残っている最初の要素の前に置く。
    for id in intended.iter().skip(moved_index + 1) {
        if let Some(anchor) = base.iter().position(|candidate| candidate == id) {
            base.insert(anchor, moved.to_owned());
            return base;
        }
    }
    // 末尾へ動かした場合は、直前の既存要素の後ろに置く。
    for id in intended.iter().take(moved_index).rev() {
        if let Some(anchor) = base.iter().position(|candidate| candidate == id) {
            base.insert(anchor + 1, moved.to_owned());
            return base;
        }
    }
    base.insert(
        if moved_index == 0 { 0 } else { base.len() },
        moved.to_owned(),
    );
    base
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reply_to_a_thread_the_user_has_not_joined_points_to_join() {
        let join_required: anyhow::Error = zoo2_client_core::http::HttpError::Status {
            status: reqwest::StatusCode::FORBIDDEN,
            message: zoo2_client_core::http::THREAD_JOIN_REQUIRED_MESSAGE.into(),
            code: None,
        }
        .into();
        let notice = send_failure_notice(&Target::Reply("root".into()), &join_required);
        assert!(notice.contains("J で参加"), "{notice}");
        // 暗号化チャンネルの API は ApiError でエラーを返す。
        let coded: anyhow::Error = zoo2_client_core::http::ApiError {
            status: reqwest::StatusCode::FORBIDDEN,
            code: None,
            message: zoo2_client_core::http::THREAD_JOIN_REQUIRED_MESSAGE.into(),
            body: serde_json::Value::Null,
        }
        .into();
        assert!(send_failure_notice(&Target::Reply("root".into()), &coded).contains("J で参加"));
        // 返信以外では参加しても直らないので、原因をそのまま出す。
        let notice = send_failure_notice(&Target::Channel, &join_required);
        assert!(notice.starts_with("送信に失敗しました"), "{notice}");
        let forbidden: anyhow::Error = zoo2_client_core::http::HttpError::Status {
            status: reqwest::StatusCode::FORBIDDEN,
            message: "Only channel managers may post here".into(),
            code: None,
        }
        .into();
        let notice = send_failure_notice(&Target::Reply("root".into()), &forbidden);
        assert!(notice.contains("Only channel managers"), "{notice}");
    }

    #[test]
    fn notification_level_cycles_and_has_labels() {
        let level = NotificationLevel::default();
        assert_eq!(level, NotificationLevel::Mentions);
        assert_eq!(App::next_notification_level(level), NotificationLevel::Mute);
        assert_eq!(
            App::next_notification_level(NotificationLevel::Mute),
            NotificationLevel::All
        );
        assert_eq!(
            App::next_notification_level(NotificationLevel::All),
            NotificationLevel::Mentions
        );
        assert_eq!(
            App::notification_level_label(NotificationLevel::Mute),
            "メンション以外を通知しない"
        );
    }

    fn api_test_app(base: &str) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
        let http = reqwest::Client::new();
        let credentials = Credentials::Dev("u1".into());
        let api = ApiClient::new(http.clone(), base, credentials.clone());
        let (shape_tx, _shape_rx) = mpsc::unbounded_channel();
        let (app_tx, app_rx) = mpsc::unbounded_channel();
        let app = App::new(
            api,
            http,
            base.into(),
            credentials,
            Me {
                id: "u1".into(),
                user_name: "alice".into(),
                display_name: "Alice".into(),
            },
            shape_tx,
            app_tx,
        );
        (app, app_rx)
    }

    fn set_notification_level(app: &mut App, channel_id: &str, level: NotificationLevel) {
        set_notification_level_for(app, channel_id, "u1", level);
    }

    fn set_notification_level_for(
        app: &mut App,
        channel_id: &str,
        user_id: &str,
        level: NotificationLevel,
    ) {
        app.on_shape(ShapeEvent::Batch(
            ShapeId::ChannelNotificationSettings,
            Batch {
                reset: false,
                changes: vec![electric::Change::Upsert {
                    key: format!("{channel_id}:{user_id}"),
                    row: serde_json::json!({
                        "user_id": user_id,
                        "channel_id": channel_id,
                        "notification_level": level.as_str(),
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                }],
                up_to_date: true,
            },
        ));
    }

    /// 今までに届いた PATCH の本文に書かれた level を、送られた順に返す。
    async fn received_notification_level_requests(
        server: &wiremock::MockServer,
        channel_id: &str,
    ) -> Vec<String> {
        let path = format!("/api/channels/{channel_id}/notification-settings");
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| {
                request.method == wiremock::http::Method::PATCH
                    && request.url.path() == path.as_str()
            })
            .map(|request| {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                body["level"].as_str().unwrap().to_owned()
            })
            .collect()
    }

    /// PATCH の本文に書かれた level を、送られた順に返す。
    async fn notification_level_requests(
        server: &wiremock::MockServer,
        channel_id: &str,
        count: usize,
    ) -> Vec<String> {
        for _ in 0..100 {
            let levels = received_notification_level_requests(server, channel_id).await;
            if levels.len() >= count {
                return levels;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("{channel_id} への PATCH が {count} 件届かなかった");
    }

    #[tokio::test]
    async fn notification_level_cycles_without_waiting_for_shape() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let server = MockServer::start().await;
        // Shape は届かない。それでも選んだ値から次の値を計算できることを確かめる。
        for level in ["mentions", "mute", "all"] {
            Mock::given(method("PATCH"))
                .and(path("/api/channels/c1/notification-settings"))
                .and(body_json(serde_json::json!({ "level": level })))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": { "level": level },
                    "txid": "7",
                })))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
        }
        let (mut app, _rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        assert_eq!(
            app.notification_level_pending.get("c1"),
            Some(&NotificationLevel::Mentions)
        );
        app.cycle_notification_level();
        app.cycle_notification_level();
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::All)
        );
        assert_eq!(
            notification_level_requests(&server, "c1", 3).await,
            ["mentions", "mute", "all"]
        );
    }

    #[tokio::test]
    async fn notification_level_burst_is_serialized_in_input_order() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let server = MockServer::start().await;
        // 最初の PATCH の応答を遅らせる。直列化されていれば、そのあいだに次の PATCH は届かない。
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .and(body_json(serde_json::json!({ "level": "mentions" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_json(serde_json::json!({
                        "data": { "level": "mentions" },
                        "txid": "7",
                    })),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        for level in ["mute", "all"] {
            Mock::given(method("PATCH"))
                .and(path("/api/channels/c1/notification-settings"))
                .and(body_json(serde_json::json!({ "level": level })))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": { "level": level },
                    "txid": "7",
                })))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
        }
        let (mut app, _rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 1).await,
            ["mentions"]
        );
        // Shape を待たずに続けて選ぶ。
        app.cycle_notification_level();
        app.cycle_notification_level();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            received_notification_level_requests(&server, "c1").await,
            ["mentions"]
        );
        assert_eq!(
            notification_level_requests(&server, "c1", 3).await,
            ["mentions", "mute", "all"]
        );
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::All)
        );
    }

    #[tokio::test]
    async fn notification_level_other_channels_are_not_blocked() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .and(body_json(serde_json::json!({ "level": "mentions" })))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_json(serde_json::json!({
                        "data": { "level": "mentions" },
                        "txid": "7",
                    })),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c2/notification-settings"))
            .and(body_json(serde_json::json!({ "level": "mentions" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "level": "mentions" },
                "txid": "7",
            })))
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        set_notification_level(&mut app, "c2", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 1).await,
            ["mentions"]
        );
        // c1 の応答を待つあいだも、別チャンネルの操作は進む。
        app.current = Some("c2".into());
        app.cycle_notification_level();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            received_notification_level_requests(&server, "c2").await,
            ["mentions"]
        );
        assert_eq!(
            notification_level_requests(&server, "c1", 1).await,
            ["mentions"]
        );
    }

    #[tokio::test]
    async fn notification_level_operations_are_isolated_per_channel() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "level": "mentions" },
                "txid": "7",
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c2/notification-settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "level": "mentions" },
                "txid": "7",
            })))
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        set_notification_level(&mut app, "c2", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        app.current = Some("c2".into());
        app.cycle_notification_level();
        app.current = Some("c1".into());
        app.cycle_notification_level();
        // c2 の操作で c1 の選んだ値が消えていない。
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::Mute)
        );
        assert_eq!(
            app.notification_level_selected.get("c2"),
            Some(&NotificationLevel::Mentions)
        );
        assert_eq!(
            notification_level_requests(&server, "c1", 2).await,
            ["mentions", "mute"]
        );
        assert_eq!(
            notification_level_requests(&server, "c2", 1).await,
            ["mentions"]
        );
    }

    #[tokio::test]
    async fn notification_level_failure_reverts_the_selected_value() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .respond_with(
                ResponseTemplate::new(500).set_body_json(serde_json::json!({ "error": "NG" })),
            )
            .up_to_n_times(1)
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "level": "mentions" },
                "txid": "7",
            })))
            .mount(&server)
            .await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        assert!(app.notification_level_pending.is_empty());
        // 確定値 All まで戻る。Store と一致した時点で選択値は消えても、基準は All のまま。
        assert_eq!(
            app.notification_level_selected
                .get("c1")
                .copied()
                .unwrap_or_else(|| app.store.notification_level("c1", "u1")),
            NotificationLevel::All
        );
        assert_eq!(
            app.notification_level_confirmed.get("c1"),
            Some(&NotificationLevel::All)
        );
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|(message, _)| message.contains("通知設定の変更に失敗しました"))
        );
        // 失敗してもう一度選べる。store の値に戻っているので同じ値を送る。
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 2).await,
            ["mentions", "mentions"]
        );
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        assert!(app.notification_level_pending.is_empty());
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|(message, _)| message.contains("通知を メンション にしました"))
        );
    }

    #[tokio::test]
    async fn notification_level_override_is_cleared_when_shape_catches_up() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "level": "mentions" },
                "txid": "7",
            })))
            .mount(&server)
            .await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::Mentions)
        );
        // PATCH が残っているあいだは、Store が一致しても古い Shape で上書きを消さない。
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::Mentions)
        );
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        // すべての PATCH が終わり、選んだ値が届いたら store を正本に戻す。
        set_notification_level(&mut app, "c1", NotificationLevel::Mentions);
        assert!(!app.notification_level_selected.contains_key("c1"));
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 2).await,
            ["mentions", "mute"]
        );
    }

    #[tokio::test]
    async fn notification_level_stale_shape_keeps_selection_until_patches_finish() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{body_json, method, path},
        };
        let server = MockServer::start().await;
        for level in ["mentions", "mute", "all"] {
            Mock::given(method("PATCH"))
                .and(path("/api/channels/c1/notification-settings"))
                .and(body_json(serde_json::json!({ "level": level })))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "data": { "level": level },
                    "txid": "7",
                })))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
        }
        let (mut app, mut rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        app.current = Some("c1".into());
        app.cycle_notification_level();
        app.cycle_notification_level();
        app.cycle_notification_level();
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::All)
        );
        // 連打で All に戻った直後、Store がまだ All のときに古い Shape が届いても解除しない。
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::All)
        );
        // PATCH の反映で Store が Mentions になっても、選択値は All のまま。
        set_notification_level(&mut app, "c1", NotificationLevel::Mentions);
        assert_eq!(
            app.notification_level_selected.get("c1"),
            Some(&NotificationLevel::All)
        );
        for _ in 0..6 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        assert!(app.notification_level_in_flight.is_empty());
        // すべての PATCH が終わり、Store が All に追いついたので選択値を解除する。
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        assert!(!app.notification_level_selected.contains_key("c1"));
        // 4 回目は Mentions を送る（Mentions 基準だと Mute になってしまう）。
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 4).await,
            ["mentions", "mute", "all", "mentions"]
        );
    }

    #[tokio::test]
    async fn notification_level_consecutive_failures_keep_the_confirmed_value() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .respond_with(
                ResponseTemplate::new(500).set_body_json(serde_json::json!({ "error": "NG" })),
            )
            .mount(&server)
            .await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        set_notification_level(&mut app, "c1", NotificationLevel::All);
        app.current = Some("c1".into());
        // All を確定値にしたまま、Mentions と Mute を続けて要求して両方失敗させる。
        app.cycle_notification_level();
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 2).await,
            ["mentions", "mute"]
        );
        for _ in 0..4 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        assert!(app.notification_level_pending.is_empty());
        assert!(app.notification_level_in_flight.is_empty());
        assert_eq!(
            app.notification_level_confirmed.get("c1"),
            Some(&NotificationLevel::All)
        );
        assert_eq!(
            app.notification_level_selected
                .get("c1")
                .copied()
                .unwrap_or_else(|| app.store.notification_level("c1", "u1")),
            NotificationLevel::All
        );
        // 連続失敗のあとの操作は、実際に残っている All から Mentions を送る。
        app.cycle_notification_level();
        assert_eq!(
            notification_level_requests(&server, "c1", 3).await,
            ["mentions", "mute", "mentions"]
        );
    }

    #[tokio::test]
    async fn notification_level_waits_for_the_users_own_setting_row() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channels/c1/notification-settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "level": "mentions" },
                "txid": "7",
            })))
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        app.current = Some("c1".into());
        // 設定行が届いていないと既定値から計算してしまうので、何も送らない。
        app.cycle_notification_level();
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|(message, _)| message.contains("通知設定を読み込み中"))
        );
        assert!(app.notification_level_pending.is_empty());
        // 他人の設定行だけではまだ始めない。
        set_notification_level_for(&mut app, "c1", "u2", NotificationLevel::All);
        app.cycle_notification_level();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            received_notification_level_requests(&server, "c1")
                .await
                .is_empty()
        );
        // 自分の行が届いたら、実際の Mute から All へ循環する。
        // 行がないと既定の Mentions から Mute を送ってしまう。
        set_notification_level(&mut app, "c1", NotificationLevel::Mute);
        app.cycle_notification_level();
        assert_eq!(notification_level_requests(&server, "c1", 1).await, ["all"]);
    }

    #[tokio::test]
    async fn thread_refreshes_are_serial_and_bursts_collapse_to_one_followup() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/threads"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(50))
                    .set_body_json(serde_json::json!({"items":[],"next_cursor":null})),
            )
            .expect(2)
            .mount(&server)
            .await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        app.refresh_threads();
        for _ in 0..10 {
            app.refresh_threads();
        }
        assert!(app.thread_in_flight[0]);
        assert_eq!(app.thread_pending[0], Some(false));
        for _ in 0..4 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        assert!(!app.thread_in_flight[0]);
        assert!(app.thread_pending[0].is_none());
        app.terminal_active = false;
        app.refresh_threads();
        assert!(!app.thread_in_flight[0]);
        assert_eq!(app.thread_pending[0], Some(false));
    }

    #[tokio::test]
    async fn membership_changes_refresh_the_thread_index() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        for join in [true, false] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path(if join {
                    "/api/channels/c1/join"
                } else {
                    "/api/channels/c1/leave"
                }))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"data":{},"txid":"42"})),
                )
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/api/threads"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"items":[],"next_cursor":null})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let (mut app, mut rx) = api_test_app(&server.uri());
            app.focus = Focus::Messages;
            app.current = Some("c1".into());
            app.join_or_leave(join);
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(
                &event,
                AppEvent::ThreadsChanged { reopened: None }
            ));
            app.on_app_event(event);
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(&event, AppEvent::Threads { .. }));
            app.on_app_event(event);
            assert!(app.notice.is_none());
        }
    }

    #[tokio::test]
    async fn committed_reopen_stays_successful_when_the_index_is_unavailable() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/threads/root/unarchive"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":{},"txid":"42"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/threads"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        // 履歴一覧を取得しなくても、購読中の親投稿から再開結果を確定する。
        for (shape, key, row) in [
            (
                ShapeId::Channels,
                "c1",
                serde_json::json!({
                    "id":"c1", "channel_name":"general", "is_private":"f"
                }),
            ),
            (
                ShapeId::ChannelUsers,
                "cu1",
                serde_json::json!({"id":"cu1", "channel_id":"c1", "user_id":"u1"}),
            ),
            (
                ShapeId::Messages("c1".into()),
                "root",
                serde_json::json!({
                    "id":"root", "channel_id":"c1", "user_id":"u1", "content":"Thread",
                    "created_at":"2026-11-01 00:00:00", "thread_archive_at":"2026-11-10 00:00:00"
                }),
            ),
        ] {
            app.on_shape(ShapeEvent::Batch(
                shape,
                Batch {
                    changes: vec![electric::Change::Upsert {
                        key: key.into(),
                        row: row.as_object().unwrap().clone(),
                    }],
                    ..Default::default()
                },
            ));
        }
        app.current = Some("c1".into());
        assert!(app.active_threads.is_empty());
        assert!(app.archived_threads.is_empty());
        app.focus = Focus::Thread;
        app.thread = Some(ThreadView {
            root_id: "root".into(),
            selected: None,
            scroll: 0,
            inner_scroll: 0,
        });
        app.reopen_thread();
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(&event, AppEvent::ThreadsChanged { reopened: Some(id) } if id == "root"));
        app.on_app_event(event);
        assert!(app.active_threads.is_empty());
        app.close_thread();
        assert!(!app.sidebar_items().contains(&SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "root".into(),
            pinned: false,
        }));
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(&event, AppEvent::ThreadsFailed { .. }));
        app.on_app_event(event);
        assert!(app.active_threads.is_empty());
        assert!(
            app.notice
                .as_ref()
                .unwrap()
                .0
                .starts_with("スレッド一覧を取得できませんでした")
        );
    }

    #[tokio::test]
    async fn deleting_a_message_refreshes_the_thread_index() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/messages/root"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"data":{},"txid":"42"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/threads"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"items":[],"next_cursor":null})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        app.current = Some("c1".into());
        app.active_threads.push(ThreadSummary {
            id: "root".into(),
            channel_id: "c1".into(),
            content: "Thread".into(),
            thread_archive_at: None,
            reply_count: 1,
        });
        app.mode = Mode::ConfirmDelete {
            message_id: "root".into(),
        };
        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
                .await
                .unwrap()
                .unwrap();
            app.on_app_event(event);
        }
        assert!(app.active_threads.is_empty());
        assert!(app.notice.is_none());
    }

    #[tokio::test]
    async fn pending_changes_do_not_reach_the_api_after_a_thread_is_archived() {
        let server = wiremock::MockServer::start().await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        app.current = Some("c1".into());
        app.archived_threads.push(ThreadSummary {
            id: "root".into(),
            channel_id: "c1".into(),
            content: "Archived".into(),
            thread_archive_at: Some("2026-11-10".into()),
            reply_count: 1,
        });
        app.mode = Mode::ConfirmDelete {
            message_id: "root".into(),
        };
        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('y'),
            KeyModifiers::NONE,
        )));
        assert_eq!(app.mode, Mode::Normal);
        app.mode = Mode::React {
            message_id: "root".into(),
            input: "thumbsup".into(),
        };
        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.mode, Mode::Normal);
        app.mode = Mode::Insert;
        app.target = Target::Edit("root".into());
        app.composer = composer_with("Edited reply");
        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(app.composer.lines().join("\n"), "Edited reply");
        assert!(
            tokio::time::timeout(Duration::from_millis(200), rx.recv())
                .await
                .is_err()
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == "GET")
        );
    }

    #[tokio::test]
    async fn encrypted_channels_do_not_send_plaintext() {
        let server = wiremock::MockServer::start().await;
        let (mut app, _rx) = api_test_app(&server.uri());
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            Batch {
                changes: vec![electric::Change::Upsert {
                    key: "c1".into(),
                    row: serde_json::json!({
                        "id":"c1", "channel_name":"secret", "is_private":"t", "encrypted":"t"
                    })
                    .as_object()
                    .unwrap()
                    .clone(),
                }],
                ..Default::default()
            },
        ));
        app.current = Some("c1".into());
        for target in [
            Target::Channel,
            Target::Reply("root".into()),
            Target::Edit("root".into()),
        ] {
            app.mode = Mode::Insert;
            app.target = target;
            app.composer = composer_with("secret text");
            app.submit();
            // 下書きは消さずに残す。
            assert_eq!(app.composer.lines().join("\n"), "secret text");
        }
        // Channels の再同期で行が消えても、暗号化として扱い続ける。
        app.on_shape(ShapeEvent::Batch(
            ShapeId::Channels,
            Batch {
                reset: true,
                ..Default::default()
            },
        ));
        assert!(app.store.channel("c1").is_none());
        for target in [
            Target::Channel,
            Target::Reply("root".into()),
            Target::Edit("root".into()),
        ] {
            app.mode = Mode::Insert;
            app.target = target;
            app.composer = composer_with("secret after reset");
            app.submit();
            assert_eq!(app.composer.lines().join("\n"), "secret after reset");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == "GET")
        );
        assert!(app.notice.is_some());
    }

    #[tokio::test]
    async fn channels_without_a_row_do_not_send_until_synced() {
        let server = wiremock::MockServer::start().await;
        let (mut app, _rx) = api_test_app(&server.uri());
        // 暗号化か分からないチャンネル（再同期中で行がない）には送らない。
        app.current = Some("c9".into());
        app.mode = Mode::Insert;
        app.target = Target::Channel;
        app.composer = composer_with("unknown channel");
        app.submit();
        assert_eq!(app.composer.lines().join("\n"), "unknown channel");
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| request.method == "GET")
        );
    }

    /// 鍵待ちの行があるときに、端末の管理のタスクが暗号ストアのロックを持ったまま鍵を取り込み、
    /// そのあいだに別の行が届いても、鍵待ちの行を復号し直す（数えだけを進めない）。
    #[tokio::test]
    async fn rows_waiting_for_keys_are_retried_even_when_the_lock_was_busy() {
        use crate::e2ee::store::{CryptoStore, ToDeviceRow, UndecryptableReason};
        const ME: &str = "00000000-0000-4000-8000-0000000000b0";
        const SENDER: &str = "00000000-0000-4000-8000-0000000000a0";
        const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c0";
        let dir = tempfile::tempdir().unwrap();
        let mut opened = CryptoStore::open(dir.path(), ME).unwrap().store;
        opened.mark_registered().unwrap();
        let store = std::sync::Arc::new(tokio::sync::Mutex::new(opened));
        let server = wiremock::MockServer::start().await;
        let (mut app, mut rx) = api_test_app(&server.uri());
        let (_stores_tx, stores) =
            tokio::sync::watch::channel(Some((ME.to_owned(), store.clone())));
        app.attach_encryption(super::super::encryption::E2eeContext {
            oidc: None,
            stores,
            crypto_root: None,
            events: None,
        });
        app.current = Some(CHANNEL.into());

        // 送信者の端末のセッション。鍵（インデックス 0 から）は、あとで届ける。
        let sender =
            zoo_crypto::DeviceAccount::new(SENDER, &uuid::Uuid::new_v4().to_string()).unwrap();
        let mut outbound = zoo_crypto::OutboundSession::new(CHANNEL, 1, 0);
        let room_key = outbound.room_key();
        let mut encrypted_row = |id: &str| {
            let ciphertext = outbound
                .encrypt(
                    &sender,
                    &zoo_crypto::OutgoingMessage {
                        message_id: id.into(),
                        parent_id: None,
                        revision: 0,
                        content: zoo_crypto::MessageContent {
                            body: format!("本文 {id}"),
                            format: "mfm".into(),
                            kind: "message".into(),
                        },
                        mentions: Default::default(),
                        attachments: Vec::new(),
                    },
                )
                .unwrap();
            serde_json::json!({ "id": id, "user_id": SENDER, "channel_id": CHANNEL, "content": "",
                                "created_at": "2026-11-01 04:00:00", "epoch": "1", "revision": "0",
                                "ciphertext": ciphertext.to_string() })
        };
        let x = encrypted_row("x");
        let y = encrypted_row("y");
        fn feed(app: &mut App, id: &str, row: serde_json::Value) {
            app.on_shape(ShapeEvent::Batch(
                ShapeId::Messages(CHANNEL.into()),
                Batch {
                    changes: vec![electric::Change::Upsert {
                        key: id.into(),
                        row: row.as_object().unwrap().clone(),
                    }],
                    up_to_date: true,
                    ..Default::default()
                },
            ));
        }
        async fn decrypted(app: &mut App, rx: &mut mpsc::UnboundedReceiver<AppEvent>) {
            loop {
                match rx.recv().await.expect("イベントが届く") {
                    event @ AppEvent::Decrypted { .. } => {
                        app.on_app_event(event);
                        return;
                    }
                    other => app.on_app_event(other),
                }
            }
        }
        fn shown(app: &App, id: &str) -> Result<String, UndecryptableReason> {
            let message = app.store.message(CHANNEL, id).unwrap();
            match app.decryptions.shown(message, true) {
                Shown::Decrypted { message, .. } => Ok(message.body.clone()),
                Shown::Undecryptable(reason) => Err(reason),
                _ => panic!("暗号化したメッセージとして出す"),
            }
        }

        feed(&mut app, "x", x);
        app.request_decryptions();
        decrypted(&mut app, &mut rx).await;
        assert_eq!(shown(&app, "x"), Err(UndecryptableReason::KeyMissing));

        // 端末の管理のタスクがロックを持ったまま鍵を取り込む。そのあいだに行 Y が届き、復号を頼む
        // （ロックを取れないので、鍵待ちの行を復号し直すかは確かめられない）。
        let mut guard = store.lock().await;
        feed(&mut app, "y", y);
        app.request_decryptions();
        let request = guard.registration_request();
        let recipient =
            zoo_crypto::verify_device_keys(&request["device_keys"], ME, guard.device_id()).unwrap();
        let fallback = guard
            .prepare_key_upload(50, false)
            .unwrap()
            .unwrap()
            .fallback_keys;
        let key = fallback
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()
            .clone();
        let mut olm = sender
            .create_outbound_olm_session(&recipient, &key)
            .unwrap();
        let content = sender
            .encrypt_room_key(&mut olm, &recipient, &room_key)
            .unwrap();
        let row = ToDeviceRow {
            id: "1".into(),
            sender_user_id: SENDER.into(),
            r#type: "m.room.encrypted".into(),
            content,
        };
        let device_keys =
            serde_json::json!({ SENDER: { sender.device_id(): sender.device_keys() } });
        let result = guard
            .process_to_device(std::slice::from_ref(&row), &device_keys)
            .unwrap();
        assert_eq!(result.room_keys, 1);
        drop(guard);
        decrypted(&mut app, &mut rx).await;
        assert_eq!(shown(&app, "y").as_deref(), Ok("本文 y"));
        assert_eq!(
            shown(&app, "x").as_deref(),
            Ok("本文 x"),
            "鍵が届いたので、鍵待ちの行も復号し直す"
        );
    }

    fn upsert_rows(app: &mut App, shape: ShapeId, rows: &[(&str, serde_json::Value)]) {
        for (key, row) in rows {
            app.on_shape(ShapeEvent::Batch(
                shape.clone(),
                Batch {
                    changes: vec![electric::Change::Upsert {
                        key: (*key).into(),
                        row: row.as_object().unwrap().clone(),
                    }],
                    ..Default::default()
                },
            ));
        }
    }

    fn mark_synced(app: &mut App, shape: ShapeId) {
        app.on_shape(ShapeEvent::Batch(
            shape,
            Batch {
                up_to_date: true,
                ..Default::default()
            },
        ));
    }

    fn pinned_test_app(server: &wiremock::MockServer) -> (App, mpsc::UnboundedReceiver<AppEvent>) {
        let (mut app, rx) = api_test_app(&server.uri());
        upsert_rows(
            &mut app,
            ShapeId::Channels,
            &[
                (
                    "c1",
                    serde_json::json!({
                        "id": "c1", "channel_name": "general", "is_private": "f"
                    }),
                ),
                (
                    "c2",
                    serde_json::json!({
                        "id": "c2", "channel_name": "design", "is_private": "f"
                    }),
                ),
            ],
        );
        upsert_rows(
            &mut app,
            ShapeId::UserPinnedChannels,
            &[
                (
                    "p1",
                    serde_json::json!({
                        "user_id": "u1", "channel_id": "c1", "position": 0,
                        "created_at": "2026-11-11 00:00:00", "updated_at": "2026-11-11 00:00:00"
                    }),
                ),
                (
                    "p2",
                    serde_json::json!({
                        "user_id": "u1", "channel_id": "c2", "position": 1,
                        "created_at": "2026-11-11 00:00:00", "updated_at": "2026-11-11 00:00:00"
                    }),
                ),
            ],
        );
        app.focus = Focus::Sidebar;
        app.sidebar_index = 0;
        (app, rx)
    }

    #[tokio::test]
    async fn pressing_p_on_an_archived_unpinned_channel_does_not_send_a_request() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channel-pins"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {},
                "txid": "1"
            })))
            .expect(0)
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        upsert_rows(
            &mut app,
            ShapeId::Channels,
            &[(
                "c1",
                serde_json::json!({
                    "id": "c1", "channel_name": "archived", "is_private": "f",
                    "archive_at": "2026-11-10 00:00:00"
                }),
            )],
        );
        app.focus = Focus::Sidebar;
        app.sidebar_index = 0;

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::NONE,
        )));

        assert!(
            app.notice
                .as_ref()
                .is_some_and(|(message, _)| message.contains("アーカイブ")),
            "unexpected notice: {:?}",
            app.notice
        );
        let posts = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| {
                request.method == wiremock::http::Method::POST
                    && request.url.path() == "/api/channel-pins"
            })
            .count();
        assert_eq!(posts, 0);
    }

    #[tokio::test]
    async fn pressing_p_on_an_unjoined_public_channel_as_a_guest_does_not_send_a_request() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channel-pins"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {},
                "txid": "1"
            })))
            .expect(0)
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        upsert_rows(
            &mut app,
            ShapeId::Users,
            &[(
                "u1",
                serde_json::json!({
                    "id": "u1", "user_name": "alice", "display_name": "Alice",
                    "is_guest": "t"
                }),
            )],
        );
        mark_synced(&mut app, ShapeId::Users);
        upsert_rows(
            &mut app,
            ShapeId::Channels,
            &[(
                "c1",
                serde_json::json!({
                    "id": "c1", "channel_name": "public", "is_private": "f"
                }),
            )],
        );
        mark_synced(&mut app, ShapeId::ChannelUsers);
        app.focus = Focus::Sidebar;
        app.sidebar_index = 0;

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::NONE,
        )));

        assert!(
            app.notice
                .as_ref()
                .is_some_and(|(message, _)| message.contains("ゲスト")),
            "unexpected notice: {:?}",
            app.notice
        );
        let posts = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| {
                request.method == wiremock::http::Method::POST
                    && request.url.path() == "/api/channel-pins"
            })
            .count();
        assert_eq!(posts, 0);
    }

    #[tokio::test]
    async fn pressing_p_on_a_joined_public_channel_as_a_guest_sends_a_request() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channel-pins"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "version": 1 },
                "txid": "1"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        upsert_rows(
            &mut app,
            ShapeId::Users,
            &[(
                "u1",
                serde_json::json!({
                    "id": "u1", "user_name": "alice", "display_name": "Alice",
                    "is_guest": "t"
                }),
            )],
        );
        mark_synced(&mut app, ShapeId::Users);
        upsert_rows(
            &mut app,
            ShapeId::Channels,
            &[(
                "c1",
                serde_json::json!({
                    "id": "c1", "channel_name": "public", "is_private": "f"
                }),
            )],
        );
        upsert_rows(
            &mut app,
            ShapeId::ChannelUsers,
            &[(
                "u1:c1",
                serde_json::json!({ "user_id": "u1", "channel_id": "c1" }),
            )],
        );
        mark_synced(&mut app, ShapeId::ChannelUsers);
        app.focus = Focus::Sidebar;
        app.sidebar_index = 0;

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::NONE,
        )));

        let mut sent = false;
        for _ in 0..100 {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| {
                    request.method == wiremock::http::Method::POST
                        && request.url.path() == "/api/channel-pins"
                })
            {
                sent = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(sent, "a joined guest should be able to pin the channel");
    }

    #[tokio::test]
    async fn pressing_p_for_a_channel_not_yet_in_the_store_sends_a_request() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channel-pins"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "version": 1 },
                "txid": "1"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (mut app, _rx) = api_test_app(&server.uri());
        // 作成直後など、チャンネル行がまだ Shape で届いていない状態。
        app.focus = Focus::Messages;
        app.current = Some("new-channel".into());

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('p'),
            KeyModifiers::NONE,
        )));

        let mut sent = false;
        for _ in 0..100 {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| {
                    request.method == wiremock::http::Method::POST
                        && request.url.path() == "/api/channel-pins"
                })
            {
                sent = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            sent,
            "a channel missing from the store should be left to the server"
        );
    }

    #[tokio::test]
    async fn j_on_a_pinned_row_joins_instead_of_reordering() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/channels/c1/join"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {},
                "txid": "42"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let (mut app, _rx) = pinned_test_app(&server);
        assert!(matches!(app.sidebar_items()[0], SidebarItem::Pinned(_)));

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char('J'),
            KeyModifiers::NONE,
        )));

        let mut joined = false;
        for _ in 0..100 {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|request| {
                    request.method == wiremock::http::Method::POST
                        && request.url.path() == "/api/channels/c1/join"
                })
            {
                joined = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(joined, "J should join the channel on a pinned row");
    }

    #[tokio::test]
    async fn brackets_and_alt_arrows_reorder_pinned_rows() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/channel-pins/order"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": { "version": 1, "pins": [] },
                "txid": "43"
            })))
            .mount(&server)
            .await;
        let (mut app, _rx) = pinned_test_app(&server);

        async fn reorder_count(server: &wiremock::MockServer) -> usize {
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| {
                    request.method == wiremock::http::Method::PATCH
                        && request.url.path() == "/api/channel-pins/order"
                })
                .count()
        }

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::NONE,
        )));
        for _ in 0..100 {
            if reorder_count(&server).await == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        app.sidebar_index = 1;
        app.on_terminal_event(Event::Key(KeyEvent::new(KeyCode::Up, KeyModifiers::ALT)));
        for _ in 0..100 {
            if reorder_count(&server).await >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<serde_json::Value> = requests
            .iter()
            .filter(|request| {
                request.method == wiremock::http::Method::PATCH
                    && request.url.path() == "/api/channel-pins/order"
            })
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0]["channel_ids"], serde_json::json!(["c2", "c1"]));
        assert_eq!(bodies[1]["channel_ids"], serde_json::json!(["c2", "c1"]));
    }

    #[tokio::test]
    async fn reorder_retry_keeps_pins_added_on_another_device() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/channel-pins"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pins": [
                    { "channel_id": "c1", "position": 0, "archived": false },
                    { "channel_id": "c2", "position": 1, "archived": false },
                    { "channel_id": "c3", "position": 2, "archived": false }
                ],
                "version": 7
            })))
            .mount(&server)
            .await;
        // 最初の要求は 409 になり、最新には他端末が追加した c3 が入っている。
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = attempts.clone();
        Mock::given(method("PATCH"))
            .and(path("/api/channel-pins/order"))
            .respond_with(move |_request: &wiremock::Request| {
                if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(409).set_body_json(serde_json::json!({
                        "code": "CHANNEL_PINS_CONFLICT",
                        "version": 7,
                        "pins": [
                            { "channel_id": "c2", "position": 0, "archived": false },
                            { "channel_id": "c1", "position": 1, "archived": false },
                            { "channel_id": "c3", "position": 2, "archived": false }
                        ]
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "data": { "version": 8, "pins": [] },
                        "txid": "44"
                    }))
                }
            })
            .mount(&server)
            .await;
        let (mut app, _rx) = pinned_test_app(&server);
        app.pin_version = 6;
        app.sidebar_index = 0;

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::NONE,
        )));
        for _ in 0..100 {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.method == wiremock::http::Method::PATCH)
                .count()
                >= 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<serde_json::Value> = requests
            .iter()
            .filter(|request| {
                request.method == wiremock::http::Method::PATCH
                    && request.url.path() == "/api/channel-pins/order"
            })
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(
            bodies[1]["channel_ids"],
            serde_json::json!(["c2", "c1", "c3"])
        );
        assert_eq!(bodies[1]["expected_version"], 7);
    }

    #[tokio::test]
    async fn reorder_retry_places_the_moved_pin_next_to_its_neighbor_when_another_device_inserted_before_it()
     {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        // 手元は [c1, c2] で c1 を下へ動かす。ほかの端末が先頭に c3 を足している。
        Mock::given(method("GET"))
            .and(path("/api/channel-pins"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "pins": [
                    { "channel_id": "c3", "position": 0, "archived": false },
                    { "channel_id": "c1", "position": 1, "archived": false },
                    { "channel_id": "c2", "position": 2, "archived": false }
                ],
                "version": 7
            })))
            .mount(&server)
            .await;
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = attempts.clone();
        Mock::given(method("PATCH"))
            .and(path("/api/channel-pins/order"))
            .respond_with(move |_request: &wiremock::Request| {
                if counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(409).set_body_json(serde_json::json!({
                        "code": "CHANNEL_PINS_CONFLICT",
                        "version": 7,
                        "pins": [
                            { "channel_id": "c3", "position": 0, "archived": false },
                            { "channel_id": "c1", "position": 1, "archived": false },
                            { "channel_id": "c2", "position": 2, "archived": false }
                        ]
                    }))
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "data": { "version": 8, "pins": [] },
                        "txid": "45"
                    }))
                }
            })
            .mount(&server)
            .await;
        let (mut app, _rx) = pinned_test_app(&server);
        app.pin_version = 6;
        app.sidebar_index = 0;

        app.on_terminal_event(Event::Key(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::NONE,
        )));
        for _ in 0..100 {
            if server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.method == wiremock::http::Method::PATCH)
                .count()
                >= 2
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let requests = server.received_requests().await.unwrap();
        let bodies: Vec<serde_json::Value> = requests
            .iter()
            .filter(|request| {
                request.method == wiremock::http::Method::PATCH
                    && request.url.path() == "/api/channel-pins/order"
            })
            .map(|request| serde_json::from_slice(&request.body).unwrap())
            .collect();
        assert_eq!(bodies.len(), 2);
        // 最新の添字 (1) ではなく、`c1` の直後に置いた `c2` の手前へ戻す。
        assert_eq!(
            bodies[1]["channel_ids"],
            serde_json::json!(["c3", "c2", "c1"])
        );
        assert_eq!(bodies[1]["expected_version"], 7);
    }

    #[tokio::test]
    async fn reorder_conflict_stops_when_the_moved_pin_is_archived_or_deleted_elsewhere() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        for latest_pins in [
            serde_json::json!([
                { "channel_id": "c1", "position": 0, "archived": true },
                { "channel_id": "c2", "position": 1, "archived": false }
            ]),
            serde_json::json!([
                { "channel_id": "c2", "position": 0, "archived": false }
            ]),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/channel-pins"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "pins": latest_pins,
                    "version": 7
                })))
                .mount(&server)
                .await;
            Mock::given(method("PATCH"))
                .and(path("/api/channel-pins/order"))
                .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
                    "code": "CHANNEL_PINS_CONFLICT",
                    "version": 7,
                    "pins": []
                })))
                .mount(&server)
                .await;
            let (mut app, mut rx) = pinned_test_app(&server);
            app.pin_version = 6;
            app.sidebar_index = 0;

            app.on_terminal_event(Event::Key(KeyEvent::new(
                KeyCode::Char(']'),
                KeyModifiers::NONE,
            )));
            let mut notice = None;
            for _ in 0..100 {
                match tokio::time::timeout(Duration::from_millis(50), rx.recv()).await {
                    Ok(Some(AppEvent::Notice(message))) => {
                        notice = Some(message);
                        break;
                    }
                    Ok(Some(_)) => {}
                    _ => break,
                }
            }
            assert!(
                notice
                    .as_deref()
                    .is_some_and(|message| message.contains("アーカイブ")),
                "unexpected notice: {notice:?}"
            );
            let patches = server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.method == wiremock::http::Method::PATCH)
                .count();
            assert_eq!(
                patches, 1,
                "must not retry after an archived/deleted conflict"
            );
        }
    }

    #[test]
    fn step_clamps_and_starts_from_latest() {
        assert_eq!(step(None, 1, 0), None);
        assert_eq!(step(None, -1, 3), Some(2));
        assert_eq!(step(Some(0), -1, 3), Some(0));
        assert_eq!(step(Some(1), 5, 3), Some(2));
        assert_eq!(step(Some(2), isize::MIN, 3), Some(0));
        assert_eq!(step(Some(0), isize::MAX, 3), Some(2));
    }

    #[test]
    fn resolves_shortcodes_and_raw_emoji() {
        assert_eq!(resolve_emoji(":thumbsup:").as_deref(), Some("👍"));
        assert_eq!(resolve_emoji(" tada ").as_deref(), Some("🎉"));
        assert_eq!(resolve_emoji("👍").as_deref(), Some("👍"));
        assert_eq!(resolve_emoji("not-an-emoji"), None);
        assert_eq!(
            resolve_emoji(":blobcat_custom:").as_deref(),
            Some(":blobcat_custom:")
        );
        assert_eq!(resolve_emoji(":../cat:"), None);
        assert_eq!(resolve_emoji("::"), None);
    }

    #[test]
    fn move_id_recovers_when_selection_disappears() {
        let ids = vec!["a".to_string(), "b".to_string()];
        assert_eq!(move_id(&ids, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(move_id(&ids, Some("gone"), -1).as_deref(), Some("b"));
    }

    #[test]
    fn merge_pinned_order_keeps_a_pin_added_before_the_moved_one() {
        fn ids(values: &[&str]) -> Vec<String> {
            values.iter().map(|value| (*value).to_string()).collect()
        }
        // 手元は [c1, c2] で c1 を下へ。ほかの端末が先頭に c3 を足している。
        // 最新の添字 1 に入れると [c3, c1, c2] に戻ってしまう。
        assert_eq!(
            merge_pinned_order(&ids(&["c2", "c1"]), "c1", &ids(&["c3", "c1", "c2"])),
            ids(&["c3", "c2", "c1"])
        );
        // 上へ動かすときも、意図した並びで直後にある c1 の手前へ戻す。
        assert_eq!(
            merge_pinned_order(&ids(&["c2", "c1"]), "c2", &ids(&["c3", "c1", "c2"])),
            ids(&["c3", "c2", "c1"])
        );
        // 末尾へ動かすときは、直前の既存要素の後ろに置く。
        assert_eq!(
            merge_pinned_order(
                &ids(&["c2", "c3", "c1"]),
                "c1",
                &ids(&["c4", "c1", "c2", "c3"])
            ),
            ids(&["c4", "c2", "c3", "c1"])
        );
        // 隣の目印が無くなったときは、最初へ動かした場合だけ先頭に置く。
        assert_eq!(
            merge_pinned_order(&ids(&["c2", "c1"]), "c1", &ids(&["c3"])),
            ids(&["c3", "c1"])
        );
        assert_eq!(
            merge_pinned_order(&ids(&["c3", "c2"]), "c3", &ids(&["c1"])),
            ids(&["c3", "c1"])
        );
        // 動かした項目が意図した並びに無ければ、最新の並びをそのまま使う。
        assert_eq!(
            merge_pinned_order(&ids(&["c2"]), "gone", &ids(&["c1", "c2"])),
            ids(&["c1", "c2"])
        );
    }
}
