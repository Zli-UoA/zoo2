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
        manager::destroy_for_logout(&api, &root, target)
            .await
            .unwrap();
    }
    assert!(!root.join(BOB).exists());
    let pending = store::read_pending_device_deletion(&root).unwrap();
    assert_eq!(pending.device_id, device_id);

    // 次の起動で消し直す。
    Mock::given(method("DELETE"))
        .and(path(format!("/api/e2ee/devices/{device_id}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "data": {}, "txid": null })))
        .mount(&server)
        .await;
    manager::retry_pending_device_deletion(&api, &root, BOB).await;
    assert!(store::read_pending_device_deletion(&root).is_none());
}
