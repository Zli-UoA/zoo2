use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};
use zoo_crypto::{DeviceAccount, OutboundSession, verify_device_keys};
use zoo2_tui::api::ApiClient;
use zoo2_tui::e2ee::{
    manager,
    store::{self, CryptoStore, ToDeviceRow},
};
use zoo2_tui::http::Credentials;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c1";

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn creates_and_reopens_the_same_device() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let opened = CryptoStore::open(&root, BOB).unwrap();
    assert!(opened.replaced_device_id.is_none());
    let mut bob = opened.store;
    let request = bob.registration_request();
    assert_eq!(request["device_keys"]["device_id"], bob.device_id());
    assert_eq!(request["one_time_keys"].as_object().unwrap().len(), 50);
    assert_eq!(request["fallback_keys"].as_object().unwrap().len(), 1);
    bob.mark_registered().unwrap();
    let device_id = bob.device_id().to_owned();
    #[cfg(unix)]
    {
        assert_eq!(mode(&root.join(BOB).join("state.json")), 0o600);
        assert_eq!(mode(&root.join(BOB).join("storage.key")), 0o600);
    }
    // 同じユーザーの暗号ストアは、2 つ目のプロセスでは開けない。
    assert!(CryptoStore::open(&root, BOB).is_err());
    drop(bob);

    let mut reopened = CryptoStore::open(&root, BOB).unwrap().store;
    assert_eq!(reopened.device_id(), device_id);
    assert!(reopened.is_registered());
    assert!(reopened.prepare_key_upload(50, true).unwrap().is_none());
    let upload = reopened.prepare_key_upload(45, false).unwrap().unwrap();
    assert_eq!(upload.one_time_keys.as_object().unwrap().len(), 5);
    assert_eq!(upload.fallback_keys.as_object().unwrap().len(), 1);
}

#[test]
fn recreates_the_store_when_the_storage_key_is_lost() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let first = CryptoStore::open(&root, BOB).unwrap().store;
    let device_id = first.device_id().to_owned();
    drop(first);
    std::fs::remove_file(root.join(BOB).join("storage.key")).unwrap();
    let second = CryptoStore::open(&root, BOB).unwrap();
    assert_eq!(
        second.replaced_device_id.as_deref(),
        Some(device_id.as_str())
    );
    assert_ne!(second.store.device_id(), device_id);
}

#[test]
fn processes_room_keys_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let mut bob = CryptoStore::open(&root, BOB).unwrap().store;
    let request = bob.registration_request();
    bob.mark_registered().unwrap();

    let alice = DeviceAccount::new(ALICE, &uuid::Uuid::new_v4().to_string()).unwrap();
    let recipient = verify_device_keys(&request["device_keys"], BOB, bob.device_id()).unwrap();
    let one_time_key = request["one_time_keys"]
        .as_object()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .clone();
    let mut session = alice
        .create_outbound_olm_session(&recipient, &one_time_key)
        .unwrap();
    let outbound = OutboundSession::new(CHANNEL, 1, 0);
    let content = alice
        .encrypt_room_key(&mut session, &recipient, &outbound.room_key())
        .unwrap();
    let row = ToDeviceRow {
        id: "7".into(),
        sender_user_id: ALICE.into(),
        r#type: "m.room.encrypted".into(),
        content,
    };
    let device_keys = json!({ ALICE: { alice.device_id(): alice.device_keys() } });

    let result = bob
        .process_to_device(std::slice::from_ref(&row), &device_keys)
        .unwrap();
    assert_eq!(result.processed, vec!["7"]);
    assert_eq!(
        (result.room_keys, result.dropped, result.new_olm_sessions),
        (1, 0, 1)
    );
    assert_eq!(bob.pending_backup_count(), 1);
    drop(bob);

    // ack の前に終了して開き直しても、同じメッセージを 2 回処理しない。
    let mut bob = CryptoStore::open(&root, BOB).unwrap().store;
    let again = bob
        .process_to_device(std::slice::from_ref(&row), &device_keys)
        .unwrap();
    assert_eq!((again.room_keys, again.dropped), (0, 0));
    bob.acknowledged(&again.processed).unwrap();
    assert_eq!(bob.pending_backup_count(), 1);

    // 壊れたメッセージと知らない type は捨てて ack する。
    let broken = bob
        .process_to_device(
            &[
                ToDeviceRow {
                    id: "8".into(),
                    content: json!({ "algorithm": "x" }),
                    ..row.clone()
                },
                ToDeviceRow {
                    id: "9".into(),
                    r#type: "m.other".into(),
                    ..row.clone()
                },
            ],
            &Value::Null,
        )
        .unwrap();
    assert_eq!(broken.processed, vec!["8", "9"]);
    assert_eq!(broken.dropped, 2);
}

#[tokio::test]
async fn logout_destroys_the_store_and_remembers_a_failed_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let mut bob = CryptoStore::open(&root, BOB).unwrap().store;
    bob.mark_registered().unwrap();
    let device_id = bob.device_id().to_owned();
    drop(bob);

    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/e2ee/devices/{device_id}")))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let api = ApiClient::new(
        reqwest::Client::new(),
        &server.uri(),
        Credentials::Dev(BOB.into()),
    );
    let targets = manager::open_for_logout(&root).unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].pending_backup, 0);
    for target in targets {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    assert!(!root.join(BOB).exists());
    let pending = store::read_pending_device_deletions(&root).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].device_id, device_id);

    // 次の起動で消し直す。
    Mock::given(method("DELETE"))
        .and(path(format!("/api/e2ee/devices/{device_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .mount(&server)
        .await;
    manager::retry_pending_device_deletion(&api, &root, BOB, &mut Default::default()).await;
    assert!(
        store::read_pending_device_deletions(&root)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn logout_deletes_a_device_whose_registration_response_was_lost() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    // 登録はサーバーで済んだが、応答をなくして手元では未登録のまま。
    let bob = CryptoStore::open(&root, BOB).unwrap().store;
    let device_id = bob.device_id().to_owned();
    drop(bob);
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/e2ee/devices/{device_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .expect(1)
        .mount(&server)
        .await;
    let api = ApiClient::new(
        reqwest::Client::new(),
        &server.uri(),
        Credentials::Dev(BOB.into()),
    );
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    assert!(
        store::read_pending_device_deletions(&root)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pending_deletions_of_other_users_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let a = store::PendingDeviceDeletion::new(ALICE, "a", false);
    let b = store::PendingDeviceDeletion::new(BOB, "b", false);
    store::mark_device_deletion_pending(&root, &a).unwrap();
    store::mark_device_deletion_pending(&root, &b).unwrap();
    store::clear_pending_device_deletion(&root, &b).unwrap();
    assert_eq!(
        store::read_pending_device_deletions(&root).unwrap(),
        vec![a]
    );
}

#[test]
fn reads_shape_rows_with_jsonb_as_text() {
    // Electric の Shape は jsonb の列を文字列で配る（backend/tests/e2ee_to_device_test.rs）。
    let row = json!({
        "id": "12",
        "sender_user_id": ALICE,
        "type": "m.room.encrypted",
        "content": "{\"algorithm\":\"m.olm.v1.curve25519-aes-sha2\"}",
    });
    let parsed = manager::to_device_row(row.as_object().unwrap());
    assert_eq!(parsed.id, "12");
    assert_eq!(parsed.content["algorithm"], "m.olm.v1.curve25519-aes-sha2");
    let object = json!({ "id": 13, "content": { "algorithm": "x" } });
    let parsed = manager::to_device_row(object.as_object().unwrap());
    assert_eq!(parsed.id, "13");
    assert_eq!(parsed.content["algorithm"], "x");
}

#[tokio::test]
async fn logout_also_deletes_the_device_replaced_after_losing_the_storage_key() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let mut bob = CryptoStore::open(&root, BOB).unwrap().store;
    bob.mark_registered().unwrap();
    let old_device = bob.device_id().to_owned();
    drop(bob);
    std::fs::remove_file(root.join(BOB).join("storage.key")).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .mount(&server)
        .await;
    let api = ApiClient::new(
        reqwest::Client::new(),
        &server.uri(),
        Credentials::Dev(BOB.into()),
    );
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    let deleted: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert!(deleted.contains(&format!("/api/e2ee/devices/{old_device}")));
    assert!(
        store::read_pending_device_deletions(&root)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn logout_keeps_another_users_device_for_their_next_login() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let mut alice = CryptoStore::open(&root, ALICE).unwrap().store;
    alice.mark_registered().unwrap();
    let device_id = alice.device_id().to_owned();
    drop(alice);

    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(404))
        .expect(0)
        .mount(&server)
        .await;
    // Bob のトークンで、Alice の暗号ストアを消す（Alice でログアウトせずに Bob でログインした）。
    let api = ApiClient::new(
        reqwest::Client::new(),
        &server.uri(),
        Credentials::Dev(BOB.into()),
    );
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    assert!(!root.join(ALICE).exists());
    let pending = store::read_pending_device_deletions(&root).unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(
        (pending[0].user_id.as_str(), pending[0].device_id.as_str()),
        (ALICE, device_id.as_str())
    );
}

#[test]
fn retry_interval_returns_to_the_first_after_a_working_connection() {
    use std::time::Duration;
    let mut backoff = Duration::from_secs(5);
    let mut delays = Vec::new();
    for _ in 0..8 {
        let (delay, next) = manager::retry_delay(backoff, false);
        delays.push(delay.as_secs());
        backoff = next;
    }
    assert_eq!(delays, vec![5, 10, 20, 40, 80, 160, 300, 300]);
    // Shape を受け取れていたあとの切断は、5 秒から数え直す。
    let (delay, next) = manager::retry_delay(backoff, true);
    assert_eq!((delay.as_secs(), next.as_secs()), (5, 10));
}

#[test]
fn losing_the_storage_key_records_the_old_device_before_removing_the_state() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let first = CryptoStore::open(&root, BOB).unwrap().store;
    let old_device = first.device_id().to_owned();
    drop(first);
    std::fs::remove_file(root.join(BOB).join("storage.key")).unwrap();
    let reopened = CryptoStore::open(&root, BOB).unwrap();
    assert_eq!(
        reopened.replaced_device_id.as_deref(),
        Some(old_device.as_str())
    );
    // 作り直しが途中で失敗しても前の端末を消せるよう、state.json を消す前に削除待ちに残してある。
    let pending = store::read_pending_device_deletions(&root).unwrap();
    assert!(
        pending
            .iter()
            .any(|p| p.user_id == BOB && p.device_id == old_device)
    );
}

#[test]
fn the_user_lock_lives_outside_the_store_and_is_held_while_destroying() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let bob = CryptoStore::open(&root, BOB).unwrap().store;
    assert!(root.join(format!("{BOB}.lock")).exists());
    assert!(!root.join(BOB).join("lock").exists());
    bob.destroy().unwrap();
    assert!(!root.join(BOB).exists());
    // 消し終えたらロックを返すので、開き直せる（新しい端末になる）。
    assert!(CryptoStore::open(&root, BOB).is_ok());
    // 削除待ちの一覧を探すときに、ロックファイルをユーザーと取り違えない。
    assert_eq!(store::stored_user_ids(&root).unwrap(), vec![BOB.to_owned()]);
}

fn api_for(server: &MockServer) -> ApiClient {
    ApiClient::new(
        reqwest::Client::new(),
        &server.uri(),
        Credentials::Dev(BOB.into()),
    )
}

#[tokio::test]
async fn not_found_for_a_registered_device_finishes_the_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let mut bob = CryptoStore::open(&root, BOB).unwrap().store;
    bob.mark_registered().unwrap();
    drop(bob);
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let api = api_for(&server);
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    assert!(
        store::read_pending_device_deletions(&root)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn not_found_for_an_unconfirmed_device_is_retried_until_it_repeats() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    // 登録の応答をなくして、手元では未登録のまま。あとから登録が届くかもしれない。
    let bob = CryptoStore::open(&root, BOB).unwrap().store;
    let device_id = bob.device_id().to_owned();
    drop(bob);
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/e2ee/devices/{device_id}")))
        .respond_with(ResponseTemplate::new(404))
        .expect(3)
        .mount(&server)
        .await;
    let api = api_for(&server);
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    let pending = store::read_pending_device_deletions(&root).unwrap();
    assert_eq!(
        pending,
        vec![store::PendingDeviceDeletion {
            not_found: 1,
            ..store::PendingDeviceDeletion::new(BOB, &device_id, true)
        }]
    );

    // 次の起動ごとに消し直す。続けて 3 回 404 なら済んだものとする。
    manager::retry_pending_device_deletion(&api, &root, BOB, &mut Default::default()).await;
    assert_eq!(
        store::read_pending_device_deletions(&root).unwrap()[0].not_found,
        2
    );
    manager::retry_pending_device_deletion(&api, &root, BOB, &mut Default::default()).await;
    assert_eq!(manager::MAX_NOT_FOUND, 3);
    assert!(
        store::read_pending_device_deletions(&root)
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn not_found_is_counted_once_per_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    store::mark_device_deletion_pending(&root, &store::PendingDeviceDeletion::new(BOB, "d", true))
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/api/e2ee/devices/d"))
        .respond_with(ResponseTemplate::new(404))
        .expect(4)
        .mount(&server)
        .await;
    let api = api_for(&server);
    // 同じ起動の中の試し直し（`spawn` のループ）では、404 を 1 回だけ数える。
    let mut counted = std::collections::BTreeSet::new();
    for _ in 0..4 {
        manager::retry_pending_device_deletion(&api, &root, BOB, &mut counted).await;
    }
    let pending = store::read_pending_device_deletions(&root).unwrap();
    assert_eq!((pending.len(), pending[0].not_found), (1, 1));
}

#[tokio::test]
async fn a_failure_between_not_founds_restarts_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    store::mark_device_deletion_pending(&root, &store::PendingDeviceDeletion::new(BOB, "d", true))
        .unwrap();
    let server = MockServer::start().await;
    let api = api_for(&server);
    let mut counts = Vec::new();
    // 次の起動ごとに 1 回ずつ消し直す（404 → 404 → 503 → 404）。
    for status in [404, 404, 503, 404] {
        server.reset().await;
        Mock::given(method("DELETE"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        manager::retry_pending_device_deletion(&api, &root, BOB, &mut Default::default()).await;
        counts.push(store::read_pending_device_deletions(&root).unwrap()[0].not_found);
    }
    assert_eq!(counts, vec![1, 2, 0, 1]);
}

#[tokio::test]
async fn logout_destroys_the_store_without_a_storage_key_even_if_the_pending_list_is_broken() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let mut bob = CryptoStore::open(&root, BOB).unwrap().store;
    bob.mark_registered().unwrap();
    let old_device = bob.device_id().to_owned();
    drop(bob);
    std::fs::remove_file(root.join(BOB).join("storage.key")).unwrap();
    let file = root.join("pending-device-delete.json");
    std::fs::write(&file, b"{broken").unwrap();
    // 起動では、前の端末を記録できなければ開かない。
    assert!(CryptoStore::open(&root, BOB).is_err());

    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .mount(&server)
        .await;
    let api = api_for(&server);
    // ログアウトでは開いて消し、前の端末も削除 API で消す。一覧は書き換えない。
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    assert!(!root.join(BOB).exists());
    let deleted: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect();
    assert!(deleted.contains(&format!("/api/e2ee/devices/{old_device}")));
    assert_eq!(std::fs::read(&file).unwrap(), b"{broken");
}

#[tokio::test]
async fn an_unconfirmed_device_registered_late_is_deleted_on_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let bob = CryptoStore::open(&root, BOB).unwrap().store;
    let device_id = bob.device_id().to_owned();
    drop(bob);
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let api = api_for(&server);
    for target in manager::open_for_logout(&root).unwrap() {
        manager::destroy_for_logout(&api, &root, target, Some(BOB))
            .await
            .unwrap();
    }
    assert_eq!(
        store::read_pending_device_deletions(&root).unwrap().len(),
        1
    );
    // ログアウトのあとで登録がコミットされ、サーバーに端末ができた。
    Mock::given(method("DELETE"))
        .and(path(format!("/api/e2ee/devices/{device_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .expect(1)
        .mount(&server)
        .await;
    manager::retry_pending_device_deletion(&api, &root, BOB, &mut Default::default()).await;
    assert!(
        store::read_pending_device_deletions(&root)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn losing_the_storage_key_records_whether_the_old_device_was_registered() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let first = CryptoStore::open(&root, BOB).unwrap().store;
    let unregistered = first.device_id().to_owned();
    drop(first);
    std::fs::remove_file(root.join(BOB).join("storage.key")).unwrap();
    let mut second = CryptoStore::open(&root, BOB).unwrap();
    assert!(!second.replaced_registered);
    second.store.mark_registered().unwrap();
    let registered = second.store.device_id().to_owned();
    drop(second);
    std::fs::remove_file(root.join(BOB).join("storage.key")).unwrap();
    let third = CryptoStore::open(&root, BOB).unwrap();
    assert!(third.replaced_registered);
    assert_eq!(
        store::read_pending_device_deletions(&root).unwrap(),
        vec![
            store::PendingDeviceDeletion::new(BOB, &unregistered, true),
            store::PendingDeviceDeletion::new(BOB, &registered, false),
        ]
    );
}

#[test]
fn marking_a_device_again_keeps_unconfirmed_and_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let entry = store::PendingDeviceDeletion::new(BOB, "d", true);
    store::mark_device_deletion_pending(&root, &entry).unwrap();
    store::update_pending_device_deletion(
        &root,
        &store::PendingDeviceDeletion {
            not_found: 2,
            ..entry.clone()
        },
    )
    .unwrap();
    store::mark_device_deletion_pending(&root, &store::PendingDeviceDeletion::new(BOB, "d", false))
        .unwrap();
    let pending = store::read_pending_device_deletions(&root).unwrap();
    assert_eq!((pending[0].unconfirmed, pending[0].not_found), (true, 2));
}

#[tokio::test]
async fn a_broken_pending_list_is_neither_retried_nor_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("pending-device-delete.json");
    std::fs::write(&file, b"{broken").unwrap();
    let server = MockServer::start().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .expect(0)
        .mount(&server)
        .await;
    let api = api_for(&server);
    manager::retry_pending_device_deletion(&api, &root, BOB, &mut Default::default()).await;
    assert!(store::read_pending_device_deletions(&root).is_err());
    let entry = store::PendingDeviceDeletion::new(BOB, "d", false);
    assert!(store::mark_device_deletion_pending(&root, &entry).is_err());
    assert!(store::clear_pending_device_deletion(&root, &entry).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"{broken");
}

#[test]
fn reads_pending_deletions_of_the_previous_format_as_confirmed() {
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("pending-device-delete.json"),
        serde_json::to_vec(&json!([{ "user_id": BOB, "device_id": "d" }])).unwrap(),
    )
    .unwrap();
    assert_eq!(
        store::read_pending_device_deletions(&root).unwrap(),
        vec![store::PendingDeviceDeletion::new(BOB, "d", false)]
    );
}

#[tokio::test]
async fn waits_only_while_the_screen_still_holds_the_store() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    let dir = tempfile::tempdir().unwrap();
    let root = store::crypto_root(dir.path());
    let open = || {
        Arc::new(tokio::sync::Mutex::new(
            CryptoStore::open(&root, BOB).unwrap().store,
        ))
    };
    // ほかに持ち主がいなければ、待たずに戻る。
    tokio::time::timeout(Duration::from_secs(1), manager::wait_until_released(open()))
        .await
        .expect("持ち主がいないのに待ち続けた");
    // 画面が持っている間は待ち、手放したら開き直せる。
    let store = open();
    let held = store.clone();
    let started = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(held);
    });
    manager::wait_until_released(store).await;
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(CryptoStore::open(&root, BOB).is_ok());
}
