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
    /// 鍵バックアップの今の版と、上がっている行。
    backup_version: Option<Value>,
    backup_rows: Vec<Value>,
    /// 上がってきた行の数（取り込んだセッションを上げ直していないかを見る）。
    backup_posted: usize,
    /// 読まれたページの `after`。
    backup_reads: Vec<Option<String>>,
    /// この `after` のページを読まれたら、1 回だけ 500 にする。
    fail_backup_read: Option<Option<String>>,
    /// ページを読まれたら、いつも 500 にする。
    fail_backup_reads: bool,
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
    /// `keys/query` の応答（全ユーザー）。
    fn query(&self) -> Value {
        let mut device_keys = serde_json::Map::new();
        let mut master_keys = serde_json::Map::new();
        let mut self_signing_keys = serde_json::Map::new();
        let mut signatures = Vec::new();
        for (user_id, keys) in &self.users {
            device_keys.insert(user_id.clone(), json!(keys.devices));
            master_keys.insert(user_id.clone(), keys.upload["master_key"].clone());
            self_signing_keys.insert(user_id.clone(), keys.upload["self_signing_key"].clone());
            signatures.extend(keys.signatures.iter().cloned());
        }
        json!({
            "device_keys": device_keys,
            "master_keys": master_keys,
            "self_signing_keys": self_signing_keys,
            "signatures": signatures,
        })
    }

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
            ("GET", "/api/e2ee/key_backup/versions/current") => match &state.backup_version {
                Some(version) => ResponseTemplate::new(200).set_body_json(version.clone()),
                None => ResponseTemplate::new(404),
            },
            ("POST", "/api/e2ee/key_backup/keys") => {
                state.backup_posted += body["sessions"].as_array().unwrap().len();
                for row in body["sessions"].as_array().unwrap() {
                    let same = |other: &Value| {
                        ["channel_id", "sender_key", "session_id"]
                            .iter()
                            .all(|field| other[field] == row[field])
                    };
                    state.backup_rows.retain(|other| !same(other));
                    state.backup_rows.push(row.clone());
                }
                ok(json!({}))
            }
            ("GET", "/api/e2ee/key_backup/keys") => {
                // 1 行ずつのページで返す（`next` をたどることを確かめる）。
                let after = request
                    .url
                    .query_pairs()
                    .find(|(key, _)| key == "after")
                    .map(|(_, value)| value.into_owned());
                state.backup_reads.push(after.clone());
                if state.fail_backup_reads {
                    return ResponseTemplate::new(500);
                }
                if state.fail_backup_read.as_ref() == Some(&after) {
                    state.fail_backup_read = None;
                    return ResponseTemplate::new(500);
                }
                let index: usize = after.map_or(0, |after| after.parse().unwrap());
                let page: Vec<Value> = state
                    .backup_rows
                    .iter()
                    .skip(index)
                    .take(1)
                    .cloned()
                    .collect();
                let next = (index + 1 < state.backup_rows.len()).then(|| (index + 1).to_string());
                ResponseTemplate::new(200).set_body_json(json!({ "sessions": page, "next": next }))
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
    secrets: OwnSecrets,
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
        let (rows, query) = {
            let mut state = backend.0.lock().unwrap();
            let queue = state.queues.remove(&self.device_id).unwrap_or_default();
            let query = state.query();
            let rows: Vec<ToDeviceRow> = queue
                .into_iter()
                .map(|(sender, content)| ToDeviceRow {
                    id: uuid::Uuid::new_v4().to_string(),
                    sender_user_id: sender,
                    r#type: "m.room.encrypted".into(),
                    content,
                })
                .collect();
            (rows, query)
        };
        self.store
            .lock()
            .await
            .process_to_device_with_keys(&rows, &query)
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
    let secrets = OwnSecrets {
        cross_signing: CrossSigningKeys::generate(user_id).unwrap(),
        backup_key: BackupDecryptionKey::generate(),
    };
    member_with(server, backend, user_id, secrets)
}

/// 同じユーザーの別の端末（同じ秘密鍵で解錠した端末）。
fn other_device(server: &MockServer, backend: &Backend, of: &Member) -> Member {
    let user_id = of.secrets.cross_signing.master.user_id();
    let pair = |pair: &zoo_crypto::CrossSigningKeyPair| {
        zoo_crypto::CrossSigningKeyPair::from_seed(user_id, pair.usage(), &pair.seed()).unwrap()
    };
    let secrets = OwnSecrets {
        cross_signing: CrossSigningKeys {
            master: pair(&of.secrets.cross_signing.master),
            self_signing: pair(&of.secrets.cross_signing.self_signing),
            user_signing: pair(&of.secrets.cross_signing.user_signing),
        },
        backup_key: BackupDecryptionKey::from_bytes(&of.secrets.backup_key.to_bytes()),
    };
    member_with(server, backend, user_id, secrets)
}

fn member_with(
    server: &MockServer,
    backend: &Backend,
    user_id: &str,
    secrets: OwnSecrets,
) -> Member {
    let dir = tempfile::tempdir().unwrap();
    let mut store = CryptoStore::open(dir.path(), user_id).unwrap().store;
    store.mark_registered().unwrap();
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
    {
        let mut state = backend.0.lock().unwrap();
        let keys = state
            .users
            .entry(user_id.to_owned())
            .or_insert_with(|| UserKeys {
                upload: secrets.cross_signing.upload_json(),
                devices: BTreeMap::new(),
                signatures: Vec::new(),
                one_time_keys: BTreeMap::new(),
            });
        keys.devices.insert(device_id.clone(), device_keys);
        keys.signatures.push(json!({
            "signer_user_id": user_id,
            "signer_key_id": signature.signer_key_id,
            "target_user_id": user_id,
            "target_key_id": signature.target_key_id,
            "signature": signature.signature,
        }));
        keys.one_time_keys.insert(device_id.clone(), one_time_keys);
    }
    Member {
        store: Arc::new(Mutex::new(store)),
        api: ApiClient::new(
            reqwest::Client::new(),
            &server.uri(),
            Credentials::Dev(user_id.into()),
        ),
        sender: ChannelSender::default(),
        device_id,
        secrets,
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
    let mut store = CryptoStore::open(dir.path(), ALICE).unwrap().store;
    assert_eq!(
        store.known_encrypted_channels().collect::<Vec<_>>(),
        vec![CHANNEL]
    );
    // 画面は、暗号ストアを開く前（ほかが開いているあいだ）にも記録を読める。
    assert_eq!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE).unwrap(),
        vec![CHANNEL.to_owned()]
    );
    assert!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), BOB)
            .unwrap()
            .is_empty()
    );

    // 保存できなければ記録に入れない（次の機会に記録し直せるように）。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        const OTHER: &str = "00000000-0000-4000-8000-0000000000c2";
        let user_dir = dir.path().join(ALICE);
        std::fs::set_permissions(&user_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let failed = store.remember_encrypted_channel(OTHER);
        std::fs::set_permissions(&user_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        if failed.is_err() {
            assert!(!store.known_encrypted_channels().any(|id| id == OTHER));
        }
        store.remember_encrypted_channel(OTHER).unwrap();
        assert!(store.known_encrypted_channels().any(|id| id == OTHER));
    }

    // 保存鍵を失って作り直しても、記録は残る。
    drop(store);
    std::fs::remove_file(dir.path().join(ALICE).join("storage.key")).unwrap();
    let opened = CryptoStore::open(dir.path(), ALICE).unwrap();
    assert!(opened.replaced_device_id.is_some(), "端末は作り直す");
    assert!(
        opened
            .store
            .known_encrypted_channels()
            .any(|id| id == CHANNEL)
    );
    drop(opened);

    // 作り直しの途中（前の `state.json` を別の名前に移したあと、新しいものを保存する前）に止まっても、
    // 記録を読めて、次に開いたときに引き継ぐ。
    let user_dir = dir.path().join(ALICE);
    std::fs::rename(
        user_dir.join("state.json"),
        user_dir.join("state.replaced.json"),
    )
    .unwrap();
    std::fs::remove_file(user_dir.join("storage.key")).unwrap();
    assert!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE)
            .unwrap()
            .contains(&CHANNEL.to_owned())
    );
    let opened = CryptoStore::open(dir.path(), ALICE).unwrap();
    assert!(
        opened
            .store
            .known_encrypted_channels()
            .any(|id| id == CHANNEL)
    );
    assert!(
        !user_dir.join("state.replaced.json").exists(),
        "保存し終えたら消す"
    );
    drop(opened);

    // 記録のファイルを読めなければ、記録がないものとせずにエラーにする。
    std::fs::write(dir.path().join(ALICE).join("state.json"), b"{broken").unwrap();
    assert!(zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE).is_err());
}

#[test]
fn records_written_before_the_store_opens_are_read_back() {
    const OTHER: &str = "00000000-0000-4000-8000-0000000000c3";
    let dir = tempfile::tempdir().unwrap();
    // 暗号ストアを開く前（端末の登録の前）でも、画面がその場で記録できる。
    zoo2_tui::e2ee::store::record_encrypted_channels(dir.path(), ALICE, &[CHANNEL.to_owned()])
        .unwrap();
    assert_eq!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE).unwrap(),
        vec![CHANNEL.to_owned()]
    );
    // 暗号ストアを開いて、その記録（`state.json`）と合わせて読む。
    {
        let mut store = CryptoStore::open(dir.path(), ALICE).unwrap().store;
        store.remember_encrypted_channel(OTHER).unwrap();
    }
    assert_eq!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE).unwrap(),
        vec![CHANNEL.to_owned(), OTHER.to_owned()]
    );
    // 別のユーザーの記録としては読まない。
    assert!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), BOB)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_reopened_store_starts_its_own_import_while_the_old_one_waits() {
    use zoo2_tui::e2ee::setup;
    let (server, backend, alice, _bob) = setup().await;
    {
        let mut state = backend.0.lock().unwrap();
        state.backup_version = Some(json!({
            "version": "1",
            "algorithm": zoo_crypto::BACKUP_ALGORITHM,
            "auth_data": alice.secrets.backup_key.auth_data(&alice.secrets.cross_signing.master).unwrap(),
        }));
        state.fail_backup_reads = true;
    }
    let reads = |backend: &Backend| backend.0.lock().unwrap().backup_reads.len();
    async fn wait_for(backend: &Backend, count: usize) {
        for _ in 0..200 {
            if backend.0.lock().unwrap().backup_reads.len() >= count {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    // 古い暗号ストアの取り込みが失敗して、試し直しを待っている。
    let old = other_device(&server, &backend, &alice);
    setup::spawn_backup_import(old.api.clone(), &old.store);
    wait_for(&backend, 1).await;
    assert_eq!(reads(&backend), 1);
    // 古い暗号ストアを手放し、開き直した暗号ストアで取り込みを頼む。
    drop(old);
    let reopened = other_device(&server, &backend, &alice);
    setup::spawn_backup_import(reopened.api.clone(), &reopened.store);
    wait_for(&backend, 2).await;
    assert_eq!(reads(&backend), 2, "開き直した暗号ストアの取り込みを始める");
    // 同じ暗号ストアには重ねない。
    setup::spawn_backup_import(reopened.api.clone(), &reopened.store);
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(reads(&backend), 2);
}

#[test]
fn concurrent_records_keep_every_channel() {
    // 同じユーザーの zoo2-tui が 2 つ動いていて、同時に別のチャンネルを記録しても、どれも消えない。
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_owned();
    let ids: Vec<String> = (0..16)
        .map(|i| format!("00000000-0000-4000-8000-{i:012}"))
        .collect();
    let threads: Vec<_> = ids
        .iter()
        .cloned()
        .map(|id| {
            let root = root.clone();
            std::thread::spawn(move || {
                zoo2_tui::e2ee::store::record_encrypted_channels(&root, ALICE, &[id]).unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        zoo2_tui::e2ee::store::read_encrypted_channels(&root, ALICE).unwrap(),
        ids
    );
}

#[test]
fn records_survive_a_failed_rebuild_after_losing_the_storage_key() {
    let dir = tempfile::tempdir().unwrap();
    let user_dir = dir.path().join(ALICE);
    {
        let mut store = CryptoStore::open(dir.path(), ALICE).unwrap().store;
        store.remember_encrypted_channel(CHANNEL).unwrap();
    }
    std::fs::remove_file(user_dir.join("storage.key")).unwrap();
    // 新しい `state.json` を保存できない（一時ファイルの場所をディレクトリーでふさぐ）。
    std::fs::create_dir(user_dir.join("state.tmp")).unwrap();
    assert!(CryptoStore::open(dir.path(), ALICE).is_err());
    assert!(!user_dir.join("state.json").exists());
    // 作り直しに失敗しても、前の記録は読める。
    assert!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE)
            .unwrap()
            .contains(&CHANNEL.to_owned())
    );
    // 作り直しの前に、その場の記録（作り直しで消さない）にも写してある。`state.json` も退避したものも
    // 見えない順に当たっても（退避したものを消したあとに読んだときと同じ）、記録を読める。
    let replaced = std::fs::read(user_dir.join("state.replaced.json")).unwrap();
    std::fs::remove_file(user_dir.join("state.replaced.json")).unwrap();
    assert!(!user_dir.join("state.json").exists());
    assert!(
        zoo2_tui::e2ee::store::read_encrypted_channels(dir.path(), ALICE)
            .unwrap()
            .contains(&CHANNEL.to_owned())
    );
    std::fs::write(user_dir.join("state.replaced.json"), replaced).unwrap();
    // 次に作り直せたら、記録を引き継ぎ、前のものを消す。
    std::fs::remove_dir(user_dir.join("state.tmp")).unwrap();
    let opened = CryptoStore::open(dir.path(), ALICE).unwrap();
    assert!(
        opened
            .store
            .known_encrypted_channels()
            .any(|id| id == CHANNEL)
    );
    assert!(!user_dir.join("state.replaced.json").exists());
}

#[test]
fn reading_records_while_the_store_is_rebuilt_keeps_them() {
    let dir = tempfile::tempdir().unwrap();
    let user_dir = dir.path().join(ALICE);
    {
        let mut store = CryptoStore::open(dir.path(), ALICE).unwrap().store;
        store.remember_encrypted_channel(CHANNEL).unwrap();
    }
    // 保存鍵を失った作り直しが、前の `state.json` を退避したところで止まっている。
    std::fs::remove_file(user_dir.join("storage.key")).unwrap();
    std::fs::create_dir(user_dir.join("state.tmp")).unwrap();
    assert!(CryptoStore::open(dir.path(), ALICE).is_err());
    assert!(!user_dir.join("state.json").exists());
    assert!(user_dir.join("state.replaced.json").exists());
    // 読み取りが `state.json` を見つけられなかった直後に止まり、そのあいだに作り直しが終わる（退避した
    // ファイルは消える）。その場の記録にも頼らないよう、それも消しておく。
    let root = dir.path().to_owned();
    let channels =
        zoo2_tui::e2ee::store::read_encrypted_channels_pausing(dir.path(), ALICE, || {
            std::fs::remove_file(user_dir.join("encrypted-channels.json")).unwrap();
            std::fs::remove_dir(user_dir.join("state.tmp")).unwrap();
            let opened = CryptoStore::open(&root, ALICE).unwrap();
            assert!(!user_dir.join("state.replaced.json").exists());
            drop(opened);
        })
        .unwrap();
    // 読み直した新しい `state.json` に引き継がれた記録を読む（画面はこれで格下げを見つけ、平文で送らない）。
    assert_eq!(channels, vec![CHANNEL.to_owned()]);
}

#[tokio::test]
async fn the_previous_recipients_are_kept_when_saving_fails() {
    let (_server, backend, alice, _bob) = setup().await;
    send::send_encrypted_message(&alice.ctx(), &message("2 人"), &mut None)
        .await
        .unwrap();
    const CAROL: &str = "00000000-0000-4000-8000-00000000000c";
    let mut store = alice.store.lock().await;
    let members = vec![ALICE.to_owned(), BOB.to_owned(), CAROL.to_owned()];
    // 書き込めないディレクトリーで保存を失敗させる（root などで書けてしまう環境では、成功として確かめる）。
    #[cfg(unix)]
    let saved = {
        use std::os::unix::fs::PermissionsExt;
        let user_dir = alice._dir.path().join(ALICE);
        std::fs::set_permissions(&user_dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let first = store.megolm_note_recipients(CHANNEL, &members);
        std::fs::set_permissions(&user_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        first.is_ok()
    };
    #[cfg(not(unix))]
    let saved = store.megolm_note_recipients(CHANNEL, &members).is_ok();
    let retried = store.megolm_note_recipients(CHANNEL, &members).unwrap();
    if saved {
        // 保存できていれば、記録は更新済みなので、同じ配布先で増えた人はいない。
        assert!(retried.is_empty());
    } else {
        // 保存できなかったときは前の記録に戻すので、次の判定で増えた人をもう一度出す。
        assert_eq!(retried, vec![CAROL.to_owned()]);
    }
    let _ = backend;
}

fn sender_of(decryption: &Decryption) -> SenderTrust {
    match decryption {
        Decryption::Decrypted(decrypted) => decrypted.sender,
        other => panic!("復号できていない: {other:?}"),
    }
}

#[tokio::test]
async fn messages_from_my_other_device_are_verified() {
    let (server, backend, alice, _bob) = setup().await;
    let laptop = other_device(&server, &backend, &alice);
    let sent = message("別の端末から");
    send::send_encrypted_message(&laptop.ctx(), &sent, &mut None)
        .await
        .unwrap();
    assert_eq!(alice.receive(&backend).await, 1);
    assert_eq!(
        sender_of(&alice.decrypt(&backend, &sent.message_id).await),
        SenderTrust::Verified
    );
}

#[tokio::test]
async fn messages_from_users_trusted_by_tofu_are_verified() {
    let (_server, backend, alice, bob) = setup().await;
    // Bob はまだ Alice を判定していない（記録がない）ので、受信の処理だけでは信頼しない。
    let before = message("Bob が判定する前");
    send::send_encrypted_message(&alice.ctx(), &before, &mut None)
        .await
        .unwrap();
    bob.receive(&backend).await;
    assert_eq!(
        sender_of(&bob.decrypt(&backend, &before.message_id).await),
        SenderTrust::Unverified
    );
    // Bob が送ると、Alice を TOFU で信頼する（ピンを置く）。そのあとの Alice の新しいセッションは検証済み。
    send::send_encrypted_message(&bob.ctx(), &message("Bob から"), &mut None)
        .await
        .unwrap();
    {
        let mut state = backend.0.lock().unwrap();
        state.epoch += 1;
    }
    let after = NewMessage {
        shape_epoch: Some(2),
        ..message("Bob が判定したあと")
    };
    send::send_encrypted_message(&alice.ctx(), &after, &mut None)
        .await
        .unwrap();
    bob.receive(&backend).await;
    assert_eq!(
        sender_of(&bob.decrypt(&backend, &after.message_id).await),
        SenderTrust::Verified
    );
}

/// Alice のものとして登録された `account` の端末から、Bob に直接セッション鍵を送り、そのセッションで
/// 暗号化したメッセージを置く（送信の判定を通さない端末。盗んだトークンで登録した端末など）。
fn send_from_rogue_device(
    backend: &Backend,
    account: &zoo_crypto::DeviceAccount,
    bob: &Member,
    message_id: &str,
) {
    let mut state = backend.0.lock().unwrap();
    let bob_keys = state.users[BOB].devices[&bob.device_id].clone();
    let recipient = zoo_crypto::verify_device_keys(&bob_keys, BOB, &bob.device_id).unwrap();
    let (_, one_time_key) = state
        .users
        .get_mut(BOB)
        .unwrap()
        .one_time_keys
        .get_mut(&bob.device_id)
        .unwrap()
        .pop()
        .unwrap();
    let mut olm = account
        .create_outbound_olm_session(&recipient, &one_time_key)
        .unwrap();
    let mut outbound = zoo_crypto::OutboundSession::new(CHANNEL, state.epoch, 0);
    let content = account
        .encrypt_room_key(&mut olm, &recipient, &outbound.room_key())
        .unwrap();
    state
        .queues
        .entry(bob.device_id.clone())
        .or_default()
        .push((ALICE.to_owned(), content));
    let ciphertext = outbound
        .encrypt(
            account,
            &zoo_crypto::OutgoingMessage {
                message_id: message_id.to_owned(),
                parent_id: None,
                revision: 0,
                content: zoo_crypto::MessageContent {
                    body: "なりすまし".into(),
                    format: "mfm".into(),
                    kind: "message".into(),
                },
                mentions: Default::default(),
                attachments: Vec::new(),
            },
        )
        .unwrap();
    let epoch = state.epoch;
    state.messages.insert(
        message_id.to_owned(),
        json!({ "channel_id": CHANNEL, "user_id": ALICE, "epoch": epoch, "revision": 0,
                "ciphertext": ciphertext }),
    );
}

#[tokio::test]
async fn devices_without_a_valid_self_signing_signature_stay_unverified() {
    let (_server, backend, _alice, bob) = setup().await;
    // Bob は Alice を TOFU で信頼している。
    send::send_encrypted_message(&bob.ctx(), &message("Bob から"), &mut None)
        .await
        .unwrap();

    // 1. 自己署名鍵の署名がない端末。
    let unsigned =
        zoo_crypto::DeviceAccount::new(ALICE, &uuid::Uuid::new_v4().to_string()).unwrap();
    // 2. Alice のものではない（偽の）自己署名鍵で署名した端末。
    let forged = zoo_crypto::DeviceAccount::new(ALICE, &uuid::Uuid::new_v4().to_string()).unwrap();
    let fake_self_signing =
        zoo_crypto::CrossSigningKeyPair::generate(ALICE, zoo_crypto::KeyUsage::SelfSigning)
            .unwrap();
    let forged_signature = fake_self_signing
        .sign_device(&forged.device_keys())
        .unwrap();
    {
        let mut state = backend.0.lock().unwrap();
        let alice = state.users.get_mut(ALICE).unwrap();
        alice
            .devices
            .insert(unsigned.device_id().to_owned(), unsigned.device_keys());
        alice
            .devices
            .insert(forged.device_id().to_owned(), forged.device_keys());
        alice.signatures.push(json!({
            "signer_user_id": ALICE,
            "signer_key_id": forged_signature.signer_key_id,
            "target_user_id": ALICE,
            "target_key_id": forged_signature.target_key_id,
            "signature": forged_signature.signature,
        }));
    }
    send_from_rogue_device(
        &backend,
        &unsigned,
        &bob,
        "00000000-0000-4000-8000-0000000000e1",
    );
    send_from_rogue_device(
        &backend,
        &forged,
        &bob,
        "00000000-0000-4000-8000-0000000000e2",
    );
    assert_eq!(bob.receive(&backend).await, 2);
    for id in [
        "00000000-0000-4000-8000-0000000000e1",
        "00000000-0000-4000-8000-0000000000e2",
    ] {
        assert_eq!(
            sender_of(&bob.decrypt(&backend, id).await),
            SenderTrust::Unverified,
            "{id}"
        );
    }
}

#[tokio::test]
async fn a_new_device_imports_past_sessions_from_the_key_backup() {
    use zoo2_tui::e2ee::setup;
    let (server, backend, alice, _bob) = setup().await;
    // 今の版（マスター鍵の署名と、SSSS のバックアップ鍵の公開鍵）。
    backend.0.lock().unwrap().backup_version = Some(json!({
        "version": "1",
        "algorithm": zoo_crypto::BACKUP_ALGORITHM,
        "auth_data": alice.secrets.backup_key.auth_data(&alice.secrets.cross_signing.master).unwrap(),
    }));
    let first = message("新しい端末を足す前 1");
    let second = message("新しい端末を足す前 2");
    send::send_encrypted_message(&alice.ctx(), &first, &mut None)
        .await
        .unwrap();
    // 2 通目は別のセッションにする（取り込みでページをたどることを確かめる）。
    backend.0.lock().unwrap().epoch += 1;
    send::send_encrypted_message(
        &alice.ctx(),
        &NewMessage {
            shape_epoch: Some(2),
            ..second.clone()
        },
        &mut None,
    )
    .await
    .unwrap();
    // 送信用セッションの写しは、鍵バックアップに上がる（送信のときに裏で送り始めたものも待つ）。
    assert!(matches!(
        setup::upload_key_backup(&alice.api, &alice.store)
            .await
            .unwrap(),
        setup::BackupUploadResult::Done { .. }
    ));
    assert_eq!(backend.0.lock().unwrap().backup_rows.len(), 2);

    // 新しい端末は、鍵が届いていないので読めない。解錠したあとに取り込むと読める。
    let laptop = other_device(&server, &backend, &alice);
    assert_eq!(
        laptop.decrypt(&backend, &first.message_id).await,
        Decryption::Undecryptable(UndecryptableReason::KeyMissing)
    );
    assert_eq!(
        setup::import_key_backup(&laptop.api, &laptop.store)
            .await
            .unwrap(),
        setup::BackupImportResult::Done { imported: 2 }
    );
    for (sent, body) in [
        (&first, "新しい端末を足す前 1"),
        (&second, "新しい端末を足す前 2"),
    ] {
        let decrypted = laptop.decrypt(&backend, &sent.message_id).await;
        assert_eq!(body_of(&decrypted), body);
        // バックアップ由来のセッションで復号したものは「送信者を確かめられない」。
        assert_eq!(sender_of(&decrypted), SenderTrust::Unauthenticated);
    }
    // 取り込んだセッションは、同じ版に上げ直さない（新しい端末で初めて見た版でも、取り込みの前に
    // 受け入れているので、そのあとのアップロードで送信待ちに戻さない）。
    assert_eq!(laptop.store.lock().await.pending_backup_count(), 0);
    let posted = backend.0.lock().unwrap().backup_posted;
    setup::upload_key_backup(&laptop.api, &laptop.store)
        .await
        .unwrap();
    assert_eq!(backend.0.lock().unwrap().backup_posted, posted);
    // 取り込み終えた版は、読み直さない。
    let reads = backend.0.lock().unwrap().backup_reads.len();
    assert_eq!(
        setup::import_key_backup(&laptop.api, &laptop.store)
            .await
            .unwrap(),
        setup::BackupImportResult::Done { imported: 0 }
    );
    assert_eq!(backend.0.lock().unwrap().backup_reads.len(), reads);

    // 元の端末で取り込んでも、Olm 由来（自分の送信用セッションの写し）を置き換えない。
    setup::import_key_backup(&alice.api, &alice.store)
        .await
        .unwrap();
    assert_eq!(
        sender_of(&alice.decrypt(&backend, &first.message_id).await),
        SenderTrust::Verified
    );

    // 解錠し直しが要るあいだは取り込まない。
    let other = other_device(&server, &backend, &alice);
    other.store.lock().await.forget_secrets_revision().unwrap();
    assert_eq!(
        setup::import_key_backup(&other.api, &other.store)
            .await
            .unwrap(),
        setup::BackupImportResult::None
    );

    // 取り込みが途中で止まっても、続きのページから読む。
    let tablet = other_device(&server, &backend, &alice);
    {
        let mut state = backend.0.lock().unwrap();
        state.backup_reads.clear();
        state.fail_backup_read = Some(Some("1".into()));
    }
    assert!(
        setup::import_key_backup(&tablet.api, &tablet.store)
            .await
            .is_err()
    );
    // 1 ページ目（どちらのセッションかは行の並びによる）は取り込めている。
    let mut readable = 0;
    for sent in [&first, &second] {
        if matches!(
            tablet.decrypt(&backend, &sent.message_id).await,
            Decryption::Decrypted(_)
        ) {
            readable += 1;
        }
    }
    assert_eq!(readable, 1);
    assert_eq!(
        setup::import_key_backup(&tablet.api, &tablet.store)
            .await
            .unwrap(),
        setup::BackupImportResult::Done { imported: 1 }
    );
    assert_eq!(
        backend.0.lock().unwrap().backup_reads,
        vec![None, Some("1".into()), Some("1".into())],
        "止まったページから読み直す"
    );
    for (sent, body) in [
        (&first, "新しい端末を足す前 1"),
        (&second, "新しい端末を足す前 2"),
    ] {
        assert_eq!(
            body_of(&tablet.decrypt(&backend, &sent.message_id).await),
            body
        );
    }
}
