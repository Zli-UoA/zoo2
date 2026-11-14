//! 暗号化の画面（`E`）：初回設定と、リカバリーキーでの解錠（docs/e2ee.md の「初回設定と解錠の流れ」。
//! #360）。Web の設定の「暗号化」タブと同じ流れ。
//!
//! - 初回設定：鍵を作り、リカバリーキーを表示する。利用者が控えたら（`y`）、保存の直前に再認証し
//!   （ブラウザーで開き、ループバックのリダイレクトで受け取る）、保存する。保存が 403 になったら、
//!   同じ鍵のまま再認証してやり直す。
//! - 解錠：リカバリーキーを打ち込む。初回設定の再開でクロス署名鍵がなければ、再認証してから続きを
//!   保存する。
//!
//! マスター鍵の秘密鍵と SSSS 鍵は [`SetupSession`] にだけ置き、画面を閉じたら捨てる。続きは、
//! SSSS を開け直して行う（初回設定の再開）。

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use zeroize::Zeroizing;
use zoo_crypto::{CryptoError, SetupSession};

use super::app::AppEvent;
use crate::api::ApiClient;
use crate::auth::reauth::REAUTH_TIMEOUT;
use crate::config::OidcConfig;
use crate::e2ee::manager::StoreWatch;
use crate::e2ee::reauth;
use crate::e2ee::setup::{self, E2eeStatus, LockReason, NoSecretStorage, SetupError, SharedStore};

/// 画面に渡す E2EE の文脈（`ui::run` の引数）。
pub struct E2eeContext {
    /// OIDC の設定。dev モードでは `None`（サーバーも再認証を確かめないので、再認証しない）。
    pub oidc: Option<OidcConfig>,
    /// 端末の管理のタスクが開いた暗号ストア。
    pub stores: StoreWatch,
}

/// 秘密を Debug に出さないための包み。
#[derive(Clone)]
pub struct SessionHandle(Arc<SetupSession>);

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionHandle(..)")
    }
}

/// 裏の処理から画面に返す結果。`generation` が今の画面のものでなければ捨てる。
#[derive(Debug)]
pub enum EncryptionEvent {
    Status {
        generation: u64,
        result: Result<E2eeStatus, String>,
    },
    /// 再認証の待ち受けを始めた。
    ReauthWaiting {
        generation: u64,
        url: String,
        port: u16,
    },
    /// 再認証を終えて、保存している。
    Saving {
        generation: u64,
    },
    /// 初回設定か、その続きを最後まで保存した。
    Saved {
        generation: u64,
    },
    /// SSSS を開けた。`needs_reauth` なら、再認証してから続きを保存する。
    Unlocked {
        generation: u64,
        session: SessionHandle,
        secrets_revision: Option<u64>,
        needs_reauth: bool,
    },
    Failed {
        generation: u64,
        failure: Failure,
    },
}

/// 失敗の種類（表示と、次にできる操作を分ける）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// 再認証が要る（403）か、再認証できなかった。同じ鍵のまま、再認証からやり直せる。
    Reauth(String),
    /// 別の端末が先に初回設定を始めた。表示したリカバリーキーは使えない。
    Conflict,
    /// サーバーの鍵が SSSS の鍵と違う。
    Mismatch(String),
    /// リカバリーキーの打ち間違いか、鍵が違う。打ち直せる。
    WrongKey(String),
    /// そのほか（通信の失敗など）。同じ操作をやり直せる。
    Other(String),
}

/// 何を保存するか（再認証のあと）。
#[derive(Debug, Clone, Copy)]
enum Save {
    /// 初回設定（SSSS から）。
    Setup,
    /// 解錠のあとの続き。
    Resume { secrets_revision: Option<u64> },
}

/// 画面の状態。
pub enum Screen {
    Loading,
    /// 暗号ストアの準備ができていない、または状態を読めない。
    Unavailable(String),
    NotSetUp,
    /// 初回設定のリカバリーキーを控えてもらう。
    RecoveryKey(Zeroizing<String>),
    /// 再認証の待ち受け（URL を出す前は `None`）。
    Reauth {
        url: Option<String>,
        port: Option<u16>,
    },
    Working(&'static str),
    /// リカバリーキーを打ち込む。
    Locked {
        reason: LockReason,
        input: Zeroizing<String>,
        error: Option<String>,
    },
    /// 解錠したが、続きの保存に再認証が要る。
    NeedsReauth,
    /// 保存に失敗した。`retry` なら、同じ鍵のまま再認証からやり直せる。
    SaveFailed {
        message: String,
        retry: bool,
    },
    /// 別の端末が先に初回設定を始めた、またはサーバーの鍵が違う。
    Stopped(String),
    Unlocked,
}

/// 暗号化の画面と、その裏の処理。
pub struct Encryption {
    api: ApiClient,
    http: reqwest::Client,
    oidc: Option<OidcConfig>,
    stores: Option<StoreWatch>,
    tx: mpsc::UnboundedSender<AppEvent>,
    pub screen: Screen,
    /// 最後に読んだ状態（ステータス行の案内に使う）。
    pub status: Option<E2eeStatus>,
    session: Option<SessionHandle>,
    save: Save,
    task: Option<JoinHandle<()>>,
    generation: u64,
    pub over_ssh: bool,
    /// 直近の描画でリカバリーキーを全部表示できたか。表示できていなければ、控えたこと（`y`）にしない。
    pub recovery_key_shown: bool,
}

impl Encryption {
    pub fn new(api: ApiClient, http: reqwest::Client, tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self {
            api,
            http,
            oidc: None,
            stores: None,
            tx,
            screen: Screen::Loading,
            status: None,
            session: None,
            save: Save::Setup,
            task: None,
            generation: 0,
            over_ssh: reauth::over_ssh(),
            recovery_key_shown: false,
        }
    }

    /// 端末の管理のタスクと OIDC の設定をつなぐ。
    pub fn attach(&mut self, context: E2eeContext) {
        self.oidc = context.oidc;
        self.stores = Some(context.stores);
    }

    fn store(&self) -> Option<(String, SharedStore)> {
        self.stores.as_ref().and_then(|rx| rx.borrow().clone())
    }

    /// 暗号ストアが開いたら（開き直したら）、状態を読み直す（ステータス行の案内のため）。
    pub fn on_tick(&mut self, screen_open: bool) {
        let changed = self
            .stores
            .as_mut()
            .is_some_and(|rx| rx.has_changed().unwrap_or(false));
        if changed {
            if let Some(rx) = self.stores.as_mut() {
                rx.borrow_and_update();
            }
            if !screen_open || matches!(self.screen, Screen::Loading | Screen::Unavailable(_)) {
                self.reload();
            }
        }
    }

    /// 今の画面の世代（裏の処理の結果を照らし合わせる）。
    #[cfg(test)]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    fn next_generation(&mut self) -> u64 {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.generation += 1;
        self.generation
    }

    /// 画面を開く。
    pub fn open(&mut self) {
        self.reload();
    }

    /// 画面を閉じる。メモリーの秘密を捨てる（続きは SSSS を開け直して行う）。
    pub fn close(&mut self) {
        self.next_generation();
        self.session = None;
        self.screen = Screen::Loading;
    }

    /// 状態を読み直す。
    pub fn reload(&mut self) {
        let generation = self.next_generation();
        self.session = None;
        self.screen = Screen::Loading;
        let Some((user_id, store)) = self.store() else {
            self.screen = Screen::Unavailable(
                "端末を登録しています。少し待ってから、もう一度開いてください".into(),
            );
            return;
        };
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.task = Some(tokio::spawn(async move {
            let result = setup::load_status(&api, &store, &user_id)
                .await
                .map_err(|e| e.to_string());
            drop(store);
            let _ = tx.send(AppEvent::Encryption(EncryptionEvent::Status {
                generation,
                result,
            }));
        }));
    }

    pub fn on_event(&mut self, event: EncryptionEvent) {
        let generation = match &event {
            EncryptionEvent::Status { generation, .. }
            | EncryptionEvent::ReauthWaiting { generation, .. }
            | EncryptionEvent::Saving { generation }
            | EncryptionEvent::Saved { generation }
            | EncryptionEvent::Unlocked { generation, .. }
            | EncryptionEvent::Failed { generation, .. } => *generation,
        };
        if generation != self.generation {
            return;
        }
        match event {
            EncryptionEvent::Status { result, .. } => match result {
                Ok(status) => {
                    self.status = Some(status);
                    self.screen = match status {
                        E2eeStatus::NotSetUp => Screen::NotSetUp,
                        E2eeStatus::Locked(reason) => Screen::Locked {
                            reason,
                            input: Zeroizing::new(String::new()),
                            error: None,
                        },
                        E2eeStatus::Unlocked => Screen::Unlocked,
                    };
                }
                Err(message) => {
                    self.screen = Screen::Unavailable(format!("状態を読めません: {message}"));
                }
            },
            EncryptionEvent::ReauthWaiting { url, port, .. } => {
                if !self.over_ssh {
                    reauth::open_browser(&url);
                }
                self.screen = Screen::Reauth {
                    url: Some(url),
                    port: Some(port),
                };
            }
            EncryptionEvent::Saving { .. } => self.screen = Screen::Working("保存しています…"),
            EncryptionEvent::Saved { .. } => {
                self.session = None;
                self.status = Some(E2eeStatus::Unlocked);
                self.screen = Screen::Unlocked;
            }
            EncryptionEvent::Unlocked {
                session,
                secrets_revision,
                needs_reauth,
                ..
            } => {
                if needs_reauth {
                    self.session = Some(session);
                    self.save = Save::Resume { secrets_revision };
                    self.screen = Screen::NeedsReauth;
                } else {
                    self.session = None;
                    self.status = Some(E2eeStatus::Unlocked);
                    self.screen = Screen::Unlocked;
                }
            }
            EncryptionEvent::Failed { failure, .. } => self.on_failure(failure),
        }
    }

    fn on_failure(&mut self, failure: Failure) {
        match failure {
            Failure::Reauth(message) => {
                self.screen = Screen::SaveFailed {
                    message,
                    retry: self.session.is_some(),
                };
            }
            Failure::Conflict => {
                self.session = None;
                self.screen = Screen::Stopped(SetupError::Conflict.to_string());
            }
            Failure::Mismatch(message) => {
                self.session = None;
                self.screen = Screen::Stopped(message);
            }
            Failure::WrongKey(message) => {
                let reason = match self.status {
                    Some(E2eeStatus::Locked(reason)) => reason,
                    _ => LockReason::New,
                };
                self.screen = Screen::Locked {
                    reason,
                    input: Zeroizing::new(String::new()),
                    error: Some(message),
                };
            }
            Failure::Other(message) => {
                self.screen = match &self.screen {
                    Screen::Working("解錠しています…") => Screen::Locked {
                        reason: match self.status {
                            Some(E2eeStatus::Locked(reason)) => reason,
                            _ => LockReason::New,
                        },
                        input: Zeroizing::new(String::new()),
                        error: Some(message),
                    },
                    _ => Screen::SaveFailed {
                        message,
                        retry: self.session.is_some(),
                    },
                };
            }
        }
    }

    /// キー操作。画面を閉じるときは `false` を返す。
    pub fn on_key(&mut self, key: KeyEvent, user_id: &str) -> bool {
        if key.code == KeyCode::Esc {
            if matches!(self.screen, Screen::Reauth { .. }) {
                // 再認証をやめても、鍵は捨てない。同じ鍵のまま、もう一度再認証できる。
                self.next_generation();
                self.screen = Screen::SaveFailed {
                    message: "再認証を取りやめました".into(),
                    retry: self.session.is_some(),
                };
                return true;
            }
            self.close();
            return false;
        }
        match &mut self.screen {
            Screen::NotSetUp if key.code == KeyCode::Enter => self.begin(user_id),
            Screen::RecoveryKey(_)
                if self.recovery_key_shown && matches!(key.code, KeyCode::Char('y' | 'Y')) =>
            {
                self.save = Save::Setup;
                self.reauth_and_save();
            }
            Screen::Locked { input, error, .. } => match key.code {
                KeyCode::Enter => {
                    let recovery_key = std::mem::take(input);
                    *error = None;
                    self.unlock(recovery_key);
                }
                KeyCode::Backspace => {
                    input.pop();
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.push(c);
                }
                _ => {}
            },
            Screen::NeedsReauth if key.code == KeyCode::Enter => self.reauth_and_save(),
            Screen::SaveFailed { retry: true, .. } if key.code == KeyCode::Enter => {
                self.reauth_and_save()
            }
            Screen::SaveFailed { retry: false, .. }
            | Screen::Stopped(_)
            | Screen::Unavailable(_)
                if key.code == KeyCode::Enter =>
            {
                self.reload()
            }
            Screen::Unlocked if key.code == KeyCode::Enter => {
                self.close();
                return false;
            }
            _ => {}
        }
        true
    }

    /// 貼り付け（リカバリーキーの入力欄だけ）。
    pub fn on_paste(&mut self, text: &str) {
        if let Screen::Locked { input, .. } = &mut self.screen {
            input.push_str(text);
        }
    }

    fn begin(&mut self, user_id: &str) {
        match SetupSession::begin(user_id) {
            Ok((session, recovery_key)) => {
                self.next_generation();
                self.session = Some(SessionHandle(Arc::new(session)));
                self.recovery_key_shown = false;
                self.screen = Screen::RecoveryKey(recovery_key);
            }
            Err(error) => {
                self.screen = Screen::Unavailable(format!("鍵を作れません: {error}"));
            }
        }
    }

    fn unlock(&mut self, recovery_key: Zeroizing<String>) {
        let generation = self.next_generation();
        let Some((user_id, store)) = self.store() else {
            self.screen = Screen::Unavailable("暗号ストアを開けていません".into());
            return;
        };
        self.screen = Screen::Working("解錠しています…");
        let api = self.api.clone();
        let tx = self.tx.clone();
        self.task = Some(tokio::spawn(async move {
            let event = match setup::unlock(&api, &store, &user_id, &recovery_key).await {
                Ok(unlocked) => EncryptionEvent::Unlocked {
                    generation,
                    session: SessionHandle(Arc::new(unlocked.session)),
                    secrets_revision: unlocked.secrets_revision,
                    needs_reauth: unlocked.needs_reauth,
                },
                Err(error) => EncryptionEvent::Failed {
                    generation,
                    failure: classify(&error),
                },
            };
            drop(store);
            let _ = tx.send(AppEvent::Encryption(event));
        }));
    }

    /// 保存の直前に再認証し、保存する（初回設定か、解錠のあとの続き）。dev モードでは再認証しない。
    fn reauth_and_save(&mut self) {
        let Some(session) = self.session.clone() else {
            self.reload();
            return;
        };
        let generation = self.next_generation();
        let save = self.save;
        let api = self.api.clone();
        let http = self.http.clone();
        let oidc = self.oidc.clone();
        let stores = self.stores.clone();
        let tx = self.tx.clone();
        self.screen = match oidc {
            Some(_) => Screen::Reauth {
                url: None,
                port: None,
            },
            None => Screen::Working("保存しています…"),
        };
        self.task = Some(tokio::spawn(async move {
            let send = |event| {
                let _ = tx.send(AppEvent::Encryption(event));
            };
            let failed = |failure| EncryptionEvent::Failed {
                generation,
                failure,
            };
            let token = match &oidc {
                None => None,
                Some(oidc) => {
                    let token = async {
                        let (pending, discovery) = reauth::start(&http, oidc).await?;
                        send(EncryptionEvent::ReauthWaiting {
                            generation,
                            url: pending.url().to_owned(),
                            port: pending.port(),
                        });
                        pending
                            .finish(&http, &discovery, oidc, REAUTH_TIMEOUT)
                            .await
                    }
                    .await;
                    match token {
                        Ok(token) => Some(Zeroizing::new(token)),
                        Err(error) => {
                            send(failed(Failure::Reauth(format!(
                                "再認証できませんでした: {error}"
                            ))));
                            return;
                        }
                    }
                }
            };
            send(EncryptionEvent::Saving { generation });
            // 再認証を待つ間は暗号ストアを持たない（端末の管理のタスクが開き直せるように）。
            let Some((user_id, store)) = stores.and_then(|rx| rx.borrow().clone()) else {
                send(failed(Failure::Other("暗号ストアを開けていません".into())));
                return;
            };
            let token = token.as_deref().map(String::as_str);
            let result = match save {
                Save::Setup => {
                    setup::complete_setup(&api, &store, &user_id, &session.0, token).await
                }
                Save::Resume { secrets_revision } => {
                    setup::save_remaining(
                        &api,
                        &store,
                        &user_id,
                        &session.0,
                        secrets_revision,
                        token,
                    )
                    .await
                }
            };
            drop(store);
            send(match result {
                Ok(()) => EncryptionEvent::Saved { generation },
                Err(error) => failed(classify(&error)),
            });
        }));
    }
}

/// エラーを画面の失敗の種類にする。
pub fn classify(error: &anyhow::Error) -> Failure {
    if let Some(error) = error.downcast_ref::<SetupError>() {
        return match error {
            SetupError::ReauthRequired => Failure::Reauth(
                "再認証が必要です（ログインし直してから時間がたちました）。同じ鍵のまま、もう一度再認証してください"
                    .into(),
            ),
            SetupError::Conflict => Failure::Conflict,
            SetupError::ServerKeyMismatch(_) => Failure::Mismatch(error.to_string()),
        };
    }
    if error.downcast_ref::<NoSecretStorage>().is_some() {
        return Failure::Other(error.to_string());
    }
    if let Some(error) = error.downcast_ref::<CryptoError>() {
        return match error {
            CryptoError::InvalidRecoveryKey(_) => Failure::WrongKey(
                "リカバリーキーの形式が違います。打ち間違いがないか確かめてください".into(),
            ),
            CryptoError::KeySlotUnlock | CryptoError::SsssKeyMismatch => {
                Failure::WrongKey("リカバリーキーが違います".into())
            }
            CryptoError::NoKeySlot => {
                Failure::WrongKey("リカバリーキーで開ける鍵スロットがありません".into())
            }
            other => Failure::Other(other.to_string()),
        };
    }
    Failure::Other(error.to_string())
}
