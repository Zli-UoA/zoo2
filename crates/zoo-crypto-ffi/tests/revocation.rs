//! 失効の手順（#362）のバインディングを、Kotlin と同じ流れ（Web の `revocationSession.ts`）で通しで確かめる。

use std::sync::Arc;

use serde_json::{Value, json};
use zoo_crypto_ffi::*;

const ALICE: &str = "00000000-0000-4000-8000-00000000000a";
const BOB: &str = "00000000-0000-4000-8000-00000000000b";
const DEVICE: &str = "00000000-0000-4000-8000-0000000000a1";
const OTHER_DEVICE: &str = "00000000-0000-4000-8000-0000000000a2";
const CHANNEL: &str = "00000000-0000-4000-8000-0000000000c1";

fn parse(json: &str) -> Value {
    serde_json::from_str(json).unwrap()
}

/// サーバーを模した状態（SSSS、自分だけの `keys/query`、版の一覧）。
struct Server {
    storage: Value,
    keys: serde_json::Map<String, Value>,
    versions: Vec<Value>,
}

impl Server {
    fn secrets_revision(&self) -> u64 {
        self.storage["secrets_revision"].as_u64().unwrap_or(0)
    }

    fn storage(&self) -> String {
        self.storage.to_string()
    }

    /// `POST /api/e2ee/secret_storage/keys`。
    fn apply_keys(&mut self, request: &str) {
        let request = parse(request);
        let key_id = request["key_id"].clone();
        let mut secrets = request["secrets"].clone();
        for secret in secrets.as_object_mut().unwrap().values_mut() {
            secret["key_id"] = key_id.clone();
        }
        self.storage = json!({
            "default_key_id": key_id,
            "keys": { key_id.as_str().unwrap(): request["key"] },
            "secrets": secrets,
            "secrets_revision": self.secrets_revision() + 1,
            "slots": request["slots"],
        });
    }

    /// `POST /api/e2ee/secret_storage/secrets`。
    fn apply_secrets(&mut self, request: &str) {
        let request = parse(request);
        assert_eq!(request["key_id"], self.storage["default_key_id"]);
        for (name, secret) in request["secrets"].as_object().unwrap() {
            let mut secret = secret.clone();
            secret["key_id"] = request["key_id"].clone();
            self.storage["secrets"][name] = secret;
        }
        self.storage["secrets_revision"] = json!(self.secrets_revision() + 1);
    }

    fn apply_cross_signing(&mut self, body: &str) {
        let body = parse(body);
        for field in ["master_key", "self_signing_key", "user_signing_key"] {
            if let Some(key) = body.get(field) {
                self.keys.insert(field.into(), key.clone());
            }
        }
    }

    fn apply_version(&mut self, body: &str) -> String {
        let body = parse(body);
        let number = (self.versions.len() + 1).to_string();
        assert_eq!(body["auth_data"]["version"], json!(number));
        self.versions.push(json!({
            "version": number,
            "algorithm": body["algorithm"],
            "auth_data": body["auth_data"],
            "deleted": false,
        }));
        number
    }

    fn keys_query(&self) -> String {
        let mut value = json!({
            "device_keys": {},
            "master_keys": {},
            "self_signing_keys": {},
            "user_signing_keys": {},
            "signatures": [],
        });
        for (field, target) in [
            ("master_key", "master_keys"),
            ("self_signing_key", "self_signing_keys"),
            ("user_signing_key", "user_signing_keys"),
        ] {
            if let Some(key) = self.keys.get(field) {
                value[target][ALICE] = key.clone();
            }
        }
        value.to_string()
    }

    fn versions(&self) -> String {
        json!({ "versions": self.versions }).to_string()
    }

    fn unlock(&self, recovery: &RecoveryKey) -> Arc<UnlockedSecretStorage> {
        parse_secret_storage(ALICE.into(), self.storage())
            .unwrap()
            .unwrap()
            .unlock_with_recovery_key(recovery)
            .unwrap()
    }
}

/// 初回設定を済ませた端末（SSSS、サーバーの鍵、版 1）と、Bob を受け入れた記録。
struct Alice {
    server: Server,
    recovery: Arc<RecoveryKey>,
    account: Arc<DeviceAccount>,
    storage_key: Vec<u8>,
    own: Arc<OwnKeys>,
    trust: Arc<TrustStore>,
}

fn setup() -> Alice {
    let account = DeviceAccount::new(ALICE.into(), DEVICE.into()).unwrap();
    let storage_key = vec![5u8; 32];
    let start = begin_setup(ALICE.into()).unwrap();
    let session = start.session;
    let mut server = Server {
        storage: json!({}),
        keys: Default::default(),
        versions: Vec::new(),
    };
    server.apply_keys(&session.secret_storage_request().unwrap());
    let plan = session
        .plan(
            server.keys_query(),
            server.versions(),
            account.device_keys(),
        )
        .unwrap();
    server.apply_cross_signing(plan.cross_signing.as_ref().unwrap());
    server.apply_version(plan.backup_version.as_ref().unwrap());
    let own = OwnKeys::from_pickle(
        session
            .own_keys_pickle(&account, storage_key.clone())
            .unwrap(),
        &account,
        storage_key.clone(),
    )
    .unwrap();
    let trust = TrustStore::new(ALICE.into()).unwrap();
    session.record_own_keys(&trust).unwrap();
    // Bob を受け入れて、confirmed_usk のある記録を作る（失効でピンを作り直す相手）。
    let bob = CrossSigningKeyPair::generate(BOB.into(), KeyUsage::Master).unwrap();
    let accepted = trust
        .accept_master_key(&own.user_signing(), BOB.into(), bob.public_key_json())
        .unwrap();
    trust.confirm_accepted_master_key(BOB.into(), bob.public_key());
    assert_eq!(accepted.signature.target_user_id, BOB);
    Alice {
        server,
        recovery: RecoveryKey::parse(start.recovery_key).unwrap(),
        account,
        storage_key,
        own,
        trust,
    }
}

fn begin(alice: &Alice, scope: RevocationScope) -> Arc<Revocation> {
    Revocation::begin(
        RevocationInput {
            scope,
            keys_query: alice.server.keys_query(),
            versions: alice.server.versions(),
            pins: Vec::new(),
            targets_query: None,
            deleted_devices: vec![OTHER_DEVICE.into()],
        },
        &alice.server.unlock(&alice.recovery),
        &alice.trust,
        Some(alice.own.backup_key()),
        Some(alice.own.user_signing()),
    )
    .unwrap()
}

/// 古い版 1 に、Megolm セッション 1 つの行を作る（`BackupUploadState.next_upload` の本文）。
fn old_backup_rows(alice: &Alice) -> Vec<String> {
    let store = InboundSessionStore::new();
    let outbound = OutboundSession::new(CHANNEL.into(), 1, 0);
    store.add_outbound(&alice.account, &outbound);
    let upload = BackupUploadState::new(ALICE.into()).unwrap();
    upload.set_secrets_revision(Some(alice.server.secrets_revision()));
    upload
        .accept_current_version(
            alice.server.versions[0].to_string(),
            alice.own.master_public_key(),
            &alice.own.backup_key(),
        )
        .unwrap();
    upload.enqueue_all(&store);
    let body = parse(&upload.next_upload(&store, 100).unwrap().unwrap().body());
    body["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(Value::to_string)
        .collect()
}

#[test]
fn full_revocation_runs_end_to_end() {
    let mut alice = setup();
    let old_master = alice.own.master_public_key();
    let old_key_id = alice.server.storage["default_key_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let rows = old_backup_rows(&alice);
    let old_storage = alice.server.storage();

    let revocation = begin(&alice, RevocationScope::Full);
    let summary = revocation.summary();
    assert_eq!(summary.scope, RevocationScope::Full);
    assert_eq!(summary.stage, RevocationStage::Prepared);
    assert_eq!(summary.deleted_devices, [OTHER_DEVICE]);
    assert_eq!(summary.resigned_users, [BOB]);
    assert_ne!(summary.key_id, old_key_id);
    assert!(!summary.revocation_id.is_empty());

    // 暗号ストアに保存して、解錠の前に読める。SSSS を保存する前の 2〜5 は続けられない。
    let sealed = revocation
        .pickle(&alice.account, alice.storage_key.clone())
        .unwrap();
    assert_eq!(
        peek_revocation(sealed.clone(), &alice.account, alice.storage_key.clone()).unwrap(),
        summary
    );
    assert!(matches!(
        peek_revocation(sealed.clone(), &alice.account, vec![6u8; 32]),
        Err(ZooCryptoError::InvalidPickle { .. } | ZooCryptoError::Pickle { .. })
    ));
    assert!(matches!(
        Revocation::resume(
            sealed,
            &alice.account,
            alice.storage_key.clone(),
            &alice.server.unlock(&alice.recovery)
        ),
        Err(ZooCryptoError::RevocationRestart { .. })
    ));

    // 2〜5：新しい SSSS 鍵のリカバリーキーのスロットで保存する。
    let recovery = RecoveryKey::generate();
    let slot = new_recovery_key_slot(ALICE.into(), &revocation.ssss_key(), &recovery).unwrap();
    assert!(revocation.secret_storage_request(Vec::new()).is_err());
    alice
        .server
        .apply_keys(&revocation.secret_storage_request(vec![slot]).unwrap());
    revocation.secret_storage_saved(Some(alice.server.secrets_revision()));
    assert_eq!(
        revocation.summary().stage,
        RevocationStage::SecretStorageSaved
    );

    // クロス署名鍵。保存したら記録と自分の鍵を新しい鍵にそろえる。
    let body = revocation.cross_signing_request();
    assert!(parse(&body).get("master_key").is_some());
    alice.server.apply_cross_signing(&body);
    revocation.advance(RevocationStage::CrossSigningSaved);
    revocation.apply_to_trust(&alice.trust).unwrap();
    let new_own = OwnKeys::from_pickle(
        revocation
            .own_keys_pickle(&alice.account, alice.storage_key.clone())
            .unwrap(),
        &alice.account,
        alice.storage_key.clone(),
    )
    .unwrap();
    assert_ne!(new_own.master_public_key(), old_master);
    assert_eq!(
        alice.trust.own_master_key(),
        Some(new_own.master_public_key())
    );
    assert_eq!(
        revocation.own_keys().self_signing_public_key(),
        new_own.self_signing_public_key()
    );
    assert_eq!(
        revocation.summary().revocation_id,
        new_own.self_signing_public_key()
    );

    // 署名。Bob のピンを新しいユーザー署名鍵で作り直す。
    let signatures = revocation.signatures(alice.account.device_keys()).unwrap();
    assert_eq!(signatures.device.target_key_id, DEVICE);
    assert_eq!(signatures.pins.len(), 1);
    assert_eq!(parse(&signatures.pins[0])["target_user_id"], BOB);
    assert!(signatures.master_keys.is_empty());
    assert!(
        revocation
            .drop_stale_master_keys(json!({}).to_string())
            .unwrap()
            .is_empty()
    );
    revocation.advance(RevocationStage::SignaturesSaved);

    // 新しい版を作り、古い版の行を移す。
    let version = alice
        .server
        .apply_version(&revocation.backup_version_request().unwrap());
    assert_eq!(version, "2");
    revocation.backup_created(version.clone());
    let migrate = revocation
        .versions_to_migrate(alice.server.versions())
        .unwrap();
    assert_eq!(migrate.len(), 1);
    assert_eq!(migrate[0].version, "1");
    assert_eq!(
        migrate[0].key.public_key(),
        alice.own.backup_key().public_key()
    );
    let mut with_garbage = rows.clone();
    let mut moved = parse(&rows[0]);
    moved["session_id"] = json!("other");
    with_garbage.push(moved.to_string());
    let reencrypted = revocation
        .reencrypt_rows(&migrate[0].key, with_garbage)
        .unwrap();
    assert_eq!(reencrypted.rows.len(), 1);
    assert_eq!(reencrypted.dropped, 1);
    assert_eq!(parse(&reencrypted.rows[0])["is_verified"], false);
    // 新しいバックアップ鍵で読めて、古い鍵では読めない（`import_rows` は捨てた行の数を返す）。
    let imported = InboundSessionStore::new();
    assert_eq!(
        new_own
            .backup_key()
            .import_rows(&imported, reencrypted.rows.clone())
            .unwrap(),
        0
    );
    assert_eq!(imported.session_refs().len(), 1);
    assert_eq!(
        alice
            .own
            .backup_key()
            .import_rows(&InboundSessionStore::new(), reencrypted.rows)
            .unwrap(),
        1
    );
    revocation.record_dropped("1".into(), reencrypted.dropped);
    assert_eq!(revocation.summary().dropped_sessions, 1);

    // 移した行を新しい版に送るときの証明は、新しいバックアップ鍵で作る。
    let upload_key = DeviceAccount::new(ALICE.into(), OTHER_DEVICE.into())
        .unwrap()
        .curve25519_key();
    let proof = revocation
        .upload_proof(upload_key.clone(), ALICE.into(), version.clone())
        .unwrap();
    assert_eq!(
        proof,
        new_own
            .backup_key()
            .upload_proof(upload_key.clone(), ALICE.into(), version.clone())
            .unwrap()
    );
    assert_ne!(
        proof,
        alice
            .own
            .backup_key()
            .upload_proof(upload_key, ALICE.into(), version.clone())
            .unwrap()
    );
    revocation.advance(RevocationStage::Migrated);

    // 読み直して一致を確かめる。古い SSSS なら一致しない。
    let current = alice.server.versions.last().unwrap().to_string();
    revocation
        .verify(
            alice.server.storage(),
            alice.server.keys_query(),
            current.clone(),
        )
        .unwrap();
    assert!(matches!(
        revocation.verify(old_storage, alice.server.keys_query(), current),
        Err(ZooCryptoError::ServerKeyMismatch { .. })
    ));

    // 保存した進み具合から、新しいリカバリーキーで開け直した SSSS で続けられる。
    let sealed = revocation
        .pickle(&alice.account, alice.storage_key.clone())
        .unwrap();
    let resumed = Revocation::resume(
        sealed,
        &alice.account,
        alice.storage_key.clone(),
        &alice.server.unlock(&recovery),
    )
    .unwrap();
    assert_eq!(resumed.summary(), revocation.summary());
    assert_eq!(resumed.summary().stage, RevocationStage::Migrated);
}

#[test]
fn device_only_revocation_keeps_master_key() {
    let mut alice = setup();
    let revocation = begin(&alice, RevocationScope::DeviceOnly);
    assert_eq!(
        revocation.summary().key_id,
        alice.server.storage["default_key_id"]
    );
    let request = revocation.secret_storage_request(Vec::new()).unwrap();
    alice.server.apply_secrets(&request);
    revocation.secret_storage_saved(Some(alice.server.secrets_revision()));
    assert!(
        parse(&revocation.cross_signing_request())
            .get("master_key")
            .is_none()
    );
    assert_eq!(
        revocation.own_keys().master_public_key(),
        alice.own.master_public_key()
    );

    // 端末だけの失効は、同じ SSSS 鍵で開け直して続けられる。
    let sealed = revocation
        .pickle(&alice.account, alice.storage_key.clone())
        .unwrap();
    let resumed = Revocation::resume(
        sealed,
        &alice.account,
        alice.storage_key.clone(),
        &alice.server.unlock(&alice.recovery),
    )
    .unwrap();
    assert_eq!(
        resumed.summary().revocation_id,
        revocation.summary().revocation_id
    );
    assert_eq!(resumed.summary().stage, RevocationStage::SecretStorageSaved);
}
