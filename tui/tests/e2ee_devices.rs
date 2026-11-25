//! TUI の端末の失効と、他のユーザーの絵文字 SAS 検証（docs/e2ee.md の「失効の手順」「失効と検証の実装
//! （#362）」「絵文字 SAS 検証」。#362、#368）。
//!
//! 複数のユーザーを扱う偽のサーバー（wiremock の応答に状態を持たせ、`X-Dev-User` でユーザーを分ける）で、
//! 次を確かめる。
//!
//! - 再認証を通して、1〜5 の失効を最後まで行える（再認証がなければ 403 で止まり、同じ鍵のまま続けられる）。
//!   選んだ端末を削除し、マスター鍵と SSSS 鍵とリカバリーキーを作り直し、古い版のバックアップを新しい版に
//!   移して古い版を消す。記録がある相手のピンは新しいユーザー署名鍵で作り直し、記録がない相手は一覧に出す。
//! - 暗号化チャンネルを抜けた相手（ピンと SAS の署名だけがあり、今は暗号化チャンネルを共有しない相手）に
//!   付けた署名も、失効のあとで付け直される。
//! - 端末だけの失効は、マスター鍵と SSSS 鍵を残して秘密を置き換える。応答をなくしても、続きから終えられる。
//! - 1〜5 の失効を SSSS の保存のあとで止めて画面を閉じても、新しいリカバリーキーで開け直して続けられる。
//! - 2 人の TUI が、to-device を通して絵文字 SAS を最後まで行い、相手のマスター鍵に署名して保存する。

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{Mutex, watch};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::any};
use zoo_crypto::{
    DeviceAccount, OutboundSession, RecoveryKey, ResignNote, RevocationScope, SasPhase,
    SecretStorage, verify_device_keys,
};
use zoo2_tui::api::ApiClient;
use zoo2_tui::e2ee::revocation::{self, Resumed, RevocationError};
use zoo2_tui::e2ee::setup::{self, E2eeStatus, SharedStore};
use zoo2_tui::e2ee::store::{self, CryptoStore, ToDeviceRow};
use zoo2_tui::e2ee::verification::{Action, SaveState, VerificationController, VerificationEvent};
use zoo2_tui::http::Credentials;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const CAROL: &str = "00000000-0000-4000-8000-00000000000c";
const DAVE: &str = "00000000-0000-4000-8000-00000000000d";
const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c1";
const LOST_DEVICE: &str = "LOSTDEVICE";
/// 再認証で受け取った ID トークン（偽のサーバーは、この値だけを直前の再認証として認める）。
const FRESH: &str = "fresh-id-token";

/// 保存したあと、応答をなくす（反映してから 500）か、受け取る前に止める（何も変えずに 500）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Before(&'static str),
    After(&'static str),
}

#[derive(Default)]
struct Account {
    ssss: Option<Value>,
    master: Option<Value>,
    self_signing: Option<Value>,
    user_signing: Option<Value>,
    versions: Vec<Value>,
    /// 版ごとの鍵バックアップの行。
    rows: BTreeMap<String, Vec<Value>>,
    /// `device_id` → 端末の行（`device_keys`、ワンタイム鍵、フォールバック鍵）。
    devices: BTreeMap<String, Value>,
    /// 相手 → 自分が置いたピン。
    pins: BTreeMap<String, Value>,
}

impl Account {
    fn default_key_id(&self) -> Value {
        self.ssss
            .as_ref()
            .map_or(Value::Null, |ssss| ssss["default_key_id"].clone())
    }

    fn secrets_revision(&self) -> Value {
        self.ssss
            .as_ref()
            .map_or(Value::Null, |ssss| ssss["secrets_revision"].clone())
    }

    fn current_version(&self) -> Option<&Value> {
        self.versions.iter().rev().find(|v| v["deleted"] == false)
    }
}

#[derive(Default)]
struct State {
    accounts: BTreeMap<String, Account>,
    /// `e2ee_key_signatures`（`signer_user_id` 付き）。
    signatures: Vec<Value>,
    /// (`user_id`, `device_id`) → 届いた to-device の行。
    to_device: BTreeMap<(String, String), Vec<ToDeviceRow>>,
    deleted_devices: Vec<String>,
    stop: Option<Stop>,
    /// 直前の再認証を確かめるか（dev モードのサーバーは確かめない）。
    require_reauth: bool,
}

#[derive(Clone)]
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

fn key_of(key: &Value) -> Option<String> {
    key.get("keys")?
        .as_object()?
        .values()
        .next()?
        .as_str()
        .map(str::to_owned)
}

impl Respond for Backend {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut guard = self.0.lock().unwrap();
        let state: &mut State = &mut guard;
        let path = request.url.path().to_owned();
        let method = request.method.as_str().to_owned();
        let me = request
            .headers
            .get("x-dev-user")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let reauth = !state.require_reauth
            || request
                .headers
                .get("x-zoo-reauth")
                .and_then(|value| value.to_str().ok())
                == Some(FRESH);
        let mut lose_response = false;
        if method != "GET" {
            match state.stop {
                Some(Stop::Before(stop)) if stop == path => {
                    state.stop = None;
                    return ResponseTemplate::new(500);
                }
                Some(Stop::After(stop)) if stop == path => {
                    state.stop = None;
                    lose_response = true;
                }
                _ => {}
            }
        }
        let query = |name: &str| {
            request
                .url
                .query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        let reauth_required = || coded(403, "E2EE_REAUTH_REQUIRED", Value::Null);
        let response = match (method.as_str(), path.as_str()) {
            ("POST", "/api/e2ee/keys/query") => {
                let ids: Vec<String> = body["user_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_owned())
                    .collect();
                let mut value = json!({
                    "device_keys": {}, "master_keys": {}, "self_signing_keys": {},
                    "user_signing_keys": {}, "tofu_pins": {},
                    "signatures": state.signatures.iter()
                        .filter(|row| ids.iter().any(|id| row["target_user_id"] == json!(id)))
                        .cloned().collect::<Vec<_>>(),
                });
                for id in &ids {
                    let Some(account) = state.accounts.get(id) else {
                        continue;
                    };
                    for (field, key) in [
                        ("master_keys", &account.master),
                        ("self_signing_keys", &account.self_signing),
                    ] {
                        if let Some(key) = key {
                            value[field][id] = key.clone();
                        }
                    }
                    if *id == me
                        && let Some(key) = &account.user_signing
                    {
                        value["user_signing_keys"][id] = key.clone();
                    }
                    let devices: serde_json::Map<String, Value> = account
                        .devices
                        .iter()
                        .map(|(device_id, row)| (device_id.clone(), row["device_keys"].clone()))
                        .collect();
                    value["device_keys"][id] = Value::Object(devices);
                    if let Some(pin) = state.accounts.get(&me).and_then(|a| a.pins.get(id)) {
                        value["tofu_pins"][id] = pin.clone();
                    }
                }
                ResponseTemplate::new(200).set_body_json(value)
            }
            ("GET", "/api/e2ee/devices") => {
                let devices: Vec<Value> = state
                    .accounts
                    .get(&me)
                    .map(|a| {
                        a.devices
                            .iter()
                            .map(|(id, row)| {
                                json!({
                                    "device_id": id,
                                    "display_name": row["display_name"],
                                    "created_at": "2026-11-01 10:00:00+00",
                                    "last_seen_at": "2026-11-20 10:00:00+00",
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(devices)
            }
            ("DELETE", p) if p.starts_with("/api/e2ee/devices/") => {
                let id = p.trim_start_matches("/api/e2ee/devices/").to_owned();
                if let Some(account) = state.accounts.get_mut(&me) {
                    account.devices.remove(&id);
                }
                state.signatures.retain(|row| {
                    !(row["signer_user_id"] == json!(me)
                        && row["target_user_id"] == json!(me)
                        && row["target_key_id"] == json!(id))
                });
                state.deleted_devices.push(id);
                ok(json!({}))
            }
            ("POST", "/api/e2ee/keys/claim") => {
                let mut claimed = json!({});
                for (user, devices) in body["one_time_keys"].as_object().unwrap() {
                    for device in devices.as_object().unwrap().keys() {
                        let Some(row) = state
                            .accounts
                            .get_mut(user)
                            .and_then(|a| a.devices.get_mut(device))
                        else {
                            continue;
                        };
                        let keys = row["one_time_keys"].as_object_mut().unwrap();
                        let taken = keys.keys().next().cloned();
                        let (key_id, key) = match taken {
                            Some(key_id) => {
                                let key = keys.remove(&key_id).unwrap();
                                (key_id, key)
                            }
                            None => {
                                let fallback = row["fallback_keys"].as_object().unwrap();
                                let (id, key) = fallback.iter().next().unwrap();
                                (id.clone(), key.clone())
                            }
                        };
                        claimed[user][device] = json!({ key_id: key });
                    }
                }
                ok(json!({ "one_time_keys": claimed }))
            }
            ("POST", "/api/e2ee/to_device") => {
                let sender_device = body["device_id"].as_str().unwrap().to_owned();
                for (user, devices) in body["messages"].as_object().unwrap() {
                    for (device, content) in devices.as_object().unwrap() {
                        let _ = &sender_device;
                        state
                            .to_device
                            .entry((user.clone(), device.clone()))
                            .or_default()
                            .push(ToDeviceRow {
                                id: uuid::Uuid::new_v4().to_string(),
                                sender_user_id: me.clone(),
                                r#type: "m.room.encrypted".into(),
                                content: content.clone(),
                            });
                    }
                }
                ok(json!({ "undelivered": [] }))
            }
            ("GET", "/api/e2ee/tofu_pins") => {
                let pins: Vec<Value> = state
                    .accounts
                    .get(&me)
                    .map(|a| a.pins.values().cloned().collect())
                    .unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(json!({ "pins": pins, "next": null }))
            }
            ("POST", "/api/e2ee/tofu_pins") => {
                let replace = body["replace"] == true;
                if replace && !reauth {
                    return reauth_required();
                }
                let account = state.accounts.entry(me.clone()).or_default();
                for pin in body["pins"].as_array().unwrap() {
                    let target = pin["target_user_id"].as_str().unwrap().to_owned();
                    if !replace
                        && let Some(existing) = account.pins.get(&target)
                        && existing["master_key"] != pin["master_key"]
                    {
                        return coded(409, "E2EE_PIN_CONFLICT", json!({ "pin": existing }));
                    }
                    let mut stored = pin.clone();
                    stored["type"] = json!("zoo.tofu_pin");
                    stored["user_id"] = json!(me);
                    account.pins.insert(target, stored);
                }
                ok(json!({}))
            }
            ("POST", "/api/e2ee/cross_signing/signatures") => {
                let rows = body["signatures"].as_array().unwrap();
                if rows.iter().any(|row| row["target_user_id"] != json!(me)) && !reauth {
                    return reauth_required();
                }
                for upload in rows {
                    // 相手のマスター鍵への署名は、相手の今のマスター鍵にだけ付けられる。
                    let target = upload["target_user_id"].as_str().unwrap();
                    if target != me {
                        let current = state
                            .accounts
                            .get(target)
                            .and_then(|a| a.master.as_ref())
                            .and_then(key_of);
                        if current.as_deref() != upload["target_key_id"].as_str() {
                            return coded(404, "E2EE_NOT_FOUND", Value::Null);
                        }
                    }
                    let mut row = upload.clone();
                    row["signer_user_id"] = json!(me);
                    state.signatures.push(row);
                }
                ok(json!({}))
            }
            ("POST", "/api/e2ee/cross_signing/keys") => {
                let account = state.accounts.entry(me.clone()).or_default();
                if let Some(expected) = body.get("expected_default_key_id")
                    && !expected.is_null()
                    && *expected != account.default_key_id()
                {
                    return coded(409, "E2EE_SSSS_CONFLICT", Value::Null);
                }
                let same = [
                    ("master_key", &account.master),
                    ("self_signing_key", &account.self_signing),
                    ("user_signing_key", &account.user_signing),
                ]
                .iter()
                .all(|(field, current)| {
                    body.get(*field)
                        .is_none_or(|key| current.as_ref() == Some(key))
                });
                if same {
                    return ok(json!({}));
                }
                if account.master.is_some() && !reauth {
                    return reauth_required();
                }
                if let Some(expected) = body.get("expected_keys") {
                    let current = |key: &Option<Value>| {
                        key.as_ref()
                            .and_then(key_of)
                            .map_or(Value::Null, Value::String)
                    };
                    if expected["master"] != current(&account.master)
                        || expected["self_signing"] != current(&account.self_signing)
                        || expected["user_signing"] != current(&account.user_signing)
                    {
                        return coded(409, "E2EE_CROSS_SIGNING_CONFLICT", Value::Null);
                    }
                }
                let old_usk = account.user_signing.as_ref().and_then(key_of);
                for (field, slot) in [
                    ("master_key", &mut account.master),
                    ("self_signing_key", &mut account.self_signing),
                    ("user_signing_key", &mut account.user_signing),
                ] {
                    if let Some(key) = body.get(field) {
                        *slot = Some(key.clone());
                    }
                }
                // 古いユーザー署名鍵による署名は、新しい鍵を保存した時点で消える。
                if let Some(old) = old_usk
                    && account.user_signing.as_ref().and_then(key_of) != Some(old.clone())
                {
                    let signer = format!("ed25519:{old}");
                    state.signatures.retain(|row| {
                        !(row["signer_user_id"] == json!(me)
                            && row["signer_key_id"] == json!(signer))
                    });
                }
                ok(json!({}))
            }
            ("GET", "/api/e2ee/secret_storage") => {
                let ssss = state.accounts.get(&me).and_then(|a| a.ssss.clone());
                ResponseTemplate::new(200).set_body_json(ssss.unwrap_or_else(|| {
                    json!({"default_key_id": null, "keys": {}, "secrets": {}, "secrets_revision": null, "slots": []})
                }))
            }
            ("POST", "/api/e2ee/secret_storage/keys") => {
                let account = state.accounts.entry(me.clone()).or_default();
                if !reauth {
                    return reauth_required();
                }
                if account.ssss.is_some()
                    && (body["expected_default_key_id"] != account.default_key_id()
                        || body["expected_secrets_revision"] != account.secrets_revision())
                {
                    return coded(409, "E2EE_SSSS_CONFLICT", Value::Null);
                }
                let key_id = body["key_id"].as_str().unwrap().to_owned();
                let mut secrets = body["secrets"].clone();
                for secret in secrets.as_object_mut().unwrap().values_mut() {
                    secret["key_id"] = json!(key_id);
                }
                let revision = account.secrets_revision().as_u64().unwrap_or(0) + 1;
                account.ssss = Some(json!({
                    "default_key_id": key_id,
                    "keys": { key_id.clone(): body["key"] },
                    "secrets": secrets,
                    "secrets_revision": revision,
                    "slots": body["slots"],
                }));
                ok(json!({ "secrets_revision": revision }))
            }
            ("POST", "/api/e2ee/secret_storage/secrets") => {
                let account = state.accounts.entry(me.clone()).or_default();
                if !reauth {
                    return reauth_required();
                }
                if body["key_id"] != account.default_key_id()
                    || body["expected_secrets_revision"] != account.secrets_revision()
                {
                    return coded(409, "E2EE_SSSS_CONFLICT", Value::Null);
                }
                let ssss = account.ssss.as_mut().unwrap();
                for (name, secret) in body["secrets"].as_object().unwrap() {
                    let mut secret = secret.clone();
                    secret["key_id"] = body["key_id"].clone();
                    ssss["secrets"][name] = secret;
                }
                let revision = ssss["secrets_revision"].as_u64().unwrap() + 1;
                ssss["secrets_revision"] = json!(revision);
                ok(json!({ "secrets_revision": revision }))
            }
            ("POST", p) if p.starts_with("/api/e2ee/secret_storage/slots/") => ok(json!({})),
            ("GET", "/api/e2ee/key_backup/versions") => {
                let versions = state
                    .accounts
                    .get(&me)
                    .map(|a| a.versions.clone())
                    .unwrap_or_default();
                ResponseTemplate::new(200).set_body_json(json!({ "versions": versions }))
            }
            ("GET", "/api/e2ee/key_backup/versions/current") => {
                match state.accounts.get(&me).and_then(Account::current_version) {
                    Some(version) => ResponseTemplate::new(200).set_body_json(version.clone()),
                    None => coded(404, "E2EE_NOT_FOUND", Value::Null),
                }
            }
            ("GET", p) if p.starts_with("/api/e2ee/key_backup/versions/") => {
                let number = p.trim_start_matches("/api/e2ee/key_backup/versions/");
                match state.accounts.get(&me).and_then(|a| {
                    a.versions
                        .iter()
                        .find(|v| v["version"] == json!(number) && v["deleted"] == false)
                }) {
                    Some(version) => ResponseTemplate::new(200).set_body_json(version.clone()),
                    None => coded(404, "E2EE_NOT_FOUND", Value::Null),
                }
            }
            ("DELETE", p) if p.starts_with("/api/e2ee/key_backup/versions/") => {
                let number = p
                    .trim_start_matches("/api/e2ee/key_backup/versions/")
                    .to_owned();
                let account = state.accounts.entry(me.clone()).or_default();
                let current = account.current_version().map(|v| v["version"].clone());
                if current == Some(json!(number)) && !reauth {
                    return reauth_required();
                }
                match account
                    .versions
                    .iter_mut()
                    .find(|v| v["version"] == json!(number) && v["deleted"] == false)
                {
                    Some(version) => {
                        version["deleted"] = json!(true);
                        account.rows.remove(&number);
                        ok(json!({}))
                    }
                    None => coded(404, "E2EE_NOT_FOUND", Value::Null),
                }
            }
            ("POST", "/api/e2ee/key_backup/versions") => {
                let account = state.accounts.entry(me.clone()).or_default();
                if let Some(expected) = body.get("expected_default_key_id")
                    && !expected.is_null()
                    && *expected != account.default_key_id()
                {
                    return coded(409, "E2EE_SSSS_CONFLICT", Value::Null);
                }
                if let Some(current) = account.current_version()
                    && current["auth_data"]["public_key"] == body["auth_data"]["public_key"]
                {
                    let version = current["version"].clone();
                    return ok(json!({ "version": version }));
                }
                if account.current_version().is_some() && !reauth {
                    return reauth_required();
                }
                let current = account.current_version().map(|v| v["version"].clone());
                if body.get("expected_current_version").is_some()
                    && body["expected_current_version"] != current.unwrap_or(Value::Null)
                {
                    return coded(409, "E2EE_WRONG_BACKUP_VERSION", Value::Null);
                }
                let number = (account.versions.len() + 1).to_string();
                assert_eq!(body["auth_data"]["version"], json!(number));
                account.versions.push(json!({
                    "version": number,
                    "algorithm": body["algorithm"],
                    "auth_data": body["auth_data"],
                    "deleted": false,
                }));
                ok(json!({ "version": number }))
            }
            ("GET", "/api/e2ee/key_backup/keys") => {
                let version = query("version").unwrap();
                match state.accounts.get(&me).and_then(|a| {
                    a.versions
                        .iter()
                        .any(|v| v["version"] == json!(version) && v["deleted"] == false)
                        .then(|| a.rows.get(&version).cloned().unwrap_or_default())
                }) {
                    Some(rows) => ResponseTemplate::new(200)
                        .set_body_json(json!({ "sessions": rows, "next": null })),
                    None => coded(404, "E2EE_NOT_FOUND", Value::Null),
                }
            }
            ("POST", "/api/e2ee/key_backup/keys") => {
                let account = state.accounts.entry(me.clone()).or_default();
                let current = account.current_version().map(|v| v["version"].clone());
                if current != query("version").map(Value::String) {
                    return coded(409, "E2EE_WRONG_BACKUP_VERSION", Value::Null);
                }
                if Some(account.secrets_revision().to_string()) != query("secrets_revision") {
                    return coded(409, "E2EE_SSSS_CONFLICT", Value::Null);
                }
                let version = query("version").unwrap();
                let sessions = body["sessions"].as_array().unwrap().clone();
                let count = sessions.len();
                account.rows.entry(version).or_default().extend(sessions);
                ok(json!({ "count": count }))
            }
            _ => ResponseTemplate::new(404).set_body_json(json!({ "error": "not found" })),
        };
        if lose_response {
            return ResponseTemplate::new(500);
        }
        response
    }
}

struct World {
    _server: MockServer,
    backend: Backend,
    uri: String,
    _dirs: Vec<tempfile::TempDir>,
}

impl World {
    async fn new(require_reauth: bool) -> Self {
        let server = MockServer::start().await;
        let backend = Backend(Arc::new(StdMutex::new(State {
            require_reauth,
            ..State::default()
        })));
        Mock::given(any())
            .respond_with(backend.clone())
            .mount(&server)
            .await;
        Self {
            uri: server.uri(),
            _server: server,
            backend,
            _dirs: Vec::new(),
        }
    }

    fn api(&self, user: &str) -> ApiClient {
        ApiClient::new(
            reqwest::Client::new(),
            &self.uri,
            Credentials::Dev(user.into()),
        )
    }

    /// 端末を作って登録する（サーバーに端末鍵とワンタイム鍵を置く）。
    fn device(&mut self, user: &str) -> SharedStore {
        let dir = tempfile::tempdir().unwrap();
        let root = store::crypto_root(dir.path());
        let mut store = CryptoStore::open(&root, user).unwrap().store;
        let upload = store.prepare_key_upload(0, false).unwrap().unwrap();
        store.mark_registered().unwrap();
        store.mark_keys_published().unwrap();
        let device_id = store.device_id().to_owned();
        let row = json!({
            "device_keys": store.device_keys(),
            "one_time_keys": upload.one_time_keys,
            "fallback_keys": upload.fallback_keys,
            "display_name": "zoo2-tui",
        });
        self.backend
            .0
            .lock()
            .unwrap()
            .accounts
            .entry(user.into())
            .or_default()
            .devices
            .insert(device_id, row);
        self._dirs.push(dir);
        Arc::new(Mutex::new(store))
    }

    /// 初回設定を済ませる。リカバリーキーを返す。
    async fn set_up(&mut self, user: &str) -> (SharedStore, String) {
        let store = self.device(user);
        let (session, recovery_key) = zoo_crypto::SetupSession::begin(user).unwrap();
        setup::complete_setup(&self.api(user), &store, user, &session, Some(FRESH))
            .await
            .unwrap();
        (store, recovery_key.to_string())
    }

    fn account<T>(&self, user: &str, f: impl FnOnce(&Account) -> T) -> T {
        f(self.backend.0.lock().unwrap().accounts.get(user).unwrap())
    }
}

/// 別のユーザーの端末から、Olm で Megolm セッションの鍵を受け取る（鍵バックアップの送信待ちになる）。
async fn receive_room_key(store: &SharedStore, me: &str) {
    let mut bob = store.lock().await;
    let request = bob.registration_request();
    let sender = DeviceAccount::new(DAVE, &uuid::Uuid::new_v4().to_string()).unwrap();
    let recipient = verify_device_keys(&request["device_keys"], me, bob.device_id()).unwrap();
    let fallback = bob
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
    let mut session = sender
        .create_outbound_olm_session(&recipient, &key)
        .unwrap();
    let outbound = OutboundSession::new(CHANNEL, 1, 0);
    let content = sender
        .encrypt_room_key(&mut session, &recipient, &outbound.room_key())
        .unwrap();
    let row = ToDeviceRow {
        id: uuid::Uuid::new_v4().to_string(),
        sender_user_id: DAVE.into(),
        r#type: "m.room.encrypted".into(),
        content,
    };
    let device_keys = json!({ DAVE: { sender.device_id(): sender.device_keys() } });
    let result = bob
        .process_to_device(std::slice::from_ref(&row), &device_keys)
        .unwrap();
    assert_eq!(result.room_keys, 1);
}

/// 相手を TOFU で記録する（配布先の判定と同じ。ピンを置いて記録を確定する）。
async fn record_by_tofu(api: &ApiClient, store: &SharedStore, me: &str, peer: &str) {
    let keys = api
        .e2ee_query_keys(&[peer.to_owned(), me.to_owned()])
        .await
        .unwrap();
    let result = store
        .lock()
        .await
        .evaluate_users(&keys, &[peer.to_owned()])
        .unwrap();
    assert_eq!(result.new_pins.len(), 1);
    for pin in &result.new_pins {
        assert!(api.e2ee_post_tofu_pin(&pin.pin).await.unwrap());
    }
    store
        .lock()
        .await
        .megolm_settle_pins(&result.new_pins, &[])
        .unwrap();
}

fn pin_signer(pin: &Value, signer: &str) -> bool {
    pin["signatures"][signer]
        .as_object()
        .is_some_and(|keys| !keys.is_empty())
}

fn usk_key_id(world: &World, user: &str) -> String {
    format!(
        "ed25519:{}",
        world.account(user, |a| key_of(a.user_signing.as_ref().unwrap()).unwrap())
    )
}

#[tokio::test]
async fn full_revocation_completes_after_reauth_and_resigns_recorded_peers() {
    let mut world = World::new(true).await;
    let (alice, recovery_key) = world.set_up(ALICE).await;
    let api = world.api(ALICE);
    let _ = world.set_up(BOB).await;
    let _ = world.set_up(CAROL).await;
    // なくした端末（同じユーザーの別の端末）。
    world.device(ALICE);
    let lost = {
        let mut state = world.backend.0.lock().unwrap();
        let account = state.accounts.get_mut(ALICE).unwrap();
        let current = alice.try_lock().unwrap().device_id().to_owned();
        let other = account
            .devices
            .keys()
            .find(|id| **id != current)
            .unwrap()
            .clone();
        let row = account.devices.remove(&other).unwrap();
        account.devices.insert(LOST_DEVICE.into(), row);
        LOST_DEVICE.to_owned()
    };
    // 古い版にセッションを入れる（失効で新しい版に移す）。
    receive_room_key(&alice, ALICE).await;
    assert!(matches!(
        setup::upload_key_backup(&api, &alice).await.unwrap(),
        setup::BackupUploadResult::Done { uploaded: 1 }
    ));
    assert_eq!(world.account(ALICE, |a| a.rows["1"].len()), 1);
    // ボブは TOFU で記録がある（confirmed_usk あり）。キャロルは、別の端末が置いたピンだけがある。
    record_by_tofu(&api, &alice, ALICE, BOB).await;
    {
        let other_dir = tempfile::tempdir().unwrap();
        let other_root = store::crypto_root(other_dir.path());
        let mut other = CryptoStore::open(&other_root, ALICE).unwrap().store;
        other.mark_registered().unwrap();
        let other: SharedStore = Arc::new(Mutex::new(other));
        setup::unlock(&api, &other, ALICE, &recovery_key)
            .await
            .unwrap();
        record_by_tofu(&api, &other, ALICE, CAROL).await;
    }
    let old_master = world.account(ALICE, |a| key_of(a.master.as_ref().unwrap()));
    let old_usk = usk_key_id(&world, ALICE);
    assert!(pin_signer(
        &world.account(ALICE, |a| a.pins[BOB].clone()),
        ALICE
    ));

    // 1：選んだ端末を削除する。2〜5：SSSS を開け直して、新しい鍵を作る。
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    revocation::delete_devices(&api, std::slice::from_ref(&lost))
        .await
        .unwrap();
    let mut session = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::Full,
        vec![lost.clone()],
        &opened,
        None,
    )
    .await
    .unwrap();
    let new_recovery_key = session.new_recovery_key(ALICE).unwrap();
    let notes = revocation::not_resigned_users(session.state());
    assert_eq!(
        notes,
        vec![(CAROL.to_owned(), vec![ResignNote::NoRecord])],
        "記録がない相手は署名し直さず、一覧に出す"
    );
    assert!(alice.lock().await.has_pending_revocation());

    // 再認証がなければ 403 で止まる。同じ鍵のまま、再認証して最後まで保存する。
    let progress = |_| {};
    let err = revocation::run(&api, &alice, ALICE, &mut session, None, &progress)
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<RevocationError>(),
        Some(&RevocationError::ReauthRequired)
    );
    let outcome = revocation::run(&api, &alice, ALICE, &mut session, Some(FRESH), &progress)
        .await
        .unwrap();
    assert_eq!(outcome.dropped_sessions, 0);
    assert_eq!(outcome.not_resigned.len(), 1);

    // 端末を消し、マスター鍵を作り直した。
    assert!(
        world
            .backend
            .0
            .lock()
            .unwrap()
            .deleted_devices
            .contains(&lost)
    );
    let new_master = world.account(ALICE, |a| key_of(a.master.as_ref().unwrap()));
    assert_ne!(new_master, old_master);
    let new_usk = usk_key_id(&world, ALICE);
    assert_ne!(new_usk, old_usk);
    // 古い版を消し、新しい版にセッションを移した。
    world.account(ALICE, |a| {
        assert_eq!(a.versions[0]["deleted"], true);
        assert_eq!(a.current_version().unwrap()["version"], "2");
        assert!(!a.rows["2"].is_empty(), "古い版のセッションを移した");
    });
    // ボブのピンを新しいユーザー署名鍵で作り直した。キャロルのピンは作り直さない。
    world.account(ALICE, |a| {
        let usk = new_usk.clone();
        assert!(a.pins[BOB]["signatures"][ALICE].get(&usk).is_some());
        assert!(a.pins[CAROL]["signatures"][ALICE].get(&usk).is_none());
    });
    // 新しいリカバリーキーで開ける。古いリカバリーキーでは開けない。
    let storage = api.e2ee_secret_storage().await.unwrap();
    let parsed = SecretStorage::from_json(ALICE, &storage).unwrap().unwrap();
    assert!(
        parsed
            .unlock_with_recovery_key(&RecoveryKey::parse(&new_recovery_key).unwrap())
            .is_ok()
    );
    assert!(
        parsed
            .unlock_with_recovery_key(&RecoveryKey::parse(&recovery_key).unwrap())
            .is_err()
    );
    // 暗号ストアは新しい鍵にそろい、失効の記録は消えた。
    assert!(!alice.lock().await.has_pending_revocation());
    assert_eq!(
        setup::load_status(&api, &alice, ALICE).await.unwrap(),
        E2eeStatus::Unlocked
    );
    let devices = revocation::load_devices(&api, ALICE, None).await.unwrap();
    assert_eq!(devices.len(), 1);
    assert!(
        devices[0].signed,
        "失効を行った端末は新しい自己署名鍵で署名した"
    );
}

/// 暗号化チャンネルを抜けた相手（ピンと SAS の署名だけを持ち、今は共有するチャンネルがない）にも、失効の
/// あとで署名を付け直す（#362 の完了条件）。
#[tokio::test]
async fn signatures_for_peers_who_left_encrypted_channels_are_renewed() {
    let mut world = World::new(true).await;
    let (alice, recovery_key) = world.set_up(ALICE).await;
    let api = world.api(ALICE);
    let _ = world.set_up(BOB).await;
    // SAS か受け入れで、ボブのマスター鍵に署名した（`replace: true` のピンと署名）。
    let keys = api.e2ee_query_keys(&[BOB.to_owned()]).await.unwrap();
    let bob_master = keys["master_keys"][BOB].clone();
    let expected = key_of(&bob_master).unwrap();
    let accepted = alice
        .lock()
        .await
        .accept_master_key(BOB, &bob_master, &expected)
        .unwrap();
    zoo2_tui::e2ee::verification::save_accepted(&api, &alice, &accepted, Some(FRESH))
        .await
        .unwrap();
    let old_usk = usk_key_id(&world, ALICE);
    assert!(
        world
            .backend
            .0
            .lock()
            .unwrap()
            .signatures
            .iter()
            .any(|row| { row["target_user_id"] == BOB && row["signer_key_id"] == json!(old_usk) })
    );

    // 失効（端末だけ）。ボブとは暗号化チャンネルを共有していない（判定の対象にしない）が、ピンと記録から
    // 候補になる。
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    let mut session = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::DeviceOnly,
        Vec::new(),
        &opened,
        None,
    )
    .await
    .unwrap();
    assert!(revocation::not_resigned_users(session.state()).is_empty());
    let old_master = world.account(ALICE, |a| key_of(a.master.as_ref().unwrap()));
    let old_key_id = world.account(ALICE, |a| a.default_key_id());
    revocation::run(&api, &alice, ALICE, &mut session, Some(FRESH), &|_| {})
        .await
        .unwrap();

    let new_usk = usk_key_id(&world, ALICE);
    assert_ne!(new_usk, old_usk);
    let state = world.backend.0.lock().unwrap();
    assert!(
        state.signatures.iter().any(|row| {
            row["target_user_id"] == BOB
                && row["target_key_id"] == json!(expected)
                && row["signer_key_id"] == json!(new_usk)
        }),
        "ボブのマスター鍵への署名を、新しいユーザー署名鍵で付け直した"
    );
    let account = &state.accounts[ALICE];
    assert!(
        account.pins[BOB]["signatures"][ALICE]
            .get(&new_usk)
            .is_some()
    );
    // 端末だけの失効は、マスター鍵と SSSS 鍵を変えない。
    assert_eq!(key_of(account.master.as_ref().unwrap()), old_master);
    assert_eq!(account.default_key_id(), old_key_id);
}

#[tokio::test]
async fn device_only_revocation_finishes_after_a_lost_response() {
    let mut world = World::new(true).await;
    let (alice, recovery_key) = world.set_up(ALICE).await;
    let api = world.api(ALICE);
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    let mut session = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::DeviceOnly,
        Vec::new(),
        &opened,
        None,
    )
    .await
    .unwrap();
    // クロス署名鍵を保存したあとで応答をなくす。
    world.backend.0.lock().unwrap().stop = Some(Stop::After("/api/e2ee/cross_signing/keys"));
    assert!(
        revocation::run(&api, &alice, ALICE, &mut session, Some(FRESH), &|_| {})
            .await
            .is_err()
    );
    assert!(alice.lock().await.has_pending_revocation());
    revocation::run(&api, &alice, ALICE, &mut session, Some(FRESH), &|_| {})
        .await
        .unwrap();
    assert!(!alice.lock().await.has_pending_revocation());
    assert_eq!(
        setup::load_status(&api, &alice, ALICE).await.unwrap(),
        E2eeStatus::Unlocked
    );
    // 古いリカバリーキーのまま開け直せ、SSSS の秘密は新しい鍵になった。
    let check = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    let ssk = check.0.cross_signing_key(zoo_crypto::KeyUsage::SelfSigning);
    assert!(ssk.is_ok());
}

#[tokio::test]
async fn a_stopped_full_revocation_resumes_with_the_new_recovery_key() {
    let mut world = World::new(true).await;
    let (alice, recovery_key) = world.set_up(ALICE).await;
    let api = world.api(ALICE);
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    let mut session = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::Full,
        Vec::new(),
        &opened,
        None,
    )
    .await
    .unwrap();
    let new_recovery_key = session.new_recovery_key(ALICE).unwrap();
    // SSSS を保存したあと、クロス署名鍵の保存の前で止まり、画面を閉じた（メモリーの鍵を捨てた）。
    world.backend.0.lock().unwrap().stop = Some(Stop::Before("/api/e2ee/cross_signing/keys"));
    assert!(
        revocation::run(&api, &alice, ALICE, &mut session, Some(FRESH), &|_| {})
            .await
            .is_err()
    );
    drop(session);

    // 古いリカバリーキーでは開けない（SSSS 鍵を作り直した）。新しいリカバリーキーで開け直して続ける。
    assert!(
        revocation::open_secret_storage(&api, ALICE, &recovery_key)
            .await
            .is_err()
    );
    let opened = revocation::open_secret_storage(&api, ALICE, &new_recovery_key)
        .await
        .unwrap();
    let Resumed::Resumed(mut session) = revocation::resume(&alice, opened).await.unwrap() else {
        panic!("記録した段から続けられる");
    };
    revocation::run(&api, &alice, ALICE, &mut session, Some(FRESH), &|_| {})
        .await
        .unwrap();
    assert!(!alice.lock().await.has_pending_revocation());
    assert_eq!(
        setup::load_status(&api, &alice, ALICE).await.unwrap(),
        E2eeStatus::Unlocked
    );
}

#[tokio::test]
async fn a_full_revocation_stopped_before_saving_starts_over() {
    let mut world = World::new(true).await;
    let (alice, recovery_key) = world.set_up(ALICE).await;
    let api = world.api(ALICE);
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    let session = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::Full,
        vec![LOST_DEVICE.into()],
        &opened,
        None,
    )
    .await
    .unwrap();
    drop(session);
    // SSSS を保存する前なので、マスター鍵と SSSS 鍵がない。範囲と削除した端末を引き継いで、始め直す。
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    let (revocation_id, opened) = match revocation::resume(&alice, opened).await.unwrap() {
        Resumed::Restart {
            revocation_id,
            scope,
            deleted_devices,
            opened,
        } => {
            assert_eq!(scope, RevocationScope::Full);
            assert_eq!(deleted_devices, vec![LOST_DEVICE.to_owned()]);
            (revocation_id, opened)
        }
        other => panic!("始め直す: {other:?}"),
    };
    // 始め直すまで、元の記録は残す（始め直しに失敗しても、範囲と削除した端末を失わない）。
    assert!(alice.lock().await.has_pending_revocation());
    // 捨てる失効の ID を渡さなければ、残っている記録を上書きしない。
    let error = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::Full,
        vec![LOST_DEVICE.into()],
        &opened,
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RevocationError>(),
        Some(&RevocationError::Pending)
    );
    // ID を渡すと、新しい失効の記録で置き換える。
    let restarted = revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::Full,
        vec![LOST_DEVICE.into()],
        &opened,
        Some(&revocation_id),
    )
    .await
    .unwrap();
    assert_ne!(restarted.revocation_id(), revocation_id);
    let stored = alice.lock().await.revocation_state().unwrap().unwrap();
    assert_eq!(
        revocation::revocation_id(&stored),
        restarted.revocation_id()
    );
    assert_eq!(stored.deleted_devices, vec![LOST_DEVICE.to_owned()]);
}

#[tokio::test]
async fn the_chosen_scope_and_devices_survive_until_the_revocation_is_recorded() {
    let mut world = World::new(true).await;
    let (alice, recovery_key) = world.set_up(ALICE).await;
    let api = world.api(ALICE);
    // 端末を削除する前に残した範囲と端末は、失効の記録を保存するまで残る（ログアウトの前にも確かめる）。
    let intent = zoo2_tui::e2ee::store::RevocationIntent {
        scope: RevocationScope::DeviceOnly,
        deleted_devices: vec![LOST_DEVICE.into()],
    };
    alice
        .lock()
        .await
        .save_revocation_intent(intent.clone())
        .unwrap();
    assert_eq!(alice.lock().await.revocation_intent(), Some(intent));
    assert!(alice.lock().await.has_pending_revocation());
    let opened = revocation::open_secret_storage(&api, ALICE, &recovery_key)
        .await
        .unwrap();
    revocation::begin(
        &api,
        &alice,
        ALICE,
        RevocationScope::DeviceOnly,
        vec![LOST_DEVICE.into()],
        &opened,
        None,
    )
    .await
    .unwrap();
    // 失効の記録と同じ書き込みで消す。
    assert_eq!(alice.lock().await.revocation_intent(), None);
    assert!(alice.lock().await.revocation_state().unwrap().is_some());
}

/// 検証の知らせを集める。
#[derive(Clone, Default)]
struct Events(Arc<StdMutex<Vec<VerificationEvent>>>);

impl Events {
    fn notify(&self) -> zoo2_tui::e2ee::verification::Notify {
        let events = self.0.clone();
        Arc::new(move |event| events.lock().unwrap().push(event))
    }

    /// 最後の表示の段階と `transaction_id`。
    fn phase(&self) -> Option<(String, SasPhase)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match event {
                VerificationEvent::View(view) => Some((view.transaction_id.clone(), view.phase)),
                _ => None,
            })
    }

    /// `transaction_id` の最新の表示が比べる画面なら、その絵文字（比べる画面でなければ `None`）。
    fn compare_emoji(&self, transaction_id: &str) -> Option<Option<[u8; 7]>> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|event| match event {
                VerificationEvent::View(view) if view.transaction_id == transaction_id => {
                    Some(view)
                }
                _ => None,
            })
            .filter(|view| view.phase == SasPhase::Compare)
            .map(|view| view.emoji)
    }

    fn saved(&self) -> bool {
        self.0.lock().unwrap().iter().any(|event| {
            matches!(
                event,
                VerificationEvent::Save {
                    state: SaveState::Saved,
                    ..
                }
            )
        })
    }
}

/// 届いた to-device を処理し、検証のメッセージを渡す（端末の管理のタスクと同じ）。
async fn pump(world: &World, user: &str, store: &SharedStore, sas: &VerificationController) {
    let device_id = store.lock().await.device_id().to_owned();
    let rows = world
        .backend
        .0
        .lock()
        .unwrap()
        .to_device
        .remove(&(user.to_owned(), device_id))
        .unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let mut senders: Vec<String> = rows.iter().map(|r| r.sender_user_id.clone()).collect();
    senders.push(user.to_owned());
    senders.sort();
    senders.dedup();
    let keys = world.api(user).e2ee_query_keys(&senders).await.unwrap();
    let result = store
        .lock()
        .await
        .process_to_device_with_keys(&rows, &keys)
        .unwrap();
    sas.handle_messages(result.verifications);
}

/// どちらかの段階が `until` になるまで、両方の to-device を運ぶ。
async fn run_until(
    world: &World,
    sides: [(&str, &SharedStore, &VerificationController, &Events); 2],
    until: impl Fn(&Events, &Events) -> bool,
) {
    for _ in 0..200 {
        if until(sides[0].3, sides[1].3) {
            return;
        }
        for (user, store, sas, _) in sides {
            pump(world, user, store, sas).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "進みませんでした: {:?} / {:?}",
        sides[0].3.phase(),
        sides[1].3.phase()
    );
}

#[tokio::test]
async fn two_tui_users_verify_each_other_with_emoji() {
    // dev モードのサーバー（再認証を確かめない）。
    let mut world = World::new(false).await;
    let (alice, _) = world.set_up(ALICE).await;
    let (bob, _) = world.set_up(BOB).await;
    let controller = |user: &str, store: &SharedStore, events: &Events| {
        let (tx, rx) = watch::channel(Some((user.to_owned(), store.clone())));
        std::mem::forget(tx);
        VerificationController::new(
            world.api(user),
            reqwest::Client::new(),
            None,
            rx,
            events.notify(),
        )
    };
    let (alice_events, bob_events) = (Events::default(), Events::default());
    let alice_sas = controller(ALICE, &alice, &alice_events);
    let bob_sas = controller(BOB, &bob, &bob_events);

    alice_sas.request(BOB.to_owned());
    run_until(
        &world,
        [
            (ALICE, &alice, &alice_sas, &alice_events),
            (BOB, &bob, &bob_sas, &bob_events),
        ],
        |_, bob| bob.phase().is_some_and(|(_, p)| p == SasPhase::Incoming),
    )
    .await;
    let (txn, _) = bob_events.phase().unwrap();
    bob_sas.act(&txn, Action::Accept);
    run_until(
        &world,
        [
            (ALICE, &alice, &alice_sas, &alice_events),
            (BOB, &bob, &bob_sas, &bob_events),
        ],
        |alice, _| {
            alice
                .phase()
                .is_some_and(|(_, p)| p == SasPhase::ChooseMethod)
        },
    )
    .await;
    // TUI は QR を読み取れないので、絵文字を選ぶ（この端末の QR は出せる）。
    alice_sas.act(&txn, Action::StartSas);
    run_until(
        &world,
        [
            (ALICE, &alice, &alice_sas, &alice_events),
            (BOB, &bob, &bob_sas, &bob_events),
        ],
        |alice, bob| {
            alice.phase().is_some_and(|(_, p)| p == SasPhase::Compare)
                && bob.phase().is_some_and(|(_, p)| p == SasPhase::Compare)
        },
    )
    .await;
    // 比べる画面に、両方とも絵文字が出ていることを確かめてから見比べる（`None == None` で通さない）。
    let alice_emoji = alice_events
        .compare_emoji(&txn)
        .expect("比べる画面")
        .expect("絵文字が出る");
    let bob_emoji = bob_events
        .compare_emoji(&txn)
        .expect("比べる画面")
        .expect("絵文字が出る");
    assert_eq!(alice_emoji, bob_emoji, "同じ絵文字が出る");
    alice_sas.act(&txn, Action::Confirm);
    bob_sas.act(&txn, Action::Confirm);
    run_until(
        &world,
        [
            (ALICE, &alice, &alice_sas, &alice_events),
            (BOB, &bob, &bob_sas, &bob_events),
        ],
        |alice, bob| alice.saved() && bob.saved(),
    )
    .await;

    // 互いのマスター鍵にユーザー署名鍵で署名し、`replace: true` のピンを置いた。
    let state = world.backend.0.lock().unwrap();
    for (me, peer) in [(ALICE, BOB), (BOB, ALICE)] {
        let usk = format!(
            "ed25519:{}",
            key_of(state.accounts[me].user_signing.as_ref().unwrap()).unwrap()
        );
        let master = key_of(state.accounts[peer].master.as_ref().unwrap()).unwrap();
        assert!(state.signatures.iter().any(|row| {
            row["signer_user_id"] == json!(me)
                && row["target_user_id"] == json!(peer)
                && row["target_key_id"] == json!(master)
                && row["signer_key_id"] == json!(usk)
        }));
        assert!(
            state.accounts[me].pins[peer]["signatures"][me]
                .get(&usk)
                .is_some()
        );
    }
}

#[tokio::test]
async fn a_mismatch_cancels_both_sides_without_saving() {
    let mut world = World::new(false).await;
    let (alice, _) = world.set_up(ALICE).await;
    let (bob, _) = world.set_up(BOB).await;
    let make = |user: &str, store: &SharedStore, events: &Events| {
        let (tx, rx) = watch::channel(Some((user.to_owned(), store.clone())));
        std::mem::forget(tx);
        VerificationController::new(
            world.api(user),
            reqwest::Client::new(),
            None,
            rx,
            events.notify(),
        )
    };
    let (alice_events, bob_events) = (Events::default(), Events::default());
    let alice_sas = make(ALICE, &alice, &alice_events);
    let bob_sas = make(BOB, &bob, &bob_events);
    let sides = [
        (ALICE, &alice, &alice_sas, &alice_events),
        (BOB, &bob, &bob_sas, &bob_events),
    ];
    alice_sas.request(BOB.to_owned());
    run_until(&world, sides, |_, bob| {
        bob.phase().is_some_and(|(_, p)| p == SasPhase::Incoming)
    })
    .await;
    let (txn, _) = bob_events.phase().unwrap();
    bob_sas.act(&txn, Action::Accept);
    run_until(&world, sides, |alice, _| {
        alice
            .phase()
            .is_some_and(|(_, p)| p == SasPhase::ChooseMethod)
    })
    .await;
    alice_sas.act(&txn, Action::StartSas);
    run_until(&world, sides, |alice, bob| {
        alice.phase().is_some_and(|(_, p)| p == SasPhase::Compare)
            && bob.phase().is_some_and(|(_, p)| p == SasPhase::Compare)
    })
    .await;
    bob_sas.act(&txn, Action::Mismatch);
    run_until(&world, sides, |alice, bob| {
        alice.phase().is_some_and(|(_, p)| p == SasPhase::Cancelled)
            && bob.phase().is_some_and(|(_, p)| p == SasPhase::Cancelled)
    })
    .await;
    assert!(!alice_events.saved() && !bob_events.saved());
    let state = world.backend.0.lock().unwrap();
    assert!(
        !state
            .signatures
            .iter()
            .any(|row| row["target_user_id"] == json!(BOB) && row["signer_user_id"] == json!(ALICE))
    );
}
