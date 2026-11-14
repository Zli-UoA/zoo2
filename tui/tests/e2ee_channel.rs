//! TUI の暗号化チャンネルと DM の送受信（docs/e2ee.md の「membership_epoch と送信の順序」「鍵の配布先の
//! 判定」「受信時の検証」。#361）。
//!
//! サーバーを模した状態（wiremock の応答に状態を持たせる）で、2 人のユーザーの暗号ストアのあいだで
//! 次を確かめる。暗号の処理は Web と同じ `zoo-crypto` なので、形式は Web とそのまま通じる。
//!
//! - 送信の要求に平文が入らず、相手は to-device で受け取った鍵で復号できる。自分の送ったものも読める。
//! - 409 `E2EE_STALE_EPOCH` を受けたら、新しいスナップショットで作り直したセッションで、同じ
//!   `message_id` で送り直す。
//! - 退出したメンバーには、それ以降のセッションの鍵を配らない。
//! - キューの上限で配れなかった端末（`undelivered`）には、次の送信の前に配り直す。
//! - 自己署名鍵で署名されていない端末には配らない。
//! - 解錠し直しが要るあいだは送らない。

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use tokio::sync::Mutex;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::any};
use zoo_crypto::{
    AttachmentInfo, BackupDecryptionKey, CrossSigningKeys, OwnSecrets, RevisionStatus,
    encrypt_attachment,
};
use zoo2_tui::api::ApiClient;
use zoo2_tui::e2ee::send::{self, ChannelSender, Edit, NewMessage, SendContext};
use zoo2_tui::e2ee::setup::SharedStore;
use zoo2_tui::e2ee::store::{
    CryptoStore, Decryption, EncryptedRow, MegolmError, SenderTrust, ToDeviceRow,
    UndecryptableReason,
};
use zoo2_tui::http::Credentials;
use zoo2_tui::mentions::Mentions;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c1";

/// 1 人のユーザーのサーバー側の鍵。
struct UserKeys {
    upload: Value,
    devices: BTreeMap<String, Value>,
    signatures: Vec<Value>,
    /// `keys/claim` で払い出すワンタイム鍵（端末ごと）。
    one_time_keys: BTreeMap<String, Vec<(String, Value)>>,
}

#[derive(Default)]
struct State {
    epoch: u64,
    members: BTreeSet<String>,
    users: BTreeMap<String, UserKeys>,
    /// `(ピンを置いた人, 相手)` → ピン。
    pins: BTreeMap<(String, String), Value>,
    /// 宛先の端末 → to-device の `(送信者, content)`。
    queues: BTreeMap<String, Vec<(String, Value)>>,
    /// 届いた送信と編集の本文。
    requests: Vec<Value>,
    /// メッセージの行。
    messages: BTreeMap<String, Value>,
    /// 次の送信を、epoch を 1 進めてから 409 にする。
    bump_before_next_message: bool,
    /// 次の to-device で、この端末を `undelivered` にする。
    undeliver_once: Option<String>,
}

#[derive(Clone, Default)]
struct Backend(Arc<StdMutex<State>>);

fn ok(data: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({ "data": data, "txid": "1" }))
}

fn coded(status: u16, code: &str, extra: Value) -> ResponseTemplate {
    let mut body = json!({ "error": code, "code": code });
    if let Value::Object(extra) = extra {
        body.as_object_mut().unwrap().extend(extra);
    }
    ResponseTemplate::new(status).set_body_json(body)
}

impl State {
    fn snapshot(&self, caller: &str) -> Value {
        let mut device_keys = serde_json::Map::new();
        let mut master_keys = serde_json::Map::new();
        let mut self_signing_keys = serde_json::Map::new();
        let mut signatures = Vec::new();
        for user_id in &self.members {
            let Some(keys) = self.users.get(user_id) else {
                continue;
            };
            device_keys.insert(user_id.clone(), json!(keys.devices));
            master_keys.insert(user_id.clone(), keys.upload["master_key"].clone());
            self_signing_keys.insert(user_id.clone(), keys.upload["self_signing_key"].clone());
            signatures.extend(keys.signatures.iter().cloned());
        }
        let tofu_pins: serde_json::Map<String, Value> = self
            .pins
            .iter()
            .filter(|((owner, _), _)| owner == caller)
            .map(|((_, target), pin)| (target.clone(), pin.clone()))
            .collect();
        json!({
            "channel_id": CHANNEL,
            "membership_epoch": self.epoch,
            "user_ids": self.members,
            "device_keys": device_keys,
            "master_keys": master_keys,
            "self_signing_keys": self_signing_keys,
            "user_signing_keys": { caller: self.users[caller].upload["user_signing_key"] },
            "signatures": signatures,
            "tofu_pins": tofu_pins,
            "next_after": null,
        })
    }
}

impl Respond for Backend {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut guard = self.0.lock().unwrap();
        let state: &mut State = &mut guard;
        let path = request.url.path().to_owned();
        let method = request.method.as_str().to_owned();
        let caller = request
            .headers
            .get("x-dev-user")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        match (method.as_str(), path.as_str()) {
            ("GET", p) if p == format!("/api/e2ee/channels/{CHANNEL}/recipients") => {
                if !state.members.contains(&caller) {
                    return ResponseTemplate::new(404);
                }
                ResponseTemplate::new(200).set_body_json(state.snapshot(&caller))
            }
            ("POST", "/api/e2ee/tofu_pins") => {
                for pin in body["pins"].as_array().unwrap() {
                    let target = pin["target_user_id"].as_str().unwrap().to_owned();
                    state.pins.insert((caller.clone(), target), pin.clone());
                }
                ok(json!({}))
            }
            ("POST", "/api/e2ee/keys/claim") => {
                let mut claimed = serde_json::Map::new();
                for (user_id, devices) in body["one_time_keys"].as_object().unwrap() {
                    let mut out = serde_json::Map::new();
                    for device_id in devices.as_object().unwrap().keys() {
                        if let Some((key_id, key)) = state
                            .users
                            .get_mut(user_id)
                            .and_then(|user| user.one_time_keys.get_mut(device_id))
                            .and_then(Vec::pop)
                        {
                            out.insert(device_id.clone(), json!({ key_id: key }));
                        }
                    }
                    claimed.insert(user_id.clone(), Value::Object(out));
                }
                ok(json!({ "one_time_keys": claimed }))
            }
            ("POST", "/api/e2ee/to_device") => {
                let mut undelivered = Vec::new();
                for (user_id, devices) in body["messages"].as_object().unwrap() {
                    for (device_id, content) in devices.as_object().unwrap() {
                        if state.undeliver_once.as_deref() == Some(device_id.as_str()) {
                            state.undeliver_once = None;
                            undelivered.push(json!({ "user_id": user_id, "device_id": device_id }));
                            continue;
                        }
                        state
                            .queues
                            .entry(device_id.clone())
                            .or_default()
                            .push((caller.clone(), content.clone()));
                    }
                }
                ok(json!({ "undelivered": undelivered }))
            }
            ("POST", "/api/messages") => {
                state.requests.push(body.clone());
                if state.bump_before_next_message {
                    state.bump_before_next_message = false;
                    state.epoch += 1;
                }
                if body["content"] != "" || body["ciphertext"].is_null() {
                    return coded(400, "E2EE_PLAINTEXT_FORBIDDEN", json!({}));
                }
                if body["epoch"].as_u64() != Some(state.epoch) {
                    return coded(
                        409,
                        "E2EE_STALE_EPOCH",
                        json!({ "current_epoch": state.epoch }),
                    );
                }
                let id = body["id"].as_str().unwrap().to_owned();
                let mut row = body.clone();
                row["user_id"] = json!(caller);
                state.messages.insert(id, row);
                ok(json!({}))
            }
            ("PATCH", p) if p.starts_with("/api/messages/") => {
                state.requests.push(body.clone());
                let id = p.trim_start_matches("/api/messages/").to_owned();
                let Some(row) = state.messages.get_mut(&id) else {
                    return ResponseTemplate::new(404);
                };
                let current = row["revision"].as_u64().unwrap();
                if body["revision"].as_u64() != Some(current + 1) {
                    return coded(
                        409,
                        "E2EE_STALE_REVISION",
                        json!({ "current_revision": current }),
                    );
                }
                row["revision"] = body["revision"].clone();
                row["ciphertext"] = body["ciphertext"].clone();
                row["epoch"] = body["epoch"].clone();
                ok(json!({}))
            }
            _ => ResponseTemplate::new(404),
        }
    }
}

struct Member {
    store: SharedStore,
    api: ApiClient,
    sender: ChannelSender,
    device_id: String,
    _dir: tempfile::TempDir,
}

impl Member {
    fn ctx(&self) -> SendContext {
        SendContext {
            api: self.api.clone(),
            store: self.store.clone(),
            sender: self.sender.clone(),
        }
    }

    /// to-device のキューから自分あてのものを受け取る。
    async fn receive(&self, backend: &Backend) -> usize {
        let (rows, device_keys) = {
            let mut state = backend.0.lock().unwrap();
            let queue = state.queues.remove(&self.device_id).unwrap_or_default();
            let mut device_keys = serde_json::Map::new();
            for (user_id, keys) in &state.users {
                device_keys.insert(user_id.clone(), json!(keys.devices));
            }
            let rows: Vec<ToDeviceRow> = queue
                .into_iter()
                .map(|(sender, content)| ToDeviceRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    sender_user_id: sender,
                    r#type: "m.room.encrypted".into(),
                    content,
                })
                .collect();
            (rows, Value::Object(device_keys))
        };
        self.store
            .lock()
            .await
            .process_to_device(&rows, &device_keys)
            .unwrap()
            .room_keys
    }

    async fn decrypt(&self, backend: &Backend, message_id: &str) -> Decryption {
        let row = backend.0.lock().unwrap().messages[message_id].clone();
        self.store.lock().await.decrypt_message(&EncryptedRow {
            channel_id: row["channel_id"].as_str().unwrap().into(),
            message_id: message_id.into(),
            parent_id: row["parent_id"].as_str().map(str::to_owned),
            user_id: row["user_id"].as_str().unwrap().into(),
            message_kind: "message".into(),
            epoch: row["epoch"].as_u64(),
            revision: row["revision"].as_u64().unwrap(),
            ciphertext: row["ciphertext"].clone(),
        })
    }
}

/// 初回設定を終えた（自分の鍵を保存し、端末に自己署名した）ユーザーの端末を作り、サーバーに鍵を置く。
fn member(server: &MockServer, backend: &Backend, user_id: &str) -> Member {
    let dir = tempfile::tempdir().unwrap();
    let mut store = CryptoStore::open(dir.path(), user_id).unwrap().store;
    store.mark_registered().unwrap();
    let secrets = OwnSecrets {
        cross_signing: CrossSigningKeys::generate(user_id).unwrap(),
        backup_key: BackupDecryptionKey::generate(),
    };
    store.save_own_keys(&secrets, Some(1)).unwrap();
    let device_id = store.device_id().to_owned();
    let device_keys = store.device_keys();
    let signature = secrets
        .cross_signing
        .self_signing
        .sign_device(&device_keys)
        .unwrap();
    let mut one_time_keys = Vec::new();
    if let Some(upload) = store.prepare_key_upload(0, false).unwrap() {
        for (key_id, key) in upload.one_time_keys.as_object().unwrap() {
            one_time_keys.push((key_id.clone(), key.clone()));
        }
        store.mark_keys_published().unwrap();
    }
    backend.0.lock().unwrap().users.insert(
        user_id.to_owned(),
        UserKeys {
            upload: secrets.cross_signing.upload_json(),
            devices: BTreeMap::from([(device_id.clone(), device_keys)]),
            signatures: vec![json!({
                "signer_user_id": user_id,
                "signer_key_id": signature.signer_key_id,
                "target_user_id": user_id,
                "target_key_id": signature.target_key_id,
                "signature": signature.signature,
            })],
            one_time_keys: BTreeMap::from([(device_id.clone(), one_time_keys)]),
        },
    );
    Member {
        store: Arc::new(Mutex::new(store)),
        api: ApiClient::new(
            reqwest::Client::new(),
            &server.uri(),
            Credentials::Dev(user_id.into()),
        ),
        sender: ChannelSender::default(),
        device_id,
        _dir: dir,
    }
}

async fn setup() -> (MockServer, Backend, Member, Member) {
    let server = MockServer::start().await;
    let backend = Backend::default();
    Mock::given(any())
        .respond_with(backend.clone())
        .mount(&server)
        .await;
    let alice = member(&server, &backend, ALICE);
    let bob = member(&server, &backend, BOB);
    {
        let mut state = backend.0.lock().unwrap();
        state.epoch = 1;
        state.members = BTreeSet::from([ALICE.to_owned(), BOB.to_owned()]);
    }
    (server, backend, alice, bob)
}

fn message(body: &str) -> NewMessage {
    NewMessage {
        channel_id: CHANNEL.into(),
        shape_epoch: Some(1),
        message_id: uuid::Uuid::new_v4().to_string(),
        parent_id: None,
        body: body.into(),
        mentions: Mentions::default(),
    }
}

fn body_of(decryption: &Decryption) -> &str {
    match decryption {
        Decryption::Decrypted(decrypted) => &decrypted.body,
        other => panic!("復号できていない: {other:?}"),
    }
}

#[tokio::test]
async fn alice_and_bob_exchange_encrypted_messages_without_plaintext_on_the_server() {
    let (_server, backend, alice, bob) = setup().await;
    let first = message("こんにちは @bob");
    let mut recipients = None;
    send::send_encrypted_message(&alice.ctx(), &first, &mut recipients)
        .await
        .unwrap();
    let summary = recipients.expect("配布先を判定した");
    assert_eq!(summary.user_count, 2);
    assert_eq!(summary.device_count, 1);
    assert!(summary.undeliverable.is_empty());
    // 初めて判定したチャンネルでは「増えた人」を出さない。
    assert!(summary.added_user_ids.is_empty());

    // サーバーに届いた要求に、平文の本文はない。
    {
        let state = backend.0.lock().unwrap();
        let sent = serde_json::to_string(&state.requests).unwrap();
        assert!(!sent.contains("こんにちは"));
        assert_eq!(state.requests[0]["content"], "");
        assert_eq!(state.requests[0]["epoch"], 1);
        assert_eq!(state.requests[0]["revision"], 0);
    }

    assert_eq!(bob.receive(&backend).await, 1);
    let received = bob.decrypt(&backend, &first.message_id).await;
    assert_eq!(body_of(&received), "こんにちは @bob");
    // 自分の送ったメッセージは、送信用セッションの受信用の写しで読める。
    let own = alice.decrypt(&backend, &first.message_id).await;
    assert_eq!(body_of(&own), "こんにちは @bob");
    let Decryption::Decrypted(own) = own else {
        unreachable!()
    };
    assert_eq!(own.sender, SenderTrust::Verified);

    // 返信（Bob から Alice へ）。
    let reply = NewMessage {
        parent_id: Some(first.message_id.clone()),
        ..message("返事です")
    };
    send::send_encrypted_message(&bob.ctx(), &reply, &mut None)
        .await
        .unwrap();
    assert_eq!(alice.receive(&backend).await, 1);
    assert_eq!(
        body_of(&alice.decrypt(&backend, &reply.message_id).await),
        "返事です"
    );

    // 続けて送っても、セッションを作り直さず、鍵も配り直さない。
    send::send_encrypted_message(&alice.ctx(), &message("2 通目"), &mut None)
        .await
        .unwrap();
    assert!(backend.0.lock().unwrap().queues.is_empty());

    // 編集は版を 1 つ進め、相手は新しい版を読める。
    let edit = Edit {
        channel_id: CHANNEL.into(),
        shape_epoch: Some(1),
        message_id: first.message_id.clone(),
        parent_id: None,
        revision: 1,
        body: "こんにちは（編集）".into(),
        kind: "message".into(),
        mentions: Mentions::default(),
        attachments: Vec::new(),
    };
    send::edit_encrypted_message(&alice.ctx(), &edit, &mut None)
        .await
        .unwrap();
    let edited = bob.decrypt(&backend, &first.message_id).await;
    assert_eq!(body_of(&edited), "こんにちは（編集）");
    let Decryption::Decrypted(edited) = edited else {
        unreachable!()
    };
    assert_eq!(edited.revision, RevisionStatus::Latest);
    // 同じ版の編集を送り直すと 409 `E2EE_STALE_REVISION`（画面が今の版と比べて扱う）。
    let error = send::edit_encrypted_message(&alice.ctx(), &edit, &mut None)
        .await
        .unwrap_err();
    assert_eq!(zoo2_tui::api::stale_revision(&error), Some(1));
}

fn attachment(name: &str) -> AttachmentInfo {
    AttachmentInfo {
        attachment_id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        mimetype: "application/pdf".into(),
        size: 3,
        width: None,
        height: None,
        sensitive: false,
        file: encrypt_attachment(b"pdf").file,
    }
}

#[tokio::test]
async fn editing_keeps_the_encrypted_attachments_of_the_original() {
    let (_server, backend, alice, bob) = setup().await;
    let first = message("添付付き");
    send::send_encrypted_message(&alice.ctx(), &first, &mut None)
        .await
        .unwrap();
    bob.receive(&backend).await;
    // Web などから添付付きで送った版（添付の鍵などは暗号化した本文にだけある）。
    let attached = vec![attachment("plan.pdf")];
    let mut edit = Edit {
        channel_id: CHANNEL.into(),
        shape_epoch: Some(1),
        message_id: first.message_id.clone(),
        parent_id: None,
        revision: 1,
        body: "添付付き".into(),
        kind: "message".into(),
        mentions: Mentions::default(),
        attachments: attached.clone(),
    };
    send::edit_encrypted_message(&alice.ctx(), &edit, &mut None)
        .await
        .unwrap();
    // TUI で本文だけを編集する。画面は、復号した元の版の添付をそのまま渡す。
    let Decryption::Decrypted(shown) = alice.decrypt(&backend, &first.message_id).await else {
        panic!("自分の送ったものを復号できない");
    };
    edit.revision = 2;
    edit.body = "本文だけ直した".into();
    edit.attachments = shown.attachments;
    send::edit_encrypted_message(&alice.ctx(), &edit, &mut None)
        .await
        .unwrap();
    let Decryption::Decrypted(received) = bob.decrypt(&backend, &first.message_id).await else {
        panic!("編集した版を復号できない");
    };
    assert_eq!(received.body, "本文だけ直した");
    assert_eq!(
        received.attachments, attached,
        "添付の鍵、IV、ハッシュが残る"
    );
}

#[tokio::test]
async fn a_stale_epoch_resends_the_same_message_with_a_new_session() {
    let (_server, backend, alice, bob) = setup().await;
    send::send_encrypted_message(&alice.ctx(), &message("最初"), &mut None)
        .await
        .unwrap();
    bob.receive(&backend).await;

    backend.0.lock().unwrap().bump_before_next_message = true;
    let second = message("epoch が変わったあと");
    send::send_encrypted_message(&alice.ctx(), &second, &mut None)
        .await
        .unwrap();
    {
        let state = backend.0.lock().unwrap();
        let tries: Vec<&Value> = state
            .requests
            .iter()
            .filter(|request| request["id"] == second.message_id.as_str())
            .collect();
        assert_eq!(tries.len(), 2, "同じ message_id で送り直す");
        assert_eq!(tries[0]["epoch"], 1);
        assert_eq!(tries[1]["epoch"], 2);
        assert_ne!(
            tries[0]["ciphertext"]["session_id"], tries[1]["ciphertext"]["session_id"],
            "新しいセッションで送り直す"
        );
    }
    // 新しいセッションの鍵が配られ、Bob は読める。
    assert_eq!(bob.receive(&backend).await, 1);
    assert_eq!(
        body_of(&bob.decrypt(&backend, &second.message_id).await),
        "epoch が変わったあと"
    );
}

#[tokio::test]
async fn a_member_who_left_gets_no_keys_for_later_messages() {
    let (_server, backend, alice, bob) = setup().await;
    send::send_encrypted_message(&alice.ctx(), &message("退出の前"), &mut None)
        .await
        .unwrap();
    bob.receive(&backend).await;

    // Bob が退出した（サーバーは epoch を進める）。Channels Shape も新しい epoch を返す。
    {
        let mut state = backend.0.lock().unwrap();
        state.members.remove(BOB);
        state.epoch += 1;
    }
    let after = NewMessage {
        shape_epoch: Some(2),
        ..message("退出のあと")
    };
    let mut recipients = None;
    send::send_encrypted_message(&alice.ctx(), &after, &mut recipients)
        .await
        .unwrap();
    assert_eq!(recipients.unwrap().user_count, 1);
    assert!(
        backend.0.lock().unwrap().queues.is_empty(),
        "退出した人には鍵を配らない"
    );
    assert_eq!(
        bob.decrypt(&backend, &after.message_id).await,
        Decryption::Undecryptable(UndecryptableReason::KeyMissing)
    );
}

#[tokio::test]
async fn undelivered_devices_get_the_key_before_the_next_message() {
    let (_server, backend, alice, bob) = setup().await;
    backend.0.lock().unwrap().undeliver_once = Some(bob.device_id.clone());
    let first = message("キューがいっぱい");
    send::send_encrypted_message(&alice.ctx(), &first, &mut None)
        .await
        .unwrap();
    assert_eq!(bob.receive(&backend).await, 0);

    let second = message("次の送信");
    send::send_encrypted_message(&alice.ctx(), &second, &mut None)
        .await
        .unwrap();
    // 同じセッションの鍵を、今のインデックスから配り直す。
    assert_eq!(bob.receive(&backend).await, 1);
    assert_eq!(
        body_of(&bob.decrypt(&backend, &second.message_id).await),
        "次の送信"
    );
    // 配った時点より前のメッセージは読めない（Megolm のラチェットは前にしか進まない）。
    assert_eq!(
        bob.decrypt(&backend, &first.message_id).await,
        Decryption::Undecryptable(UndecryptableReason::KeyMissing)
    );
}

#[tokio::test]
async fn devices_without_a_self_signing_signature_get_no_keys() {
    let (server, backend, alice, _bob) = setup().await;
    // Bob の 2 台目は、自己署名鍵で署名されていない（トークンを盗んだ攻撃者が登録した端末など）。
    let intruder = member(&server, &Backend::default(), BOB);
    {
        let mut state = backend.0.lock().unwrap();
        let bob = state.users.get_mut(BOB).unwrap();
        bob.devices.insert(
            intruder.device_id.clone(),
            intruder.store.try_lock().unwrap().device_keys(),
        );
    }
    let mut recipients = None;
    send::send_encrypted_message(&alice.ctx(), &message("秘密"), &mut recipients)
        .await
        .unwrap();
    assert_eq!(recipients.unwrap().device_count, 1);
    let state = backend.0.lock().unwrap();
    assert!(!state.queues.contains_key(&intruder.device_id));
    assert_eq!(state.queues.len(), 1);
}

#[tokio::test]
async fn sending_stops_while_the_device_needs_to_unlock_again() {
    let (_server, backend, alice, _bob) = setup().await;
    alice.store.lock().await.forget_secrets_revision().unwrap();
    let error = send::send_encrypted_message(&alice.ctx(), &message("送らない"), &mut None)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<MegolmError>(),
        Some(&MegolmError::NeedsUnlock)
    );
    let state = backend.0.lock().unwrap();
    assert!(state.requests.is_empty());
    assert!(state.queues.is_empty());
}

#[tokio::test]
async fn recipients_added_since_the_last_message_are_reported() {
    let (server, backend, alice, _bob) = setup().await;
    send::send_encrypted_message(&alice.ctx(), &message("2 人"), &mut None)
        .await
        .unwrap();
    // サーバーが Carol を足した（epoch が進む）。
    const CAROL: &str = "00000000-0000-4000-8000-00000000000c";
    let _carol = member(&server, &backend, CAROL);
    {
        let mut state = backend.0.lock().unwrap();
        state.members.insert(CAROL.to_owned());
        state.epoch += 1;
    }
    let mut recipients = None;
    send::send_encrypted_message(
        &alice.ctx(),
        &NewMessage {
            shape_epoch: Some(2),
            ..message("3 人")
        },
        &mut recipients,
    )
    .await
    .unwrap();
    let summary = recipients.unwrap();
    assert_eq!(summary.added_user_ids, vec![CAROL.to_owned()]);
    assert_eq!(summary.user_count, 3);
}

#[tokio::test]
async fn remembers_encrypted_channels_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut store = CryptoStore::open(dir.path(), ALICE).unwrap().store;
        store.remember_encrypted_channel(CHANNEL).unwrap();
    }
    let store = CryptoStore::open(dir.path(), ALICE).unwrap().store;
    assert_eq!(
        store.known_encrypted_channels().collect::<Vec<_>>(),
        vec![CHANNEL]
    );
}
