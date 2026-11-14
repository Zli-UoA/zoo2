//! TUI の暗号ストア（docs/e2ee.md の「暗号ストア」）。
//!
//! 設定ディレクトリーの `crypto/<user_id>/` に、ユーザーごとに置く。
//!
//! - `state.json`：端末アカウント（`DeviceAccount::pickle`）と、Olm セッション、受信用セッション、
//!   配布先の判定の記録（`zoo-crypto` の保存用の封筒）。値はどれも保存鍵で暗号化してある。
//!   解錠したあとの自分の鍵（`OwnKeys::pickle`。#360）も同じく暗号化して置く。
//!   `user_id`、`device_id`、登録済みか、処理済みの to-device の ID、最後に解錠したときの
//!   `secrets_revision`、鍵バックアップの送信の状態（`BackupUploadState::to_json`。版の番号と送信待ちの
//!   セッションの ID だけ）、暗号化チャンネルとして記録したチャンネルの ID と、前回判定した配布先の
//!   ユーザー ID（#361）は平文で、秘密ではない。送信用の Olm セッション（#361）は保存鍵で暗号化してある。
//! - `storage.key`：32 バイトの保存鍵。
//! - `../<user_id>.lock`：同じユーザーの暗号ストアを 2 つのプロセスで開かないためのロック
//!   （ディレクトリーの外に置き、消しているあいだも持ち続ける）。
//!
//! ファイルはどれも 0600 で作る。`state.json` は一時ファイルに書いて `rename` で置き換えるので、
//! 書いている途中で終了しても、前の状態か次の状態のどちらかが残る。

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use zeroize::Zeroizing;
use zoo_crypto::{
    BackupUpload, BackupUploadState, DeviceAccount, InboundSessionStore, KeySnapshot, ListedDevice,
    OlmSession, OwnKeys, OwnSecrets, ServerKeyState, SessionRef, StartupCheck, TrustStore,
    VersionChange,
};

mod megolm;
pub use megolm::{
    Decrypted, Decryption, EncryptedRow, MegolmError, RecipientSummary, SenderTrust, SharePlan,
    ShareResult, UndecryptableReason, UndeliverableReason,
};

const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "state.json";
const KEY_FILE: &str = "storage.key";
/// 保存鍵を失って暗号ストアを作り直すあいだ、前の `state.json` を置いておく名前。新しい `state.json` を
/// 保存し終えたら消す。作り直しの途中で止まっても、暗号化チャンネルの記録と前回の配布先を失わない
/// （開き直したときと、記録を読むときに、ここから引き継ぐ）。
const REPLACED_STATE_FILE: &str = "state.replaced.json";

/// `state.json` のうち、作り直しで引き継ぐ記録（秘密ではない ID だけ）。
#[derive(Default, Deserialize)]
struct KeptRecords {
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    encrypted_channels: BTreeSet<String>,
    #[serde(default)]
    recipients: BTreeMap<String, Vec<String>>,
}

/// `path` の記録を読む。ファイルがなければ `None`。読めない、形式が壊れている、別のユーザーのものなら
/// エラー。
fn read_kept_records(path: &Path, user_id: &str) -> Result<Option<KeptRecords>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("{} を読めません", path.display())),
    };
    let records: KeptRecords = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} を読めません", path.display()))?;
    if records.user_id != user_id {
        bail!("暗号ストアが別のユーザーのものです");
    }
    Ok(Some(records))
}
/// 削除待ちの一覧を更新するときのロック（全ユーザーで 1 つ）。
const PENDING_LOCK_FILE: &str = "pending-device-delete.lock";

#[derive(Default, Serialize, Deserialize)]
struct State {
    version: u32,
    user_id: String,
    device_id: String,
    registered: bool,
    account: String,
    #[serde(default)]
    olm_sessions: Vec<StoredOlmSession>,
    #[serde(default)]
    inbound_sessions: BTreeMap<String, StoredInboundSession>,
    #[serde(default)]
    trust_store: Option<String>,
    /// 処理を終えて、まだ ack を送れていない to-device の ID。
    #[serde(default)]
    processed_to_device: BTreeSet<String>,
    /// 解錠した（または初回設定で作った）自分の鍵（`OwnKeys::pickle`。マスター鍵の秘密鍵は入れない）。
    #[serde(default)]
    own_keys: Option<String>,
    /// 最後に解錠したとき（自分で SSSS を書いたときは、その応答）の `secrets_revision`。
    #[serde(default)]
    secrets_revision: Option<u64>,
    /// 鍵バックアップの送信の状態（`BackupUploadState::to_json`）。
    #[serde(default)]
    backup_upload: Option<String>,
    /// 送信用の Olm セッション（相手の端末の識別鍵 → `OlmSession::pickle`。#361）。受信で作った
    /// セッションとは分けて置く。
    #[serde(default)]
    outbound_olm: BTreeMap<String, String>,
    /// 一度 `encrypted = true` を見たチャンネル（docs/e2ee.md の「暗号化の格下げを防ぐ」。#361）。
    #[serde(default)]
    encrypted_channels: BTreeSet<String>,
    /// チャンネルごとに、前回判定した配布先のメンバー（`user_id` の昇順。#361）。
    #[serde(default)]
    recipients: BTreeMap<String, Vec<String>>,
    /// 鍵バックアップからの取り込みの進み具合（秘密は入らない。Web の `meta` の `backup_import`）。
    #[serde(default)]
    backup_import: Option<BackupImportProgress>,
}

/// 鍵バックアップからの取り込みの進み具合。途中で止まったら、同じ版なら続きから取り込み、版が変わったら
/// 始めから取り込む。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BackupImportProgress {
    /// 取り込んでいる（取り込んだ）版。
    version: String,
    /// 次に読むページのカーソル（`GET /api/e2ee/key_backup/keys` の `after`）。
    after: Option<String>,
    /// 最後のページまで取り込んだ。
    done: bool,
}

/// [`CryptoStore::begin_backup_import`] の結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackupImportStart {
    /// `after` のカーソル（`None` なら最初）から読んで取り込む。
    Import {
        version: String,
        after: Option<String>,
    },
    /// この版は取り込み終えている。
    Done { version: String },
    /// 解錠していない（解錠し直しが要る）、または今の版がない。
    None,
    /// 今の版を使えない（アップロードの `Rejected` と同じ）。取り込まない。
    Rejected(String),
}

/// 今の版の受け入れの結果。
enum AcceptedVersion {
    Accepted(String),
    None,
    Rejected(String),
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredOlmSession {
    their_identity_key: String,
    session_id: String,
    pickle: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredInboundSession {
    pickle: String,
    /// 鍵バックアップに送ったか。送っていないものの数を、ログアウトの前に見せる。
    backed_up: bool,
}

/// to-device の Shape の行のうち、処理に使う列。
#[derive(Debug, Clone)]
pub struct ToDeviceRow {
    pub id: String,
    pub sender_user_id: String,
    pub r#type: String,
    pub content: Value,
}

/// [`CryptoStore::process_to_device`] の結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ToDeviceResult {
    /// 処理を終えた（保存した、または捨てた）ID。ack を送ってよい。
    pub processed: Vec<String>,
    pub room_keys: usize,
    pub dropped: usize,
    /// 新しく作った Olm セッションの数（ワンタイム鍵を使われたので補充する）。
    pub new_olm_sessions: usize,
}

/// [`CryptoStore::prepare_backup_upload`] の結果。
#[derive(Debug)]
pub enum BackupUploadRequest {
    /// 送るものがない（解錠していない、今の版がない、送信待ちがない）。
    None,
    /// 今の版を使えない（署名がない、SSSS の鍵と違う、版の巻き戻りなど）。送らない。
    Rejected(String),
    /// `POST /api/e2ee/key_backup/keys` に送る。
    Upload {
        id: u64,
        version: String,
        secrets_revision: u64,
        body: Value,
        count: usize,
    },
}

/// 送っている途中のアップロードと、送ったときのセッションの pickle。
struct PendingUpload {
    upload: BackupUpload,
    sessions: Vec<(String, String)>,
}

/// `POST /api/e2ee/keys/upload` に送る鍵。
pub struct KeyUpload {
    pub one_time_keys: Value,
    pub fallback_keys: Value,
}

/// 1 つのユーザーの暗号ストア。
pub struct CryptoStore {
    dir: PathBuf,
    _lock: File,
    storage_key: Zeroizing<[u8; 32]>,
    account: DeviceAccount,
    olm_sessions: Vec<OlmSession>,
    inbound: InboundSessionStore,
    loaded_inbound: BTreeSet<String>,
    trust: TrustStore,
    state: State,
    /// 鍵バックアップの送信の状態。受け入れた今の版は保存せず、起動のたびに検証し直す。
    backup_state: Option<BackupUploadState>,
    pending_uploads: BTreeMap<u64, PendingUpload>,
    next_upload_id: u64,
    /// チャンネルごとの送信用の Megolm セッション（メモリーにだけ置く。#361）。
    outbound: BTreeMap<String, megolm::Outbound>,
    /// チャンネルごとの、判定した配布先。
    pending: BTreeMap<String, megolm::PendingDistribution>,
    /// 受信用セッションが増えた回数。
    inbound_generation: u64,
    /// 取り込んだが、保存に失敗した受信用セッション（やり直しで保存し直す）。
    unsaved_inbound: BTreeSet<String>,
}

/// 開いた結果。保存鍵を失って作り直したら、前の端末の ID を返す（サーバーから消す）。
pub struct Opened {
    pub store: CryptoStore,
    pub replaced_device_id: Option<String>,
    /// 前の端末の登録が済んでいたか（`replaced_device_id` がないときは `false`）。済んでいなければ、
    /// 削除 API の 404 でもすぐには消えたものとしない（登録の応答をなくしただけかもしれない）。
    pub replaced_registered: bool,
}

/// 暗号ストアの置き場所（`<設定ディレクトリー>/crypto`）。
pub fn crypto_root(config_dir: &Path) -> PathBuf {
    config_dir.join("crypto")
}

fn user_dir(root: &Path, user_id: &str) -> Result<PathBuf> {
    // user_id は UUID。パスに使う前に、区切り文字などが入っていないことを確かめる。
    if user_id.is_empty()
        || !user_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        bail!("ユーザー ID をパスに使えません: {user_id:?}");
    }
    Ok(root.join(user_id))
}

/// 0600 で書く。一時ファイルに書いてから置き換える。
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options
            .open(&tmp)
            .with_context(|| format!("{} を作れません", tmp.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("{} を置き換えられません", path.display()))?;
    // rename したディレクトリーのエントリーも、ディスクに書き出す（停電で前の状態に戻らないように）。
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .with_context(|| format!("{} を同期できません", parent.display()))?;
    }
    Ok(())
}

fn create_private_dir(dir: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(dir)
        .with_context(|| format!("{} を作れません", dir.display()))
}

fn open_lock_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
        .open(path)
        .with_context(|| format!("{} を開けません", path.display()))
}

/// `<root>/<user_id>.lock` を排他でロックする。ロックファイルは暗号ストアのディレクトリーの外に
/// 置くので、消している（`destroy`）あいだもロックを持ち続け、ほかのプロセスが消しかけの
/// ストアを開くことはない。ロックファイル自体は消さない（消すと、別の inode を新しく作った
/// プロセスと同時にロックを持ててしまう）。
fn lock_user(root: &Path, user_id: &str) -> Result<File> {
    let file = open_lock_file(&root.join(format!("{user_id}.lock")))?;
    file.try_lock().map_err(|_| {
        anyhow!(
            "ほかの zoo2-tui がこのユーザーの暗号ストアを使っています。そちらを終了してください"
        )
    })?;
    Ok(file)
}

fn read_key(path: &Path) -> Result<Option<Zeroizing<[u8; 32]>>> {
    match fs::read(path) {
        Ok(bytes) => {
            let bytes = Zeroizing::new(bytes);
            let key: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow!("保存鍵が 32 バイトではありません"))?;
            Ok(Some(Zeroizing::new(key)))
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn new_key(path: &Path) -> Result<Zeroizing<[u8; 32]>> {
    let mut key = Zeroizing::new([0u8; 32]);
    getrandom::fill(key.as_mut_slice()).map_err(|e| anyhow!("乱数を取れません: {e}"))?;
    write_private(path, key.as_slice())?;
    Ok(key)
}

/// 受信用セッションのキーを 1 つの文字列にする。
fn inbound_key(session: &SessionRef) -> String {
    serde_json::to_string(&[
        &session.channel_id,
        &session.sender_key,
        &session.session_id,
    ])
    .expect("文字列の配列は JSON にできる")
}

impl CryptoStore {
    /// `user_id` の暗号ストアを開く。なければ端末を作る（`device_id` は UUID v4）。
    ///
    /// 保存鍵を失っていて、前の端末を削除待ちに記録できない（一覧のファイルを読み書きできないなど）
    /// ときは開かない（前の端末の ID を失わないため）。中身を読めない一覧は退避して記録する。
    pub fn open(root: &Path, user_id: &str) -> Result<Opened> {
        Self::open_with(root, user_id, false)
    }

    /// ログアウトのために開く。[`CryptoStore::open`] と違い、前の端末を削除待ちに記録できなくても
    /// 開く。このあとすぐに暗号ストアを消し、前の端末も削除 API で消す（`destroy_for_logout`）ので、
    /// 記録できないためにログアウトできなくなる（暗号ストアとトークンが残る）よりよい。
    pub fn open_for_logout(root: &Path, user_id: &str) -> Result<Opened> {
        Self::open_with(root, user_id, true)
    }

    fn open_with(root: &Path, user_id: &str, for_logout: bool) -> Result<Opened> {
        let dir = user_dir(root, user_id)?;
        create_private_dir(root)?;
        let lock = lock_user(root, user_id)?;
        create_private_dir(&dir)?;
        let state_path = dir.join(STATE_FILE);
        let key_path = dir.join(KEY_FILE);
        let state: Option<State> = match fs::read(&state_path) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("{} を読めません", state_path.display()))?,
            ),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };
        let mut replaced_device_id = None;
        let mut replaced_registered = false;
        // 保存鍵を失って作り直すときも、暗号化チャンネルの記録と前回の配布先は引き継ぐ（どちらも
        // 暗号化していない ID で、捨てると格下げを見つけられなくなる）。前の作り直しが途中で止まって
        // いたら、置いておいた前の `state.json` から引き継ぐ。
        let replaced_path = dir.join(REPLACED_STATE_FILE);
        let mut kept = (BTreeSet::new(), BTreeMap::new());
        if state.is_none()
            && let Some(records) = read_kept_records(&replaced_path, user_id)?
        {
            kept = (records.encrypted_channels, records.recipients);
        }
        if let Some(mut state) = state {
            if state.version != STATE_VERSION {
                bail!("暗号ストアの版 {} には対応していません", state.version);
            }
            if state.user_id != user_id {
                bail!("暗号ストアが別のユーザーのものです");
            }
            if let Some(storage_key) = read_key(&key_path)? {
                let store = Self::restore(dir, lock, storage_key, state)?;
                return Ok(Opened {
                    store,
                    replaced_device_id,
                    replaced_registered,
                });
            }
            // 保存鍵がなければ、どの値も読めない。作り直して、新しい端末として登録し直す。
            // 前の端末の ID は state.json にしかない。消す前に削除待ちに残し、作り直しが途中で
            // 失敗しても、次の起動で前の端末を消せるようにする。
            let replaced = PendingDeviceDeletion::new(user_id, &state.device_id, !state.registered);
            if let Err(err) = mark_device_deletion_pending(root, &replaced)
                && !for_logout
            {
                return Err(err);
            }
            replaced_registered = state.registered;
            kept = (
                std::mem::take(&mut state.encrypted_channels),
                std::mem::take(&mut state.recipients),
            );
            replaced_device_id = Some(state.device_id);
            // 新しい保存鍵で読めない `state.json` を残さないよう、前のものは別の名前に移す（消さない。
            // 新しい `state.json` を保存し終えるまで、記録をここから読めるようにする）。
            fs::rename(&state_path, &replaced_path).with_context(|| {
                format!(
                    "{} を {} に移せません",
                    state_path.display(),
                    replaced_path.display()
                )
            })?;
        }
        let storage_key = match read_key(&key_path)? {
            Some(key) => key,
            None => new_key(&key_path)?,
        };
        let mut account = DeviceAccount::new(user_id, &uuid::Uuid::new_v4().to_string())?;
        account.generate_one_time_keys(account.max_number_of_one_time_keys());
        account.generate_fallback_key();
        let trust = TrustStore::new(user_id)?;
        let state = State {
            version: STATE_VERSION,
            user_id: user_id.to_owned(),
            device_id: account.device_id().to_owned(),
            registered: false,
            account: account.pickle(&storage_key),
            trust_store: Some(trust.pickle(&account, &storage_key)),
            encrypted_channels: kept.0,
            recipients: kept.1,
            ..Default::default()
        };
        let mut store = Self {
            dir,
            _lock: lock,
            storage_key,
            account,
            olm_sessions: Vec::new(),
            inbound: InboundSessionStore::new(),
            loaded_inbound: BTreeSet::new(),
            trust,
            state,
            backup_state: None,
            pending_uploads: BTreeMap::new(),
            next_upload_id: 0,
            outbound: BTreeMap::new(),
            pending: BTreeMap::new(),
            inbound_generation: 0,
            unsaved_inbound: BTreeSet::new(),
        };
        store.save()?;
        // 記録を引き継いだ新しい `state.json` を保存できたので、前のものは要らない。
        let _ = fs::remove_file(&replaced_path);
        Ok(Opened {
            store,
            replaced_device_id,
            replaced_registered,
        })
    }

    fn restore(
        dir: PathBuf,
        lock: File,
        storage_key: Zeroizing<[u8; 32]>,
        state: State,
    ) -> Result<Self> {
        let account = DeviceAccount::from_pickle(
            &state.account,
            &storage_key,
            &state.user_id,
            &state.device_id,
        )?;
        let olm_sessions = state
            .olm_sessions
            .iter()
            .map(|s| OlmSession::from_pickle(&s.pickle, &account, &storage_key))
            .collect::<Result<Vec<_>, _>>()?;
        let trust = match &state.trust_store {
            Some(pickle) => TrustStore::from_pickle(pickle, &account, &storage_key)?,
            None => TrustStore::new(&state.user_id)?,
        };
        Ok(Self {
            dir,
            _lock: lock,
            storage_key,
            account,
            olm_sessions,
            inbound: InboundSessionStore::new(),
            loaded_inbound: BTreeSet::new(),
            trust,
            state,
            backup_state: None,
            pending_uploads: BTreeMap::new(),
            next_upload_id: 0,
            outbound: BTreeMap::new(),
            pending: BTreeMap::new(),
            inbound_generation: 0,
            unsaved_inbound: BTreeSet::new(),
        })
    }

    fn save(&mut self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.state)?;
        write_private(&self.dir.join(STATE_FILE), &bytes)
    }

    pub fn user_id(&self) -> &str {
        &self.state.user_id
    }

    pub fn device_id(&self) -> &str {
        &self.state.device_id
    }

    pub fn is_registered(&self) -> bool {
        self.state.registered
    }

    /// 配布先の判定の記録（#361 以降が使う）。
    pub fn trust_store(&mut self) -> &mut TrustStore {
        &mut self.trust
    }

    /// 配布先の判定の記録を保存し直す。
    pub fn save_trust_store(&mut self) -> Result<()> {
        self.state.trust_store = Some(self.trust.pickle(&self.account, &self.storage_key));
        self.save()
    }

    /// `POST /api/e2ee/devices` の本文（`display_name` を除く）。応答をなくしても同じ鍵で送り直す。
    pub fn registration_request(&self) -> Value {
        serde_json::json!({
            "device_keys": self.account.device_keys(),
            "one_time_keys": self.account.one_time_keys(),
            "fallback_keys": self.account.fallback_keys(),
        })
    }

    pub fn mark_registered(&mut self) -> Result<()> {
        self.account.mark_keys_as_published();
        self.state.account = self.account.pickle(&self.storage_key);
        self.state.registered = true;
        self.save()
    }

    /// サーバーのワンタイム鍵を最大数に戻し、使われたフォールバック鍵を作り直す。
    /// 送るものがなければ `None`。まだ公開していない鍵（応答をなくした送信の分）も入れる。
    pub fn prepare_key_upload(
        &mut self,
        one_time_key_count: usize,
        has_unused_fallback_key: bool,
    ) -> Result<Option<KeyUpload>> {
        let unpublished = self.account.one_time_keys().len();
        let max = self.account.max_number_of_one_time_keys();
        let mut changed = false;
        let missing = max.saturating_sub(one_time_key_count + unpublished);
        if missing > 0 {
            self.account.generate_one_time_keys(missing);
            changed = true;
        }
        if !has_unused_fallback_key && self.account.fallback_keys().is_empty() {
            self.account.generate_fallback_key();
            changed = true;
        }
        if changed {
            self.state.account = self.account.pickle(&self.storage_key);
            self.save()?;
        }
        let one_time_keys = self.account.one_time_keys();
        let fallback_keys = self.account.fallback_keys();
        if one_time_keys.is_empty() && fallback_keys.is_empty() {
            return Ok(None);
        }
        Ok(Some(KeyUpload {
            one_time_keys: Value::Object(one_time_keys),
            fallback_keys: Value::Object(fallback_keys),
        }))
    }

    pub fn mark_keys_published(&mut self) -> Result<()> {
        self.account.mark_keys_as_published();
        self.state.account = self.account.pickle(&self.storage_key);
        self.save()
    }

    /// to-device メッセージを処理する（React の `CryptoStore.processToDevice` と同じ規則）。
    /// `device_keys` は送信ユーザーの `keys/query` の応答の `device_keys` だけで、送信端末を検証済みに
    /// しない（[`process_to_device_with_keys`](Self::process_to_device_with_keys) を使う）。
    pub fn process_to_device(
        &mut self,
        rows: &[ToDeviceRow],
        device_keys: &Value,
    ) -> Result<ToDeviceResult> {
        self.process_to_device_with_keys(rows, &serde_json::json!({ "device_keys": device_keys }))
    }

    /// to-device メッセージを処理する。`query` は送信ユーザーと自分の `keys/query` の応答（`device_keys`、
    /// `master_keys`、`self_signing_keys`、`user_signing_keys`、`signatures`、`tofu_pins`）。他のユーザーの
    /// マスター鍵の判定には、自分のマスター鍵とユーザー署名鍵が要る。
    ///
    /// 1 件ごとに、変わった状態と処理済みの印を保存してから次に進む。ack の前に終了しても、
    /// 同じメッセージを 2 回復号しない。送信端末の検証（受信時の検証の手順 4）には、
    /// `TrustStore::sender_trust` が返す送信ユーザーの信頼した自己署名鍵を使う。
    pub fn process_to_device_with_keys(
        &mut self,
        rows: &[ToDeviceRow],
        query: &Value,
    ) -> Result<ToDeviceResult> {
        // 手順 4 の判定（`TrustStore::sender_trust`）に使う、送信ユーザーと自分の `keys/query`。
        // 解錠し直しが要るあいだは、暗号ストアの記録で送信端末を検証済みにしない（#360 の決まり）。
        let snapshot = if self.needs_unlock() {
            None
        } else {
            KeySnapshot::from_json(query).ok()
        };
        let mut result = ToDeviceResult::default();
        for row in rows {
            if !self.state.processed_to_device.contains(&row.id) {
                if row.r#type == "m.room.encrypted" {
                    self.process_encrypted(row, query, snapshot.as_ref(), &mut result)?;
                } else {
                    result.dropped += 1;
                }
                self.state.processed_to_device.insert(row.id.clone());
                self.save()?;
            }
            result.processed.push(row.id.clone());
        }
        Ok(result)
    }

    fn process_encrypted(
        &mut self,
        row: &ToDeviceRow,
        query: &Value,
        snapshot: Option<&KeySnapshot>,
        result: &mut ToDeviceResult,
    ) -> Result<()> {
        let device_keys = &query["device_keys"];
        let before = self.olm_sessions.len();
        let Ok(decrypted) = self
            .account
            .decrypt_to_device(&mut self.olm_sessions, &row.content)
        else {
            // 復号できないメッセージは何度試しても読めない。捨てて ack する。
            result.dropped += 1;
            return Ok(());
        };
        // 復号できたら、セッションの状態が進んでいる。検証の結果に関わらず保存する。
        result.new_olm_sessions += self.olm_sessions.len() - before;
        self.state.account = self.account.pickle(&self.storage_key);
        let sender_key = decrypted.sender_key().to_base64();
        for session in &self.olm_sessions {
            if session.their_identity_key().to_base64() != sender_key {
                continue;
            }
            let stored = StoredOlmSession {
                their_identity_key: sender_key.clone(),
                session_id: session.session_id(),
                pickle: session.pickle(&self.account, &self.storage_key),
            };
            match self
                .state
                .olm_sessions
                .iter_mut()
                .find(|s| s.their_identity_key == sender_key && s.session_id == stored.session_id)
            {
                Some(existing) => *existing = stored,
                None => self.state.olm_sessions.push(stored),
            }
        }
        let listed = decrypted
            .sender()
            .zip(decrypted.sender_device())
            .and_then(|(user, device)| device_keys.get(user)?.get(device));
        let listed = match listed {
            Some(keys) => ListedDevice::Listed(keys),
            None => ListedDevice::NotListed,
        };
        // 送信ユーザーの信頼した自己署名鍵（受信時の検証の手順 4。Web と同じ `TrustStore::sender_trust`）。
        // 規則 3 の署名や共有ピンの採用で記録が変わったら、処理済みの印と同じ書き込みで保存する。
        let trust = match (snapshot, decrypted.sender()) {
            (Some(snapshot), Some(sender)) => {
                let before = self.trust.to_json();
                let trust = self.trust.sender_trust(
                    snapshot,
                    sender,
                    decrypted.sender_device().unwrap_or_default(),
                );
                if self.trust.to_json() != before {
                    self.state.trust_store =
                        Some(self.trust.pickle(&self.account, &self.storage_key));
                }
                trust
            }
            _ => None,
        };
        let Ok(room_key) = decrypted.verify_room_key(&self.account, listed, trust.as_ref()) else {
            result.dropped += 1;
            return Ok(());
        };
        let session_ref = SessionRef {
            channel_id: room_key.channel_id().to_owned(),
            sender_key: room_key.sender_key().to_base64(),
            session_id: room_key.session_id(),
        };
        let key = inbound_key(&session_ref);
        self.load_inbound(&key)?;
        if self.inbound.add_room_key(room_key) != zoo_crypto::AddOutcome::Kept
            && let Some(pickle) =
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
            self.inbound_generation += 1;
            result.room_keys += 1;
        }
        Ok(())
    }

    /// 鍵バックアップからの取り込みを始める（docs/e2ee.md の「新しい端末での解錠」の手順 5。Web の
    /// `CryptoStore.beginBackupImport` と同じ）。
    ///
    /// `current_version` は `GET /api/e2ee/key_backup/versions/current` の応答。アップロードと同じ検証
    /// （`BackupUploadState::accept_current_version`）を通った今の版だけから取り込む。版が変わったときに
    /// 手元のセッションを送信待ちに戻すのも、取り込みの前に済ませる（取り込んだセッションを、そのあとの
    /// アップロードで同じ版に上げ直さないため）。途中で止まった取り込みは、同じ版なら続きのカーソルから
    /// 始める。その版を取り込み終えていれば `Done`。解錠し直しが要るあいだは取り込まない。
    pub fn begin_backup_import(
        &mut self,
        current_version: Option<&Value>,
    ) -> Result<BackupImportStart> {
        if self.needs_unlock() {
            return Ok(BackupImportStart::None);
        }
        let version = match self.accept_backup_version(current_version)? {
            AcceptedVersion::Accepted(version) => version,
            AcceptedVersion::None => return Ok(BackupImportStart::None),
            AcceptedVersion::Rejected(message) => return Ok(BackupImportStart::Rejected(message)),
        };
        Ok(match &self.state.backup_import {
            Some(progress) if progress.version == version && progress.done => {
                BackupImportStart::Done { version }
            }
            Some(progress) if progress.version == version => BackupImportStart::Import {
                after: progress.after.clone(),
                version,
            },
            _ => BackupImportStart::Import {
                version,
                after: None,
            },
        })
    }

    /// `GET /api/e2ee/key_backup/keys` の 1 ページを、バックアップ鍵で復号してバックアップ由来として
    /// 取り込む（docs/e2ee.md の「受信時の検証」の「鍵バックアップから取り込んだとき」。規則は
    /// `InboundSessionStore::import_backup_session`）。取り込んだ数を返す。
    ///
    /// `next` は応答の `next`（最後のページなら `None`）で、取り込んだセッションと同じ書き込みで進み具合
    /// として保存する。復号できない行、平文が行と合わない行、`session_id` が合わない行は捨てる。新しく
    /// 足したセッションは、同じ版に上げ直さないよう送信済みにする。すでにあるセッション（Olm 由来など）の
    /// 送信待ちの印は変えない。受け入れた今の版と違う版のページは取り込まない。保存できなかったら
    /// 書き込む前の状態に戻し、保存できていないセッションを覚えておいて、やり直しで保存し直す。
    pub fn import_backup_page(
        &mut self,
        version: &str,
        rows: &[zoo_crypto::BackupRow],
        next: Option<&str>,
    ) -> Result<usize> {
        let accepted = self
            .backup_state
            .as_ref()
            .and_then(|state| state.current_version())
            .map(|current| current.version.clone());
        if accepted.as_deref() != Some(version) {
            bail!("鍵バックアップの今の版が変わりました");
        }
        if self.needs_unlock() {
            bail!("解錠し直しが必要です");
        }
        let own = self
            .own_keys()?
            .ok_or_else(|| anyhow!("解錠していません"))?;
        let mut changed: BTreeMap<String, Option<StoredInboundSession>> = BTreeMap::new();
        let mut imported = 0;
        for row in rows {
            let Ok(session) = own.backup_key.decrypt_row(row) else {
                continue;
            };
            let session_ref = SessionRef {
                channel_id: session.channel_id.clone(),
                sender_key: session.sender_key.to_base64(),
                session_id: session.session_id.clone(),
            };
            let key = inbound_key(&session_ref);
            self.load_inbound(&key)?;
            let Ok(outcome) = self.inbound.import_backup_session(session) else {
                continue;
            };
            // 前の保存に失敗したセッションは、取り込みの結果が変わらなくても保存し直す。
            if outcome == zoo_crypto::AddOutcome::Kept && !self.unsaved_inbound.contains(&key) {
                continue;
            }
            let Some(pickle) =
                self.inbound
                    .pickle_session(&session_ref, &self.account, &self.storage_key)
            else {
                continue;
            };
            let previous = self.state.inbound_sessions.get(&key).cloned();
            let backed_up = previous.as_ref().is_none_or(|stored| stored.backed_up);
            self.state
                .inbound_sessions
                .insert(key.clone(), StoredInboundSession { pickle, backed_up });
            changed.entry(key).or_insert(previous);
            imported += 1;
        }
        let previous_progress = self.state.backup_import.replace(BackupImportProgress {
            version: version.to_owned(),
            after: next.map(str::to_owned),
            done: next.is_none(),
        });
        if let Err(error) = self.save() {
            // 書けなければ、保存した状態をディスクと同じに戻す（メモリーのセッションは残し、次の
            // やり直しで保存し直す）。
            self.state.backup_import = previous_progress;
            for (key, previous) in changed {
                match previous {
                    Some(previous) => {
                        self.state.inbound_sessions.insert(key.clone(), previous);
                    }
                    None => {
                        self.state.inbound_sessions.remove(&key);
                    }
                }
                self.unsaved_inbound.insert(key);
            }
            return Err(error);
        }
        for key in changed.keys() {
            self.unsaved_inbound.remove(key);
        }
        if imported > 0 {
            self.inbound_generation += 1;
        }
        Ok(imported)
    }

    /// 今の版を検証して受け入れる（アップロードと取り込みの前に行う。Web の `acceptBackupVersion` と同じ）。
    ///
    /// `auth_data` に自分のマスター鍵の署名があり、公開鍵が SSSS のバックアップ鍵と同じで、前に見た版より
    /// 古くないときだけ受け入れる。版が変わったら、すべての受信用セッションを送信待ちに戻す（新しい版を
    /// 見たことと同じ書き込みで記録する）。
    fn accept_backup_version(
        &mut self,
        current_version: Option<&Value>,
    ) -> Result<AcceptedVersion> {
        let Some(own) = self.own_keys()? else {
            return Ok(AcceptedVersion::None);
        };
        let mut backup = self.load_backup_state()?;
        backup.set_secrets_revision(self.state.secrets_revision);
        let Some(current_version) = current_version else {
            backup.suspend();
            self.backup_state = Some(backup);
            return Ok(AcceptedVersion::None);
        };
        let change = match backup.accept_current_version(
            current_version,
            &own.master_public_key,
            &own.backup_key,
        ) {
            Ok(change) => change,
            Err(error) => {
                self.backup_state = Some(backup);
                return Ok(AcceptedVersion::Rejected(error.to_string()));
            }
        };
        if change == VersionChange::Changed {
            // 新しい版には、手元のセッションをすべて送り直す。新しい版を見たことと同じ書き込みで記録する。
            for session in self.state.inbound_sessions.values_mut() {
                session.backed_up = false;
            }
        }
        self.state.backup_upload = Some(backup.to_json());
        let saved = self.save();
        let version = backup
            .current_version()
            .map(|current| current.version.clone());
        self.backup_state = Some(backup);
        saved?;
        Ok(match version {
            Some(version) => AcceptedVersion::Accepted(version),
            None => AcceptedVersion::None,
        })
    }

    /// ack を送れた ID を、処理済みの印から外す。
    pub fn acknowledged(&mut self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            self.state.processed_to_device.remove(id);
        }
        self.save()
    }

    /// 今の端末の署名付きの端末鍵。
    pub fn device_keys(&self) -> Value {
        self.account.device_keys()
    }

    /// 自分の鍵を保存してあるか（解錠済みの端末か）。
    pub fn has_own_keys(&self) -> bool {
        self.state.own_keys.is_some()
    }

    /// 保存した自分の鍵。まだ解錠していなければ `None`。
    pub fn own_keys(&self) -> Result<Option<OwnKeys>> {
        self.state
            .own_keys
            .as_deref()
            .map(|sealed| OwnKeys::from_pickle(sealed, &self.account, &self.storage_key))
            .transpose()
            .map_err(Into::into)
    }

    /// 最後に解錠したときの `secrets_revision`。
    pub fn secrets_revision(&self) -> Option<u64> {
        self.state.secrets_revision
    }

    /// 解錠し直しが要るか（自分の鍵はあるが、SSSS の秘密が書き換えられたと分かった）。
    pub fn needs_unlock(&self) -> bool {
        self.has_own_keys() && self.state.secrets_revision.is_none()
    }

    /// 鍵バックアップの送信が 409 `E2EE_SSSS_CONFLICT` になった。記録した `secrets_revision` を消して、
    /// 解錠し直すまで、暗号ストアの鍵を使う処理（署名し直し、鍵バックアップの送信）を止める。状態を読むと
    /// サーバーの値と違うので、解錠し直しを求める。
    pub fn forget_secrets_revision(&mut self) -> Result<()> {
        if let Some(backup) = &mut self.backup_state {
            backup.set_secrets_revision(None);
            backup.suspend();
        }
        if self.state.secrets_revision.is_none() {
            return Ok(());
        }
        self.state.secrets_revision = None;
        self.save()
    }

    /// 解錠した（または初回設定で作った）自分の鍵を保存する（#360。React の `saveOwnKeys` と同じ）。
    /// マスター鍵の秘密鍵は保存せず、自分のマスター鍵の公開鍵とユーザー署名鍵を、配布先の判定の
    /// 記録に入れる。
    pub fn save_own_keys(
        &mut self,
        secrets: &OwnSecrets,
        secrets_revision: Option<u64>,
    ) -> Result<()> {
        // 記録はコピーで更新し、すべて保存できたときだけ置き換える。
        let mut trust = TrustStore::from_json(&self.trust.to_json(), self.user_id())?;
        trust.trust_own_master_key(&secrets.cross_signing.master)?;
        trust.record_own_user_signing_key(&secrets.cross_signing.user_signing)?;
        let own_keys = secrets.own_keys().pickle(&self.account, &self.storage_key);
        let trust_store = trust.pickle(&self.account, &self.storage_key);
        let previous = (
            self.state.own_keys.replace(own_keys),
            std::mem::replace(&mut self.state.secrets_revision, secrets_revision),
            self.state.trust_store.replace(trust_store),
        );
        if let Err(error) = self.save() {
            // 書けなければ、メモリーの状態も前に戻す。
            (
                self.state.own_keys,
                self.state.secrets_revision,
                self.state.trust_store,
            ) = previous;
            return Err(error);
        }
        self.trust = trust;
        // 新しい秘密を記録したので、鍵バックアップの今の版は検証し直す。
        if let Some(backup) = &mut self.backup_state {
            backup.suspend();
        }
        Ok(())
    }

    /// 暗号ストアの自分の鍵とサーバーの状態から、起動時にすることを決める。まだ解錠していなければ `None`。
    pub fn startup_check(&self, server: &ServerKeyState) -> Result<Option<StartupCheck>> {
        let Some(own) = self.own_keys()? else {
            return Ok(None);
        };
        Ok(Some(zoo_crypto::startup_check(
            &own,
            server,
            &self.device_keys(),
        )?))
    }

    fn load_backup_state(&mut self) -> Result<BackupUploadState> {
        if let Some(state) = self.backup_state.take() {
            return Ok(state);
        }
        let saved = self
            .state
            .backup_upload
            .as_deref()
            .and_then(|json| BackupUploadState::from_json(json, &self.state.user_id).ok());
        match saved {
            Some(state) => Ok(state),
            None => Ok(BackupUploadState::new(&self.state.user_id)?),
        }
    }

    /// 保存した受信用セッションを、まだなら読み込む。読み込めたときだけ読み込み済みにする（壊れた
    /// pickle は、次の試行でもエラーになる。読めないセッションを送ったことにしない）。
    fn load_inbound(&mut self, key: &str) -> Result<()> {
        if self.loaded_inbound.contains(key) {
            return Ok(());
        }
        if let Some(stored) = self.state.inbound_sessions.get(key) {
            self.inbound
                .restore_session(&stored.pickle, &self.account, &self.storage_key)?;
        }
        self.loaded_inbound.insert(key.to_owned());
        Ok(())
    }

    /// 鍵バックアップへ送る次の要求を作る（docs/e2ee.md の「鍵バックアップ」。React の
    /// `CryptoStore.prepareBackupUpload` と同じ規則）。
    ///
    /// `current_version` は `GET /api/e2ee/key_backup/versions/current` の応答（なければ `None`）。
    /// 今の版の `auth_data` に自分のマスター鍵の署名があり、公開鍵が SSSS のバックアップ鍵と同じで、
    /// 前に見た版より古くないときだけ送る。版が変わったら、すべてのセッションを送り直す。
    pub fn prepare_backup_upload(
        &mut self,
        current_version: Option<&Value>,
        limit: usize,
    ) -> Result<BackupUploadRequest> {
        // 送信は 1 つずつ（`setup::upload_key_backup`）なので、ここで残っている要求は、送っている途中で
        // 捨てられたもの。記録を捨てる（入れたセッションは送信待ちのまま）。
        self.pending_uploads.clear();
        match self.accept_backup_version(current_version)? {
            AcceptedVersion::Accepted(_) => {}
            AcceptedVersion::None => return Ok(BackupUploadRequest::None),
            AcceptedVersion::Rejected(message) => {
                return Ok(BackupUploadRequest::Rejected(message));
            }
        }

        let keys: Vec<String> = self
            .state
            .inbound_sessions
            .iter()
            .filter(|(_, session)| !session.backed_up)
            .map(|(key, _)| key.clone())
            .take(limit)
            .collect();
        if keys.is_empty() {
            return Ok(BackupUploadRequest::None);
        }
        let mut refs = Vec::with_capacity(keys.len());
        for key in &keys {
            self.load_inbound(key)?;
            let [channel_id, sender_key, session_id]: [String; 3] =
                serde_json::from_str(key).context("受信用セッションのキーを読めません")?;
            refs.push(SessionRef {
                channel_id,
                sender_key,
                session_id,
            });
        }
        let backup = self.backup_state.as_mut().expect("直前に入れた状態がある");
        for session in refs {
            backup.enqueue(session);
        }
        let Some(upload) = backup.next_upload(&self.inbound, limit)? else {
            return Ok(BackupUploadRequest::None);
        };
        let sent: BTreeSet<String> = upload.sessions.iter().map(inbound_key).collect();
        let sessions: Vec<(String, String)> = keys
            .into_iter()
            .filter(|key| sent.contains(key))
            .filter_map(|key| {
                let pickle = self.state.inbound_sessions.get(&key)?.pickle.clone();
                Some((key, pickle))
            })
            .collect();
        self.next_upload_id += 1;
        let id = self.next_upload_id;
        let request = BackupUploadRequest::Upload {
            id,
            version: upload.version.clone(),
            secrets_revision: upload.secrets_revision,
            body: upload.body.clone(),
            count: sessions.len(),
        };
        self.pending_uploads
            .insert(id, PendingUpload { upload, sessions });
        Ok(request)
    }

    /// [`prepare_backup_upload`](Self::prepare_backup_upload) の `id` の要求が成功した。その要求に入れた
    /// セッションだけを送信済みにする（送ったあとに書き換わったものは除く）。
    pub fn backup_uploaded(&mut self, id: u64) -> Result<()> {
        let Some(pending) = self.pending_uploads.remove(&id) else {
            return Ok(());
        };
        if let Some(backup) = &mut self.backup_state {
            backup.upload_succeeded(&pending.upload);
            self.state.backup_upload = Some(backup.to_json());
        }
        for (key, pickle) in pending.sessions {
            if let Some(session) = self.state.inbound_sessions.get_mut(&key)
                && session.pickle == pickle
            {
                session.backed_up = true;
            }
        }
        self.save()
    }

    /// `id` の要求が失敗した。入れたセッションは送信待ちのまま残す。409 のときは、今の版を
    /// 取り直して検証できるまで送らない。
    pub fn backup_upload_failed(&mut self, id: u64, conflict: bool) {
        self.pending_uploads.remove(&id);
        if conflict && let Some(backup) = &mut self.backup_state {
            backup.suspend();
        }
    }

    /// 送っている途中（応答を待っている）の鍵バックアップの要求の数。
    pub fn uploads_in_flight(&self) -> usize {
        self.pending_uploads.len()
    }

    /// 鍵バックアップにまだ送っていない受信用セッションの数。
    pub fn pending_backup_count(&self) -> usize {
        self.state
            .inbound_sessions
            .values()
            .filter(|session| !session.backed_up)
            .count()
    }

    /// 暗号ストアを消す（ログアウト）。消し終えるまで、ユーザーのロックを持ち続ける。
    pub fn destroy(self) -> Result<()> {
        let Self { dir, _lock, .. } = self;
        let result =
            fs::remove_dir_all(&dir).with_context(|| format!("{} を消せません", dir.display()));
        drop(_lock);
        result
    }
}

/// 暗号ストアを開かずに、暗号化チャンネルとして記録したチャンネルを読む（#361）。起動の直後、端末の管理の
/// タスクが暗号ストアを開くより前から、格下げを見つけて平文の送信を止めるため。`state.json` は `rename` で
/// 置き換えるので、ロックを持たずに読んでも、書きかけを読むことはない。まだ暗号ストアがなければ空。
/// ファイルを読めない、形式が壊れているときはエラーにする（記録がないものとして扱わない）。
pub fn read_encrypted_channels(root: &Path, user_id: &str) -> Result<Vec<String>> {
    let dir = user_dir(root, user_id)?;
    // 保存鍵を失って作り直しているあいだ（`state.json` がない）は、置いておいた前の記録を読む。
    // 読むあいだに作り直しを終えて前の記録を消したときは、新しい `state.json` を読み直す。
    let records = match read_kept_records(&dir.join(STATE_FILE), user_id)? {
        Some(records) => Some(records),
        None => match read_kept_records(&dir.join(REPLACED_STATE_FILE), user_id)? {
            Some(records) => Some(records),
            None => read_kept_records(&dir.join(STATE_FILE), user_id)?,
        },
    };
    Ok(records
        .map(|records| records.encrypted_channels.into_iter().collect())
        .unwrap_or_default())
}

/// `root` にある暗号ストアのユーザー ID（ログアウトで消すものを探す）。
pub fn stored_user_ids(root: &Path) -> Result<Vec<String>> {
    let mut users = Vec::new();
    match fs::read_dir(root) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if entry.file_type()?.is_dir()
                    && let Some(name) = entry.file_name().to_str()
                {
                    users.push(name.to_owned());
                }
            }
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    users.sort();
    Ok(users)
}

/// 削除 API が失敗した端末。同じユーザーの次の起動で消し直す。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDeviceDeletion {
    pub user_id: String,
    pub device_id: String,
    /// 登録できたかを確かめられていない（登録の要求の結果を受け取っていない）。この端末の 404 は、
    /// あとから届いた登録で端末ができうるので、すぐには消えたものとしない。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unconfirmed: bool,
    /// `unconfirmed` の端末で、続けて 404 を受け取った回数。
    #[serde(default, skip_serializing_if = "is_zero")]
    pub not_found: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

impl PendingDeviceDeletion {
    pub fn new(user_id: &str, device_id: &str, unconfirmed: bool) -> Self {
        Self {
            user_id: user_id.to_owned(),
            device_id: device_id.to_owned(),
            unconfirmed,
            not_found: 0,
        }
    }

    /// 同じ端末かどうか（記録の状態は比べない）。
    pub fn same_device(&self, other: &Self) -> bool {
        self.user_id == other.user_id && self.device_id == other.device_id
    }
}

const PENDING_FILE: &str = "pending-device-delete.json";
/// 読めなくなった削除待ちの一覧を退避する先（最後に退避したものだけを持つ）。
const PENDING_UNREADABLE_FILE: &str = "pending-device-delete.unreadable.json";

/// 削除待ちの一覧を読んで書き戻すあいだ、プロセスをまたいで直列にする（`<root>/pending-device-delete.lock`）。
/// ユーザーのロックとは別にする（一覧は全ユーザーで共有し、ストアを開く前にも更新するため）。
fn with_pending_lock<T>(root: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    create_private_dir(root)?;
    let lock = open_lock_file(&root.join(PENDING_LOCK_FILE))?;
    lock.lock()
        .with_context(|| format!("{} をロックできません", PENDING_LOCK_FILE))?;
    let result = f();
    drop(lock);
    result
}

/// 削除待ちに足す。ユーザーと端末の組ごとに残し、別のユーザーの削除待ちを上書きしない。
/// 同じ端末があれば、`unconfirmed` を合わせ（どちらかが真なら真）、404 の回数は保つ。
///
/// 中身を読めない一覧は退避してから空の一覧に足す（[`read_for_update`]）。
pub fn mark_device_deletion_pending(root: &Path, pending: &PendingDeviceDeletion) -> Result<()> {
    with_pending_lock(root, || {
        let mut all = read_for_update(root)?;
        if let Some(existing) = all.iter_mut().find(|entry| entry.same_device(pending)) {
            if existing.unconfirmed || !pending.unconfirmed {
                return Ok(());
            }
            existing.unconfirmed = true;
        } else {
            all.push(PendingDeviceDeletion {
                not_found: 0,
                ..pending.clone()
            });
        }
        write_private(&root.join(PENDING_FILE), &serde_json::to_vec(&all)?)
    })
}

/// 同じ端末の記録を `pending` に置き換える（404 の回数を数えるため）。なければ何もしない。
pub fn update_pending_device_deletion(root: &Path, pending: &PendingDeviceDeletion) -> Result<()> {
    with_pending_lock(root, || {
        let mut all = read_for_update(root)?;
        let Some(existing) = all.iter_mut().find(|entry| entry.same_device(pending)) else {
            return Ok(());
        };
        *existing = pending.clone();
        write_private(&root.join(PENDING_FILE), &serde_json::to_vec(&all)?)
    })
}

/// 削除待ちの端末（全ユーザー分）。ファイルがなければ空。読めない、形式が違うときはエラー
/// （ファイルは、次に書き換えるときに退避する）。
pub fn read_pending_device_deletions(root: &Path) -> Result<Vec<PendingDeviceDeletion>> {
    let path = root.join(PENDING_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    parse_pending(&bytes).with_context(|| format!("{} を読めません", path.display()))
}

fn parse_pending(bytes: &[u8]) -> serde_json::Result<Vec<PendingDeviceDeletion>> {
    serde_json::from_slice(bytes)
}

/// 書き換えるために一覧を読む（削除待ちのロックの中で呼ぶ）。中身を読めない（形式が違う）ファイルは
/// `pending-device-delete.unreadable.json` に退避して、空の一覧から始める。読めないファイルを残しても
/// 消し直しには使えず、以後の削除待ちを記録できなくなる（保存鍵を失ったときは暗号ストアも開けない）
/// ため。ファイルを読み出せない（権限など）ときはエラーにし、ファイルを変えない。
fn read_for_update(root: &Path) -> Result<Vec<PendingDeviceDeletion>> {
    let path = root.join(PENDING_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.into()),
    };
    match parse_pending(&bytes) {
        Ok(all) => Ok(all),
        Err(_) => {
            fs::rename(&path, root.join(PENDING_UNREADABLE_FILE))
                .with_context(|| format!("{} を退避できません", path.display()))?;
            Ok(Vec::new())
        }
    }
}

/// 消せた端末を、削除待ちから外す。
pub fn clear_pending_device_deletion(root: &Path, done: &PendingDeviceDeletion) -> Result<()> {
    with_pending_lock(root, || {
        let mut rest = read_for_update(root)?;
        let before = rest.len();
        rest.retain(|entry| !entry.same_device(done));
        if rest.len() == before {
            return Ok(());
        }
        if rest.is_empty() {
            match fs::remove_file(root.join(PENDING_FILE)) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
                _ => Ok(()),
            }
        } else {
            write_private(&root.join(PENDING_FILE), &serde_json::to_vec(&rest)?)
        }
    })
}

/// テストと manager が使う、`keys/query` の応答の形の空の値。
pub fn empty_device_keys() -> Value {
    Value::Object(Map::new())
}
