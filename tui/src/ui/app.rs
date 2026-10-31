//! 画面の状態とキー操作。描画は `view.rs`、通信は `api` と `electric` に任せる。

use std::{
    collections::HashMap,
    future::Future,
    time::{Duration, Instant},
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::{sync::mpsc, task::JoinHandle};
use tui_textarea::TextArea;

use crate::{
    api::{ApiClient, Me},
    electric::{self, ShapeEvent, ShapeId},
    http::Credentials,
    store::Store,
};

const NOTICE_TTL: Duration = Duration::from_secs(6);

/// 非同期の API 呼び出しから画面に返す結果。
#[derive(Debug)]
pub enum AppEvent {
    Notice(String),
    /// 送信に失敗した下書きを入力欄に戻す。
    RestoreDraft(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Sidebar,
    Messages,
    Thread,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// 入力欄に書いている。
    Insert,
    /// 削除の確認。
    ConfirmDelete {
        message_id: String,
    },
    /// チャンネル名で絞り込む。
    Search(String),
    Help,
}

/// 入力欄の送り先。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Channel,
    Reply(String),
    Edit(String),
}

pub struct ThreadView {
    pub root_id: String,
    pub selected: Option<String>,
    pub scroll: usize,
}

pub struct App {
    pub api: ApiClient,
    http: reqwest::Client,
    api_base_url: String,
    credentials: Credentials,
    pub me: Me,
    pub store: Store,
    subscriptions: HashMap<ShapeId, JoinHandle<()>>,
    shape_tx: mpsc::UnboundedSender<ShapeEvent>,
    app_tx: mpsc::UnboundedSender<AppEvent>,

    pub focus: Focus,
    pub mode: Mode,
    pub sidebar_index: usize,
    pub current: Option<String>,
    /// 選択中のルートメッセージ。None なら最新を追う。
    pub selected: Option<String>,
    pub scroll: usize,
    pub thread: Option<ThreadView>,
    pub composer: TextArea<'static>,
    pub target: Target,
    pub notice: Option<(String, Instant)>,
    pub should_quit: bool,
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
        let mut app = Self {
            api,
            http,
            api_base_url,
            credentials,
            me,
            store: Store::default(),
            subscriptions: HashMap::new(),
            shape_tx,
            app_tx,
            focus: Focus::Sidebar,
            mode: Mode::Normal,
            sidebar_index: 0,
            current: None,
            selected: None,
            scroll: 0,
            thread: None,
            composer: new_composer(),
            target: Target::Channel,
            notice: None,
            should_quit: false,
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

    pub fn on_shape(&mut self, event: ShapeEvent) {
        match event {
            ShapeEvent::Batch(id, batch) => self.store.apply(&id, batch),
            ShapeEvent::Failed(id, err) => {
                self.subscriptions.remove(&id);
                self.notify(format!("同期を停止しました ({id:?}): {err}"));
            }
        }
        self.sidebar_index = self
            .sidebar_index
            .min(self.store.channels().len().saturating_sub(1));
    }

    pub fn on_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Notice(message) => self.notify(message),
            AppEvent::RestoreDraft(draft) => {
                if self.composer.is_empty() {
                    self.composer = composer_with(&draft);
                }
            }
        }
    }

    pub fn on_tick(&mut self) {
        if self
            .notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() > NOTICE_TTL)
        {
            self.notice = None;
        }
    }

    pub fn on_terminal_event(&mut self, event: Event) {
        let Event::Key(key) = event else {
            return;
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
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    // スレッドの親を消したら、スレッドも閉じる。
                    if self
                        .thread
                        .as_ref()
                        .is_some_and(|thread| thread.root_id == message_id)
                    {
                        self.close_thread();
                    }
                    let api = self.api.clone();
                    self.spawn_api("削除", async move { api.delete_message(&message_id).await });
                }
                self.mode = Mode::Normal;
            }
            Mode::Search(query) => self.on_search_key(key, query),
            Mode::Help => self.mode = Mode::Normal,
        }
    }

    fn on_normal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Tab => self.cycle_focus(true),
            KeyCode::BackTab => self.cycle_focus(false),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('g') | KeyCode::Home => self.move_selection(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_selection(isize::MAX),
            KeyCode::Enter => self.activate(),
            KeyCode::Esc => self.back(),
            KeyCode::Char('i') => self.start_compose(),
            KeyCode::Char('e') => self.start_edit(),
            KeyCode::Char('d') => self.confirm_delete(),
            KeyCode::Char('J') => self.join_or_leave(true),
            KeyCode::Char('L') => self.join_or_leave(false),
            KeyCode::Char('/') => self.mode = Mode::Search(String::new()),
            _ => {}
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
                self.composer.input(key);
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

    /// 一覧の表示名に `query` を含むチャンネルの添字 (大文字小文字を区別しない)。
    pub fn search_matches(&self, query: &str) -> Vec<usize> {
        let query = query.to_lowercase();
        self.store
            .channels()
            .iter()
            .enumerate()
            .filter(|(_, channel)| {
                self.store
                    .channel_label(channel, &self.me.id)
                    .to_lowercase()
                    .contains(&query)
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
                let len = self.store.channels().len();
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
            }
        }
    }

    fn activate(&mut self) {
        match self.focus {
            Focus::Sidebar => self.activate_sidebar(),
            Focus::Messages => {
                if let Some(root_id) = self.selected.clone() {
                    self.thread = Some(ThreadView {
                        root_id,
                        selected: None,
                        scroll: 0,
                    });
                    self.focus = Focus::Thread;
                }
            }
            Focus::Thread => {}
        }
    }

    fn activate_sidebar(&mut self) {
        let Some(channel) = self
            .store
            .channels()
            .get(self.sidebar_index)
            .map(|c| c.id.clone())
        else {
            return;
        };
        self.open_channel(channel);
        self.focus = Focus::Messages;
    }

    pub fn open_channel(&mut self, channel_id: String) {
        if self.current.as_deref() != Some(channel_id.as_str()) {
            self.selected = None;
            self.scroll = 0;
            self.thread = None;
            self.target = Target::Channel;
        }
        self.subscribe(ShapeId::Messages(channel_id.clone()));
        self.current = Some(channel_id);
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
        let Some(message) = self.store.message(&channel_id, &message_id) else {
            return;
        };
        if message.user_id != self.me.id {
            self.notify("自分のメッセージだけを編集できます");
            return;
        }
        self.composer = composer_with(&message.content);
        self.target = Target::Edit(message_id);
        self.mode = Mode::Insert;
    }

    fn confirm_delete(&mut self) {
        let Some((channel_id, message_id)) = self.focused_message() else {
            return;
        };
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
                .store
                .channels()
                .get(self.sidebar_index)
                .map(|c| c.id.clone()),
            _ => self.current.clone(),
        };
        let Some(channel_id) = channel_id else {
            return;
        };
        let api = self.api.clone();
        if join {
            self.spawn_api("参加", async move { api.join_channel(&channel_id).await });
        } else {
            self.spawn_api("退出", async move { api.leave_channel(&channel_id).await });
        }
    }

    fn submit(&mut self) {
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
                let _ = tx.send(AppEvent::RestoreDraft(content));
            }
        });
    }
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
    fn move_id_recovers_when_selection_disappears() {
        let ids = vec!["a".to_string(), "b".to_string()];
        assert_eq!(move_id(&ids, Some("a"), 1).as_deref(), Some("b"));
        assert_eq!(move_id(&ids, Some("gone"), -1).as_deref(), Some("b"));
    }
}
