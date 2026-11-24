//! 端末と検証の画面（`D`）：自分の端末一覧と失効、他のユーザーの検証（再検証と絵文字 SAS、QR の表示）、
//! 解錠の手段（鍵スロット）の管理（docs/e2ee.md の「失効と検証の実装（#362）」「絵文字 SAS 検証」。#362、
//! #368）。Web の設定の「暗号化」タブの `DevicesSettings` と `EncryptionSettings` の鍵のセクション、Android の
//! 設定の「暗号化」と同じ機能と文言。
//!
//! - 端末一覧は、端末名、登録日時、最後に使った日時、`device_id` を並べ、今の自己署名鍵の署名がない端末に
//!   「未署名」を付ける。端末の削除は失効ではないと表示し、失効を勧める。
//! - 失効は、削除する端末と範囲を選ぶ、AuthZ で行うことを確かめる、リカバリーキーで SSSS を開け直す、
//!   （1〜5 なら）新しいリカバリーキーを控える、署名し直さない相手を確かめる、再認証して実行する、の順
//!   （`e2ee::revocation`）。
//! - 他のユーザーの検証は、暗号化チャンネルを共有する相手の判定を表示し、再検証（受け入れ）と絵文字 SAS
//!   （`e2ee::verification`）を行う。TUI はカメラを使えないので、相手の QR は読み取らず、この端末の QR を
//!   表示するだけにする。
//! - 鍵スロットは、パスキーとリカバリーキーのスロットを一覧にし、削除と、リカバリーキーの作り直しを行う
//!   （TUI はパスキーを使えないので、パスキーのスロットは作らない）。
//!
//! マスター鍵の秘密鍵と SSSS 鍵は、失効の [`RevocationSession`] と、開け直した SSSS にだけ置き、画面を閉じたら
//! 捨てる（続きは、暗号ストアの記録から SSSS を開け直して行う）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;
use zoo_crypto::{
    KeySlot, RecoveryKey, ResignNote, RevocationScope, RevocationStage, SasPhase, TrustWarning,
    UserState,
};

use super::app::AppEvent;
use crate::api::ApiClient;
use crate::auth::reauth::REAUTH_TIMEOUT;
use crate::config::OidcConfig;
use crate::e2ee::manager::StoreWatch;
use crate::e2ee::reauth;
use crate::e2ee::revocation::{
    self, DeviceView, OpenedSecretStorage, Resumed, RevocationError, RevocationOutcome,
    RevocationSession,
};
use crate::e2ee::setup::SharedStore;
use crate::e2ee::store::{ReverifyKeyChanged, master_public_key};
use crate::e2ee::verification::{
    Action, SasErrorCode, SasView, SaveState, VerificationController, VerificationEvent,
};
use crate::http::ApiError;

/// 他のユーザーの検証状態（Web の `UserVerification`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserVerification {
    pub user_id: String,
    pub state: UserState,
}

/// 鍵スロット 1 つの表示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotView {
    pub id: String,
    /// `passkey` か `recovery`。
    pub kind: String,
    pub label: Option<String>,
    pub created_at: Option<String>,
    pub last_used_at: Option<String>,
}

/// 終わっていない失効の表示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingRevocation {
    pub scope: RevocationScope,
    pub stage: RevocationStage,
}

/// 画面の一覧に出すもの。
#[derive(Debug, Clone, Default)]
pub struct Overview {
    pub devices: Vec<DeviceView>,
    /// 他のユーザーの検証状態（暗号化チャンネルを共有する相手）。
    pub users: Vec<UserVerification>,
    pub warnings: Vec<TrustWarning>,
    /// 検証状態を読めなかったとき（暗号ストアを開けていないなど）の理由。
    pub users_error: Option<String>,
    /// あなたの鍵の指紋（自分のマスター鍵）。
    pub own_master_key: Option<String>,
    /// この端末を解錠したか（失効と再検証は、解錠した端末で行える）。
    pub unlocked: bool,
    pub revocation: Option<PendingRevocation>,
    pub slots: Vec<SlotView>,
}

/// 一覧で選べるもの。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Device(usize),
    User(usize),
    Slot(usize),
}

/// 再検証の確認の画面に出す鍵（Web の `ReverifyPreview`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReverifyPreview {
    pub user_id: String,
    /// サーバーが今返す、相手のマスター鍵の公開鍵。署名はこの鍵にだけ行う。
    pub master_key: String,
    /// この端末の記録のマスター鍵（前に信頼した鍵）。
    pub previous_master_key: Option<String>,
}

/// 再認証のあとに行うこと。
#[derive(Debug, Clone)]
enum AfterReauth {
    /// 失効を実行する（記録した段の続きから）。
    Revoke,
    /// 再検証を保存する。
    Reverify(ReverifyPreview),
    /// 鍵スロットを消す。
    DeleteSlot(String),
    /// 作り直したリカバリーキーのスロットを保存する。
    SaveRecoverySlot(Box<KeySlot>),
}

/// 失効の画面で、失敗のあとにできること。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevokeRetry {
    /// 同じ鍵のまま、再認証して続ける。
    Run,
    /// 記録を捨てて、SSSS を開け直してやり直す（範囲と端末は引き継ぐ）。
    Restart,
}

/// 画面の状態。
pub enum Screen {
    Loading,
    Overview,
    /// 端末名を変える。
    Rename {
        device_id: String,
        input: String,
    },
    /// 端末の削除の確認（削除は失効ではない）。
    ConfirmDelete {
        device_id: String,
    },
    /// 失効：削除する端末と範囲を選ぶ。
    RevokeSelect {
        selected: BTreeSet<String>,
        cursor: usize,
        scope: RevocationScope,
        error: Option<String>,
    },
    /// 失効：AuthZ で行うことを確かめる。
    RevokeAuthz {
        selected: BTreeSet<String>,
        scope: RevocationScope,
    },
    /// 失効：リカバリーキーで SSSS を開け直す。`resume` なら止まった失効を続ける。
    RevokeUnlock {
        resume: bool,
        selected: BTreeSet<String>,
        scope: RevocationScope,
        input: Zeroizing<String>,
        error: Option<String>,
    },
    /// 失効：新しいリカバリーキーを控えてもらう（1〜5）。
    RevokeRecoveryKey(Zeroizing<String>),
    /// 失効：署名し直さない相手を確かめて、実行する。
    RevokeConfirm,
    /// 失効を保存している。
    RevokeWorking(RevocationStage),
    RevokeDone(RevocationOutcome),
    RevokeFailed {
        message: String,
        retry: RevokeRetry,
    },
    /// 再検証の確認（指紋の読み合わせ）。
    ReverifyConfirm(ReverifyPreview),
    /// 鍵スロットの削除の確認。
    ConfirmDeleteSlot {
        slot_id: String,
    },
    /// リカバリーキーの作り直し：今のリカバリーキーで SSSS を開ける。
    SlotUnlock {
        input: Zeroizing<String>,
        error: Option<String>,
    },
    /// リカバリーキーの作り直し：新しいリカバリーキーを控えてもらう。
    SlotRecoveryKey {
        recovery_key: Zeroizing<String>,
        slot: Box<KeySlot>,
    },
    /// 再認証の待ち受け（URL を出す前は `None`）。
    Reauth {
        url: Option<String>,
        port: Option<u16>,
    },
    Working(&'static str),
    /// 結果の表示（`Enter` で一覧に戻る）。
    Message {
        text: String,
        error: bool,
    },
}

/// 裏の処理の結果。
pub enum Outcome {
    Overview(Box<Overview>),
    OverviewFailed(String),
    ReauthWaiting {
        url: String,
        port: u16,
    },
    ReauthFailed(String),
    /// 失効の段が進んだ。
    Stage(RevocationStage),
    /// 失効を始めた（続けた）。
    Revocation(Box<RevocationSession>),
    /// 1〜5 を SSSS の保存の前に止めたので、最初からやり直した。
    RevocationRestarted(Box<RevocationSession>),
    /// 別の端末が SSSS を作り直したので続けられない。
    RevocationConflict {
        revocation_id: String,
        scope: RevocationScope,
        deleted: Vec<String>,
    },
    RevocationFinished(RevocationOutcome),
    /// 失効に失敗した。`session` は、同じ鍵で続けられるときに戻す。
    RevocationFailed {
        error: String,
        retry: RevokeRetry,
        session: Option<Box<RevocationSession>>,
        unlock_error: bool,
        device_only_unavailable: bool,
    },
    ReverifyPreview(ReverifyPreview),
    SlotOpened {
        recovery_key: Zeroizing<String>,
        slot: Box<KeySlot>,
    },
    SlotUnlockFailed(String),
    /// 終わった（結果を出して、一覧を読み直す）。
    Done(String),
    Failed(String),
}

/// 裏の処理から画面に返す結果。`generation` が今の画面のものでなければ捨てる。
pub struct DevicesEvent {
    pub generation: u64,
    pub outcome: Outcome,
}

impl std::fmt::Debug for DevicesEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DevicesEvent(..)")
    }
}

/// 1 つの検証の表示（Web の `SasEntry`）。
#[derive(Debug, Clone)]
pub struct SasEntry {
    pub view: SasView,
    pub busy: bool,
    pub save: SaveState,
    pub error: Option<SasErrorCode>,
    /// 再認証の URL（待っているあいだ）。
    pub reauth: Option<(String, u16)>,
}

/// AuthZ のセッション（`/settings/security`）とサービス接続（`/settings/services`）の画面の URL。issuer と
/// 同じ origin の画面（Web の `accountRevocationUrls`）。
pub fn authz_urls(issuer: &str) -> Option<(String, String)> {
    let url = reqwest::Url::parse(issuer).ok()?;
    let host = url.host_str()?;
    let origin = match url.port() {
        Some(port) => format!("{}://{host}:{port}", url.scheme()),
        None => format!("{}://{host}", url.scheme()),
    };
    Some((
        format!("{origin}/settings/security"),
        format!("{origin}/settings/services"),
    ))
}

/// マスター鍵の指紋（公開鍵の base64 を 4 文字ずつ区切ったもの。Web の `fingerprint`）。
pub fn fingerprint(key: &str) -> String {
    key.chars()
        .collect::<Vec<_>>()
        .chunks(4)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// 端末と検証の画面と、その裏の処理。
pub struct Devices {
    api: ApiClient,
    http: reqwest::Client,
    oidc: Option<OidcConfig>,
    stores: Option<StoreWatch>,
    tx: mpsc::UnboundedSender<AppEvent>,
    pub screen: Screen,
    pub overview: Overview,
    /// 一覧の選択位置（[`items`](Self::items) の番号）。
    pub cursor: usize,
    /// 一覧の表示位置（描画が直す）。
    pub scroll: usize,
    /// 20 台の上限で登録できなかったときの `stale_devices`（端末の管理のタスクから）。
    device_limit: Option<watch::Receiver<Option<Vec<Value>>>>,
    /// 上限のときに選んだ、削除する端末。
    pub limit_selected: BTreeSet<String>,
    session: Option<RevocationSession>,
    /// 失効でやり直すときに引き継ぐ、範囲と削除した端末。
    restart: Option<(RevocationScope, Vec<String>)>,
    /// 失効のために SSSS を開けているときの、続けるか・選んだ端末・範囲（打ち間違いで同じ画面に戻す）。
    unlock_context: Option<(bool, BTreeSet<String>, RevocationScope)>,
    task: Option<JoinHandle<()>>,
    generation: u64,
    pub over_ssh: bool,
    /// 他のユーザーの検証。
    pub sas: Option<VerificationController>,
    pub sas_entries: BTreeMap<String, SasEntry>,
    /// 前面に出している検証。
    pub sas_open: Option<String>,
    /// 直近の描画でリカバリーキーを全部表示できたか。表示できていなければ、控えたこと（`y`）にしない。
    pub recovery_key_shown: bool,
    /// 暗号化チャンネルを共有する相手（開いたときに渡す）。
    peers: Vec<String>,
}

fn status_text(error: &anyhow::Error) -> String {
    if let Some(error) = error.downcast_ref::<RevocationError>() {
        return error.to_string();
    }
    if let Some(error) = error.downcast_ref::<zoo_crypto::CryptoError>() {
        return match error {
            zoo_crypto::CryptoError::InvalidRecoveryKey(_) => {
                "リカバリーキーの形式が違います。打ち間違いがないか確かめてください".into()
            }
            zoo_crypto::CryptoError::KeySlotUnlock | zoo_crypto::CryptoError::SsssKeyMismatch => {
                "リカバリーキーが違います".into()
            }
            zoo_crypto::CryptoError::NoKeySlot => {
                "リカバリーキーで開ける鍵スロットがありません".into()
            }
            other => other.to_string(),
        };
    }
    error.to_string()
}

fn is_unlock_error(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<zoo_crypto::CryptoError>(),
        Some(
            zoo_crypto::CryptoError::InvalidRecoveryKey(_)
                | zoo_crypto::CryptoError::KeySlotUnlock
                | zoo_crypto::CryptoError::SsssKeyMismatch
                | zoo_crypto::CryptoError::NoKeySlot
        )
    )
}

impl Devices {
    pub fn new(api: ApiClient, http: reqwest::Client, tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self {
            api,
            http,
            oidc: None,
            stores: None,
            tx,
            screen: Screen::Loading,
            overview: Overview::default(),
            cursor: 0,
            scroll: 0,
            device_limit: None,
            limit_selected: BTreeSet::new(),
            session: None,
            restart: None,
            unlock_context: None,
            task: None,
            generation: 0,
            over_ssh: reauth::over_ssh(),
            sas: None,
            sas_entries: BTreeMap::new(),
            sas_open: None,
            recovery_key_shown: false,
            peers: Vec::new(),
        }
    }

    /// 端末の管理のタスクと OIDC の設定をつなぐ。`device_limit` は 20 台の上限の知らせ。
    pub fn attach(
        &mut self,
        oidc: Option<OidcConfig>,
        stores: StoreWatch,
        device_limit: Option<watch::Receiver<Option<Vec<Value>>>>,
    ) {
        let tx = self.tx.clone();
        let notify: crate::e2ee::verification::Notify = Arc::new(move |event| {
            let _ = tx.send(AppEvent::Verification(event));
        });
        self.sas = Some(VerificationController::new(
            self.api.clone(),
            self.http.clone(),
            oidc.clone(),
            stores.clone(),
            notify,
        ));
        self.oidc = oidc;
        self.stores = Some(stores);
        self.device_limit = device_limit;
    }

    fn store(&self) -> Option<(String, SharedStore)> {
        self.stores.as_ref().and_then(|rx| rx.borrow().clone())
    }

    /// 20 台の上限で登録できなかったときの `stale_devices`。
    pub fn device_limit(&self) -> Option<Vec<Value>> {
        self.device_limit
            .as_ref()
            .and_then(|rx| rx.borrow().clone())
    }

    /// AuthZ の画面の URL（OIDC の設定がなければ `None`）。
    pub fn authz_urls(&self) -> Option<(String, String)> {
        authz_urls(&self.oidc.as_ref()?.issuer)
    }

    fn next_generation(&mut self) -> u64 {
        if let Some(task) = self.task.take() {
            task.abort();
        }
        self.generation += 1;
        self.generation
    }

    fn spawn<F>(&mut self, future: F)
    where
        F: std::future::Future<Output = Outcome> + Send + 'static,
    {
        let generation = self.generation;
        let tx = self.tx.clone();
        self.task = Some(tokio::spawn(async move {
            let outcome = future.await;
            let _ = tx.send(AppEvent::Devices(DevicesEvent {
                generation,
                outcome,
            }));
        }));
    }

    fn sender(&self) -> impl Fn(Outcome) + Send + Sync + 'static {
        let generation = self.generation;
        let tx = self.tx.clone();
        move |outcome| {
            let _ = tx.send(AppEvent::Devices(DevicesEvent {
                generation,
                outcome,
            }));
        }
    }

    /// 画面を開く。`peers` は暗号化チャンネルを共有する相手。
    pub fn open(&mut self, peers: Vec<String>) {
        self.peers = peers;
        self.cursor = 0;
        self.scroll = 0;
        if let Some(stale) = self.device_limit() {
            // 上限のときは、30 日以上使っていない端末を選んだ状態にする。
            self.limit_selected = stale
                .iter()
                .filter_map(|device| device.get("device_id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect();
        }
        self.reload();
    }

    /// 画面を閉じる。メモリーの秘密を捨てる（続きは SSSS を開け直して行う）。
    pub fn close(&mut self) {
        self.next_generation();
        self.session = None;
        self.restart = None;
        self.screen = Screen::Loading;
    }

    /// 一覧を読み直す。
    pub fn reload(&mut self) {
        let generation = self.next_generation();
        let _ = generation;
        self.session = None;
        self.screen = Screen::Loading;
        let api = self.api.clone();
        let user_store = self.store();
        let peers = self.peers.clone();
        let user_id = self.user_id_hint();
        self.spawn(async move {
            match load_overview(&api, user_store, user_id, peers).await {
                Ok(overview) => Outcome::Overview(Box::new(overview)),
                Err(error) => Outcome::OverviewFailed(status_text(&error)),
            }
        });
    }

    fn user_id_hint(&self) -> Option<String> {
        self.store().map(|(user_id, _)| user_id)
    }

    /// 一覧で選べるもの（端末、他のユーザー、鍵スロットの順）。
    pub fn items(&self) -> Vec<Item> {
        let overview = &self.overview;
        (0..overview.devices.len())
            .map(Item::Device)
            .chain((0..overview.users.len()).map(Item::User))
            .chain((0..overview.slots.len()).map(Item::Slot))
            .collect()
    }

    fn selected_item(&self) -> Option<Item> {
        self.items().get(self.cursor).copied()
    }

    pub fn on_event(&mut self, event: DevicesEvent) {
        if event.generation != self.generation {
            return;
        }
        match event.outcome {
            Outcome::Overview(overview) => {
                self.overview = *overview;
                let count = self.items().len();
                self.cursor = self.cursor.min(count.saturating_sub(1));
                self.screen = Screen::Overview;
            }
            Outcome::OverviewFailed(message) => {
                self.screen = Screen::Message {
                    text: format!("一覧を読めません: {message}"),
                    error: true,
                };
            }
            Outcome::ReauthWaiting { url, port } => {
                if !self.over_ssh {
                    reauth::open_browser(&url);
                }
                self.screen = Screen::Reauth {
                    url: Some(url),
                    port: Some(port),
                };
            }
            Outcome::ReauthFailed(message) => {
                self.screen = match self.session {
                    Some(_) => Screen::RevokeFailed {
                        message: format!("再認証できませんでした: {message}"),
                        retry: RevokeRetry::Run,
                    },
                    None => Screen::Message {
                        text: format!("再認証できませんでした: {message}"),
                        error: true,
                    },
                };
            }
            Outcome::Stage(stage) => self.screen = Screen::RevokeWorking(stage),
            Outcome::Revocation(session) => {
                self.session = Some(*session);
                self.after_begin();
            }
            Outcome::RevocationRestarted(session) => {
                self.session = Some(*session);
                self.after_begin();
            }
            Outcome::RevocationConflict {
                revocation_id,
                scope,
                deleted,
            } => {
                let _ = revocation_id;
                self.restart = Some((scope, deleted));
                self.screen = Screen::RevokeFailed {
                    message: "別の端末の失効と入り混じりました。SSSS を開け直して、失効をやり直してください（消していないバックアップの鍵は引き継ぎます）。".into(),
                    retry: RevokeRetry::Restart,
                };
            }
            Outcome::RevocationFinished(outcome) => {
                self.session = None;
                self.restart = None;
                self.screen = Screen::RevokeDone(outcome);
            }
            Outcome::RevocationFailed {
                error,
                retry,
                session,
                unlock_error,
                device_only_unavailable,
            } => {
                if let Some(session) = session {
                    self.session = Some(*session);
                }
                if device_only_unavailable {
                    let selected = self.restart.take().map(|(_, deleted)| deleted);
                    self.screen = Screen::RevokeSelect {
                        selected: selected.unwrap_or_default().into_iter().collect(),
                        cursor: 0,
                        scope: RevocationScope::Full,
                        error: Some("別の端末が SSSS の鍵を作り直したので、端末だけの失効は行えません。範囲を「パスキー、リカバリーキー、または SSSS の鍵」に切り替えたので、確かめて進めてください。".into()),
                    };
                    return;
                }
                if unlock_error && let Some((resume, selected, scope)) = self.unlock_context.take()
                {
                    // リカバリーキーの打ち間違い。打ち直せる。
                    self.screen = Screen::RevokeUnlock {
                        resume,
                        selected,
                        scope,
                        input: Zeroizing::new(String::new()),
                        error: Some(error),
                    };
                    return;
                }
                self.screen = Screen::RevokeFailed {
                    message: error,
                    retry,
                };
            }
            Outcome::ReverifyPreview(preview) => self.screen = Screen::ReverifyConfirm(preview),
            Outcome::SlotOpened { recovery_key, slot } => {
                self.recovery_key_shown = false;
                self.screen = Screen::SlotRecoveryKey { recovery_key, slot };
            }
            Outcome::SlotUnlockFailed(message) => {
                self.screen = Screen::SlotUnlock {
                    input: Zeroizing::new(String::new()),
                    error: Some(message),
                };
            }
            Outcome::Done(text) => {
                self.screen = Screen::Message { text, error: false };
            }
            Outcome::Failed(text) => {
                self.screen = Screen::Message { text, error: true };
            }
        }
    }

    /// 失効を始めた（続けた）あと：1〜5 で新しいリカバリーキーをまだ作っていなければ作って控えてもらい、
    /// そうでなければ確かめる画面に進む。
    fn after_begin(&mut self) {
        let user_id = self.store().map(|(user, _)| user).unwrap_or_default();
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let full_before_storage = session.state().scope == RevocationScope::Full
            && session.state().stage < RevocationStage::SecretStorageSaved;
        if full_before_storage && !session.has_recovery_slot() {
            match session.new_recovery_key(&user_id) {
                Ok(key) => {
                    self.recovery_key_shown = false;
                    self.screen = Screen::RevokeRecoveryKey(key);
                }
                Err(error) => {
                    self.screen = Screen::RevokeFailed {
                        message: error.to_string(),
                        retry: RevokeRetry::Restart,
                    }
                }
            }
            return;
        }
        self.screen = Screen::RevokeConfirm;
    }

    /// 進めている失効の状態（確かめる画面に出す）。
    pub fn session_state(&self) -> Option<&zoo_crypto::RevocationState> {
        self.session.as_ref().map(RevocationSession::state)
    }

    /// 他のユーザーの検証の知らせ。届いた要求と自分から始めた検証は前面に出す（`true` を返す）。
    pub fn on_verification(&mut self, event: VerificationEvent) -> bool {
        match event {
            VerificationEvent::View(view) => {
                let txn = view.transaction_id.clone();
                let fresh = !self.sas_entries.contains_key(&txn);
                let entry = self.sas_entries.entry(txn.clone()).or_insert(SasEntry {
                    view: view.clone(),
                    busy: false,
                    save: SaveState::Idle,
                    error: None,
                    reauth: None,
                });
                entry.view = view;
                if fresh && !entry.view.finished() {
                    if self.sas_open.is_none() {
                        self.sas_open = Some(txn);
                    }
                    return true;
                }
            }
            VerificationEvent::Busy {
                transaction_id,
                busy,
            } => {
                if let Some(entry) = self.sas_entries.get_mut(&transaction_id) {
                    entry.busy = busy;
                }
            }
            VerificationEvent::Error {
                transaction_id,
                error,
            } => {
                if let Some(entry) = self.sas_entries.get_mut(&transaction_id) {
                    entry.error = error;
                }
            }
            VerificationEvent::Save {
                transaction_id,
                state,
            } => {
                if let Some(entry) = self.sas_entries.get_mut(&transaction_id) {
                    entry.save = state;
                    if state != SaveState::Saving {
                        entry.reauth = None;
                    }
                }
            }
            VerificationEvent::ReauthWaiting {
                transaction_id,
                url,
                port,
            } => {
                if !self.over_ssh {
                    reauth::open_browser(&url);
                }
                if let Some(entry) = self.sas_entries.get_mut(&transaction_id) {
                    entry.reauth = Some((url, port));
                }
            }
            VerificationEvent::RequestFailed { user_id, error } => {
                let _ = user_id;
                self.screen = Screen::Message {
                    text: match error {
                        SasErrorCode::NeedsUnlock => "この端末は解錠していないため、検証できません。設定の「暗号化」で解錠してください。".into(),
                        _ => "絵文字の検証を始められませんでした。相手が暗号化を設定しているか確かめて、もう一度試してください。".into(),
                    },
                    error: true,
                };
            }
            VerificationEvent::Saved { .. } => {
                if matches!(self.screen, Screen::Overview) {
                    self.reload();
                }
            }
        }
        false
    }

    /// 定期的な処理（検証の期限切れと送り直し）。
    pub fn on_tick(&mut self) {
        if let Some(sas) = &self.sas {
            sas.tick();
        }
    }

    /// 前面の検証へのキー操作。閉じたら `sas_open` を外す。
    fn on_sas_key(&mut self, key: KeyEvent) {
        let Some(txn) = self.sas_open.clone() else {
            return;
        };
        let Some(entry) = self.sas_entries.get(&txn).cloned() else {
            self.sas_open = None;
            return;
        };
        let Some(sas) = self.sas.clone() else {
            return;
        };
        if key.code == KeyCode::Esc {
            if entry.view.finished() {
                sas.dismiss(&txn);
                self.sas_entries.remove(&txn);
            } else {
                sas.act(&txn, Action::Cancel);
            }
            if entry.view.finished() {
                self.sas_open = self.sas_entries.keys().next().cloned().filter(|other| {
                    self.sas_entries
                        .get(other)
                        .is_some_and(|e| !e.view.finished())
                });
            }
            return;
        }
        if entry.busy {
            return;
        }
        match (entry.view.phase, key.code) {
            (SasPhase::Incoming, KeyCode::Char('y' | 'Y')) => sas.act(&txn, Action::Accept),
            (SasPhase::Incoming, KeyCode::Char('n' | 'N')) => sas.act(&txn, Action::Cancel),
            (SasPhase::Requested | SasPhase::ChooseMethod, KeyCode::Char('s' | 'S'))
                if entry.view.can_start_sas =>
            {
                sas.act(&txn, Action::StartSas)
            }
            (SasPhase::Compare, KeyCode::Char('y' | 'Y')) => sas.act(&txn, Action::Confirm),
            (SasPhase::Compare, KeyCode::Char('n' | 'N')) => sas.act(&txn, Action::Mismatch),
            (SasPhase::QrConfirm, KeyCode::Char('y' | 'Y')) => sas.act(&txn, Action::ConfirmQr),
            (SasPhase::QrConfirm, KeyCode::Char('n' | 'N')) => sas.act(&txn, Action::Cancel),
            (SasPhase::Done, KeyCode::Enter)
                if matches!(
                    entry.save,
                    SaveState::Reauth | SaveState::Login | SaveState::Failed
                ) =>
            {
                sas.retry_save(&txn)
            }
            (SasPhase::Done | SasPhase::Cancelled, KeyCode::Enter) => {
                sas.dismiss(&txn);
                self.sas_entries.remove(&txn);
                self.sas_open = None;
            }
            _ => {}
        }
    }

    /// 前面に出す検証があるか（どの画面にいても、キーはまずこちらに渡す）。
    pub fn has_open_verification(&self) -> bool {
        self.sas_open
            .as_ref()
            .is_some_and(|txn| self.sas_entries.contains_key(txn))
    }

    /// 前面の検証へのキー操作（画面が閉じていても使う）。
    pub fn on_verification_key(&mut self, key: KeyEvent) {
        self.on_sas_key(key);
    }

    /// キー操作。画面を閉じるときは `false` を返す。
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        if self.has_open_verification() {
            self.on_sas_key(key);
            return true;
        }
        if key.code == KeyCode::Esc {
            match &self.screen {
                Screen::Overview | Screen::Loading | Screen::Message { .. } => {
                    self.close();
                    return false;
                }
                Screen::Reauth { .. } if self.session.is_some() => {
                    // 再認証をやめても、失効の鍵は捨てない。同じ鍵のまま、もう一度実行できる。
                    self.next_generation();
                    self.screen = Screen::RevokeFailed {
                        message: "再認証を取りやめました".into(),
                        retry: RevokeRetry::Run,
                    };
                    return true;
                }
                _ => {
                    // 失効の途中なら、記録は暗号ストアに残る（「失効を続ける」から続けられる）。
                    self.reload();
                    return true;
                }
            }
        }
        let screen = std::mem::replace(&mut self.screen, Screen::Loading);
        self.screen = self.on_screen_key(screen, key);
        true
    }

    fn on_screen_key(&mut self, screen: Screen, key: KeyEvent) -> Screen {
        match screen {
            Screen::Overview => self.on_overview_key(key),
            Screen::Rename {
                device_id,
                mut input,
            } => match key.code {
                KeyCode::Enter => {
                    let name = input.trim().to_owned();
                    if name.is_empty() {
                        return Screen::Rename { device_id, input };
                    }
                    self.next_generation();
                    let api = self.api.clone();
                    self.spawn(async move {
                        match api.e2ee_rename_device(&device_id, &name).await {
                            Ok(()) => Outcome::Done("端末名を変えました。".into()),
                            Err(error) => {
                                Outcome::Failed(format!("端末名を変えられませんでした: {error}"))
                            }
                        }
                    });
                    Screen::Working("保存しています…")
                }
                KeyCode::Backspace => {
                    input.pop();
                    Screen::Rename { device_id, input }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if input.chars().count() < 100 {
                        input.push(c);
                    }
                    Screen::Rename { device_id, input }
                }
                _ => Screen::Rename { device_id, input },
            },
            Screen::ConfirmDelete { device_id } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.next_generation();
                    let api = self.api.clone();
                    self.spawn(async move {
                        match api.e2ee_delete_device(&device_id).await {
                            Ok(()) => Outcome::Done("端末を削除しました。".into()),
                            Err(error) => {
                                Outcome::Failed(format!("端末を削除できませんでした: {error}"))
                            }
                        }
                    });
                    Screen::Working("削除しています…")
                } else if key.code == KeyCode::Char('R') {
                    self.start_revocation()
                } else {
                    Screen::Overview
                }
            }
            Screen::RevokeSelect {
                mut selected,
                mut cursor,
                mut scope,
                error,
            } => {
                let others: Vec<String> = self
                    .overview
                    .devices
                    .iter()
                    .filter(|d| !d.current)
                    .map(|d| d.device_id.clone())
                    .collect();
                match key.code {
                    KeyCode::Char('j') | KeyCode::Down => {
                        cursor = (cursor + 1).min(others.len().saturating_sub(1));
                    }
                    KeyCode::Char('k') | KeyCode::Up => cursor = cursor.saturating_sub(1),
                    KeyCode::Char(' ') => {
                        if let Some(id) = others.get(cursor)
                            && !selected.remove(id)
                        {
                            selected.insert(id.clone());
                        }
                    }
                    KeyCode::Char('t') => {
                        scope = match scope {
                            RevocationScope::Full => RevocationScope::DeviceOnly,
                            RevocationScope::DeviceOnly => RevocationScope::Full,
                        };
                    }
                    KeyCode::Enter => return Screen::RevokeAuthz { selected, scope },
                    _ => {}
                }
                Screen::RevokeSelect {
                    selected,
                    cursor,
                    scope,
                    error,
                }
            }
            Screen::RevokeAuthz { selected, scope } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    Screen::RevokeUnlock {
                        resume: false,
                        selected,
                        scope,
                        input: Zeroizing::new(String::new()),
                        error: None,
                    }
                } else {
                    Screen::RevokeAuthz { selected, scope }
                }
            }
            Screen::RevokeUnlock {
                resume,
                selected,
                scope,
                mut input,
                error,
            } => match key.code {
                KeyCode::Enter => {
                    let recovery_key = std::mem::take(&mut input);
                    self.unlock_for_revocation(resume, selected, scope, recovery_key)
                }
                KeyCode::Backspace => {
                    input.pop();
                    Screen::RevokeUnlock {
                        resume,
                        selected,
                        scope,
                        input,
                        error,
                    }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear();
                    Screen::RevokeUnlock {
                        resume,
                        selected,
                        scope,
                        input,
                        error,
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.push(c);
                    Screen::RevokeUnlock {
                        resume,
                        selected,
                        scope,
                        input,
                        error,
                    }
                }
                _ => Screen::RevokeUnlock {
                    resume,
                    selected,
                    scope,
                    input,
                    error,
                },
            },
            Screen::RevokeRecoveryKey(key_text) => {
                if self.recovery_key_shown && matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    Screen::RevokeConfirm
                } else {
                    Screen::RevokeRecoveryKey(key_text)
                }
            }
            Screen::RevokeConfirm => {
                if key.code == KeyCode::Enter {
                    self.reauth_then(AfterReauth::Revoke)
                } else {
                    Screen::RevokeConfirm
                }
            }
            Screen::RevokeFailed { message, retry } => {
                if key.code != KeyCode::Enter {
                    return Screen::RevokeFailed { message, retry };
                }
                match retry {
                    RevokeRetry::Run if self.session.is_some() => {
                        self.reauth_then(AfterReauth::Revoke)
                    }
                    _ => self.restart_revocation(),
                }
            }
            Screen::RevokeDone(outcome) => {
                if key.code == KeyCode::Enter {
                    self.reload();
                    Screen::Loading
                } else {
                    Screen::RevokeDone(outcome)
                }
            }
            Screen::ReverifyConfirm(preview) => {
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.reauth_then(AfterReauth::Reverify(preview))
                } else if key.code == KeyCode::Char('v') {
                    // 絵文字で検証できるときは、そちらに切り替えられる。
                    if let Some(sas) = &self.sas {
                        sas.request(preview.user_id.clone());
                    }
                    Screen::Overview
                } else {
                    Screen::ReverifyConfirm(preview)
                }
            }
            Screen::ConfirmDeleteSlot { slot_id } => {
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.reauth_then(AfterReauth::DeleteSlot(slot_id))
                } else {
                    Screen::Overview
                }
            }
            Screen::SlotUnlock { mut input, error } => match key.code {
                KeyCode::Enter => {
                    let recovery_key = std::mem::take(&mut input);
                    self.open_for_recovery_slot(recovery_key)
                }
                KeyCode::Backspace => {
                    input.pop();
                    Screen::SlotUnlock { input, error }
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.clear();
                    Screen::SlotUnlock { input, error }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    input.push(c);
                    Screen::SlotUnlock { input, error }
                }
                _ => Screen::SlotUnlock { input, error },
            },
            Screen::SlotRecoveryKey { recovery_key, slot } => {
                if self.recovery_key_shown && matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.reauth_then(AfterReauth::SaveRecoverySlot(slot))
                } else {
                    Screen::SlotRecoveryKey { recovery_key, slot }
                }
            }
            Screen::Message { text, error } => {
                if key.code == KeyCode::Enter {
                    self.reload();
                    Screen::Loading
                } else {
                    Screen::Message { text, error }
                }
            }
            other => other,
        }
    }

    fn on_overview_key(&mut self, key: KeyEvent) -> Screen {
        let count = self.items().len();
        let item = self.selected_item();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(count.saturating_sub(1));
            }
            KeyCode::Char('k') | KeyCode::Up => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => self.cursor = count.saturating_sub(1),
            KeyCode::Char('r') => {
                self.reload();
                return Screen::Loading;
            }
            KeyCode::Char('R') => return self.start_revocation(),
            KeyCode::Char(' ') if self.device_limit().is_some() => {
                if let Some(Item::Device(index)) = item
                    && let Some(device) = self.overview.devices.get(index)
                    && !device.current
                    && !self.limit_selected.remove(&device.device_id)
                {
                    self.limit_selected.insert(device.device_id.clone());
                }
            }
            KeyCode::Char('X') if self.device_limit().is_some() => {
                let ids: Vec<String> = self.limit_selected.iter().cloned().collect();
                if ids.is_empty() {
                    return Screen::Overview;
                }
                self.next_generation();
                let api = self.api.clone();
                self.spawn(async move {
                    match revocation::delete_devices(&api, &ids).await {
                        Ok(()) => {
                            Outcome::Done("削除しました。まもなくこの端末を登録し直します。".into())
                        }
                        Err(error) => {
                            Outcome::Failed(format!("端末を削除できませんでした: {error}"))
                        }
                    }
                });
                self.limit_selected.clear();
                return Screen::Working("削除しています…");
            }
            KeyCode::Char('n') => {
                if let Some(Item::Device(index)) = item
                    && let Some(device) = self.overview.devices.get(index)
                {
                    return Screen::Rename {
                        device_id: device.device_id.clone(),
                        input: device.display_name.clone().unwrap_or_default(),
                    };
                }
            }
            KeyCode::Char('x') => match item {
                Some(Item::Device(index)) => {
                    if let Some(device) = self.overview.devices.get(index)
                        && !device.current
                    {
                        return Screen::ConfirmDelete {
                            device_id: device.device_id.clone(),
                        };
                    }
                }
                Some(Item::Slot(index)) => {
                    if let Some(slot) = self.overview.slots.get(index) {
                        return Screen::ConfirmDeleteSlot {
                            slot_id: slot.id.clone(),
                        };
                    }
                }
                _ => {}
            },
            KeyCode::Char('v') => {
                if let Some(Item::User(index)) = item
                    && let Some(user) = self.overview.users.get(index)
                    && let Some(sas) = &self.sas
                {
                    sas.request(user.user_id.clone());
                }
            }
            KeyCode::Char('a') => {
                if let Some(Item::User(index)) = item
                    && let Some(user) = self.overview.users.get(index)
                    && matches!(user.state, UserState::NeedsReverification(_))
                {
                    return self.preview_reverify(user.user_id.clone());
                }
            }
            KeyCode::Char('K') => {
                return Screen::SlotUnlock {
                    input: Zeroizing::new(String::new()),
                    error: None,
                };
            }
            _ => {}
        }
        Screen::Overview
    }

    /// 失効を始める（終わっていない失効があれば、続ける）。
    fn start_revocation(&mut self) -> Screen {
        if !self.overview.unlocked {
            return Screen::Message {
                text: "失効と再検証は、暗号化を解錠した端末で行えます。".into(),
                error: true,
            };
        }
        if let Some(pending) = &self.overview.revocation {
            return Screen::RevokeUnlock {
                resume: true,
                selected: BTreeSet::new(),
                scope: pending.scope,
                input: Zeroizing::new(String::new()),
                error: None,
            };
        }
        Screen::RevokeSelect {
            selected: BTreeSet::new(),
            cursor: 0,
            scope: RevocationScope::Full,
            error: None,
        }
    }

    /// 記録を捨てて、同じ範囲と端末でやり直す（端末は削除済みなので、選び直さない）。
    fn restart_revocation(&mut self) -> Screen {
        let (scope, deleted) = self
            .restart
            .take()
            .or_else(|| {
                self.session
                    .as_ref()
                    .map(|s| (s.state().scope, s.state().deleted_devices.clone()))
            })
            .unwrap_or((RevocationScope::Full, Vec::new()));
        let expected = self.session.as_ref().map(RevocationSession::revocation_id);
        self.session = None;
        if let Some((_, store)) = self.store() {
            let expected = expected.clone();
            tokio::spawn(async move {
                let _ = revocation::abandon(&store, expected.as_deref()).await;
            });
        }
        self.overview.revocation = None;
        Screen::RevokeUnlock {
            resume: false,
            selected: deleted.into_iter().collect(),
            scope,
            input: Zeroizing::new(String::new()),
            error: None,
        }
    }

    /// SSSS を開け直し、失効を始める（続ける）。始めるときは、選んだ端末を先に削除する（手順 1）。
    fn unlock_for_revocation(
        &mut self,
        resume: bool,
        selected: BTreeSet<String>,
        scope: RevocationScope,
        recovery_key: Zeroizing<String>,
    ) -> Screen {
        self.next_generation();
        let Some((user_id, store)) = self.store() else {
            return Screen::Message {
                text: "暗号ストアを開けていません".into(),
                error: true,
            };
        };
        // リカバリーキーを打ち間違えたら、同じ画面に戻して打ち直してもらう。
        self.unlock_context = Some((resume, selected.clone(), scope));
        let api = self.api.clone();
        let deleted: Vec<String> = selected.into_iter().collect();
        self.spawn(async move {
            let failed = |error: anyhow::Error, retry| Outcome::RevocationFailed {
                unlock_error: is_unlock_error(&error),
                device_only_unavailable: matches!(
                    error.downcast_ref::<RevocationError>(),
                    Some(RevocationError::ServerKeyMismatch(_))
                ) && scope == RevocationScope::DeviceOnly,
                error: status_text(&error),
                retry,
                session: None,
            };
            let opened = match revocation::open_secret_storage(&api, &user_id, &recovery_key).await
            {
                Ok(opened) => opened,
                Err(error) => return failed(error, RevokeRetry::Restart),
            };
            if resume {
                return match revocation::resume(&store, opened).await {
                    Ok(Resumed::Resumed(session)) => Outcome::Revocation(session),
                    Ok(Resumed::Restart {
                        scope,
                        deleted_devices,
                        opened,
                    }) => begin(&api, &store, &user_id, scope, deleted_devices, &opened)
                        .await
                        .map_or_else(
                            |error| failed(error, RevokeRetry::Restart),
                            |session| Outcome::RevocationRestarted(Box::new(session)),
                        ),
                    Ok(Resumed::Conflict {
                        revocation_id,
                        scope,
                        deleted_devices,
                    }) => Outcome::RevocationConflict {
                        revocation_id,
                        scope,
                        deleted: deleted_devices,
                    },
                    Err(error) => failed(error, RevokeRetry::Restart),
                };
            }
            // 手順 1：選んだ端末を削除する（端末の削除は失効ではない。続けて鍵を作り直す）。
            if let Err(error) = revocation::delete_devices(&api, &deleted).await {
                return failed(error, RevokeRetry::Restart);
            }
            match begin(&api, &store, &user_id, scope, deleted, &opened).await {
                Ok(session) => Outcome::Revocation(Box::new(session)),
                Err(error) => failed(error, RevokeRetry::Restart),
            }
        });
        Screen::Working("解錠しています…")
    }

    fn preview_reverify(&mut self, user_id: String) -> Screen {
        self.next_generation();
        let Some((_, store)) = self.store() else {
            return Screen::Message {
                text: "暗号ストアを開けていません".into(),
                error: true,
            };
        };
        let api = self.api.clone();
        self.spawn(async move {
            let result = async {
                let keys = api.e2ee_query_keys(std::slice::from_ref(&user_id)).await?;
                let master_key = keys
                    .get("master_keys")
                    .and_then(|keys| keys.get(&user_id))
                    .and_then(master_public_key)
                    .ok_or_else(|| anyhow::anyhow!("相手のマスター鍵がありません"))?;
                let previous_master_key = store
                    .lock()
                    .await
                    .trust_record(&user_id)
                    .map(|record| record.master_key);
                anyhow::Ok(ReverifyPreview {
                    user_id,
                    master_key,
                    previous_master_key,
                })
            }
            .await;
            match result {
                Ok(preview) => Outcome::ReverifyPreview(preview),
                Err(error) => Outcome::Failed(format!(
                    "再検証を保存できませんでした。もう一度試してください。（{error}）"
                )),
            }
        });
        Screen::Working("鍵を読んでいます…")
    }

    fn open_for_recovery_slot(&mut self, recovery_key: Zeroizing<String>) -> Screen {
        self.next_generation();
        let Some((user_id, _)) = self.store() else {
            return Screen::Message {
                text: "暗号ストアを開けていません".into(),
                error: true,
            };
        };
        let api = self.api.clone();
        self.spawn(async move {
            let result = async {
                let opened = revocation::open_secret_storage(&api, &user_id, &recovery_key).await?;
                let recovery = RecoveryKey::generate();
                let slot = KeySlot::new_recovery(&user_id, opened.0.key(), &recovery)?;
                anyhow::Ok((recovery.to_display(), slot))
            }
            .await;
            match result {
                Ok((recovery_key, slot)) => Outcome::SlotOpened {
                    recovery_key,
                    slot: Box::new(slot),
                },
                Err(error) => Outcome::SlotUnlockFailed(status_text(&error)),
            }
        });
        Screen::Working("解錠しています…")
    }

    /// 保存の直前に再認証し（dev モードでは再認証しない）、`after` を行う。
    fn reauth_then(&mut self, after: AfterReauth) -> Screen {
        self.next_generation();
        let api = self.api.clone();
        let http = self.http.clone();
        let oidc = self.oidc.clone();
        let stores = self.stores.clone();
        let send = self.sender();
        let mut session = match after {
            AfterReauth::Revoke => self.session.take(),
            _ => None,
        };
        let screen = match oidc {
            Some(_) => Screen::Reauth {
                url: None,
                port: None,
            },
            None => Screen::Working("保存しています…"),
        };
        self.spawn(async move {
            let token = match &oidc {
                None => None,
                Some(oidc) => {
                    let token = async {
                        let (pending, discovery) = reauth::start(&http, oidc).await?;
                        send(Outcome::ReauthWaiting {
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
                            if let Some(session) = session.take() {
                                return Outcome::RevocationFailed {
                                    error: format!("再認証できませんでした: {error}"),
                                    retry: RevokeRetry::Run,
                                    session: Some(Box::new(session)),
                                    unlock_error: false,
                                    device_only_unavailable: false,
                                };
                            }
                            return Outcome::ReauthFailed(error.to_string());
                        }
                    }
                }
            };
            let token = token.as_deref().map(String::as_str);
            // 再認証を待つ間は暗号ストアを持たない（端末の管理のタスクが開き直せるように）。
            let Some((user_id, store)) = stores.and_then(|rx| rx.borrow().clone()) else {
                return Outcome::Failed("暗号ストアを開けていません".into());
            };
            match after {
                AfterReauth::Revoke => {
                    let Some(mut session) = session else {
                        return Outcome::Failed("続ける失効がありません".into());
                    };
                    let progress_send = send;
                    let progress = move |stage| progress_send(Outcome::Stage(stage));
                    send_stage_start(&progress, session.state().stage);
                    match revocation::run(&api, &store, &user_id, &mut session, token, &progress)
                        .await
                    {
                        Ok(outcome) => Outcome::RevocationFinished(outcome),
                        Err(error) => {
                            let (message, retry) = match error.downcast_ref::<RevocationError>() {
                                Some(RevocationError::ReauthRequired) => (
                                    "再認証が必要です（ログインし直してから時間がたちました）。同じ鍵のまま、もう一度実行してください".into(),
                                    RevokeRetry::Run,
                                ),
                                Some(RevocationError::Conflict(_))
                                | Some(RevocationError::ServerKeyMismatch(_)) => (
                                    "別の端末の失効と入り混じりました。SSSS を開け直して、失効をやり直してください（消していないバックアップの鍵は引き継ぎます）。".into(),
                                    RevokeRetry::Restart,
                                ),
                                _ => (
                                    format!("失効を最後まで保存できませんでした。進み具合はこの端末に残っています。もう一度実行してください。（{}）", status_text(&error)),
                                    RevokeRetry::Run,
                                ),
                            };
                            Outcome::RevocationFailed {
                                error: message,
                                retry,
                                session: Some(Box::new(session)),
                                unlock_error: false,
                                device_only_unavailable: false,
                            }
                        }
                    }
                }
                AfterReauth::Reverify(preview) => {
                    match reverify(&api, &store, &preview, token).await {
                        Ok(()) => Outcome::Done("再検証を保存しました。".into()),
                        Err(error) if error.downcast_ref::<ReverifyKeyChanged>().is_some() => {
                            Outcome::Failed("確認したあとに、相手の鍵がまた変わりました。何も保存していません。もう一度「再検証する」から確かめてください。".into())
                        }
                        Err(error) => Outcome::Failed(format!(
                            "再検証を保存できませんでした。もう一度試してください。（{}）",
                            status_text(&error)
                        )),
                    }
                }
                AfterReauth::DeleteSlot(slot_id) => {
                    match api.e2ee_delete_key_slot(&slot_id, token).await {
                        Ok(()) => Outcome::Done("鍵スロットを削除しました。".into()),
                        Err(error) => Outcome::Failed(format!(
                            "鍵スロットを削除できませんでした（最後の 1 つは削除できません）: {error}"
                        )),
                    }
                }
                AfterReauth::SaveRecoverySlot(slot) => {
                    let body = match serde_json::to_value(&*slot) {
                        Ok(body) => body,
                        Err(error) => return Outcome::Failed(error.to_string()),
                    };
                    match api.e2ee_add_key_slot(body, token).await {
                        Ok(()) => Outcome::Done("リカバリーキーを作り直しました。古いリカバリーキーでは解錠できません。".into()),
                        Err(error) if ApiError::has_code(&error, "E2EE_SSSS_CONFLICT") => {
                            Outcome::Failed("別の端末が SSSS を作り直しました。今のリカバリーキーで開け直してから、もう一度作り直してください。".into())
                        }
                        Err(error) => Outcome::Failed(format!(
                            "リカバリーキーを保存できませんでした: {error}"
                        )),
                    }
                }
            }
        });
        screen
    }

    /// 貼り付け（リカバリーキーと端末名の入力欄だけ）。
    pub fn on_paste(&mut self, text: &str) {
        match &mut self.screen {
            Screen::RevokeUnlock { input, .. } | Screen::SlotUnlock { input, .. } => {
                input.push_str(text)
            }
            Screen::Rename { input, .. } => input.push_str(text.trim()),
            _ => {}
        }
    }
}

fn send_stage_start(progress: &(dyn Fn(RevocationStage) + Sync), stage: RevocationStage) {
    progress(stage);
}

async fn begin(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    scope: RevocationScope,
    deleted: Vec<String>,
    opened: &OpenedSecretStorage,
) -> anyhow::Result<RevocationSession> {
    revocation::begin(api, store, user_id, scope, deleted, opened).await
}

/// 利用者が相手の今のマスター鍵を受け入れる（再検証）。確認の画面に出した鍵と、署名する直前の鍵が同じとき
/// だけ署名する（セキュリティレビュー M-2）。
async fn reverify(
    api: &ApiClient,
    store: &SharedStore,
    preview: &ReverifyPreview,
    reauth: Option<&str>,
) -> anyhow::Result<()> {
    let keys = api
        .e2ee_query_keys(std::slice::from_ref(&preview.user_id))
        .await?;
    let master_key = keys
        .get("master_keys")
        .and_then(|keys| keys.get(&preview.user_id))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("相手のマスター鍵がありません"))?;
    let accepted =
        store
            .lock()
            .await
            .accept_master_key(&preview.user_id, &master_key, &preview.master_key)?;
    crate::e2ee::verification::save_accepted(api, store, &accepted, reauth).await
}

/// 一覧を読む。検証状態は、暗号ストアを開けていなければ読まない（理由を出す）。
async fn load_overview(
    api: &ApiClient,
    user_store: Option<(String, SharedStore)>,
    user_id: Option<String>,
    peers: Vec<String>,
) -> anyhow::Result<Overview> {
    let user_id = match user_id {
        Some(user_id) => user_id,
        None => api.me().await?.id,
    };
    let current = match &user_store {
        Some((_, store)) => Some(store.lock().await.device_id().to_owned()),
        None => None,
    };
    let (devices, storage) = tokio::try_join!(
        revocation::load_devices(api, &user_id, current.as_deref()),
        api.e2ee_secret_storage(),
    )?;
    let text = |row: &Value, name: &str| row.get(name).and_then(Value::as_str).map(str::to_owned);
    let slots = storage
        .get("slots")
        .and_then(Value::as_array)
        .map(|slots| {
            slots
                .iter()
                .filter_map(|slot| {
                    Some(SlotView {
                        id: text(slot, "id")?,
                        kind: text(slot, "kind")?,
                        label: text(slot, "label"),
                        created_at: text(slot, "created_at"),
                        last_used_at: text(slot, "last_used_at"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let mut overview = Overview {
        devices,
        slots,
        ..Overview::default()
    };
    let Some((_, store)) = user_store else {
        overview.users_error =
            Some("端末を登録しています。少し待ってから、もう一度開いてください".into());
        return Ok(overview);
    };
    {
        let store = store.lock().await;
        overview.unlocked = store.has_own_keys() && !store.needs_unlock();
        overview.own_master_key = store.own_master_key();
        overview.revocation =
            store
                .revocation_state()
                .ok()
                .flatten()
                .map(|state| PendingRevocation {
                    scope: state.scope,
                    stage: state.stage,
                });
    }
    match load_verifications(api, &store, &user_id, peers).await {
        Ok((users, warnings)) => {
            overview.users = users;
            overview.warnings = warnings;
        }
        Err(error) => overview.users_error = Some(format!("検証状態を読めません: {error}")),
    }
    Ok(overview)
}

/// 暗号化チャンネルを共有する相手の検証状態を読む。初めて見た相手は TOFU でピンを置く（配布先の判定の
/// 規則 4。送信と同じ。Web の `loadVerifications`）。
pub async fn load_verifications(
    api: &ApiClient,
    store: &SharedStore,
    user_id: &str,
    peers: Vec<String>,
) -> anyhow::Result<(Vec<UserVerification>, Vec<TrustWarning>)> {
    let others: Vec<String> = peers
        .into_iter()
        .filter(|peer| peer != user_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if others.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut ids = others.clone();
    ids.push(user_id.to_owned());
    for attempt in 0..3 {
        let keys = api.e2ee_query_keys(&ids).await?;
        let result = store.lock().await.evaluate_users(&keys, &others)?;
        if result.new_pins.is_empty() || attempt == 2 {
            let users = others
                .iter()
                .map(|user| UserVerification {
                    user_id: user.clone(),
                    state: result
                        .users
                        .get(user)
                        .map_or(UserState::NoCrossSigningKeys, |trust| trust.state),
                })
                .collect();
            return Ok((users, result.warnings));
        }
        let mut confirmed = Vec::new();
        let mut discarded = Vec::new();
        for pin in result.new_pins {
            if api.e2ee_post_tofu_pin(&pin.pin).await? {
                confirmed.push(pin);
            } else {
                discarded.push(pin);
            }
        }
        store
            .lock()
            .await
            .megolm_settle_pins(&confirmed, &discarded)?;
    }
    Ok((Vec::new(), Vec::new()))
}

/// 失効の画面の、署名し直さなかった理由の文（Web の `e2ee.revocation.note.*`）。
pub fn note_text(note: ResignNote) -> &'static str {
    match note {
        ResignNote::NoRecord => "この端末に記録がない",
        ResignNote::NotConfirmed => "確認していない記録だけがある",
        ResignNote::PinReplaced => "サーバーの記録が違ったので、この端末の記録で置き換える",
        ResignNote::SignatureUnverified => "署名を確かめられなかった",
        ResignNote::MasterKeyMismatch => "署名した鍵が記録と違う",
        ResignNote::StaleConfirmation => {
            "古いユーザー署名鍵で確認した記録だけがある（再検証が必要）"
        }
        ResignNote::PinPending => "ピンの登録が済んでいない",
    }
}

/// 失効の段の文（Web の `e2ee.revocation.stage.*`）。表示は「次に行うこと」。
pub fn stage_text(stage: RevocationStage) -> &'static str {
    match stage {
        RevocationStage::Prepared => "SSSS を保存しています…",
        RevocationStage::SecretStorageSaved => "クロス署名鍵を保存しています…",
        RevocationStage::CrossSigningSaved => "署名し直しています…",
        RevocationStage::SignaturesSaved => "鍵バックアップの新しい版を作っています…",
        RevocationStage::BackupCreated => "古いバックアップを新しい版に移しています…",
        RevocationStage::Migrated => "保存した鍵を確かめています…",
        RevocationStage::Verified => "終わりました",
    }
}

/// 他のユーザーの検証状態の文（Web の `e2ee.verification.*`）。
pub fn user_state_text(state: &UserState) -> &'static str {
    use zoo_crypto::{ReverificationReason, TrustBasis};
    match state {
        UserState::Trusted(TrustBasis::Own) => "自分",
        UserState::Trusted(TrustBasis::UserSigningSignature) => "検証済み",
        UserState::Trusted(TrustBasis::SharedPin) => "最初に見た鍵を信頼（自分の別の端末の記録）",
        UserState::Trusted(TrustBasis::Record) => "最初に見た鍵を信頼",
        UserState::NeedsReverification(ReverificationReason::MasterKeyChanged) => {
            "鍵が変わりました。再検証するまで鍵を配りません。"
        }
        UserState::NeedsReverification(ReverificationReason::NotConfirmed) => {
            "鍵を確認していません。再検証するまで鍵を配りません。"
        }
        UserState::NeedsReverification(ReverificationReason::UserSigningKeyChanged) => {
            "あなたの署名鍵が変わったため、確認し直す必要があります。"
        }
        UserState::NoCrossSigningKeys => "暗号化を設定していません",
        UserState::InvalidCrossSigningKeys => "鍵が壊れています",
        UserState::NeedsUnlock => "解錠すると確かめられます",
        UserState::PinPending => "確かめています",
        UserState::Blocked => "自分の鍵を確かめられないため、配っていません",
    }
}

/// 判定の警告の文（Web の `e2ee.verification.warning.*`）。
pub fn warning_text(warning: TrustWarning) -> &'static str {
    match warning {
        TrustWarning::OwnIdentityUnknown => {
            "自分のマスター鍵をまだ確かめていません。解錠すると確かめられます。"
        }
        TrustWarning::OwnMasterKeyChanged => {
            "サーバーにあるあなたのマスター鍵が、この端末で確かめた鍵と違います。解錠し直すまで、どの端末にも鍵を配りません。"
        }
        TrustWarning::UntrustedUserSigningKey => {
            "あなたのユーザー署名鍵を確かめられません。他のユーザーには鍵を配りません。"
        }
        TrustWarning::UserSigningKeyRollback => {
            "あなたのユーザー署名鍵が、一度置き換わった古い鍵に戻されています。サーバーが書き換えられた可能性があるので、他のユーザーには鍵を配りません。"
        }
        TrustWarning::UserSigningKeyChanged => {
            "あなたのユーザー署名鍵が変わりました。署名し直されていない相手は再検証が必要です。"
        }
    }
}

/// 取りやめた理由の文（Web の `e2ee.sas.cancelled.*`）。
pub fn cancel_text(view: &SasView) -> &'static str {
    let Some(cancel) = &view.cancellation else {
        return "検証を中止しました。";
    };
    match cancel.code.as_str() {
        "m.user" if cancel.by_us => "検証を取りやめました。",
        "m.user" => "相手が検証を取りやめました。",
        "m.timeout" => "10 分以内に終わらなかったため、検証を中止しました。",
        "m.mismatched_sas" => {
            "絵文字が一致しなかったため、検証を中止しました。鍵が置き換えられている可能性があります。"
        }
        "m.key_mismatch" if view.method == Some(zoo_crypto::VerificationMethod::QrCode) => {
            "QR コードの鍵が、サーバーから届いた鍵と一致しなかったため、検証を中止しました。鍵が置き換えられている可能性があります。"
        }
        "m.key_mismatch" => {
            "鍵が一致しなかったため、検証を中止しました。鍵が置き換えられている可能性があります。"
        }
        "m.accepted" => "あなたの別の端末で検証が始まりました。",
        "m.unknown_method" => "相手の端末が対応していない方式のため、検証を中止しました。",
        _ => "検証を中止しました。",
    }
}

/// Matrix の SAS の絵文字の表（Web の `sasEmoji.ts`、Android の `SAS_EMOJI` と同じ。並びを変えない）と、
/// 名前（Web の `e2ee.sas.emoji.<番号>`）。
pub const SAS_EMOJI: [(&str, &str); 64] = [
    ("🐶", "犬"),
    ("🐱", "猫"),
    ("🦁", "ライオン"),
    ("🐎", "馬"),
    ("🦄", "ユニコーン"),
    ("🐷", "豚"),
    ("🐘", "象"),
    ("🐰", "うさぎ"),
    ("🐼", "パンダ"),
    ("🐓", "にわとり"),
    ("🐧", "ペンギン"),
    ("🐢", "亀"),
    ("🐟", "魚"),
    ("🐙", "タコ"),
    ("🦋", "蝶"),
    ("🌷", "花"),
    ("🌳", "木"),
    ("🌵", "サボテン"),
    ("🍄", "きのこ"),
    ("🌏", "地球"),
    ("🌙", "月"),
    ("☁️", "雲"),
    ("🔥", "火"),
    ("🍌", "バナナ"),
    ("🍎", "りんご"),
    ("🍓", "いちご"),
    ("🌽", "とうもろこし"),
    ("🍕", "ピザ"),
    ("🎂", "ケーキ"),
    ("❤️", "ハート"),
    ("😀", "笑顔"),
    ("🤖", "ロボット"),
    ("🎩", "帽子"),
    ("👓", "眼鏡"),
    ("🔧", "スパナ"),
    ("🎅", "サンタ"),
    ("👍", "いいね"),
    ("☂️", "傘"),
    ("⌛", "砂時計"),
    ("⏰", "時計"),
    ("🎁", "プレゼント"),
    ("💡", "電球"),
    ("📕", "本"),
    ("✏️", "鉛筆"),
    ("📎", "クリップ"),
    ("✂️", "はさみ"),
    ("🔒", "錠前"),
    ("🔑", "鍵"),
    ("🔨", "ハンマー"),
    ("☎️", "電話"),
    ("🏁", "旗"),
    ("🚂", "電車"),
    ("🚲", "自転車"),
    ("✈️", "飛行機"),
    ("🚀", "ロケット"),
    ("🏆", "トロフィー"),
    ("⚽", "ボール"),
    ("🎸", "ギター"),
    ("🎺", "トランペット"),
    ("🔔", "ベル"),
    ("⚓", "錨"),
    ("🎧", "ヘッドホン"),
    ("📁", "フォルダー"),
    ("📌", "画びょう"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authz_urls_use_the_issuer_origin() {
        assert_eq!(
            authz_urls("https://account.zli.works/api/auth"),
            Some((
                "https://account.zli.works/settings/security".into(),
                "https://account.zli.works/settings/services".into(),
            ))
        );
        assert_eq!(
            authz_urls("http://localhost:3000/api/auth").map(|u| u.0),
            Some("http://localhost:3000/settings/security".into())
        );
        assert_eq!(authz_urls("not a url"), None);
    }

    fn devices() -> Devices {
        let http = reqwest::Client::new();
        let api = ApiClient::new(
            http.clone(),
            "http://127.0.0.1:9",
            crate::http::Credentials::Dev("u1".into()),
        );
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut devices = Devices::new(api, http, tx);
        let (_stores_tx, stores) = watch::channel(None);
        devices.attach(None, stores, None);
        devices.overview = Overview {
            devices: vec![
                DeviceView {
                    device_id: "CURRENT".into(),
                    display_name: Some("zoo2-tui".into()),
                    created_at: String::new(),
                    last_seen_at: None,
                    current: true,
                    signed: true,
                },
                DeviceView {
                    device_id: "LOST".into(),
                    display_name: None,
                    created_at: String::new(),
                    last_seen_at: None,
                    current: false,
                    signed: false,
                },
            ],
            unlocked: true,
            ..Overview::default()
        };
        devices.screen = Screen::Overview;
        devices
    }

    fn press(devices: &mut Devices, code: KeyCode) {
        devices.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[tokio::test]
    async fn revocation_walks_through_selection_authz_and_unlock() {
        let mut devices = devices();
        press(&mut devices, KeyCode::Char('R'));
        let Screen::RevokeSelect { scope, .. } = &devices.screen else {
            panic!("削除する端末と範囲を選ぶ");
        };
        // 迷ったら 1〜5 を選ぶよう、既定は 1〜5。
        assert_eq!(*scope, RevocationScope::Full);
        press(&mut devices, KeyCode::Char(' '));
        press(&mut devices, KeyCode::Char('t'));
        let Screen::RevokeSelect {
            selected, scope, ..
        } = &devices.screen
        else {
            panic!("選んだまま");
        };
        assert!(selected.contains("LOST"), "今の端末は選べない");
        assert_eq!(*scope, RevocationScope::DeviceOnly);
        press(&mut devices, KeyCode::Enter);
        assert!(matches!(devices.screen, Screen::RevokeAuthz { .. }));
        // AuthZ で行うことを済ませたと答えるまで進まない。
        press(&mut devices, KeyCode::Enter);
        assert!(matches!(devices.screen, Screen::RevokeAuthz { .. }));
        press(&mut devices, KeyCode::Char('y'));
        assert!(matches!(
            devices.screen,
            Screen::RevokeUnlock { resume: false, .. }
        ));
        devices.on_paste("ABCD EFGH");
        press(&mut devices, KeyCode::Enter);
        // 暗号ストアを開けていなければ、始めない。
        assert!(matches!(
            devices.screen,
            Screen::Message { error: true, .. }
        ));
    }

    #[tokio::test]
    async fn a_wrong_recovery_key_returns_to_the_same_unlock_screen() {
        let mut devices = devices();
        devices.unlock_context = Some((
            false,
            ["LOST".to_owned()].into_iter().collect(),
            RevocationScope::Full,
        ));
        devices.screen = Screen::Working("解錠しています…");
        let generation = devices.generation;
        devices.on_event(DevicesEvent {
            generation,
            outcome: Outcome::RevocationFailed {
                error: "リカバリーキーが違います".into(),
                retry: RevokeRetry::Restart,
                session: None,
                unlock_error: true,
                device_only_unavailable: false,
            },
        });
        let Screen::RevokeUnlock {
            selected, error, ..
        } = &devices.screen
        else {
            panic!("打ち直せる");
        };
        assert!(selected.contains("LOST"));
        assert_eq!(error.as_deref(), Some("リカバリーキーが違います"));
    }

    #[tokio::test]
    async fn revocation_needs_an_unlocked_device() {
        let mut devices = devices();
        devices.overview.unlocked = false;
        press(&mut devices, KeyCode::Char('R'));
        let Screen::Message { text, .. } = &devices.screen else {
            panic!("案内を出す");
        };
        assert!(text.contains("解錠した端末で行えます"));
    }

    #[test]
    fn fingerprint_groups_four_characters() {
        assert_eq!(fingerprint("abcdefghij"), "abcd efgh ij");
    }
}
