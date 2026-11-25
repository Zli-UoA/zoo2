//! 他のユーザーの絵文字 SAS 検証（docs/e2ee.md の「絵文字 SAS 検証」。#368）と、QR コードの表示
//! （「QR コードによる検証」。TUI はカメラを使えないので、相手の QR は読み取らない）。
//!
//! Web の `sasController.ts`、Android の `SasController` と同じ規則。
//!
//! - 状態機械は `zoo-crypto` の [`SasVerification`] にあり、ここは届いたメッセージを渡し、返ったメッセージを
//!   Olm で暗号化して to-device で送る。処理と送信は 1 つずつ順に行う（同じ相手への `mac` と `done` の順を
//!   守るため）。vodozemac の SAS の状態は保存できないので、検証はメモリーにだけ置く（起動し直したら続け
//!   られない。相手には 10 分で `m.timeout` の取りやめになる）。
//! - 双方が確かめ終わったら（`done`）、相手のマスター鍵に署名して、`replace: true` のピンと署名を保存する
//!   （直前の再認証が要る。再認証は「一致する」か「読み取った」を押したときに始める）。保存できたら記録の
//!   登録待ちを外す。
//! - 送れなかったメッセージと、`keys/query` を取れずに処理できなかったメッセージは、[`VerificationController::tick`]
//!   のたびに試し直す。一度送ったメッセージは、同じ `txn_id` と暗号文で送り直す。
//!
//! 画面には [`VerificationEvent`] で知らせる（`AppEvent::Verification`）。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use zeroize::Zeroizing;
use zoo_crypto::{
    DeviceKeys, SasCancellation, SasPhase, SasVerification, VERIFICATION_READY_TYPE,
    VERIFICATION_REQUEST_TYPE, VERIFICATION_TIMEOUT_MS, VerificationMessage, VerificationMethod,
    VerificationOutgoing,
};

use super::manager::StoreWatch;
use super::reauth;
use super::setup::SharedStore;
use super::store::VerificationNeedsUnlock;
use crate::api::{ApiClient, TO_DEVICE_BATCH};
use crate::auth::reauth::REAUTH_TIMEOUT;
use crate::config::OidcConfig;
use crate::http::ApiError;

/// 期限切れを確かめ、処理できなかったメッセージを試し直す間隔（Web の `SAS_EXPIRE_INTERVAL_MS`）。
pub const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// 1 つの検証の表示（Web の `SasView`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SasView {
    pub transaction_id: String,
    pub their_user_id: String,
    pub their_device_id: Option<String>,
    pub we_requested: bool,
    pub started_at_ms: u64,
    pub phase: SasPhase,
    /// 絵文字の番号（0〜63 が 7 つ。`Compare` 以降）。
    pub emoji: Option<[u8; 7]>,
    pub cancellation: Option<SasCancellation>,
    pub method: Option<VerificationMethod>,
    /// この端末が出す QR のバイト列（出せる段階だけ）。
    pub qr_code: Option<Vec<u8>>,
    pub can_start_sas: bool,
}

impl SasView {
    fn of(verification: &SasVerification) -> Self {
        Self {
            transaction_id: verification.transaction_id().to_owned(),
            their_user_id: verification.their_user_id().to_owned(),
            their_device_id: verification.their_device_id().map(str::to_owned),
            we_requested: verification.we_requested(),
            started_at_ms: verification.started_at_ms(),
            phase: verification.phase(),
            emoji: verification.emoji_indices(),
            cancellation: verification.cancellation().cloned(),
            method: verification.method(),
            qr_code: verification.qr_code(),
            can_start_sas: verification.can_start_sas(),
        }
    }

    /// 検証済みか、取りやめた。
    pub fn finished(&self) -> bool {
        matches!(self.phase, SasPhase::Done | SasPhase::Cancelled)
    }
}

/// 検証が終わったあと、相手のマスター鍵への署名とピンの保存の状態（Web の `SasSaveState`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SaveState {
    /// まだ（検証が終わっていない）。
    #[default]
    Idle,
    /// 保存している（再認証を待っているあいだも含む）。
    Saving,
    Saved,
    /// 再認証できなかった、または古くなった。再認証し直して保存する。
    Reauth,
    /// まだ再認証していない。再認証して保存する。
    Login,
    Failed,
}

/// 画面に出す誤り（Web の `SasErrorCode`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SasErrorCode {
    /// 解錠していないので、受け入れられない。
    NeedsUnlock,
    /// 相手に送れなかった。
    SendFailed,
    /// 相手からのメッセージを処理するための鍵を取れなかった（あとで試し直す）。
    ReceiveFailed,
    Failed,
}

/// 画面に知らせること。
#[derive(Debug, Clone)]
pub enum VerificationEvent {
    /// 検証の表示が変わった（なければ足す）。
    View(SasView),
    /// 処理か送信の途中（ボタンを押せなくする）。
    Busy { transaction_id: String, busy: bool },
    Error {
        transaction_id: String,
        error: Option<SasErrorCode>,
    },
    Save {
        transaction_id: String,
        state: SaveState,
    },
    /// 再認証の待ち受けを始めた（URL を出す）。
    ReauthWaiting {
        transaction_id: String,
        url: String,
        port: u16,
    },
    /// 検証を求められなかった。
    RequestFailed {
        user_id: String,
        error: SasErrorCode,
    },
    /// 相手のマスター鍵への署名を保存できた（検証済みの表示を読み直す）。
    Saved { user_id: String },
}

/// 1 つの端末あての、送れていない検証のメッセージ（`message` は送る順の番号。同じメッセージの宛先は同じ番号）。
struct PendingSend {
    device: DeviceKeys,
    event_type: &'static str,
    content: Value,
    message: u64,
    queued_at: u64,
    /// 一度送った（応答がなかったか、`undelivered` だった）ときの `txn_id` と暗号文。送り直すときは同じもの
    /// を送る（サーバーは同じ送信端末・`txn_id`・宛先を 1 件にまとめる）。
    txn_id: Option<String>,
    encrypted: Option<Value>,
}

impl PendingSend {
    fn target(&self) -> (String, String) {
        (self.device.user_id.clone(), self.device.device_id.clone())
    }
}

/// 画面に知らせる関数（`AppEvent::Verification` に包んで送る）。
pub type Notify = Arc<dyn Fn(VerificationEvent) + Send + Sync>;

type ReauthTask = JoinHandle<std::result::Result<Option<Zeroizing<String>>, String>>;

#[derive(Default)]
struct Shared {
    active: BTreeMap<String, SasVerification>,
    /// 検証ごとの段階と保存の状態（保存を始めるかの判定に使う）。
    phases: BTreeMap<String, (SasPhase, SaveState)>,
    /// 届いたが、まだ処理していないメッセージ（届いた順）。Olm のメッセージは 2 回は復号できないので、
    /// `keys/query` が一時的に失敗しても捨てずに残し、次の確認で試し直す。
    inbox: Vec<(VerificationMessage, u64)>,
    /// 送れなかった検証のメッセージ（`transaction_id` ごとに、送る順）。
    unsent: BTreeMap<String, Vec<PendingSend>>,
    message_seq: u64,
    /// 「一致する」か「読み取った」を押したときに始めた再認証。
    reauths: BTreeMap<String, ReauthTask>,
    /// 保存が取り出して結果を待っている再認証（[`VerificationController::dispose`] で止めるため）。
    saving_reauths: BTreeMap<String, tokio::task::AbortHandle>,
    /// [`VerificationController::dispose`] のたびに増やす。待っていた古い処理は、状態を変える前と送る前に
    /// これを比べ、変わっていれば何もしない。
    epoch: u64,
}

struct Inner {
    api: ApiClient,
    http: reqwest::Client,
    oidc: Option<OidcConfig>,
    stores: StoreWatch,
    notify: Notify,
    shared: StdMutex<Shared>,
    /// 処理と送信を 1 つずつにする。
    queue: tokio::sync::Mutex<()>,
    ticking: AtomicBool,
}

/// 他のユーザーの検証の一覧と、その処理。画面が 1 つ持ち、クローンして裏の処理に渡す。
#[derive(Clone)]
pub struct VerificationController {
    inner: Arc<Inner>,
}

/// 検証の操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Accept,
    Confirm,
    Mismatch,
    StartSas,
    ConfirmQr,
    Cancel,
}

impl VerificationController {
    pub fn new(
        api: ApiClient,
        http: reqwest::Client,
        oidc: Option<OidcConfig>,
        stores: StoreWatch,
        notify: Notify,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                api,
                http,
                oidc,
                stores,
                notify,
                shared: StdMutex::new(Shared::default()),
                queue: tokio::sync::Mutex::new(()),
                ticking: AtomicBool::new(false),
            }),
        }
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.inner
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn send(&self, event: VerificationEvent) {
        (self.inner.notify)(event);
    }

    fn store(&self) -> Option<SharedStore> {
        self.inner
            .stores
            .borrow()
            .as_ref()
            .map(|(_, store)| store.clone())
    }

    /// 終わっていない検証か、処理を待つメッセージか、送り直すメッセージがあるか。
    pub fn is_active(&self) -> bool {
        let shared = self.shared();
        !shared.inbox.is_empty()
            || !shared.unsent.is_empty()
            || shared.phases.values().any(|(phase, save)| {
                !matches!(phase, SasPhase::Cancelled) && *save != SaveState::Saved
            })
    }

    /// `user_id` に検証を求める。
    pub fn request(&self, user_id: String) {
        let this = self.clone();
        let epoch = self.epoch();
        tokio::spawn(async move {
            let _serial = this.inner.queue.lock().await;
            let result = async {
                let keys = this
                    .inner
                    .api
                    .e2ee_query_keys(std::slice::from_ref(&user_id))
                    .await?;
                let store = this
                    .store()
                    .ok_or_else(|| anyhow!("暗号ストアの準備ができていません"))?;
                let (verification, outgoing) =
                    store
                        .lock()
                        .await
                        .request_verification(&keys, &user_id, now_ms())?;
                let txn = verification.transaction_id().to_owned();
                let view = SasView::of(&verification);
                {
                    // 待つあいだに止めた（ログアウトなど）なら、登録も送信もしない。
                    let mut shared = this.shared();
                    if shared.epoch != epoch {
                        return Ok(None);
                    }
                    shared.active.insert(txn.clone(), verification);
                }
                this.register(view, vec![outgoing]);
                anyhow::Ok(Some(txn))
            }
            .await;
            match result {
                Ok(Some(txn)) => this.flush(&txn).await,
                Ok(None) => {}
                Err(_) if this.epoch() != epoch => {}
                Err(error) => this.send(VerificationEvent::RequestFailed {
                    user_id,
                    error: error_code(&error),
                }),
            }
        });
    }

    /// to-device で届いた検証のメッセージ（`CryptoStore::process_to_device` の `verifications`）。届いた順に
    /// 受け取り、処理は裏で行う。
    pub fn handle_messages(&self, messages: Vec<VerificationMessage>) {
        if messages.is_empty() {
            return;
        }
        let received_at = now_ms();
        self.shared()
            .inbox
            .extend(messages.into_iter().map(|m| (m, received_at)));
        let this = self.clone();
        tokio::spawn(async move {
            let _serial = this.inner.queue.lock().await;
            this.drain().await;
        });
    }

    /// 利用者の操作。`Confirm` と `ConfirmQr` は、このときに再認証を始める（検証が終わったら、その ID
    /// トークンで署名とピンを保存する）。
    pub fn act(&self, transaction_id: &str, action: Action) {
        if matches!(action, Action::Confirm | Action::ConfirmQr) {
            self.start_reauth(transaction_id);
        }
        let this = self.clone();
        let txn = transaction_id.to_owned();
        tokio::spawn(async move {
            let _serial = this.inner.queue.lock().await;
            this.send(VerificationEvent::Busy {
                transaction_id: txn.clone(),
                busy: true,
            });
            this.send(VerificationEvent::Error {
                transaction_id: txn.clone(),
                error: None,
            });
            match this.step(&txn, action).await {
                Ok(true) => this.flush(&txn).await,
                Ok(false) => {}
                Err(error) => this.send(VerificationEvent::Error {
                    transaction_id: txn.clone(),
                    error: Some(error_code(&error)),
                }),
            }
            this.send(VerificationEvent::Busy {
                transaction_id: txn,
                busy: false,
            });
        });
    }

    /// 検証を進める。検証がなければ `false`。
    async fn step(&self, txn: &str, action: Action) -> Result<bool> {
        if action == Action::Accept {
            // 受け入れには、解錠した端末が要る（自分のマスター鍵とユーザー署名鍵）。
            let store = self.store().ok_or(VerificationNeedsUnlock)?;
            let store = store.lock().await;
            if !store.has_own_keys() || store.needs_unlock() {
                return Err(VerificationNeedsUnlock.into());
            }
        }
        let (view, outgoing) = {
            let mut shared = self.shared();
            let Some(verification) = shared.active.get_mut(txn) else {
                return Ok(false);
            };
            let outgoing = match action {
                Action::Accept => verification.accept()?,
                Action::Confirm => verification.confirm()?,
                Action::Mismatch => verification.mismatch(),
                Action::StartSas => verification.start_sas()?,
                Action::ConfirmQr => verification.confirm_qr_scanned()?,
                Action::Cancel => verification.cancel(),
            };
            (SasView::of(verification), outgoing)
        };
        self.register(view, outgoing);
        Ok(true)
    }

    /// 画面を閉じる。終わっていなければ取りやめてから忘れる。
    pub fn dismiss(&self, transaction_id: &str) {
        let this = self.clone();
        let txn = transaction_id.to_owned();
        tokio::spawn(async move {
            let _serial = this.inner.queue.lock().await;
            let finished = this
                .shared()
                .active
                .get(&txn)
                .is_none_or(SasVerification::is_finished);
            if !finished && let Ok(true) = this.step(&txn, Action::Cancel).await {
                this.flush(&txn).await;
            }
            // 送れなかった分（取りやめを含む）は `unsent` に残し、期限まで `tick` で送り直す。
            let mut shared = this.shared();
            shared.active.remove(&txn);
            shared.phases.remove(&txn);
            if let Some(task) = shared.reauths.remove(&txn) {
                task.abort();
            }
            if let Some(task) = shared.saving_reauths.remove(&txn) {
                task.abort();
            }
        });
    }

    /// 再認証し直して、署名とピンを保存し直す。
    pub fn retry_save(&self, transaction_id: &str) {
        self.start_reauth(transaction_id);
        let this = self.clone();
        let txn = transaction_id.to_owned();
        tokio::spawn(async move { this.save(&txn).await });
    }

    /// 止める（ログアウトや画面の終わり）。検証はすべて忘れる。
    pub fn dispose(&self) {
        let mut shared = self.shared();
        for (_, task) in std::mem::take(&mut shared.reauths) {
            task.abort();
        }
        for (_, task) in std::mem::take(&mut shared.saving_reauths) {
            task.abort();
        }
        let epoch = shared.epoch + 1;
        *shared = Shared {
            epoch,
            ..Shared::default()
        };
    }

    fn epoch(&self) -> u64 {
        self.shared().epoch
    }

    /// 期限切れを確かめ、処理できなかったメッセージと、送れなかったメッセージを試し直す。画面の tick から、
    /// [`TICK_INTERVAL`] ごとに呼ぶ。前の回が終わっていなければ何もしない。
    pub fn tick(&self) {
        if !self.is_active() || self.inner.ticking.swap(true, Ordering::AcqRel) {
            return;
        }
        let this = self.clone();
        tokio::spawn(async move {
            {
                let _serial = this.inner.queue.lock().await;
                this.drain().await;
                let txns: Vec<String> = this.shared().unsent.keys().cloned().collect();
                for txn in txns {
                    let (items, epoch) = {
                        let mut shared = this.shared();
                        (shared.unsent.remove(&txn).unwrap_or_default(), shared.epoch)
                    };
                    // 期限を過ぎたものは捨てる。捨てた端末あての後のメッセージも、順を守れないので捨てる。
                    let mut dead = BTreeSet::new();
                    let live: Vec<PendingSend> = items
                        .into_iter()
                        .filter(|item| {
                            if dead.contains(&item.target())
                                || now_ms().saturating_sub(item.queued_at) > VERIFICATION_TIMEOUT_MS
                            {
                                dead.insert(item.target());
                                false
                            } else {
                                true
                            }
                        })
                        .collect();
                    if live.is_empty() {
                        this.send(VerificationEvent::Error {
                            transaction_id: txn,
                            error: None,
                        });
                    } else {
                        this.deliver(&txn, live, epoch).await;
                    }
                }
                let expired: Vec<(SasView, Vec<VerificationOutgoing>)> = this
                    .shared()
                    .active
                    .values_mut()
                    .filter_map(|verification| {
                        let outgoing = verification.expire(now_ms());
                        (!outgoing.is_empty()).then(|| (SasView::of(verification), outgoing))
                    })
                    .collect();
                for (view, outgoing) in expired {
                    let txn = view.transaction_id.clone();
                    this.register(view, outgoing);
                    this.flush(&txn).await;
                }
            }
            // 終わったのに保存を始めていない検証（保存を始める前に処理が取り消された）は、ここで始める。
            let idle: Vec<String> = this
                .shared()
                .phases
                .iter()
                .filter(|(_, (phase, save))| *phase == SasPhase::Done && *save == SaveState::Idle)
                .map(|(txn, _)| txn.clone())
                .collect();
            for txn in idle {
                this.save(&txn).await;
            }
            this.inner.ticking.store(false, Ordering::Release);
        });
    }

    /// 届いたメッセージを、届いた順に処理する。`keys/query` を取れなければ、その検証のメッセージだけを、順を
    /// 変えずに残す（次の確認で試し直す）。呼ぶ側が `queue` を持つ。
    async fn drain(&self) {
        let (pending, epoch) = {
            let mut shared = self.shared();
            (std::mem::take(&mut shared.inbox), shared.epoch)
        };
        let mut remaining = Vec::new();
        let mut keys: BTreeMap<String, Value> = BTreeMap::new();
        let mut failed_senders = BTreeSet::new();
        let mut held = BTreeSet::new();
        let mut touched = Vec::new();
        for (message, received_at) in pending {
            let Some(txn) = message.transaction_id().map(str::to_owned) else {
                continue;
            };
            if now_ms().saturating_sub(received_at) > VERIFICATION_TIMEOUT_MS {
                continue;
            }
            // 同じ検証の前のメッセージを残したなら、順を守るため、これも残す。
            if held.contains(&txn) {
                remaining.push((message, received_at));
                continue;
            }
            let mut query = json!({});
            if [VERIFICATION_REQUEST_TYPE, VERIFICATION_READY_TYPE]
                .contains(&message.event_type.as_str())
            {
                let sender = message.sender_user_id.clone();
                if !keys.contains_key(&sender) && !failed_senders.contains(&sender) {
                    match self
                        .inner
                        .api
                        .e2ee_query_keys(std::slice::from_ref(&sender))
                        .await
                    {
                        Ok(value) => {
                            keys.insert(sender.clone(), value);
                        }
                        Err(_) => {
                            failed_senders.insert(sender.clone());
                        }
                    }
                }
                let Some(cached) = keys.get(&sender) else {
                    held.insert(txn.clone());
                    remaining.push((message, received_at));
                    self.send(VerificationEvent::Error {
                        transaction_id: txn,
                        error: Some(SasErrorCode::ReceiveFailed),
                    });
                    continue;
                };
                query = cached.clone();
            }
            // 暗号ストアを使えない（開き直しのあいだ）なら、この検証のメッセージを届いた順のまま残す。
            let Some(store) = self.store() else {
                held.insert(txn);
                remaining.push((message, received_at));
                continue;
            };
            let update = {
                let store = store.lock().await;
                let mut shared = self.shared();
                if shared.epoch != epoch {
                    // 待つあいだに止めた。取り出した分は捨てる。
                    return;
                }
                match shared.active.get_mut(&txn) {
                    Some(verification) => {
                        let snapshot =
                            zoo_crypto::KeySnapshot::from_json(&query).unwrap_or_default();
                        let outgoing = verification.receive(&message, &snapshot);
                        Some((SasView::of(verification), outgoing))
                    }
                    None if message.event_type == VERIFICATION_REQUEST_TYPE => {
                        // 受け付けない要求（古い、たどれない端末、解錠の前など）は捨てる。
                        match store.verification_from_request(&message, &query, now_ms()) {
                            Ok(verification) => {
                                let view = SasView::of(&verification);
                                shared.active.insert(txn.clone(), verification);
                                Some((view, Vec::new()))
                            }
                            Err(_) => None,
                        }
                    }
                    None => None,
                }
            };
            if let Some((view, outgoing)) = update {
                self.send(VerificationEvent::Error {
                    transaction_id: txn.clone(),
                    error: None,
                });
                self.register(view, outgoing);
                touched.push(txn);
            }
        }
        // 待つあいだに届いた分の前に、残した分を戻す（同じ検証のメッセージの届いた順を守る）。
        {
            let mut shared = self.shared();
            if shared.epoch != epoch {
                return;
            }
            let newer = std::mem::take(&mut shared.inbox);
            shared.inbox = remaining;
            shared.inbox.extend(newer);
        }
        touched.dedup();
        for txn in touched {
            self.flush(&txn).await;
        }
    }

    /// 進めた検証の表示を出し、送るメッセージを送信待ち（前に送れなかった分の後）に入れる。
    fn register(&self, view: SasView, outgoing: Vec<VerificationOutgoing>) {
        let txn = view.transaction_id.clone();
        {
            let mut shared = self.shared();
            // 止めた（dispose）か閉じた検証は、表示も送信待ちも作り直さない（`active` の登録と同じロックで判定する）。
            if !shared.active.contains_key(&txn) {
                return;
            }
            let save = shared
                .phases
                .get(&txn)
                .map_or(SaveState::Idle, |(_, save)| *save);
            shared.phases.insert(txn.clone(), (view.phase, save));
            let now = now_ms();
            let mut fresh = Vec::new();
            for out in outgoing {
                let message = shared.message_seq;
                shared.message_seq += 1;
                for device in out.devices {
                    fresh.push(PendingSend {
                        device,
                        event_type: out.event_type,
                        content: out.content.clone(),
                        message,
                        queued_at: now,
                        txn_id: None,
                        encrypted: None,
                    });
                }
            }
            if !fresh.is_empty() {
                shared.unsent.entry(txn).or_default().extend(fresh);
            }
        }
        self.send(VerificationEvent::View(view));
    }

    /// 送信待ちを送り（前に送れなかった分から順に）、検証が終わっていたら保存を始める。
    async fn flush(&self, txn: &str) {
        // 取り出すのと同じロックで世代を読み、送る処理に渡す（取り出したあとに止めたら送らない）。
        let (items, epoch) = {
            let mut shared = self.shared();
            (shared.unsent.remove(txn).unwrap_or_default(), shared.epoch)
        };
        if !items.is_empty() {
            self.deliver(txn, items, epoch).await;
        }
        let done = self
            .shared()
            .phases
            .get(txn)
            .is_some_and(|(phase, save)| *phase == SasPhase::Done && *save == SaveState::Idle);
        if done {
            // 再認証を待つので、処理の列の外で保存する。
            let this = self.clone();
            let txn = txn.to_owned();
            tokio::spawn(async move { this.save(&txn).await });
        }
    }

    /// `items` を送り、送れなかった分を送信待ちに残す（次の確認で送り直す）。
    /// `epoch` は `items` を取り出したときの世代。
    async fn deliver(&self, txn: &str, items: Vec<PendingSend>, epoch: u64) {
        let undelivered = self.send_items(items, epoch).await;
        let kept = !undelivered.is_empty();
        {
            // 画面を閉じた検証（取りやめを送れなかった）も残す。止めた（dispose）あとは残さない。
            let mut shared = self.shared();
            if kept && shared.epoch == epoch {
                let rest = shared.unsent.remove(txn).unwrap_or_default();
                let mut all = undelivered;
                all.extend(rest);
                shared.unsent.insert(txn.to_owned(), all);
            }
        }
        self.send(VerificationEvent::Error {
            transaction_id: txn.to_owned(),
            error: kept.then_some(SasErrorCode::SendFailed),
        });
    }

    /// 送るメッセージを、宛先の端末ごとに Olm で暗号化して送る。送信用の Olm セッションは鍵の配布と同じものを
    /// 使い回し、ない端末は先に `keys/claim` で払い出す。メッセージごとに分けて、この順に送る。送れなかった
    /// 分を送る順に返す（ある端末に送れなかったら、その端末あての後のメッセージは送らずに返す）。
    async fn send_items(&self, mut items: Vec<PendingSend>, epoch: u64) -> Vec<PendingSend> {
        if self.epoch() != epoch {
            return items;
        }
        let Some(store) = self.store() else {
            return items;
        };
        let device_id = store.lock().await.device_id().to_owned();
        let mut claim: BTreeMap<String, Vec<String>> = BTreeMap::new();
        {
            let store = store.lock().await;
            for item in items.iter().filter(|item| item.encrypted.is_none()) {
                if !store.has_outbound_olm(&item.device) {
                    let devices = claim.entry(item.device.user_id.clone()).or_default();
                    if !devices.contains(&item.device.device_id) {
                        devices.push(item.device.device_id.clone());
                    }
                }
            }
        }
        // 払い出せなくても、暗号化済みのものと、今の Olm セッションで暗号化できるものは送る。
        let claimed = if claim.is_empty() {
            serde_json::Map::new()
        } else {
            self.inner
                .api
                .e2ee_claim_keys(&claim)
                .await
                .unwrap_or_default()
        };
        let mut failed: BTreeSet<(String, String)> = BTreeSet::new();
        let mut undelivered: BTreeSet<usize> = BTreeSet::new();
        let mut groups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for (index, item) in items.iter().enumerate() {
            groups.entry(item.message).or_default().push(index);
        }
        for indices in groups.values() {
            for &index in indices {
                let item = &mut items[index];
                if failed.contains(&item.target()) {
                    undelivered.insert(index);
                    continue;
                }
                if item.encrypted.is_some() {
                    continue;
                }
                let key = claimed
                    .get(&item.device.user_id)
                    .and_then(|devices| devices.get(&item.device.device_id))
                    .and_then(Value::as_object)
                    .and_then(|keys| keys.values().next());
                item.encrypted = store
                    .lock()
                    .await
                    .encrypt_verification(&item.device, item.event_type, &item.content, key)
                    .ok()
                    .flatten();
                if item.encrypted.is_none() {
                    failed.insert(item.target());
                    undelivered.insert(index);
                }
            }
            // 前に送った分は前の `txn_id` ごとに、新しく暗号化した分は新しい `txn_id`（100 端末ずつ）で送る。
            let ready: Vec<usize> = indices
                .iter()
                .copied()
                .filter(|&i| !failed.contains(&items[i].target()) && items[i].encrypted.is_some())
                .collect();
            let mut requests: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for &i in &ready {
                if let Some(id) = &items[i].txn_id {
                    requests.entry(id.clone()).or_default().push(i);
                }
            }
            let fresh: Vec<usize> = ready
                .iter()
                .copied()
                .filter(|&i| items[i].txn_id.is_none())
                .collect();
            for chunk in fresh.chunks(TO_DEVICE_BATCH) {
                let id = uuid::Uuid::new_v4().to_string();
                for &i in chunk {
                    items[i].txn_id = Some(id.clone());
                }
                requests.insert(id, chunk.to_vec());
            }
            for (txn_id, request) in requests {
                // 待つあいだ（`keys/claim` など）に止めたら、送らない。
                if self.epoch() != epoch {
                    undelivered.extend(request.iter().copied());
                    continue;
                }
                let mut messages = serde_json::Map::new();
                for &i in &request {
                    let (user, device) = items[i].target();
                    let entry = messages
                        .entry(user)
                        .or_insert_with(|| Value::Object(Default::default()));
                    entry[device.as_str()] = items[i].encrypted.clone().unwrap_or(Value::Null);
                }
                let refused = self
                    .inner
                    .api
                    .e2ee_send_to_device_txn(&device_id, &txn_id, Value::Object(messages))
                    .await
                    .ok()
                    .map(|targets| {
                        targets
                            .into_iter()
                            .map(|t| (t.user_id, t.device_id))
                            .collect::<BTreeSet<_>>()
                    });
                for &i in &request {
                    // 応答がなければ、届いたか分からない。同じ `txn_id` と暗号文で送り直す。
                    if refused
                        .as_ref()
                        .is_none_or(|refused| refused.contains(&items[i].target()))
                    {
                        failed.insert(items[i].target());
                        undelivered.insert(i);
                    }
                }
            }
        }
        items
            .into_iter()
            .enumerate()
            .filter(|(i, _)| undelivered.contains(i))
            .map(|(_, item)| item)
            .collect()
    }

    /// 再認証を始める（dev モードでは再認証しない）。結果は保存のときに待つ。
    fn start_reauth(&self, txn: &str) {
        let this = self.clone();
        let id = txn.to_owned();
        let task: ReauthTask = tokio::spawn(async move {
            let Some(oidc) = this.inner.oidc.clone() else {
                return Ok(None);
            };
            let http = this.inner.http.clone();
            let result = async {
                let (pending, discovery) = reauth::start(&http, &oidc).await?;
                this.send(VerificationEvent::ReauthWaiting {
                    transaction_id: id.clone(),
                    url: pending.url().to_owned(),
                    port: pending.port(),
                });
                pending
                    .finish(&http, &discovery, &oidc, REAUTH_TIMEOUT)
                    .await
            }
            .await;
            result
                .map(|token| Some(Zeroizing::new(token)))
                .map_err(|error| error.to_string())
        });
        if let Some(previous) = self.shared().reauths.insert(txn.to_owned(), task) {
            previous.abort();
        }
    }

    fn set_save(&self, txn: &str, state: SaveState) {
        if let Some((_, save)) = self.shared().phases.get_mut(txn) {
            *save = state;
        }
        self.send(VerificationEvent::Save {
            transaction_id: txn.to_owned(),
            state,
        });
    }

    /// 検証できた相手のマスター鍵に署名し、`replace: true` のピンと署名を保存する。
    async fn save(&self, txn: &str) {
        let epoch = self.epoch();
        // 確かめることと保存中にすることを 1 つのロックの中で行う（自動の保存と保存し直しが重なっても、
        // 保存を 1 つだけ始める）。
        let (their_user_id, task) = {
            let mut shared = self.shared();
            let Some((phase, save)) = shared.phases.get(txn).copied() else {
                return;
            };
            if phase != SasPhase::Done || matches!(save, SaveState::Saving | SaveState::Saved) {
                return;
            }
            let Some(their) = shared.active.get(txn).map(|v| v.their_user_id().to_owned()) else {
                return;
            };
            let task = shared.reauths.remove(txn);
            // 取り出すのと同じロックの中で登録し、結果を待つあいだも dispose と dismiss が再認証を止められる
            // ようにする（取り出した保存だけが結果を使う）。
            if let Some(task) = &task {
                shared
                    .saving_reauths
                    .insert(txn.to_owned(), task.abort_handle());
            }
            let next = match (&task, save) {
                (Some(_), _) => SaveState::Saving,
                (None, SaveState::Idle) => SaveState::Login,
                (None, _) => SaveState::Reauth,
            };
            if let Some((_, save)) = shared.phases.get_mut(txn) {
                *save = next;
            }
            (their, task)
        };
        let Some(task) = task else {
            let state = self.shared().phases.get(txn).map(|(_, s)| *s);
            if let Some(state) = state {
                self.send(VerificationEvent::Save {
                    transaction_id: txn.to_owned(),
                    state,
                });
            }
            return;
        };
        self.send(VerificationEvent::Save {
            transaction_id: txn.to_owned(),
            state: SaveState::Saving,
        });
        let awaited = task.await;
        {
            let mut shared = self.shared();
            if shared.epoch != epoch || !shared.active.contains_key(txn) {
                return;
            }
            shared.saving_reauths.remove(txn);
        }
        let token = match awaited {
            Ok(Ok(token)) => token,
            _ => {
                self.set_save(txn, SaveState::Reauth);
                return;
            }
        };
        // 保存（`keys/query` から）は別のタスクで行い、そのタスクも dispose と dismiss が止められるように登録して
        // から始める（登録の前に止めていれば始めない）。
        let (go, ready) = tokio::sync::oneshot::channel::<()>();
        let this = self.clone();
        let txn_owned = txn.to_owned();
        let their = their_user_id.clone();
        let work = tokio::spawn(async move {
            if ready.await.is_err() {
                return Err(anyhow!("保存を取りやめました"));
            }
            let txn = txn_owned.as_str();
            let their_user_id = their;
            let keys = this
                .inner
                .api
                .e2ee_query_keys(std::slice::from_ref(&their_user_id))
                .await?;
            let master_key = keys
                .get("master_keys")
                .and_then(|keys| keys.get(&their_user_id))
                .cloned()
                .ok_or_else(|| anyhow!("相手のマスター鍵がありません"))?;
            let store = this
                .store()
                .ok_or_else(|| anyhow!("暗号ストアの準備ができていません"))?;
            let accepted = {
                let mut store = store.lock().await;
                let shared = this.shared();
                let verification = shared
                    .active
                    .get(txn)
                    .ok_or_else(|| anyhow!("検証がありません"))?;
                store.sign_verified_master_key(verification, &master_key)?
            };
            save_accepted(
                &this.inner.api,
                &store,
                &accepted,
                token.as_deref().map(String::as_str),
            )
            .await
        });
        {
            let mut shared = self.shared();
            if shared.epoch != epoch || !shared.active.contains_key(txn) {
                work.abort();
                return;
            }
            shared
                .saving_reauths
                .insert(txn.to_owned(), work.abort_handle());
        }
        let _ = go.send(());
        let Ok(result) = work.await else {
            // 止められた。
            return;
        };
        {
            let mut shared = self.shared();
            if shared.epoch != epoch || !shared.active.contains_key(txn) {
                return;
            }
            shared.saving_reauths.remove(txn);
        }
        match result {
            Ok(()) => {
                self.set_save(txn, SaveState::Saved);
                self.send(VerificationEvent::Saved {
                    user_id: their_user_id,
                });
            }
            Err(error) if ApiError::has_code(&error, "E2EE_REAUTH_REQUIRED") => {
                self.set_save(txn, SaveState::Reauth);
            }
            Err(_) => self.set_save(txn, SaveState::Failed),
        }
    }
}

/// 受け入れ（#362）か SAS（#368）で作った `replace: true` のピンとマスター鍵への署名を保存し、保存できたら
/// 記録の登録待ちを外す（Web の `saveAcceptedMasterKey`）。
pub async fn save_accepted(
    api: &ApiClient,
    store: &SharedStore,
    accepted: &zoo_crypto::AcceptedMasterKey,
    reauth: Option<&str>,
) -> Result<()> {
    api.e2ee_upload_pins(std::slice::from_ref(&accepted.pin), true, reauth)
        .await?;
    let signature = &accepted.signature;
    api.e2ee_upload_signatures_with_reauth(
        &[json!({
            "target_user_id": signature.target_user_id,
            "target_key_id": signature.target_key_id,
            "signer_key_id": signature.signer_key_id,
            "signature": signature.signature,
        })],
        reauth,
    )
    .await?;
    store
        .lock()
        .await
        .confirm_accepted_master_key(&accepted.target_user_id, &accepted.master_key)
}

fn error_code(error: &anyhow::Error) -> SasErrorCode {
    if error.downcast_ref::<VerificationNeedsUnlock>().is_some() {
        SasErrorCode::NeedsUnlock
    } else {
        SasErrorCode::Failed
    }
}
