//! 画面の状態とキー操作。描画は `view.rs`、通信は `api` と `electric` に任せる。

use std::{
    collections::{HashMap, HashSet},
    future::Future,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::{sync::mpsc, task::JoinHandle};
use tui_textarea::TextArea;
use zoo2_client_core::poll::PollGate;

use super::e2ee_chat::{
    self, ChannelEncryption, CreateForm, CreateKind, Decryptions, RowKey, Shown,
};
use crate::{
    api::{ApiClient, Me, ReadState, ThreadSummary, TypingUser, stale_revision},
    e2ee::{
        send::{self, ChannelSender, SendContext},
        store::{Decryption, EncryptedRow, RecipientSummary},
    },
    electric::{self, Batch, ShapeEvent, ShapeId},
    http::Credentials,
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
    },
    /// チャンネルか DM を作った（#361）。開く。`encrypted` は応答の値。
    Created {
        channel_id: String,
        requested_encryption: bool,
        encrypted: bool,
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
    Thread { channel_id: String, id: String },
    MoreThreads { archived: bool },
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
    Help,
    /// 暗号化の画面（初回設定と解錠。#360）。
    Encryption,
    /// チャンネルか DM を作る（#361）。
    Create(CreateForm),
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
    /// 暗号化したメッセージの復号の結果（#361）。
    pub decryptions: Decryptions,
    /// 暗号化チャンネルごとの、最後に判定した配布先（送信画面に出す。#361）。
    pub recipients: HashMap<String, RecipientSummary>,
    /// 暗号化チャンネルへの送信の直列化と、知っている epoch（#361）。
    channel_sender: ChannelSender,
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
        let mut app = Self {
            encryption,
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
            unrecorded_encrypted: HashSet::new(),
            known_encrypted_loaded: false,
            known_encrypted_store: None,
            known_encrypted_in_flight: false,
        };
        for id in [ShapeId::Channels, ShapeId::Users, ShapeId::ChannelUsers] {
            app.subscribe(id);
        }
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
                self.store.apply(&id, batch);
                if is_channels {
                    for channel in self.store.channels() {
                        if channel.encrypted
                            && self.known_encrypted_channels.insert(channel.id.clone())
                        {
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
            AppEvent::ThreadsChanged { reopened } => {
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
                // 書き込み前の一覧で、確定済みの状態を上書きしない。
                self.threads_requested_at = [Some(Instant::now()); 2];
                self.refresh_threads();
            }
            AppEvent::EmojiCatalog { shortcodes, custom } => {
                crate::mfm::set_emoji_catalog(shortcodes, custom)
            }
            AppEvent::Notice(message) => self.notify(message),
            AppEvent::RestoreDraft {
                channel_id,
                target,
                content,
            } => {
                // 入力欄に書きかけの本文があるか別のチャンネルにいるときは、上書きせずに取っておく。
                if self.current.as_deref() == Some(channel_id.as_str()) && self.composer.is_empty()
                {
                    self.composer = composer_with(&content);
                    self.target = target;
                } else {
                    self.failed_drafts.push(FailedDraft {
                        channel_id,
                        target,
                        content,
                    });
                    self.notify(format!(
                        "送れなかった本文を保存しました ({}件)。R で入力欄に戻せます",
                        self.failed_drafts.len()
                    ));
                }
            }
            AppEvent::ReadStatesPollFinished => self.read_poll.finish(Instant::now(), true),
            AppEvent::EmojiPollFinished => self.emoji_poll.finish(Instant::now(), true),
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
        if let Some(root) = &context.crypto_root {
            self.known_encrypted_channels
                .extend(crate::e2ee::store::read_encrypted_channels(
                    root,
                    &self.me.id,
                ));
        }
        self.encryption.attach(context);
    }

    pub fn on_tick(&mut self) {
        self.encryption.on_tick(self.mode == Mode::Encryption);
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
        let archived = self
            .archived_threads
            .iter()
            .position(|thread| thread.id == id)
            .map(|index| self.archived_threads.remove(index));
        if let Some(thread) = self
            .active_threads
            .iter_mut()
            .find(|thread| thread.id == id)
        {
            thread.thread_archive_at = None;
            return;
        }
        let cached = archived.or_else(|| {
            self.store.channels().into_iter().find_map(|channel| {
                let root = self.store.message(&channel.id, id)?;
                Some(ThreadSummary {
                    id: root.id.clone(),
                    channel_id: root.channel_id.clone(),
                    content: root.content.chars().take(160).collect(),
                    thread_archive_at: None,
                    reply_count: self.store.replies(&channel.id, id).len() as u64,
                })
            })
        });
        if let Some(mut thread) = cached {
            thread.thread_archive_at = None;
            self.active_threads.insert(0, thread);
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

    pub fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();
        for channel in self.store.channels() {
            items.push(SidebarItem::Channel(channel.id.clone()));
            let joined = !channel.is_dm && self.store.is_member(&channel.id, &self.me.id);
            for thread in self.threads().filter(|thread| {
                joined
                    && thread.channel_id == channel.id
                    && (!self.thread_is_archived(&channel.id, &thread.id)
                        || (self.current.as_deref() == Some(channel.id.as_str())
                            && self
                                .thread
                                .as_ref()
                                .is_some_and(|selected| selected.root_id == thread.id)))
            }) {
                items.push(SidebarItem::Thread {
                    channel_id: channel.id.clone(),
                    id: thread.id.clone(),
                });
            }
            if self.current.as_deref() == Some(channel.id.as_str())
                && let Some(thread) = &self.thread
            {
                let selected = SidebarItem::Thread {
                    channel_id: channel.id.clone(),
                    id: thread.root_id.clone(),
                };
                if !items.contains(&selected)
                    && self.store.message(&channel.id, &thread.root_id).is_some()
                    && (self.thread_is_archived(&channel.id, &thread.root_id)
                        || !self.store.replies(&channel.id, &thread.root_id).is_empty())
                {
                    items.push(selected);
                }
            }
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
            Mode::React { message_id, input } => self.on_react_key(key, message_id, input),
            Mode::Help => self.mode = Mode::Normal,
            Mode::Encryption => {
                let user_id = self.me.id.clone();
                if !self.encryption.on_key(key, &user_id) {
                    self.mode = Mode::Normal;
                }
            }
            Mode::Create(form) => self.on_create_key(key, form),
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
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_selection(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_selection(isize::MAX),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.scroll_inside(true)
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
            KeyCode::Char('J') => self.join_or_leave(true),
            KeyCode::Char('L') => self.join_or_leave(false),
            KeyCode::Char('/') => self.mode = Mode::Search(String::new()),
            KeyCode::Char('R') => self.restore_failed_draft(),
            KeyCode::Char('u') => self.reopen_thread(),
            KeyCode::Char('c') => self.mode = Mode::Create(CreateForm::channel()),
            KeyCode::Char('m') => self.mode = Mode::Create(CreateForm::dm()),
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
        if let Mode::Create(form) = &mut self.mode {
            form.input.extend(text.chars().filter(|c| !c.is_control()));
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

    /// チャンネル名・スレッド名に `query` を含む行の添字。
    pub fn search_matches(&self, query: &str) -> Vec<usize> {
        let query = query.to_lowercase();
        self.sidebar_items()
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                let label = match item {
                    SidebarItem::Channel(id) => self
                        .store
                        .channel(id)
                        .map(|channel| self.store.channel_label(channel, &self.me.id))
                        .unwrap_or_default(),
                    SidebarItem::MoreThreads { .. } => return false,
                    SidebarItem::Thread { channel_id, id } => self
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
                let mut ids = vec![thread.root_id.clone()];
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
                    .filter(|id| **id != thread.root_id)
                    .cloned()
            }),
        };
        if let Some(root_id) = root_id {
            self.typing_poll.wake();
            self.thread = Some(ThreadView {
                root_id,
                selected: None,
                scroll: 0,
                inner_scroll: 0,
            });
            self.focus = Focus::Thread;
        }
    }

    fn activate_sidebar(&mut self) {
        match self.sidebar_items().get(self.sidebar_index).cloned() {
            Some(SidebarItem::Channel(channel_id)) => {
                self.open_channel(channel_id);
                self.focus = Focus::Messages;
            }
            Some(SidebarItem::Thread { channel_id, id }) => {
                self.open_channel(channel_id);
                self.thread = Some(ThreadView {
                    root_id: id.clone(),
                    selected: None,
                    scroll: 0,
                    inner_scroll: 0,
                });
                self.restore_sidebar_selection(Some(SidebarItem::Thread {
                    channel_id: self.current.clone().unwrap_or_default(),
                    id: id.clone(),
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
        self.thread = None;
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
        self.target = draft.target;
        self.mode = Mode::Insert;
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
                Some((decrypted, _)) => decrypted.body.clone(),
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
        let channel_id = match self.focus {
            Focus::Sidebar => self
                .sidebar_items()
                .get(self.sidebar_index)
                .and_then(|item| match item {
                    SidebarItem::Channel(id) => Some(id.clone()),
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
        // 鍵待ちの行の復号し直しは、チャンネルごとの数えで決める（別のチャンネルを開いているあいだに鍵が
        // 届いても、戻ったときに復号し直す）。
        let retry_missing = self.decryptions.has_missing_in(&channel_id)
            && store.try_lock().is_ok_and(|store| {
                self.decryptions.generations.get(&channel_id) != Some(&store.inbound_generation())
            });
        if !self.decryptions.dirty && !retry_missing {
            return;
        }
        let rows: Vec<(RowKey, EncryptedRow)> = self
            .store
            .visible_messages(&channel_id)
            .filter(|message| self.decryptions.needs(message, retry_missing))
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
        if rows.is_empty() && !retry_missing {
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
            let result = match &target {
                Target::Channel => api.send_message(&id, &channel_id, &content, None).await,
                Target::Reply(root_id) => {
                    api.send_message(&id, &channel_id, &content, Some(root_id))
                        .await
                }
                Target::Edit(message_id) => api.edit_message(message_id, &content).await,
            };
            if let Err(err) = result {
                let _ = tx.send(AppEvent::Notice(format!("送信に失敗しました: {err}")));
                let _ = tx.send(AppEvent::RestoreDraft {
                    channel_id,
                    target,
                    content,
                });
            } else {
                let reopened = match target {
                    Target::Reply(root_id) => Some(root_id),
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
                        });
                        return;
                    }
                    let _ = tx.send(AppEvent::Notice(format!("送信に失敗しました: {err}")));
                    let _ = tx.send(AppEvent::RestoreDraft {
                        channel_id,
                        target,
                        content,
                    });
                }
            }
        });
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

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(app.active_threads[0].thread_archive_at.is_none());
        app.close_thread();
        assert!(app.sidebar_items().contains(&SidebarItem::Thread {
            channel_id: "c1".into(),
            id: "root".into(),
        }));
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(&event, AppEvent::ThreadsFailed { .. }));
        app.on_app_event(event);
        assert!(app.active_threads[0].thread_archive_at.is_none());
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
}
