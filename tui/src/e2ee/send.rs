//! 暗号化チャンネルへの送信（docs/e2ee.md の「membership_epoch と送信の順序」「配布先のスナップショット」
//! 「鍵の配布先の判定」。#361）。React の `lib/e2ee/channelSend.ts` と `megolmApi.ts` と同じ流れ。
//!
//! 1. 送信用セッションがない、作り直しの条件に当たった、409 `E2EE_STALE_EPOCH` を受けた、のどれかなら、
//!    配布先のスナップショット（`recipients`）を取り直して判定する。初めて見た相手の TOFU のピンを置き、
//!    置けたら同じスナップショットで判定し直す（2 回まで）。
//! 2. 判定し直さないときも、必ず配り終えていない端末を確かめて配る（前の配布の途中の失敗、払い出され
//!    なかった端末、`undelivered` の端末）。配り終えるまで、本文は暗号化しない。配布の途中で失敗したら、
//!    だれにも配れていないかもしれないセッションで送らないよう、セッションを捨てる。
//! 3. 送る直前に暗号化して送る。409 `E2EE_STALE_EPOCH` を受けたら、`current_epoch` 以上のスナップショットで
//!    新しいセッションを作って配り直し、同じ `message_id` で送り直す。3 回続いたら止める。

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use zoo_crypto::{MessageContent, OutgoingMessage};

use super::setup::{self, SharedStore};
use super::store::{MegolmError, RecipientSummary, SharePlan};
use crate::api::{ApiClient, DeviceTarget, json_u64, stale_epoch};
use crate::mentions::Mentions;

/// 409 `E2EE_STALE_EPOCH` を続けて受けたら諦める回数（docs/e2ee.md の送信の手順 5）。
pub const STALE_EPOCH_ATTEMPTS: usize = 3;
/// `recipients` を最初のページから取り直す回数の上限。
const RECIPIENTS_ATTEMPTS: usize = 5;
/// ピンを置いたあとに判定し直す回数の上限。
const MAX_PIN_ROUNDS: usize = 2;

/// 送信を止めた（3 回続けて `E2EE_STALE_EPOCH` を受けた）。下書きに戻して利用者に知らせる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaleEpochError;

impl std::fmt::Display for StaleEpochError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("メンバーが変わり続けたため、送信できませんでした。もう一度送ってください")
    }
}

impl std::error::Error for StaleEpochError {}

#[derive(Default)]
struct Inner {
    /// チャンネルごとの、用意から暗号化までを直列にするロック。
    locks: HashMap<String, Arc<tokio::sync::Mutex<()>>>,
    /// チャンネルごとに知っている最新の `membership_epoch`。Channels Shape は遅れることがあるので、
    /// 409 で受けた `current_epoch` やスナップショットの値と大きい方を使う（遅れた値で送信用セッションを
    /// 毎回作り直さないため）。
    epochs: HashMap<String, u64>,
}

/// チャンネルごとの送信の直列化と、知っている epoch（画面が 1 つ持ち、送信のタスクに渡す）。
///
/// 続けて送ったときに、両方が同時にスナップショットを取り、同じ端末に鍵を 2 回配らないようにする。
/// 送信用セッションを捨てる処理（配布の失敗、作り直し）もこの中でだけ行うので、用意したセッションが
/// 暗号化の前に捨てられることはない（送信の要求は並べてよい）。
#[derive(Clone, Default)]
pub struct ChannelSender {
    inner: Arc<std::sync::Mutex<Inner>>,
}

impl ChannelSender {
    fn lock_for(&self, channel_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.inner
            .lock()
            .expect("送信の状態のロック")
            .locks
            .entry(channel_id.to_owned())
            .or_default()
            .clone()
    }

    fn note_epoch(&self, channel_id: &str, epoch: u64) -> u64 {
        let mut inner = self.inner.lock().expect("送信の状態のロック");
        let known = inner.epochs.entry(channel_id.to_owned()).or_insert(epoch);
        *known = (*known).max(epoch);
        *known
    }
}

/// 送信に使うもの。
#[derive(Clone)]
pub struct SendContext {
    pub api: ApiClient,
    pub store: SharedStore,
    pub sender: ChannelSender,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 配布先のスナップショット（`GET /api/e2ee/channels/{id}/recipients`）を全ページ取る。
/// ページの途中で `membership_epoch` が変わったら（409）、最初のページから取り直す。
/// `min_epoch` より古いスナップショットも取り直す（409 で受けた `current_epoch` 以上にするため）。
pub async fn fetch_recipients(
    api: &ApiClient,
    channel_id: &str,
    min_epoch: Option<u64>,
) -> Result<Vec<Value>> {
    for _ in 0..RECIPIENTS_ATTEMPTS {
        let mut pages = Vec::new();
        let mut epoch: Option<String> = None;
        let mut after: Option<String> = None;
        let fetched: Result<()> = async {
            loop {
                let page = api
                    .e2ee_channel_recipients(channel_id, after.as_deref(), epoch.as_deref())
                    .await?;
                if epoch.is_none() {
                    epoch = Some(
                        page.get("membership_epoch")
                            .and_then(json_u64)
                            .unwrap_or(0)
                            .to_string(),
                    );
                }
                after = page
                    .get("next_after")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                pages.push(page);
                if after.is_none() {
                    return Ok(());
                }
            }
        }
        .await;
        match fetched {
            Ok(()) => {}
            Err(error) if stale_epoch(&error).is_some() => continue,
            Err(error) => return Err(error),
        }
        let epoch: u64 = epoch.as_deref().unwrap_or("0").parse().unwrap_or(0);
        if min_epoch.is_some_and(|min| epoch < min) {
            continue;
        }
        return Ok(pages);
    }
    Err(anyhow!("配布先のスナップショットを取れませんでした"))
}

/// `recipients` を取り直して配布先を判定し、初めて見た相手の TOFU のピンを置く。
async fn evaluate_recipients(
    ctx: &SendContext,
    channel_id: &str,
    min_epoch: Option<u64>,
) -> Result<RecipientSummary> {
    let mut pages = fetch_recipients(&ctx.api, channel_id, min_epoch).await?;
    let mut round = 0;
    loop {
        let summary = ctx.store.lock().await.megolm_evaluate(channel_id, &pages)?;
        ctx.sender.note_epoch(channel_id, summary.epoch);
        let mut confirmed = Vec::new();
        let mut discarded = Vec::new();
        for pin in &summary.new_pins {
            if ctx.api.e2ee_post_tofu_pin(&pin.pin).await? {
                confirmed.push(pin.clone());
            } else {
                discarded.push(pin.clone());
            }
        }
        let settled = !confirmed.is_empty() || !discarded.is_empty();
        if settled {
            ctx.store
                .lock()
                .await
                .megolm_settle_pins(&confirmed, &discarded)?;
        }
        // 初めて見た相手は、ピンを置けるまで配布先に入らない。ピンを置けたら、同じスナップショットで
        // 判定し直してその端末を入れる。別の端末のピンとぶつかったら、そのピンを入れたスナップショットを
        // 取り直して判定し直す。
        if settled && round < MAX_PIN_ROUNDS {
            round += 1;
            if !discarded.is_empty() {
                pages = fetch_recipients(&ctx.api, channel_id, min_epoch).await?;
            }
            continue;
        }
        let added_user_ids = ctx
            .store
            .lock()
            .await
            .megolm_note_recipients(channel_id, &summary.user_ids)?;
        return Ok(RecipientSummary {
            added_user_ids,
            ..summary
        });
    }
}

/// 送信用セッションを用意し（epoch が違えば作り直す）、まだ配っていない端末に `zoo.room_key` を配る。
async fn share_with_pending_devices(ctx: &SendContext, channel_id: &str) -> Result<()> {
    let (plan, device_id): (SharePlan, String) = {
        let mut store = ctx.store.lock().await;
        let plan = store.megolm_start(channel_id, now_ms())?;
        (plan, store.device_id().to_owned())
    };
    if plan.created {
        // 送信用セッションの受信用の写しを、鍵バックアップに送る（解錠済みの端末だけ）。送信は待たない。
        let api = ctx.api.clone();
        let store = ctx.store.clone();
        tokio::spawn(async move {
            let _ = setup::upload_key_backup(&api, &store).await;
        });
    }
    if plan.pending == 0 {
        return Ok(());
    }
    let shared: Result<()> = async {
        let claimed = if plan.claim.is_empty() {
            serde_json::Map::new()
        } else {
            ctx.api.e2ee_claim_keys(&plan.claim).await?
        };
        let share = ctx.store.lock().await.megolm_share(channel_id, &claimed)?;
        let undelivered = ctx
            .api
            .e2ee_send_to_device(&device_id, &share.messages)
            .await?;
        // 払い出されなかった端末と `undelivered` の端末は、配った端末に入れない。次の送信の前にまた配る
        // （docs/e2ee.md の「to-device メッセージ」の送り直し。Web と同じ）。
        let delivered: Vec<DeviceTarget> = share
            .messages
            .iter()
            .flat_map(|(user_id, devices)| {
                devices.keys().map(|device_id| DeviceTarget {
                    user_id: user_id.clone(),
                    device_id: device_id.clone(),
                })
            })
            .filter(|target| !undelivered.contains(target))
            .collect();
        ctx.store
            .lock()
            .await
            .megolm_mark_shared(channel_id, &delivered);
        Ok(())
    }
    .await;
    if shared.is_err() {
        ctx.store.lock().await.megolm_discard(channel_id);
    }
    shared
}

/// 送信用セッションを用意し、まだ配っていない検証済みの端末に鍵を配る。`min_epoch` は 409 で受けた
/// `current_epoch`。配布先を判定し直したら、その結果を返す。
async fn prepare_outbound_session(
    ctx: &SendContext,
    channel_id: &str,
    shape_epoch: Option<u64>,
    min_epoch: Option<u64>,
) -> Result<Option<RecipientSummary>> {
    let epoch = ctx
        .sender
        .note_epoch(channel_id, min_epoch.or(shape_epoch).unwrap_or(0));
    let refresh = {
        let mut store = ctx.store.lock().await;
        // 解錠し直しが要るあいだは、暗号ストアの鍵を使う処理を止める（#360 の決まり）。配布先の判定には
        // 自分のユーザー署名鍵が要るので、まだ解錠していない端末も送れない。
        if !store.has_own_keys() || store.needs_unlock() {
            return Err(MegolmError::NeedsUnlock.into());
        }
        min_epoch.is_some() || store.megolm_needs_snapshot(channel_id, epoch, now_ms())
    };
    let summary = if refresh {
        Some(evaluate_recipients(ctx, channel_id, min_epoch).await?)
    } else {
        None
    };
    share_with_pending_devices(ctx, channel_id).await?;
    Ok(summary)
}

/// 送る直前に暗号化して `submit` で送る。409 `E2EE_STALE_EPOCH` を受けたら、`current_epoch` 以上の
/// スナップショットで新しいセッションを作って配り直し、同じ内容を暗号化し直して送り直す。3 回続いたら
/// [`StaleEpochError`]。配布先を判定し直したら、最後の結果を返す。
async fn encrypt_and_submit<F, Fut>(
    ctx: &SendContext,
    channel_id: &str,
    shape_epoch: Option<u64>,
    message: &OutgoingMessage,
    mut submit: F,
    recipients: &mut Option<RecipientSummary>,
) -> Result<()>
where
    F: FnMut(Value, u64) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut min_epoch = None;
    for _ in 0..STALE_EPOCH_ATTEMPTS {
        // 用意から暗号化までを、同じチャンネルのほかの送信の用意と重ねない。
        let (ciphertext, epoch) = {
            let lock = ctx.sender.lock_for(channel_id);
            let _guard = lock.lock().await;
            if let Some(summary) =
                prepare_outbound_session(ctx, channel_id, shape_epoch, min_epoch).await?
            {
                *recipients = Some(summary);
            }
            ctx.store.lock().await.megolm_encrypt(channel_id, message)?
        };
        match submit(ciphertext, epoch).await {
            Ok(()) => return Ok(()),
            Err(error) => {
                let Some(current) = stale_epoch(&error) else {
                    return Err(error);
                };
                // 古いセッションでは送らない。`current_epoch` 以上のスナップショットで判定し直すと、
                // epoch の違う送信用セッションは `megolm_start` が捨てて作り直す。
                min_epoch = Some(ctx.sender.note_epoch(channel_id, current));
            }
        }
    }
    Err(StaleEpochError.into())
}

fn mentions_json(mentions: &Mentions) -> Value {
    json!({
        "user_ids": mentions.user_ids,
        "channel": mentions.channel,
        "here": mentions.here,
    })
}

fn core_mentions(mentions: &Mentions) -> zoo_crypto::Mentions {
    zoo_crypto::Mentions {
        user_ids: mentions.user_ids.clone(),
        channel: mentions.channel,
        here: mentions.here,
    }
}

/// 新しいメッセージ（または返信）。
#[derive(Debug, Clone)]
pub struct NewMessage {
    pub channel_id: String,
    /// Channels Shape の `membership_epoch`。
    pub shape_epoch: Option<u64>,
    /// `messages.id`。送り直しでも同じ ID を使う。
    pub message_id: String,
    pub parent_id: Option<String>,
    pub body: String,
    pub mentions: Mentions,
}

/// 新しいメッセージを暗号化して `POST /api/messages` で送る。配布先を判定し直したら、その結果を返す。
pub async fn send_encrypted_message(
    ctx: &SendContext,
    message: &NewMessage,
    recipients: &mut Option<RecipientSummary>,
) -> Result<()> {
    let outgoing = OutgoingMessage {
        message_id: message.message_id.clone(),
        parent_id: message.parent_id.clone(),
        revision: 0,
        content: MessageContent {
            body: message.body.clone(),
            format: "mfm".into(),
            kind: "message".into(),
        },
        mentions: core_mentions(&message.mentions),
        attachments: Vec::new(),
    };
    let api = ctx.api.clone();
    encrypt_and_submit(
        ctx,
        &message.channel_id,
        message.shape_epoch,
        &outgoing,
        |ciphertext, epoch| {
            let mut body = json!({
                "id": message.message_id,
                "channel_id": message.channel_id,
                "content": "",
                "kind": "message",
                "epoch": epoch,
                "revision": 0,
                "ciphertext": ciphertext,
                "mentions": mentions_json(&message.mentions),
            });
            if let Some(parent_id) = &message.parent_id {
                body["parent_id"] = json!(parent_id);
            }
            let api = api.clone();
            async move { api.send_encrypted_message(body).await.map(|_| ()) }
        },
        recipients,
    )
    .await
}

/// 暗号化したメッセージの編集。
#[derive(Debug, Clone)]
pub struct Edit {
    pub channel_id: String,
    pub shape_epoch: Option<u64>,
    pub message_id: String,
    pub parent_id: Option<String>,
    /// 新しい版（今の版 + 1）。
    pub revision: u64,
    pub body: String,
    /// 元の版の `content.kind`。
    pub kind: String,
    pub mentions: Mentions,
}

/// 置き換えた本文を、版 `revision` で暗号化して `PATCH /api/messages/{id}` で送る。編集では通知を
/// 作らないので `mentions` は平文に入れない（暗号化したペイロードには入れる）。
pub async fn edit_encrypted_message(
    ctx: &SendContext,
    edit: &Edit,
    recipients: &mut Option<RecipientSummary>,
) -> Result<()> {
    let outgoing = OutgoingMessage {
        message_id: edit.message_id.clone(),
        parent_id: edit.parent_id.clone(),
        revision: edit.revision,
        content: MessageContent {
            body: edit.body.clone(),
            format: "mfm".into(),
            kind: edit.kind.clone(),
        },
        mentions: core_mentions(&edit.mentions),
        attachments: Vec::new(),
    };
    let api = ctx.api.clone();
    encrypt_and_submit(
        ctx,
        &edit.channel_id,
        edit.shape_epoch,
        &outgoing,
        |ciphertext, epoch| {
            let body = json!({
                "content": "",
                "epoch": epoch,
                "revision": edit.revision,
                "ciphertext": ciphertext,
            });
            let api = api.clone();
            let id = edit.message_id.clone();
            async move { api.edit_encrypted_message(&id, body).await.map(|_| ()) }
        },
        recipients,
    )
    .await
}
