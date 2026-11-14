//! 暗号化チャンネルの送信用セッション、配布先の判定、メッセージの復号（docs/e2ee.md の
//! 「membership_epoch と送信の順序」「鍵の配布先の判定」「受信時の検証」。#361）。
//!
//! React の `lib/e2ee/store/megolm.ts` と同じ規則。暗号の処理と判定は、すべて `zoo-crypto` の
//! [`OutboundSession`]、[`TrustStore::evaluate`]、[`InboundSessionStore::decrypt`] などを使う。
//! ここは通信をせず、送る要求の本文を作り、受けた応答を記録する（通信は `e2ee::send`）。
//!
//! - 送信用の Megolm セッションは暗号ストアに保存せず、メモリーにだけ置く（Web と同じ【決定】）。
//!   開き直したら、次の送信で新しいセッションを作って配り直す。自分のメッセージは、送信用セッションを
//!   受信用セッションとしても保存するので、開き直しても読める。
//! - 送信用の Olm セッションは、相手の端末の識別鍵ごとに 1 つを `state.json` の `outbound_olm` に置いて
//!   使い回す（受信で作った Olm セッションとは分ける。`keys/claim` のワンタイム鍵を使い切らないため）。
//! - 一度 `encrypted = true` を見たチャンネル（`encrypted_channels`）と、前回判定した配布先のメンバー
//!   （`recipients`）も `state.json` に置く。どちらもサーバーが知っている ID なので暗号化しない。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use serde_json::Value;
use zoo_crypto::{
    AttachmentInfo, CryptoError, DeviceKeys, EncryptedMessage, KeySnapshot, Mentions, NewTofuPin,
    OlmSession, OutboundSession, OutgoingMessage, Rejection, RevisionStatus, SessionOrigin,
    SessionRef, UserState,
};

use super::{CryptoStore, StoredInboundSession, inbound_key};
use crate::api::{DeviceTarget, json_u64};

/// 配れない相手の理由（送信画面に出す。docs/e2ee.md の「鍵の配布先の判定」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndeliverableReason {
    /// 暗号化を設定していない（クロス署名鍵がない）。
    NotSetUp,
    /// 暗号化の鍵が正しくない。
    InvalidKeys,
    /// 再検証が必要（マスター鍵が変わったなど）。
    NeedsReverification,
    /// 初めて見た相手のピンを置いている途中。
    PinPending,
    /// 自分の鍵を確かめられないので、今は配れない。
    Blocked,
}

impl UndeliverableReason {
    fn of(state: &UserState) -> Option<Self> {
        match state {
            UserState::Trusted(_) | UserState::NeedsUnlock => None,
            UserState::NoCrossSigningKeys => Some(Self::NotSetUp),
            UserState::InvalidCrossSigningKeys => Some(Self::InvalidKeys),
            UserState::NeedsReverification(_) => Some(Self::NeedsReverification),
            UserState::PinPending => Some(Self::PinPending),
            UserState::Blocked => Some(Self::Blocked),
        }
    }

    /// 送信画面の表示（React の `e2ee.undeliverable.*` と同じ文）。
    pub fn describe(self, name: &str) -> String {
        match self {
            Self::NotSetUp => format!("{name} さんは暗号化を設定していません"),
            Self::InvalidKeys => format!("{name} さんの暗号化の鍵が正しくありません"),
            Self::NeedsReverification => format!("{name} さんの再検証が必要です"),
            Self::PinPending => format!("{name} さんの鍵を確かめています"),
            Self::Blocked => format!("{name} さんには今は鍵を配れません"),
        }
    }
}

/// 配布先の判定の結果（送信画面の表示に使う）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecipientSummary {
    /// 配布先のスナップショットの `membership_epoch`。送信用セッションの epoch になる。
    pub epoch: u64,
    /// 初めて見た相手の TOFU のピン（`POST /api/e2ee/tofu_pins` で置く）。
    pub new_pins: Vec<NewTofuPin>,
    /// 配れないユーザーと理由（`user_id` の昇順）。
    pub undeliverable: Vec<(String, UndeliverableReason)>,
    /// 配る端末の数（今の端末を除く）。
    pub device_count: usize,
    /// 配るユーザーの数（自分を含む）。
    pub user_count: usize,
    /// スナップショットのメンバー（`user_id` の昇順）。
    pub user_ids: Vec<String>,
    /// 前回判定した配布先から増えたメンバー（[`CryptoStore::megolm_note_recipients`] が入れる）。
    /// 初めて判定したチャンネルでは空。
    pub added_user_ids: Vec<String>,
}

/// まだ鍵を配っていない端末。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SharePlan {
    /// 送信用の Olm セッションがまだない端末（`keys/claim` で払い出す）。`{user_id: [device_id]}`。
    pub claim: BTreeMap<String, Vec<String>>,
    /// 鍵を配る端末の数。0 なら配るものはない。
    pub pending: usize,
    /// 送信用セッションを作り直した（受信用の写しを保存した。鍵バックアップに送る）。
    pub created: bool,
}

/// to-device で送る `zoo.room_key`。
#[derive(Debug, Default)]
pub struct ShareResult {
    /// `{user_id: {device_id: content}}`。
    pub messages: BTreeMap<String, BTreeMap<String, Value>>,
    /// ワンタイム鍵もフォールバック鍵も払い出されず、配れなかった端末。次の送信の前にまた配る。
    pub missing: Vec<DeviceTarget>,
}

/// 送信を止める理由。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MegolmError {
    /// この端末はまだ解錠していない、または解錠し直しが要る（暗号ストアの鍵を使えない）。
    NeedsUnlock,
    /// 配布先のスナップショットがない。
    NoSnapshot,
    /// 送信用セッションがない。
    NoOutboundSession,
}

impl std::fmt::Display for MegolmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NeedsUnlock => "暗号化チャンネルに送るには、この端末で暗号化を解錠してください",
            Self::NoSnapshot => "配布先を判定していません",
            Self::NoOutboundSession => "送信用セッションがありません",
        })
    }
}

impl std::error::Error for MegolmError {}

/// 送信者の確かさ。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SenderTrust {
    /// Olm 由来のセッションで、送信端末をクロス署名でたどれた。
    Verified,
    /// Olm 由来だが、送信端末を検証済みにできなかった（「未検証の送信者」）。
    Unverified,
    /// 鍵バックアップ由来のセッションで復号した（「送信者を確かめられない」）。
    Unauthenticated,
}

/// 復号できなかった理由（React の `UndecryptableReason` と同じ）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndecryptableReason {
    /// 鍵がまだ届いていない（`zoo.room_key` 待ち）。
    KeyMissing,
    /// 暗号ストアの準備ができていない。
    Locked,
    /// セッションと行の `epoch` が合わない。
    StaleEpoch,
    /// 照合（チャンネル、行、種別、送信者、リプレイ、版）に失敗して破棄した。
    Invalid,
    /// そのほかの失敗。
    Failed,
    /// 暗号化チャンネルなのに暗号文がなく、`epoch` だけがある行。本文を出さない。
    Unencrypted,
}

impl UndecryptableReason {
    /// タイムラインに出す文（React の `e2ee.undecryptable.*` と同じ）。
    pub fn describe(self) -> &'static str {
        match self {
            Self::KeyMissing => "このメッセージを復号する鍵がまだ届いていません。",
            Self::Locked => "この端末で暗号化の準備ができると、このメッセージを読めます。",
            Self::StaleEpoch => "以前のメンバー向けに暗号化されたメッセージなので読めません。",
            Self::Invalid => "このメッセージは検証に失敗したため表示しません。",
            Self::Failed => "このメッセージを復号できませんでした。",
            Self::Unencrypted => "このメッセージは暗号化されていないため表示しません。",
        }
    }
}

/// 復号したメッセージ。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decrypted {
    /// 本文（MFM）。
    pub body: String,
    /// `message` か `emote`。
    pub kind: String,
    /// 暗号化した平文の `mentions`。表示はこれだけを使う。
    pub mentions: Mentions,
    pub sender: SenderTrust,
    /// 版の照合の結果。
    pub revision: RevisionStatus,
    /// 暗号化した添付（#364）。TUI は名前だけを出す。編集では鍵などを含めてそのまま送り直す。
    pub attachments: Vec<AttachmentInfo>,
}

/// 1 行の復号の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decryption {
    Decrypted(Decrypted),
    Undecryptable(UndecryptableReason),
}

/// 復号する行（`messages` の行のうち、照合に使う列）。
#[derive(Debug, Clone)]
pub struct EncryptedRow {
    pub channel_id: String,
    pub message_id: String,
    pub parent_id: Option<String>,
    pub user_id: String,
    pub message_kind: String,
    /// 行の `epoch`。暗号文があるのに `epoch` がない行は、照合できないので破棄する。
    pub epoch: Option<u64>,
    pub revision: u64,
    pub ciphertext: Value,
}

/// メモリーにだけ置く、チャンネルごとの送信用セッション。
pub(super) struct Outbound {
    session: OutboundSession,
    /// 鍵を配った端末（`(user_id, device_id)`）。
    shared: BTreeSet<(String, String)>,
}

/// 判定した配布先（送信用セッションを用意するまで持つ）。
pub(super) struct PendingDistribution {
    epoch: u64,
    devices: Vec<DeviceKeys>,
}

impl CryptoStore {
    /// 配布先のスナップショットを取り直す必要があるか。送信用セッションがない、期間やメッセージ数を
    /// 超えた、`membership_epoch`（`epoch`）が変わった、のどれかなら、古いセッションを捨てて `true`。
    pub fn megolm_needs_snapshot(&mut self, channel_id: &str, epoch: u64, now_ms: u64) -> bool {
        let Some(current) = self.outbound.get(channel_id) else {
            return true;
        };
        if current.session.needs_rotation(now_ms, epoch) {
            self.megolm_discard(channel_id);
            return true;
        }
        false
    }

    /// `recipients` の応答（全ページ）から配布先を判定する（規則 1〜6。`TrustStore::evaluate`）。
    ///
    /// 自分のユーザー署名鍵を使うので、解錠していない、または解錠し直しが要る（`needs_unlock`）なら
    /// [`MegolmError::NeedsUnlock`]。判定の記録は保存し直す。
    pub fn megolm_evaluate(
        &mut self,
        channel_id: &str,
        pages: &[Value],
    ) -> Result<RecipientSummary> {
        let (first, rest) = pages.split_first().ok_or(MegolmError::NoSnapshot)?;
        if self.needs_unlock() {
            return Err(MegolmError::NeedsUnlock.into());
        }
        let own = self.own_keys()?.ok_or(MegolmError::NeedsUnlock)?;
        let epoch = first
            .get("membership_epoch")
            .and_then(json_u64)
            .ok_or_else(|| anyhow!("配布先のスナップショットに membership_epoch がありません"))?;
        let mut snapshot = KeySnapshot::from_json(first)?;
        for page in rest {
            snapshot.extend(KeySnapshot::from_json(page)?);
        }
        let device_id = self.state.device_id.clone();
        let distribution = self
            .trust
            .evaluate(&snapshot, &device_id, Some(&own.user_signing));
        self.save_trust_store()?;
        let mut undeliverable: Vec<(String, UndeliverableReason)> = distribution
            .users
            .iter()
            .filter_map(|(user_id, user)| {
                UndeliverableReason::of(&user.state).map(|reason| (user_id.clone(), reason))
            })
            .collect();
        undeliverable.sort_by(|a, b| a.0.cmp(&b.0));
        let summary = RecipientSummary {
            epoch,
            new_pins: distribution.new_pins,
            device_count: distribution.devices.len(),
            user_count: distribution.users.len() - undeliverable.len(),
            user_ids: distribution.users.keys().cloned().collect(),
            undeliverable,
            added_user_ids: Vec::new(),
        };
        self.pending.insert(
            channel_id.to_owned(),
            PendingDistribution {
                epoch,
                devices: distribution.devices,
            },
        );
        Ok(summary)
    }

    /// TOFU のピンを置けたもの（`confirmed`）と、409 で置けなかったもの（`discarded`）を記録する。
    pub fn megolm_settle_pins(
        &mut self,
        confirmed: &[NewTofuPin],
        discarded: &[NewTofuPin],
    ) -> Result<()> {
        for pin in confirmed {
            self.trust.confirm_tofu_pin(pin);
        }
        for pin in discarded {
            self.trust.discard_tofu_pin(pin);
        }
        self.save_trust_store()
    }

    /// 配布先のメンバーを記録し、前回記録したメンバーから増えた人を返す。初めて記録するチャンネルは空
    /// （全員を「増えた」と出さない）。
    pub fn megolm_note_recipients(
        &mut self,
        channel_id: &str,
        user_ids: &[String],
    ) -> Result<Vec<String>> {
        let mut next = user_ids.to_vec();
        next.sort();
        next.dedup();
        let previous = self.state.recipients.get(channel_id).cloned();
        if previous.as_ref() != Some(&next) {
            self.state
                .recipients
                .insert(channel_id.to_owned(), next.clone());
            self.save()?;
        }
        let Some(previous) = previous else {
            return Ok(Vec::new());
        };
        let known: BTreeSet<&String> = previous.iter().collect();
        Ok(next.into_iter().filter(|id| !known.contains(id)).collect())
    }

    /// 判定した配布先で送信用セッションを用意する。epoch が違えば作り直す。まだ鍵を配っていない端末と、
    /// そのうち送信用の Olm セッションがない端末（`keys/claim` で払い出す）を返す。
    pub fn megolm_start(&mut self, channel_id: &str, now_ms: u64) -> Result<SharePlan> {
        let epoch = self
            .pending
            .get(channel_id)
            .ok_or(MegolmError::NoSnapshot)?
            .epoch;
        if self
            .outbound
            .get(channel_id)
            .is_some_and(|current| current.session.epoch() != epoch)
        {
            self.megolm_discard(channel_id);
        }
        let mut created = false;
        if !self.outbound.contains_key(channel_id) {
            let session = OutboundSession::new(channel_id, epoch, now_ms);
            // 自分のメッセージを、この端末でも（開き直したあとも）読めるようにする。
            let session_ref = SessionRef {
                channel_id: channel_id.to_owned(),
                sender_key: self.account.identity_keys().curve25519.to_base64(),
                session_id: session.session_id(),
            };
            let key = inbound_key(&session_ref);
            self.load_inbound(&key)?;
            self.inbound.add_outbound(&self.account, &session);
            if let Some(pickle) =
                self.inbound
                    .pickle_session(&session_ref, &self.account, &self.storage_key)
            {
                self.state.inbound_sessions.insert(
                    key,
                    StoredInboundSession {
                        pickle,
                        backed_up: false,
                    },
                );
                self.save()?;
            }
            self.inbound_generation += 1;
            self.outbound.insert(
                channel_id.to_owned(),
                Outbound {
                    session,
                    shared: BTreeSet::new(),
                },
            );
            created = true;
        }
        let current = &self.outbound[channel_id];
        let pending = &self.pending[channel_id];
        let mut plan = SharePlan {
            created,
            ..Default::default()
        };
        for device in &pending.devices {
            if current
                .shared
                .contains(&(device.user_id.clone(), device.device_id.clone()))
            {
                continue;
            }
            plan.pending += 1;
            if self
                .state
                .outbound_olm
                .contains_key(&device.curve25519.to_base64())
            {
                continue;
            }
            plan.claim
                .entry(device.user_id.clone())
                .or_default()
                .push(device.device_id.clone());
        }
        Ok(plan)
    }

    /// まだ配っていない端末あての `zoo.room_key` を Olm で暗号化する。送信用の Olm セッションがなければ、
    /// `claimed`（`keys/claim` の応答の `one_time_keys`）の鍵で作る。進めた Olm セッションは保存する。
    /// 配れたかは、to-device を送れたあとに [`megolm_mark_shared`](Self::megolm_mark_shared) で記録する。
    pub fn megolm_share(
        &mut self,
        channel_id: &str,
        claimed: &serde_json::Map<String, Value>,
    ) -> Result<ShareResult> {
        let (Some(pending), Some(current)) =
            (self.pending.get(channel_id), self.outbound.get(channel_id))
        else {
            return Err(MegolmError::NoOutboundSession.into());
        };
        let room_key = current.session.room_key();
        let mut result = ShareResult::default();
        let mut sessions: BTreeMap<String, String> = BTreeMap::new();
        for device in &pending.devices {
            if current
                .shared
                .contains(&(device.user_id.clone(), device.device_id.clone()))
            {
                continue;
            }
            let identity = device.curve25519.to_base64();
            let stored = sessions
                .get(&identity)
                .or_else(|| self.state.outbound_olm.get(&identity));
            let mut olm = match stored {
                Some(pickle) => OlmSession::from_pickle(pickle, &self.account, &self.storage_key)?,
                None => {
                    let one_time_key = claimed
                        .get(&device.user_id)
                        .and_then(|devices| devices.get(&device.device_id))
                        .and_then(Value::as_object)
                        .and_then(|keys| keys.values().next());
                    let Some(one_time_key) = one_time_key else {
                        result.missing.push(DeviceTarget {
                            user_id: device.user_id.clone(),
                            device_id: device.device_id.clone(),
                        });
                        continue;
                    };
                    match self
                        .account
                        .create_outbound_olm_session(device, one_time_key)
                    {
                        Ok(olm) => olm,
                        // 払い出された鍵の署名が合わない端末には配らない（次の送信でまた払い出す）。
                        Err(_) => {
                            result.missing.push(DeviceTarget {
                                user_id: device.user_id.clone(),
                                device_id: device.device_id.clone(),
                            });
                            continue;
                        }
                    }
                }
            };
            let content = self.account.encrypt_room_key(&mut olm, device, &room_key)?;
            result
                .messages
                .entry(device.user_id.clone())
                .or_default()
                .insert(device.device_id.clone(), content);
            sessions.insert(identity, olm.pickle(&self.account, &self.storage_key));
        }
        if !sessions.is_empty() {
            self.state.outbound_olm.extend(sessions);
            self.save()?;
        }
        Ok(result)
    }

    /// to-device を送れた端末を、鍵を配った端末として記録する。
    pub fn megolm_mark_shared(&mut self, channel_id: &str, devices: &[DeviceTarget]) {
        if let Some(current) = self.outbound.get_mut(channel_id) {
            for device in devices {
                current
                    .shared
                    .insert((device.user_id.clone(), device.device_id.clone()));
            }
        }
    }

    /// 送信用セッションを捨てる（配布の途中で失敗したときなど）。次の送信で作り直して配る。
    pub fn megolm_discard(&mut self, channel_id: &str) {
        self.outbound.remove(channel_id);
    }

    /// メッセージを暗号化する。`ciphertext` と、送信の要求に入れる `epoch` を返す。
    pub fn megolm_encrypt(
        &mut self,
        channel_id: &str,
        message: &OutgoingMessage,
    ) -> Result<(Value, u64)> {
        let current = self
            .outbound
            .get_mut(channel_id)
            .ok_or(MegolmError::NoOutboundSession)?;
        let ciphertext = current.session.encrypt(&self.account, message)?;
        Ok((ciphertext, current.session.epoch()))
    }

    /// `encrypted = true` を見たチャンネルを記録する（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
    /// 保存できなければ記録に入れずにエラーを返す（呼び出し側が次の機会に記録し直す）。
    pub fn remember_encrypted_channel(&mut self, channel_id: &str) -> Result<()> {
        if self.state.encrypted_channels.insert(channel_id.to_owned())
            && let Err(error) = self.save()
        {
            self.state.encrypted_channels.remove(channel_id);
            return Err(error);
        }
        Ok(())
    }

    /// 暗号化チャンネルとして記録したチャンネル。
    pub fn known_encrypted_channels(&self) -> impl Iterator<Item = &String> {
        self.state.encrypted_channels.iter()
    }

    /// 受信用セッションが増えた回数（このプロセスで）。鍵がまだ届いていないメッセージを復号し直す
    /// きっかけに使う。
    pub fn inbound_generation(&self) -> u64 {
        self.inbound_generation
    }

    /// `messages` の行を復号し、行と照合する（docs/e2ee.md の「受信時の検証」）。
    pub fn decrypt_message(&mut self, row: &EncryptedRow) -> Decryption {
        let sender_key = row.ciphertext.get("sender_key").and_then(Value::as_str);
        let session_id = row.ciphertext.get("session_id").and_then(Value::as_str);
        let (Some(sender_key), Some(session_id), Some(epoch)) = (sender_key, session_id, row.epoch)
        else {
            return Decryption::Undecryptable(UndecryptableReason::Invalid);
        };
        let key = inbound_key(&SessionRef {
            channel_id: row.channel_id.clone(),
            sender_key: sender_key.to_owned(),
            session_id: session_id.to_owned(),
        });
        if self.load_inbound(&key).is_err() {
            return Decryption::Undecryptable(UndecryptableReason::Failed);
        }
        let result = self.inbound.decrypt(&EncryptedMessage {
            channel_id: &row.channel_id,
            message_id: &row.message_id,
            parent_id: row.parent_id.as_deref(),
            user_id: &row.user_id,
            message_kind: &row.message_kind,
            epoch,
            revision: row.revision,
            ciphertext: &row.ciphertext,
        });
        match result {
            Ok(decrypted) => Decryption::Decrypted(Decrypted {
                body: decrypted.payload.content.body,
                kind: decrypted.payload.content.kind,
                mentions: decrypted.payload.mentions,
                sender: match (decrypted.origin, decrypted.sender_verified) {
                    (SessionOrigin::Backup, _) => SenderTrust::Unauthenticated,
                    (SessionOrigin::Olm, true) => SenderTrust::Verified,
                    (SessionOrigin::Olm, false) => SenderTrust::Unverified,
                },
                revision: decrypted.revision,
                attachments: decrypted.payload.attachments,
            }),
            Err(CryptoError::MissingRoomKey { .. }) => {
                Decryption::Undecryptable(UndecryptableReason::KeyMissing)
            }
            Err(CryptoError::Rejected(Rejection::EpochMismatch)) => {
                Decryption::Undecryptable(UndecryptableReason::StaleEpoch)
            }
            Err(CryptoError::Rejected(_)) => {
                Decryption::Undecryptable(UndecryptableReason::Invalid)
            }
            Err(_) => Decryption::Undecryptable(UndecryptableReason::Failed),
        }
    }
}
