//! TUI の初回設定と解錠（docs/e2ee.md の「初回設定と解錠の流れ」。#360）。
//!
//! サーバーを模した状態（wiremock の応答に状態を持たせる）で、次を確かめる。
//!
//! - 再認証を通して、初回設定を最後まで行える。再認証がなければ 403 になり、同じ鍵のまま再認証して
//!   やり直せる。
//! - リカバリーキーで解錠できる（新しい端末）。
//! - 初回設定の各保存（SSSS、クロス署名鍵、端末への署名、バックアップの版）の直前と直後で止めて
//!   再起動しても、控えたリカバリーキーで開け直して、同じ鍵のまま最後まで終わる。
//! - 鍵バックアップへの送信は、解錠したあとだけ。今の版の検証（マスター鍵の署名、公開鍵の一致、
//!   版の巻き戻り）を通らなければ送らない。

use std::sync::{Arc, Mutex as StdMutex};

use serde_json::{Value, json};
use tokio::sync::Mutex;
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate, matchers::any};
use zoo_crypto::{
    BackupDecryptionKey, CrossSigningKeys, DeviceAccount, OutboundSession, verify_device_keys,
};
use zoo2_tui::api::ApiClient;
use zoo2_tui::e2ee::setup::{
    self, BackupUploadResult, E2eeStatus, LockReason, SetupError, SharedStore,
};
use zoo2_tui::e2ee::store::{self, CryptoStore, ToDeviceRow};
use zoo2_tui::http::Credentials;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c1";
/// 再認証で受け取った ID トークン（偽のサーバーは、この値だけを直前の再認証として認める）。
const FRESH: &str = "fresh-id-token";

/// 止める位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    /// 保存の要求を受け取る前（何も変えずに 500）。
    Before(&'static str),
    /// 保存したあと、応答をなくす（反映してから 500）。
    After(&'static str),
}

#[derive(Default)]
struct State {
    ssss: Option<Value>,
    master: Option<Value>,
    self_signing: Option<Value>,
    user_signing: Option<Value>,
    signatures: Vec<Value>,
    versions: Vec<Value>,
    uploaded_sessions: usize,
    stop: Option<Stop>,
}

impl State {
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
        self.versions
            .last()
            .filter(|version| version["deleted"] == false)
    }
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

impl Respond for Backend {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let mut guard = self.0.lock().unwrap();
        let state: &mut State = &mut guard;
        let path = request.url.path().to_owned();
        let method = request.method.as_str().to_owned();
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let reauth = request
            .headers
            .get("x-zoo-reauth")
            .and_then(|value| value.to_str().ok())
            == Some(FRESH);
        let mut lose_response = false;
        if method == "POST" {
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
        let precondition_ok = |state: &State| match body.get("expected_default_key_id") {
            None | Some(Value::Null) => true,
            Some(expected) => *expected == state.default_key_id(),
        };
        let conflict = |state: &State| {
            coded(
                409,
                "E2EE_SSSS_CONFLICT",
                json!({
                    "default_key_id": state.default_key_id(),
                    "secrets_revision": state.secrets_revision(),
                }),
            )
        };
        let response = match (method.as_str(), path.as_str()) {
            ("GET", "/api/e2ee/secret_storage") => ResponseTemplate::new(200).set_body_json(
                state.ssss.clone().unwrap_or_else(|| {
                    json!({"default_key_id": null, "keys": {}, "secrets": {}, "secrets_revision": null, "slots": []})
                }),
            ),
            ("POST", "/api/e2ee/secret_storage/keys") => {
                if !reauth {
                    return coded(403, "E2EE_REAUTH_REQUIRED", Value::Null);
                }
                if state.ssss.is_some() {
                    return conflict(state);
                }
                let key_id = body["key_id"].clone();
                let mut secrets = body["secrets"].clone();
                for secret in secrets.as_object_mut().unwrap().values_mut() {
                    secret["key_id"] = key_id.clone();
                }
                state.ssss = Some(json!({
                    "default_key_id": key_id,
                    "keys": { key_id.as_str().unwrap(): body["key"] },
                    "secrets": secrets,
                    "secrets_revision": 1,
                    "slots": body["slots"],
                }));
                ok(json!({ "secrets_revision": 1 }))
            }
            ("POST", p) if p.starts_with("/api/e2ee/secret_storage/slots/") => ok(json!({})),
            ("POST", "/api/e2ee/keys/query") => {
                let mut value = json!({
                    "device_keys": {},
                    "master_keys": {},
                    "self_signing_keys": {},
                    "user_signing_keys": {},
                    "signatures": state.signatures,
                });
                for (field, key) in [
                    ("master_keys", &state.master),
                    ("self_signing_keys", &state.self_signing),
                    ("user_signing_keys", &state.user_signing),
                ] {
                    if let Some(key) = key {
                        value[field][BOB] = key.clone();
                    }
                }
                ResponseTemplate::new(200).set_body_json(value)
            }
            ("POST", "/api/e2ee/cross_signing/keys") => {
                if !precondition_ok(state) {
                    return conflict(state);
                }
                if !reauth {
                    return coded(403, "E2EE_REAUTH_REQUIRED", Value::Null);
                }
                for (field, slot) in [
                    ("master_key", &mut state.master),
                    ("self_signing_key", &mut state.self_signing),
                    ("user_signing_key", &mut state.user_signing),
                ] {
                    if let Some(key) = body.get(field) {
                        *slot = Some(key.clone());
                    }
                }
                ok(json!({}))
            }
            ("POST", "/api/e2ee/cross_signing/signatures") => {
                for upload in body["signatures"].as_array().unwrap() {
                    let mut row = upload.clone();
                    row["signer_user_id"] = json!(BOB);
                    state.signatures.push(row);
                }
                ok(json!({}))
            }
            ("GET", "/api/e2ee/key_backup/versions") => {
                ResponseTemplate::new(200).set_body_json(json!({ "versions": state.versions }))
            }
            ("GET", "/api/e2ee/key_backup/versions/current") => match state.current_version() {
                Some(version) => ResponseTemplate::new(200).set_body_json(version.clone()),
                None => coded(404, "E2EE_NOT_FOUND", Value::Null),
            },
            ("POST", "/api/e2ee/key_backup/versions") => {
                if !precondition_ok(state) {
                    return conflict(state);
                }
                // 最初の版は再認証が要らない。同じ公開鍵の今の版があれば、それを返す（冪等）。
                if let Some(current) = state.current_version()
                    && current["auth_data"]["public_key"] == body["auth_data"]["public_key"]
                {
                    let version = current["version"].clone();
                    return ok(json!({ "version": version }));
                }
                let number = (state.versions.len() + 1).to_string();
                state.versions.push(json!({
                    "version": number,
                    "algorithm": body["algorithm"],
                    "auth_data": body["auth_data"],
                    "deleted": false,
                }));
                ok(json!({ "version": number }))
            }
            ("POST", "/api/e2ee/key_backup/keys") => {
                let query = |name: &str| {
                    request
                        .url
                        .query_pairs()
                        .find(|(key, _)| key == name)
                        .map(|(_, value)| value.into_owned())
                };
                let current = state.current_version().map(|v| v["version"].clone());
                if current != query("version").map(Value::String) {
                    return coded(409, "E2EE_WRONG_BACKUP_VERSION", Value::Null);
                }
                if Some(state.secrets_revision().to_string()) != query("secrets_revision") {
                    return conflict(state);
                }
                let count = body["sessions"].as_array().unwrap().len();
                state.uploaded_sessions += count;
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

async fn backend() -> (MockServer, Backend, ApiClient) {
    let server = MockServer::start().await;
    let backend = Backend::default();
    Mock::given(any())
        .respond_with(backend.clone())
        .mount(&server)
        .await;
    let api = ApiClient::new(
        reqwest::Client::new(),
        &server.uri(),
        Credentials::Dev(BOB.into()),
    );
    (server, backend, api)
}

fn open(root: &std::path::Path) -> SharedStore {
    let mut store = CryptoStore::open(root, BOB).unwrap().store;
    if !store.is_registered() {
        store.mark_registered().unwrap();
    }
    Arc::new(Mutex::new(store))
}

/// 端末を閉じて、開き直す（再起動）。
fn restart(store: SharedStore, root: &std::path::Path) -> SharedStore {
    drop(Arc::try_unwrap(store).ok().expect("ほかに持ち主がいない"));
    open(root)
}

/// 別のユーザーの端末から、Olm で Megolm セッションの鍵を受け取る（鍵バックアップの送信待ちになる）。
async fn receive_room_key(store: &SharedStore) {
    let mut bob = store.lock().await;
    let request = bob.registration_request();
    let alice = DeviceAccount::new(ALICE, &uuid::Uuid::new_v4().to_string()).unwrap();
    let recipient = verify_device_keys(&request["device_keys"], BOB, bob.device_id()).unwrap();
    // 登録済みの端末の公開済みのワンタイム鍵は手元にないので、フォールバック鍵で Olm を始める。
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
    let mut session = alice.create_outbound_olm_session(&recipient, &key).unwrap();
    let outbound = OutboundSession::new(CHANNEL, 1, 0);
    let content = alice
        .encrypt_room_key(&mut session, &recipient, &outbound.room_key())
        .unwrap();
    let row = ToDeviceRow {
        id: uuid::Uuid::new_v4().to_string(),
        sender_user_id: ALICE.into(),
        r#type: "m.room.encrypted".into(),
        content,
    };
    let device_keys = json!({ ALICE: { alice.device_id(): alice.device_keys() } });
    let result = bob
        .process_to_device(std::slice::from_ref(&row), &device_keys)
        .unwrap();
    assert_eq!(result.room_keys, 1);
}

/// サーバーの鍵が、SSSS の鍵と同じで、すべてそろっている（初回設定が最後まで終わった）。
fn assert_complete(backend: &Backend, master_public_key: &str) {
    let state = backend.0.lock().unwrap();
    assert!(state.ssss.is_some());
    let master = state.master.as_ref().expect("マスター鍵がある");
    assert!(
        master["keys"]
            .as_object()
            .unwrap()
            .values()
            .any(|key| key == master_public_key),
        "サーバーのマスター鍵が、初回設定の鍵と違う"
    );
    assert!(state.self_signing.is_some() && state.user_signing.is_some());
    assert_eq!(state.versions.len(), 1, "版を作り直していない");
    assert!(!state.signatures.is_empty());
}

#[tokio::test]
async fn first_setup_needs_reauth_and_completes_with_the_same_keys() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    assert_eq!(
        setup::load_status(&api, &store, BOB).await.unwrap(),
        E2eeStatus::NotSetUp
    );

    let (session, recovery_key) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    assert!(!recovery_key.is_empty());
    let master = session
        .secrets()
        .cross_signing
        .master
        .public_key()
        .to_base64();
    // 再認証なしでは 403。同じ鍵のまま、再認証してやり直す。
    let err = setup::complete_setup(&api, &store, BOB, &session, None)
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<SetupError>(),
        Some(&SetupError::ReauthRequired)
    );
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();
    assert_complete(&backend, &master);
    assert_eq!(
        setup::load_status(&api, &store, BOB).await.unwrap(),
        E2eeStatus::Unlocked
    );
    assert!(store.lock().await.has_own_keys());
    assert_eq!(store.lock().await.secrets_revision(), Some(1));

    // 新しい端末は、控えたリカバリーキーで解錠できる（再認証は要らない）。
    let other_dir = tempfile::tempdir().unwrap();
    let other_root = store::crypto_root(other_dir.path());
    let other = open(&other_root);
    assert_eq!(
        setup::load_status(&api, &other, BOB).await.unwrap(),
        E2eeStatus::Locked(LockReason::New)
    );
    let unlocked = setup::unlock(&api, &other, BOB, &recovery_key)
        .await
        .unwrap();
    assert!(!unlocked.needs_reauth);
    assert_eq!(
        setup::load_status(&api, &other, BOB).await.unwrap(),
        E2eeStatus::Unlocked
    );
    assert_complete(&backend, &master);
    // 2 つの端末への署名がある。
    assert_eq!(backend.0.lock().unwrap().signatures.len(), 2);
}

#[tokio::test]
async fn stopping_before_or_after_each_save_finishes_with_the_same_keys() {
    let paths = [
        "/api/e2ee/secret_storage/keys",
        "/api/e2ee/cross_signing/keys",
        "/api/e2ee/cross_signing/signatures",
        "/api/e2ee/key_backup/versions",
    ];
    for path in paths {
        for stop in [Stop::Before(path), Stop::After(path)] {
            let dir = tempfile::tempdir().unwrap();
            let root = store::crypto_root(dir.path());
            let (_server, backend, api) = backend().await;
            let store = open(&root);
            let (session, recovery_key) = zoo_crypto::SetupSession::begin(BOB).unwrap();
            let master = session
                .secrets()
                .cross_signing
                .master
                .public_key()
                .to_base64();
            backend.0.lock().unwrap().stop = Some(stop);
            assert!(
                setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
                    .await
                    .is_err(),
                "{stop:?} で止まらなかった"
            );
            // 端末を再起動する。メモリーの鍵（マスター鍵の秘密鍵と SSSS 鍵）は消える。
            drop(session);
            let store = restart(store, &root);
            let status = setup::load_status(&api, &store, BOB).await.unwrap();
            if stop == Stop::Before("/api/e2ee/secret_storage/keys") {
                // SSSS の保存より前に止まった。サーバーには何もないので、初回設定をやり直す。
                assert_eq!(status, E2eeStatus::NotSetUp, "{stop:?}");
                continue;
            }
            assert_eq!(status, E2eeStatus::Locked(LockReason::New), "{stop:?}");
            // 控えたリカバリーキーで開け直し、続きを保存する。
            let unlocked = setup::unlock(&api, &store, BOB, &recovery_key)
                .await
                .unwrap_or_else(|e| panic!("{stop:?}: {e}"));
            let needs_cross_signing = backend.0.lock().unwrap().master.is_none();
            assert_eq!(unlocked.needs_reauth, needs_cross_signing, "{stop:?}");
            if unlocked.needs_reauth {
                // 再認証なしでは 403。再認証して、同じ鍵のまま続ける。
                let err = setup::save_remaining(
                    &api,
                    &store,
                    BOB,
                    &unlocked.session,
                    unlocked.secrets_revision,
                    None,
                )
                .await
                .unwrap_err();
                assert_eq!(
                    err.downcast_ref::<SetupError>(),
                    Some(&SetupError::ReauthRequired)
                );
                setup::save_remaining(
                    &api,
                    &store,
                    BOB,
                    &unlocked.session,
                    unlocked.secrets_revision,
                    Some(FRESH),
                )
                .await
                .unwrap_or_else(|e| panic!("{stop:?}: {e}"));
            }
            assert_complete(&backend, &master);
            assert_eq!(
                setup::load_status(&api, &store, BOB).await.unwrap(),
                E2eeStatus::Unlocked,
                "{stop:?}"
            );
        }
    }
}

#[tokio::test]
async fn a_lost_secret_storage_response_is_resolved_by_reading_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    let (session, _) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    backend.0.lock().unwrap().stop = Some(Stop::After("/api/e2ee/secret_storage/keys"));
    assert!(
        setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
            .await
            .is_err()
    );
    // 同じ鍵で送り直すと 409 になるが、読み直して自分の鍵なら続ける。
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();
    let master = session
        .secrets()
        .cross_signing
        .master
        .public_key()
        .to_base64();
    assert_complete(&backend, &master);
}

#[tokio::test]
async fn a_second_device_starting_setup_later_gets_a_conflict() {
    let (_server, backend, api) = backend().await;
    let dir = tempfile::tempdir().unwrap();
    let first = open(&store::crypto_root(dir.path()));
    let other_dir = tempfile::tempdir().unwrap();
    let second = open(&store::crypto_root(other_dir.path()));
    let (a, _) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    let (b, _) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    setup::complete_setup(&api, &first, BOB, &a, Some(FRESH))
        .await
        .unwrap();
    let err = setup::complete_setup(&api, &second, BOB, &b, Some(FRESH))
        .await
        .unwrap_err();
    assert_eq!(
        err.downcast_ref::<SetupError>(),
        Some(&SetupError::Conflict)
    );
    assert!(!second.lock().await.has_own_keys());
    let master = a.secrets().cross_signing.master.public_key().to_base64();
    assert_complete(&backend, &master);
}

#[tokio::test]
async fn backs_up_room_keys_only_after_unlocking_and_checks_the_version() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    receive_room_key(&store).await;
    assert_eq!(store.lock().await.pending_backup_count(), 1);
    // まだ解錠していない端末は送らない。
    assert_eq!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Done { uploaded: 0 }
    );

    // 初回設定を終えると、手元のセッションを今の版に送る。
    let (session, _) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 1);
    assert_eq!(store.lock().await.pending_backup_count(), 0);

    // 再起動しても、送ったものは送り直さない。新しく受け取った鍵だけを送る。
    let store = restart(store, &root);
    setup::run_startup_check(&api, &store, BOB).await.unwrap();
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 1);
    receive_room_key(&store).await;
    assert_eq!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Done { uploaded: 1 }
    );

    // 今の版が、自分のマスター鍵の署名のない版（サーバーが差し替えた）なら送らない。
    receive_room_key(&store).await;
    {
        let mut state = backend.0.lock().unwrap();
        let rogue_master = CrossSigningKeys::generate(BOB).unwrap().master;
        let rogue = BackupDecryptionKey::generate()
            .create_version_request(&rogue_master, Some("1"), 2)
            .unwrap();
        state.versions.push(json!({
            "version": "2",
            "algorithm": rogue["algorithm"],
            "auth_data": rogue["auth_data"],
            "deleted": false,
        }));
    }
    assert!(matches!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Rejected(_)
    ));
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 2);

    // 前の版の auth_data を大きい番号の版として見せられても（版の番号は署名に入っているので）送らない。
    // 前に見た版より古い版への巻き戻しの検出は、zoo-crypto のテスト（tests/rollback.rs）で確かめる。
    backend.0.lock().unwrap().versions.pop();
    {
        let mut state = backend.0.lock().unwrap();
        let version = state.versions[0].clone();
        state.versions.push(json!({
            "version": "2",
            "algorithm": version["algorithm"],
            "auth_data": version["auth_data"],
            "deleted": false,
        }));
    }
    assert!(matches!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Rejected(_)
    ));
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 2);
    // 本物の今の版（版 1）に戻れば送る。
    backend.0.lock().unwrap().versions.pop();
    assert!(matches!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Done { .. }
    ));
}

#[tokio::test]
async fn a_changed_secrets_revision_asks_to_unlock_again_and_stops_backups() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    let (session, recovery_key) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();
    // 別の端末の失効などで、SSSS の秘密が書き換えられた（同じ鍵のまま `secrets_revision` だけが進んだ）。
    backend.0.lock().unwrap().ssss.as_mut().unwrap()["secrets_revision"] = json!(2);
    assert_eq!(
        setup::load_status(&api, &store, BOB).await.unwrap(),
        E2eeStatus::Locked(LockReason::Mismatch)
    );
    // 解錠し直すまで、古い値で鍵バックアップに送らない。
    receive_room_key(&store).await;
    assert_eq!(
        setup::run_startup_check(&api, &store, BOB).await.unwrap(),
        E2eeStatus::Locked(LockReason::Mismatch)
    );
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 0);
    // リカバリーキーで解錠し直すと、新しい値を記録して送る。
    setup::unlock(&api, &store, BOB, &recovery_key)
        .await
        .unwrap();
    assert_eq!(store.lock().await.secrets_revision(), Some(2));
    assert_eq!(
        setup::load_status(&api, &store, BOB).await.unwrap(),
        E2eeStatus::Unlocked
    );
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 1);
}

#[tokio::test]
async fn an_unreadable_session_is_never_marked_as_backed_up() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    let (session, _) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();
    receive_room_key(&store).await;
    // 送信待ちのセッションの pickle を壊して、開き直す。
    drop(Arc::try_unwrap(store).ok().expect("ほかに持ち主がいない"));
    let path = root.join(BOB).join("state.json");
    let mut state: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for session in state["inbound_sessions"]
        .as_object_mut()
        .unwrap()
        .values_mut()
    {
        if session["backed_up"] == false {
            session["pickle"] = json!("broken");
        }
    }
    std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
    let store = open(&root);
    // 2 回試しても、どちらも失敗し、送ったことにならない。
    for _ in 0..2 {
        assert!(setup::upload_key_backup(&api, &store).await.is_err());
        assert_eq!(store.lock().await.pending_backup_count(), 1);
    }
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, 0);
}

#[tokio::test]
async fn an_abandoned_upload_is_forgotten_by_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    let (session, _) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();
    receive_room_key(&store).await;
    let current = backend.0.lock().unwrap().current_version().cloned();
    // 要求を作ったまま、送らずに捨てた（送っている途中で処理を止めた）。
    let first = store
        .lock()
        .await
        .prepare_backup_upload(current.as_ref(), setup::BACKUP_UPLOAD_BATCH)
        .unwrap();
    assert!(matches!(
        first,
        store::BackupUploadRequest::Upload { count: 1, .. }
    ));
    assert_eq!(store.lock().await.uploads_in_flight(), 1);
    // 次の送信は、捨てた要求の記録を残さずに、同じセッションを送る。
    assert_eq!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Done { uploaded: 1 }
    );
    assert_eq!(store.lock().await.uploads_in_flight(), 0);
    assert_eq!(store.lock().await.pending_backup_count(), 0);
}

#[tokio::test]
async fn a_conflict_while_running_stops_backups_until_unlocked_again() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let (_server, backend, api) = backend().await;
    let store = open(&root);
    let (session, recovery_key) = zoo_crypto::SetupSession::begin(BOB).unwrap();
    setup::complete_setup(&api, &store, BOB, &session, Some(FRESH))
        .await
        .unwrap();

    // 稼働中に今の版が変わってから鍵を受け取っても、新しい版を検証して、手元のセッションを送り直す。
    // 版の番号は auth_data の署名に入るので、版 2 の auth_data に署名し直す。
    {
        let mut state = backend.0.lock().unwrap();
        let secrets = session.secrets();
        let version = state.versions[0].clone();
        state.versions.push(json!({
            "version": "2",
            "algorithm": version["algorithm"],
            "auth_data": secrets
                .backup_key
                .auth_data(&secrets.cross_signing.master, 2)
                .unwrap(),
            "deleted": false,
        }));
    }
    receive_room_key(&store).await;
    assert_eq!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Done { uploaded: 1 }
    );

    // 稼働中に SSSS の秘密だけが書き換えられ、鍵を受け取って送ると 409 になる。
    backend.0.lock().unwrap().ssss.as_mut().unwrap()["secrets_revision"] = json!(2);
    receive_room_key(&store).await;
    assert_eq!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Conflict("E2EE_SSSS_CONFLICT".into())
    );
    assert!(store.lock().await.needs_unlock());
    assert_eq!(
        setup::load_status(&api, &store, BOB).await.unwrap(),
        E2eeStatus::Locked(LockReason::Mismatch)
    );
    // 次に鍵を受け取っても、解錠し直すまで送らない（古い値で送り直さない）。
    receive_room_key(&store).await;
    let before = backend.0.lock().unwrap().uploaded_sessions;
    assert_eq!(
        setup::upload_key_backup(&api, &store).await.unwrap(),
        BackupUploadResult::Done { uploaded: 0 }
    );
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, before);
    assert_eq!(store.lock().await.pending_backup_count(), 2);

    // リカバリーキーで解錠し直すと、送信待ちを送る。
    setup::unlock(&api, &store, BOB, &recovery_key)
        .await
        .unwrap();
    assert!(!store.lock().await.needs_unlock());
    assert_eq!(store.lock().await.pending_backup_count(), 0);
    assert_eq!(backend.0.lock().unwrap().uploaded_sessions, before + 2);
}
