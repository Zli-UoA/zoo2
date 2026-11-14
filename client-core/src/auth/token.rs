//! トークンの保存と更新。トークンは設定ディレクトリの `token.json` に、所有者だけが読める
//! 権限で置く。アクセストークンは期限の 60 秒前に refresh_token で更新する。

use std::{
    fs::{File, OpenOptions, TryLockError},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex as StdMutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, watch};

use super::{Discovery, TokenResponse, TokenResult, discover, token_request};
use crate::{
    config::OidcConfig,
    http::{BearerSource, HttpError, TokenPersistenceReport, TokenPersistenceStatus},
};

/// 期限までの余裕。frontend の `REFRESH_SKEW_MS` と同じ。
const REFRESH_SKEW_SECONDS: i64 = 60;
/// `expires_in` がない応答の既定値。frontend と同じく 300 秒とみなす。
const DEFAULT_EXPIRES_IN: i64 = 300;
// Refresh holds this cross-process lock for up to TOKEN_REFRESH_TIMEOUT. Leave
// room for scheduler and persistence work so logout does not abandon a valid
// in-flight rotation just before its response can be saved.
const TOKEN_FILE_LOCK_TIMEOUT: Duration = Duration::from_secs(35);
const TOKEN_FILE_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTokens {
    /// ログイン単位の識別子。別プロセスの logout/login を古い manager が検出する。
    #[serde(default)]
    pub session_id: String,
    /// どの IdP とクライアントで得たトークンか。設定が変わったら使わない。
    pub issuer: String,
    pub client_id: String,
    pub resource: Option<String>,
    /// トークンを送ってよい API の URL。別の URL に変えたときに、このトークンを送らないために保存する。
    /// このフィールドを持たない古いファイルは空になり、どの接続先とも一致しない (ログインし直す)。
    #[serde(default)]
    pub api_base_url: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// UNIX 秒。
    pub expires_at: i64,
}

impl StoredTokens {
    pub fn new(oidc: &OidcConfig, api_base_url: &str, response: TokenResponse, now: i64) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: api_base_url.to_string(),
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            expires_at: now + response.expires_in.unwrap_or(DEFAULT_EXPIRES_IN),
        }
    }

    fn matches(&self, oidc: &OidcConfig, api_base_url: Option<&str>) -> bool {
        self.issuer == oidc.issuer
            && self.client_id == oidc.client_id
            && self.resource == oidc.resource
            && api_base_url.is_none_or(|url| self.api_base_url == url)
    }
}

fn hash_identity_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

/// Stable identity for a pre-session-id file. Logout may have captured that
/// legacy snapshot before another process migrates and refreshes it, so the
/// migrated ID must be derivable from the original contents after those writes.
fn legacy_session_id(tokens: &StoredTokens) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"zoo2-tui:legacy-session:v1\0");
    hash_identity_field(&mut hasher, tokens.issuer.as_bytes());
    hash_identity_field(&mut hasher, tokens.client_id.as_bytes());
    match tokens.resource.as_deref() {
        Some(resource) => {
            hasher.update([1]);
            hash_identity_field(&mut hasher, resource.as_bytes());
        }
        None => hasher.update([0]),
    }
    hash_identity_field(&mut hasher, tokens.api_base_url.as_bytes());
    hash_identity_field(&mut hasher, tokens.access_token.as_bytes());
    match tokens.refresh_token.as_deref() {
        Some(refresh_token) => {
            hasher.update([1]);
            hash_identity_field(&mut hasher, refresh_token.as_bytes());
        }
        None => hasher.update([0]),
    }
    hasher.update(tokens.expires_at.to_be_bytes());

    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = hasher.finalize();
    let mut session_id = String::with_capacity("legacy-".len() + digest.len() * 2);
    session_id.push_str("legacy-");
    for byte in digest {
        session_id.push(HEX[(byte >> 4) as usize] as char);
        session_id.push(HEX[(byte & 0x0f) as usize] as char);
    }
    session_id
}

pub fn token_path(config_dir: &Path) -> PathBuf {
    config_dir.join("token.json")
}

/// New save/delete/retry operations share this advisory lock across processes.
/// Older binaries did not take it and can still replace this file or recreate
/// it after logout; the lock only coordinates versions using this protocol.
fn open_token_file_lock(path: &Path, create_parent: bool) -> Result<Option<File>> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if create_parent {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("{} を作れません", parent.display()))?;
    } else {
        match std::fs::metadata(parent) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(anyhow!("{} はディレクトリではありません", parent.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("{} を読めません", parent.display()));
            }
        }
    }

    let lock_path = path.with_extension("json.lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options
        .open(&lock_path)
        .with_context(|| format!("{} を開けません", lock_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        use std::os::unix::fs::PermissionsExt;
        let metadata = file
            .metadata()
            .with_context(|| format!("{} を確認できません", lock_path.display()))?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(anyhow!(
                "{} は安全なロックファイルではありません",
                lock_path.display()
            ));
        }
        file.set_permissions(std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("{} の権限を設定できません", lock_path.display()))?;
    }
    Ok(Some(file))
}

fn acquire_token_file_lock(file: File, path: &Path) -> Result<File> {
    let lock_path = path.with_extension("json.lock");
    let deadline = Instant::now() + TOKEN_FILE_LOCK_TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(anyhow!(
                        "{} のロック待ちが時間切れになりました",
                        lock_path.display()
                    ));
                }
                std::thread::sleep(TOKEN_FILE_LOCK_RETRY_INTERVAL.min(remaining));
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("{} をロックできません", lock_path.display()));
            }
        }
    }
}

fn token_file_lock(path: &Path, create_parent: bool) -> Result<Option<File>> {
    let Some(file) = open_token_file_lock(path, create_parent)? else {
        return Ok(None);
    };
    Ok(Some(acquire_token_file_lock(file, path)?))
}

async fn token_file_lock_async(path: &Path, create_parent: bool) -> Result<Option<File>> {
    let Some(file) = open_token_file_lock(path, create_parent)? else {
        return Ok(None);
    };
    Ok(Some(
        acquire_token_file_lock_async(file, path, TOKEN_FILE_LOCK_TIMEOUT).await?,
    ))
}

async fn acquire_token_file_lock_async(file: File, path: &Path, timeout: Duration) -> Result<File> {
    let lock_path = path.with_extension("json.lock");
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(TryLockError::WouldBlock) => {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    return Err(anyhow!(
                        "{} のロック待ちが時間切れになりました",
                        lock_path.display()
                    ));
                }
                tokio::time::sleep(TOKEN_FILE_LOCK_RETRY_INTERVAL.min(deadline - now)).await;
            }
            Err(TryLockError::Error(error)) => {
                return Err(error)
                    .with_context(|| format!("{} をロックできません", lock_path.display()));
            }
        }
    }
}

/// 保存済みのトークン。ないか、別の IdP やクライアント、別の API の URL のものなら None。
/// `api_base_url` が None のときは、IdP 側の操作 (refresh_token の失効) 用に URL を照合しない。
pub fn load(
    path: &Path,
    oidc: &OidcConfig,
    api_base_url: Option<&str>,
) -> Result<Option<StoredTokens>> {
    let Some(_lock) = token_file_lock(path, false)? else {
        return Ok(None);
    };
    cleanup_stale_token_temps_unlocked(path)?;
    load_and_migrate_unlocked(path, oidc, api_base_url)
}

/// Async counterpart used by Tokio entry points so cross-process contention
/// waits without blocking an executor thread and can be cancelled by the caller.
pub async fn load_async(
    path: &Path,
    oidc: &OidcConfig,
    api_base_url: Option<&str>,
) -> Result<Option<StoredTokens>> {
    let Some(lock) = token_file_lock_async(path, false).await? else {
        return Ok(None);
    };
    let (_lock, cleanup_error) = cleanup_stale_token_temps_async(path, lock).await?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    load_and_migrate_unlocked(path, oidc, api_base_url)
}

fn load_and_migrate_unlocked(
    path: &Path,
    oidc: &OidcConfig,
    api_base_url: Option<&str>,
) -> Result<Option<StoredTokens>> {
    let Some(mut tokens) = load_unlocked(path, oidc, api_base_url)? else {
        return Ok(None);
    };
    // Give pre-session-id files a stable identity before a manager starts using
    // them. Failure to migrate is an error: otherwise a stale legacy snapshot
    // could not be distinguished from a later login.
    if tokens.session_id.is_empty() {
        tokens.session_id = legacy_session_id(&tokens);
        save_unlocked(path, &tokens)?;
    }
    Ok(Some(tokens))
}

fn load_unlocked(
    path: &Path,
    oidc: &OidcConfig,
    api_base_url: Option<&str>,
) -> Result<Option<StoredTokens>> {
    Ok(read_tokens_unlocked(path)?.filter(|tokens| tokens.matches(oidc, api_base_url)))
}

fn read_tokens_unlocked(path: &Path) -> Result<Option<StoredTokens>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = match options.open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).with_context(|| format!("{} を読めません", path.display())),
    };
    if !file
        .metadata()
        .with_context(|| format!("{} を確認できません", path.display()))?
        .is_file()
    {
        return Err(anyhow!("{} は通常ファイルではありません", path.display()));
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut file, &mut text)
        .with_context(|| format!("{} を読めません", path.display()))?;
    let tokens: StoredTokens = serde_json::from_str(&text)
        .with_context(|| format!("{} を解釈できません", path.display()))?;
    Ok(Some(tokens))
}

pub fn save(path: &Path, tokens: &StoredTokens) -> Result<()> {
    let Some(_lock) = token_file_lock(path, true)? else {
        return Err(anyhow!("{} の保存先を用意できません", path.display()));
    };
    cleanup_stale_token_temps_unlocked(path)?;
    save_unlocked(path, tokens)
}

/// Execute a guarded save after acquiring the cross-process lock. The caller can
/// acquire its in-process session lock inside `operation`, avoiding lock inversion.
pub fn with_locked_save<T>(
    path: &Path,
    operation: impl FnOnce(&dyn Fn(&StoredTokens) -> Result<()>) -> Result<T>,
) -> Result<T> {
    let Some(_lock) = token_file_lock(path, true)? else {
        return Err(anyhow!("{} の保存先を用意できません", path.display()));
    };
    cleanup_stale_token_temps_unlocked(path)?;
    operation(&|tokens| save_unlocked(path, tokens))
}

/// Await the cross-process lock before inspecting and updating a stored session.
pub async fn with_locked_save_async<T>(
    path: &Path,
    operation: impl FnOnce(Option<StoredTokens>, &dyn Fn(&StoredTokens) -> Result<()>) -> Result<T>,
) -> Result<T> {
    let Some(lock) = token_file_lock_async(path, true).await? else {
        return Err(anyhow!("{} の保存先を用意できません", path.display()));
    };
    let (_lock, cleanup_error) = cleanup_stale_token_temps_async(path, lock).await?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    let current = read_tokens_unlocked(path)?;
    operation(current, &|tokens| save_unlocked(path, tokens))
}

/// Async counterpart used by Tokio entry points for cancellable lock waiting.
pub async fn save_async(path: &Path, tokens: &StoredTokens) -> Result<()> {
    let Some(lock) = token_file_lock_async(path, true).await? else {
        return Err(anyhow!("{} の保存先を用意できません", path.display()));
    };
    let (_lock, cleanup_error) = cleanup_stale_token_temps_async(path, lock).await?;
    if let Some(error) = cleanup_error {
        return Err(error);
    }
    save_unlocked(path, tokens)
}

fn cleanup_stale_token_temps_unlocked(path: &Path) -> Result<()> {
    cleanup_stale_token_temps_unlocked_with(path, |candidate| std::fs::remove_file(candidate))
}

fn cleanup_stale_token_temps_unlocked_with<F>(path: &Path, mut remove: F) -> Result<()>
where
    F: FnMut(&Path) -> std::io::Result<()>,
{
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    let prefix = format!("{file_name}.tmp-");
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let entries = match std::fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("{} を調べられません", parent.display()));
        }
    };

    let mut failures = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(format!("ディレクトリエントリを読めません: {error}"));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(suffix) = name.strip_prefix(&prefix) else {
            continue;
        };
        if suffix.len() != 32
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            continue;
        }

        // file_type and symlink_metadata inspect the directory entry itself;
        // never follow a symlink while deciding whether it is ours to remove.
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                failures.push(format!(
                    "{} を確認できません: {error}",
                    entry.path().display()
                ));
                continue;
            }
        };
        if !file_type.is_file() {
            continue;
        }
        let metadata = match std::fs::symlink_metadata(entry.path()) {
            Ok(metadata) if metadata.file_type().is_file() => metadata,
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                failures.push(format!(
                    "{} を確認できません: {error}",
                    entry.path().display()
                ));
                continue;
            }
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            // geteuid is a read-only query for the process identity.
            let process_uid = unsafe { libc::geteuid() };
            // Leave entries with a different owner, mode, or link count alone.
            // Current token temps are always private, singly linked regular files.
            if metadata.uid() != process_uid
                || metadata.permissions().mode() & 0o7777 != 0o600
                || metadata.nlink() != 1
            {
                failures.push(format!(
                    "{} は安全な一時 token ファイルの条件を満たさないため残しました",
                    entry.path().display()
                ));
                continue;
            }
        }

        match remove(&entry.path()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "一時 token {} を削除できません: {error}",
                entry.path().display()
            )),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "古い token 一時ファイルを回収できませんでした: {}",
            failures.join("; ")
        ))
    }
}

async fn cleanup_stale_token_temps_async(
    path: &Path,
    lock: File,
) -> Result<(File, Option<anyhow::Error>)> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let cleanup_error = cleanup_stale_token_temps_unlocked(&path).err();
        Ok::<_, anyhow::Error>((lock, cleanup_error))
    })
    .await
    .context("token 一時ファイルの回収処理が中断されました")?
}

fn save_unlocked(path: &Path, tokens: &StoredTokens) -> Result<()> {
    let encoded = serde_json::to_vec_pretty(tokens).context("トークンをJSONに変換できません")?;
    let temporary = unique_temporary_path(path);
    write_atomic_file(
        path,
        &temporary,
        &encoded,
        std::io::Write::write_all,
        |file| file.sync_all(),
    )
}

/// Current writers use a unique sibling and exclusive creation; old binaries
/// may still write the fixed `token.json.tmp` name without the lock.
fn unique_temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("token"));
    let mut temporary_name = file_name.to_os_string();
    temporary_name.push(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
    path.with_file_name(temporary_name)
}

fn open_new_temp_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        options.custom_flags(libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options
        .open(path)
        .with_context(|| format!("{} を排他的に作成できません", path.display()))
}

struct PendingTokenTemp {
    path: PathBuf,
    file: Option<File>,
    committed: bool,
}

impl PendingTokenTemp {
    fn file(&self) -> &File {
        self.file.as_ref().expect("temporary file remains open")
    }

    fn file_mut(&mut self) -> &mut File {
        self.file.as_mut().expect("temporary file remains open")
    }

    fn cleanup(&mut self) -> std::io::Result<()> {
        drop(self.file.take());
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

impl Drop for PendingTokenTemp {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.cleanup();
        }
    }
}

fn write_atomic_file<W, S>(
    destination: &Path,
    temporary: &Path,
    contents: &[u8],
    write: W,
    sync: S,
) -> Result<()>
where
    W: FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
    S: FnOnce(&File) -> std::io::Result<()>,
{
    let file = open_new_temp_file(temporary)?;
    let mut pending = PendingTokenTemp {
        path: temporary.to_path_buf(),
        file: Some(file),
        committed: false,
    };
    let result = (|| {
        let metadata = pending
            .file()
            .metadata()
            .with_context(|| format!("{} を確認できません", temporary.display()))?;
        if !metadata.is_file() {
            return Err(anyhow!(
                "{} は通常ファイルではありません",
                temporary.display()
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if metadata.nlink() != 1 {
                return Err(anyhow!(
                    "{} は安全な一時ファイルではありません",
                    temporary.display()
                ));
            }
            pending
                .file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("{} の権限を設定できません", temporary.display()))?;
        }
        write(pending.file_mut(), contents)
            .with_context(|| format!("{} に書き込めません", temporary.display()))?;
        sync(pending.file())
            .with_context(|| format!("{} を同期できません", temporary.display()))?;
        drop(pending.file.take());
        std::fs::rename(temporary, destination)
            .with_context(|| format!("{} に保存できません", destination.display()))?;
        pending.committed = true;
        Ok(())
    })();

    if let Err(error) = result {
        return match pending.cleanup() {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(anyhow!(
                "{error:#}; 一時トークンファイル {} の後始末にも失敗しました: {cleanup_error}",
                temporary.display()
            )),
        };
    }
    Ok(())
}

pub fn delete(path: &Path) -> Result<bool> {
    let Some(_lock) = token_file_lock(path, false)? else {
        return Ok(false);
    };
    cleanup_stale_token_temps_unlocked(path)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).with_context(|| format!("{} を消せません", path.display())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteOutcome {
    Deleted,
    Missing,
    Changed,
}

const LOGOUT_REVOCATION_TIMEOUT: Duration = Duration::from_secs(2);

/// Lock the current session, revoke its latest refresh token, then remove only
/// that session. Concurrent refreshes and logins using this protocol wait for
/// logout. Older binaries that ignore the lock can still rewrite the file.
pub async fn logout(
    path: &Path,
    oidc: &OidcConfig,
    http: &reqwest::Client,
) -> Result<DeleteOutcome> {
    // Capture the session before waiting for a potentially long refresh. If a
    // different login acquires the lock first, the comparison below prevents
    // this logout request from revoking or deleting that replacement.
    let expected = read_tokens_unlocked(path)?;
    let Some(lock) = token_file_lock_async(path, false).await? else {
        return Ok(DeleteOutcome::Missing);
    };
    let (_lock, cleanup_error) = cleanup_stale_token_temps_async(path, lock).await?;
    logout_with_cleanup_result(path, oidc, http, expected, _lock, cleanup_error).await
}

async fn logout_with_cleanup_result(
    path: &Path,
    oidc: &OidcConfig,
    http: &reqwest::Client,
    expected: Option<StoredTokens>,
    lock: File,
    cleanup_error: Option<anyhow::Error>,
) -> Result<DeleteOutcome> {
    let _lock = lock;
    let Some(expected) = expected else {
        return report_logout_cleanup_failure(cleanup_error, DeleteOutcome::Missing);
    };
    if !expected.matches(oidc, None) {
        let outcome = match read_tokens_unlocked(path)? {
            None => DeleteOutcome::Missing,
            Some(current) if current == expected => match std::fs::remove_file(path) {
                Ok(()) => DeleteOutcome::Deleted,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    DeleteOutcome::Missing
                }
                Err(error) => {
                    return Err(include_logout_cleanup_error(error.into(), cleanup_error));
                }
            },
            Some(_) => DeleteOutcome::Changed,
        };
        return report_logout_cleanup_failure(cleanup_error, outcome);
    }
    // Logout must remain possible for legacy files even when migration cannot
    // be persisted (for example, an obstructed .tmp path). The held file lock
    // is the identity boundary for this legacy session.
    let mut latest = match load_unlocked(path, oidc, None) {
        Ok(Some(latest)) => latest,
        Ok(None) => {
            let outcome = if read_tokens_unlocked(path)?.is_some() {
                DeleteOutcome::Changed
            } else {
                DeleteOutcome::Missing
            };
            return report_logout_cleanup_failure(cleanup_error, outcome);
        }
        Err(error) => return Err(include_logout_cleanup_error(error, cleanup_error)),
    };
    if !same_logout_session(&latest, &expected, oidc) {
        return report_logout_cleanup_failure(cleanup_error, DeleteOutcome::Changed);
    }
    let revoke = async {
        if latest.refresh_token.is_some() {
            let discovery = discover(http, &oidc.issuer).await?;
            if let Some(endpoint) = discovery.revocation_endpoint {
                loop {
                    if let Some(refresh_token) = latest.refresh_token.as_deref() {
                        let form = [
                            ("token", refresh_token),
                            ("token_type_hint", "refresh_token"),
                            ("client_id", oidc.client_id.as_str()),
                        ];
                        let _ = http.post(&endpoint).form(&form).send().await;
                    }
                    let Some(current) = read_tokens_unlocked(path)? else {
                        return Ok::<(), anyhow::Error>(());
                    };
                    if !same_logout_session(&current, &latest, oidc) {
                        return Ok(());
                    }
                    if current.refresh_token == latest.refresh_token {
                        return Ok(());
                    }
                    latest = current;
                }
            }
        }
        Ok::<(), anyhow::Error>(())
    };
    // Discovery and all revocation attempts share one deadline. If the IdP is
    // unavailable or keeps delaying, local logout still proceeds below.
    let _ = tokio::time::timeout(LOGOUT_REVOCATION_TIMEOUT, revoke).await;
    let outcome = match delete_current_session_unlocked(path, oidc, &latest) {
        Ok(outcome) => outcome,
        Err(error) => return Err(include_logout_cleanup_error(error, cleanup_error)),
    };
    report_logout_cleanup_failure(cleanup_error, outcome)
}

fn include_logout_cleanup_error(
    error: anyhow::Error,
    cleanup_error: Option<anyhow::Error>,
) -> anyhow::Error {
    match cleanup_error {
        Some(cleanup_error) => error.context(format!(
            "古い token 一時ファイルも削除できませんでした: {cleanup_error:#}"
        )),
        None => error,
    }
}

fn report_logout_cleanup_failure(
    cleanup_error: Option<anyhow::Error>,
    outcome: DeleteOutcome,
) -> Result<DeleteOutcome> {
    if let Some(error) = cleanup_error {
        Err(anyhow!(
            "ログアウト結果は {outcome:?} ですが、古い token 一時ファイルを削除できませんでした: {error:#}"
        ))
    } else {
        Ok(outcome)
    }
}

fn same_logout_session(current: &StoredTokens, expected: &StoredTokens, oidc: &OidcConfig) -> bool {
    expected.matches(oidc, None)
        && current.matches(oidc, None)
        && if expected.session_id.is_empty() {
            // Recognize either the untouched legacy snapshot or its stable
            // migration ID, which remains unchanged through refresh rotation.
            current == expected || current.session_id == legacy_session_id(expected)
        } else {
            current.session_id == expected.session_id
        }
}

fn delete_current_session_unlocked(
    path: &Path,
    oidc: &OidcConfig,
    expected: &StoredTokens,
) -> Result<DeleteOutcome> {
    let Some(current) = read_tokens_unlocked(path)? else {
        return Ok(DeleteOutcome::Missing);
    };
    if !same_logout_session(&current, expected, oidc) {
        return Ok(DeleteOutcome::Changed);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(DeleteOutcome::Deleted),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(DeleteOutcome::Missing),
        Err(error) => Err(error).with_context(|| format!("{} を消せません", path.display())),
    }
}

/// Delete only the same login session, including a newer token rotation.
pub fn delete_if_same_session(
    path: &Path,
    oidc: &OidcConfig,
    expected: &StoredTokens,
) -> Result<(DeleteOutcome, Option<StoredTokens>)> {
    let Some(_lock) = token_file_lock(path, false)? else {
        return Ok((DeleteOutcome::Missing, None));
    };
    cleanup_stale_token_temps_unlocked(path)?;
    let current = read_tokens_unlocked(path)?;
    let outcome = delete_current_session_unlocked(path, oidc, expected)?;
    Ok((
        outcome,
        current.filter(|_| outcome == DeleteOutcome::Deleted),
    ))
}

/// Read a snapshot for later comparison before deleting a token file.
pub fn read_token_snapshot(path: &Path) -> Result<Option<StoredTokens>> {
    read_tokens_unlocked(path)
}

/// Delete only the snapshot observed before revocation, while holding the file lock.
pub fn delete_if_unchanged(path: &Path, expected: Option<&StoredTokens>) -> Result<DeleteOutcome> {
    let Some(_lock) = token_file_lock(path, false)? else {
        return Ok(DeleteOutcome::Missing);
    };
    let cleanup_error = cleanup_stale_token_temps_unlocked(path).err();
    let current = match read_tokens_unlocked(path) {
        Ok(current) => current,
        Err(error) => return Err(include_logout_cleanup_error(error, cleanup_error)),
    };
    if current.as_ref() != expected {
        return report_logout_cleanup_failure(
            cleanup_error,
            if current.is_some() {
                DeleteOutcome::Changed
            } else {
                DeleteOutcome::Missing
            },
        );
    }
    if current.is_none() {
        return report_logout_cleanup_failure(cleanup_error, DeleteOutcome::Missing);
    }
    let outcome = match std::fs::remove_file(path) {
        Ok(()) => DeleteOutcome::Deleted,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => DeleteOutcome::Missing,
        Err(err) => {
            return Err(include_logout_cleanup_error(
                anyhow!(err).context(format!("{} を消せません", path.display())),
                cleanup_error,
            ));
        }
    };
    report_logout_cleanup_failure(cleanup_error, outcome)
}

pub fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

struct State {
    tokens: StoredTokens,
    discovery: Option<Discovery>,
    /// Disk snapshot that remained after a successful rotation could not be saved.
    pending_persist: Option<StoredTokens>,
    session_changed: bool,
    pending_error_reason: Option<&'static str>,
    refresh_outcome_unknown: bool,
}

struct TokenTaskTracker {
    state: StdMutex<TokenTaskTrackerState>,
    active_tx: watch::Sender<usize>,
}

struct TokenTaskTrackerState {
    active: usize,
    shutting_down: bool,
}

struct TokenTaskRegistration(Arc<TokenTaskTracker>);

impl TokenTaskTracker {
    fn new() -> Arc<Self> {
        let (active_tx, _) = watch::channel(0);
        Arc::new(Self {
            state: StdMutex::new(TokenTaskTrackerState {
                active: 0,
                shutting_down: false,
            }),
            active_tx,
        })
    }

    fn register(self: &Arc<Self>) -> Option<TokenTaskRegistration> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.shutting_down {
            return None;
        }
        state.active += 1;
        self.active_tx.send_replace(state.active);
        Some(TokenTaskRegistration(self.clone()))
    }

    async fn shutdown_and_wait(&self) {
        let mut active = self.active_tx.subscribe();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.shutting_down = true;
            self.active_tx.send_replace(state.active);
        }
        loop {
            if *active.borrow_and_update() == 0 {
                return;
            }
            if active.changed().await.is_err() {
                return;
            }
        }
    }
}

impl Drop for TokenTaskRegistration {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active = state.active.saturating_sub(1);
        self.0.active_tx.send_replace(state.active);
    }
}

#[derive(Debug, thiserror::Error)]
#[error(
    "IdP は refresh token を更新しましたが token.json に保存できませんでした。ストレージを復旧してアプリ内で再試行してください。未保存の token は、別 process の logout から失効できない可能性があります"
)]
pub struct RefreshPersistencePending;

#[derive(Debug, thiserror::Error)]
#[error("token manager is shutting down and no longer accepts token requests")]
struct TokenManagerShuttingDown;

/// Let startup retain the token manager only for the recoverable persistence failure.
pub fn is_refresh_persistence_pending(error: &anyhow::Error) -> bool {
    error.downcast_ref::<RefreshPersistencePending>().is_some()
}

/// Safe, non-secret explanation for the startup recovery screen.
pub fn refresh_persistence_retry_reason(error: &anyhow::Error) -> &'static str {
    for cause in error.chain() {
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            return match error.kind() {
                std::io::ErrorKind::PermissionDenied => {
                    "token 保存先へのアクセスが拒否されました。権限と保存先を確認してください。"
                }
                _ => "token 保存先の読み書きに失敗しました。ディスクと保存先を確認してください。",
            };
        }
        if cause.downcast_ref::<reqwest::Error>().is_some() {
            return "IdP との通信に失敗しました。接続状態を確認してください。";
        }
        if cause.downcast_ref::<serde_json::Error>().is_some() {
            return "保存済み token を読み取れません。token ファイルを確認してください。";
        }
        if cause.is::<HttpError>() {
            return "IdP が refresh token を拒否しました。認証状態を確認してください。";
        }
        let message = cause.to_string();
        if message.contains("ロック待ちが時間切れ") {
            return "別 process が token ファイルを使用中です。少し待って再試行してください。";
        }
        if message.contains("時間切れ") {
            return "IdP の応答が時間切れになりました。接続状態を確認してください。";
        }
        if message.contains("トークンを更新できませんでした") {
            return "IdP が refresh token を更新できませんでした。状態を確認して再試行してください。";
        }
    }
    "token 保存または refresh の再試行に失敗しました。原因を確認して再試行してください。"
}

/// Don't hold the cross-process token lock indefinitely while an IdP is down.
const TOKEN_REFRESH_TIMEOUT: Duration = Duration::from_secs(30);
/// Bound discovery too: access_token currently serializes manager state while
/// fetching metadata, so a stalled IdP must release that state for retries.
const TOKEN_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(5);

fn reconcile_disk_snapshot(state: &mut State, disk: Option<StoredTokens>) -> Result<StoredTokens> {
    let Some(disk) = disk else {
        state.pending_persist = None;
        state.session_changed = true;
        state.pending_error_reason = None;
        state.refresh_outcome_unknown = false;
        return Err(HttpError::Unauthorized.into());
    };
    if disk.session_id != state.tokens.session_id || disk.api_base_url != state.tokens.api_base_url
    {
        state.pending_persist = None;
        state.session_changed = true;
        state.pending_error_reason = None;
        state.refresh_outcome_unknown = false;
        return Err(HttpError::Unauthorized.into());
    }
    state.session_changed = false;

    match state.pending_persist.as_ref() {
        // A prior refresh rotated in memory but could not be persisted. Keep
        // the newest memory snapshot while the known old disk snapshot remains.
        Some(expected_disk) if expected_disk == &disk => {}
        // Another process saved the in-memory value successfully.
        Some(_) if state.tokens == disk => state.pending_persist = None,
        // Same login, different snapshot: another process won the rotation.
        Some(_) => {
            state.tokens = disk.clone();
            state.pending_persist = None;
        }
        None if state.tokens != disk => state.tokens = disk.clone(),
        None => {}
    }
    if state.pending_persist.is_none() && !state.refresh_outcome_unknown {
        state.pending_error_reason = None;
    }
    Ok(disk)
}

fn refresh_persistence_error(error: anyhow::Error) -> anyhow::Error {
    error.context(RefreshPersistencePending)
}

fn retry_pending_persistence_unlocked(
    state: &mut State,
    path: &Path,
    disk: &StoredTokens,
) -> Result<()> {
    if state.pending_persist.as_ref() != Some(disk) || state.tokens == *disk {
        return Ok(());
    }
    match save_unlocked(path, &state.tokens) {
        Ok(()) => {
            state.pending_persist = None;
            state.session_changed = false;
            if !state.refresh_outcome_unknown {
                state.pending_error_reason = None;
            }
            Ok(())
        }
        Err(error) => Err(refresh_persistence_error(error)),
    }
}

/// REST とシェイプにアクセストークンを渡す。同時に期限切れを迎えても、ロックで更新を1回にまとめる。
pub struct TokenManager {
    http: reqwest::Client,
    oidc: OidcConfig,
    path: PathBuf,
    state: Arc<Mutex<State>>,
    task_tracker: Arc<TokenTaskTracker>,
    persist: std::sync::Mutex<bool>,
}

impl TokenManager {
    pub fn new(
        http: reqwest::Client,
        oidc: OidcConfig,
        path: PathBuf,
        tokens: StoredTokens,
    ) -> Self {
        Self {
            http,
            oidc,
            path,
            state: Arc::new(Mutex::new(State {
                tokens,
                discovery: None,
                pending_persist: None,
                session_changed: false,
                pending_error_reason: None,
                refresh_outcome_unknown: false,
            })),
            task_tracker: TokenTaskTracker::new(),
            persist: std::sync::Mutex::new(true),
        }
    }

    async fn persistence_snapshot(&self) -> TokenPersistenceReport {
        let state = self.state.lock().await;
        let status = if state.session_changed {
            TokenPersistenceStatus::SessionChanged
        } else if state.refresh_outcome_unknown {
            TokenPersistenceStatus::Unknown
        } else if state.pending_persist.is_some() {
            TokenPersistenceStatus::Pending
        } else {
            TokenPersistenceStatus::Saved
        };
        TokenPersistenceReport {
            status,
            reason: state.pending_error_reason,
        }
    }

    /// ログアウトする。以後このマネージャーはトークンを返さず、保存もしない。
    /// 保存と同じロックの下で `token.json` を消すので、進行中の更新が後から
    /// 終わっても、ファイルは作り直されない。
    pub async fn sign_out(&self) -> Result<bool> {
        *self.persist.lock().unwrap_or_else(|e| e.into_inner()) = false;
        let expected = self.state.lock().await.tokens.clone();
        let Some(lock) = token_file_lock_async(&self.path, false).await? else {
            return Ok(false);
        };
        let (_lock, cleanup_error) = cleanup_stale_token_temps_async(&self.path, lock).await?;
        let outcome = match delete_current_session_unlocked(&self.path, &self.oidc, &expected) {
            Ok(outcome) => outcome,
            Err(error) => return Err(include_logout_cleanup_error(error, cleanup_error)),
        };
        Ok(matches!(
            report_logout_cleanup_failure(cleanup_error, outcome)?,
            DeleteOutcome::Deleted
        ))
    }

    /// ログアウトする。以後この manager は token を保存せず、保存済みの session の
    /// refresh token は、IdP に失効を要求してから `token.json` を消す。
    /// 失効の要求は期限付きで、IdP に届かなくてもローカルのログアウトは進む。
    pub async fn sign_out_and_revoke(&self) -> Result<bool> {
        *self.persist.lock().unwrap_or_else(|e| e.into_inner()) = false;
        Ok(matches!(
            logout(&self.path, &self.oidc, &self.http).await?,
            DeleteOutcome::Deleted
        ))
    }

    fn signed_out(&self) -> bool {
        !*self.persist.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub async fn access_token(&self) -> Result<String> {
        Ok(self.access_token_with_expiry().await?.0)
    }

    pub async fn access_token_with_expiry(&self) -> Result<(String, i64)> {
        if self.signed_out() {
            return Err(HttpError::Unauthorized.into());
        }
        // The caller may have its own request timeout (for example, the notes
        // client). Detaching this owned task means cancelling that caller does
        // not cancel an in-flight refresh after the IdP may already have
        // rotated the refresh token. The task retains the state and file locks
        // until it saves the response, so logout/account replacement serialize
        // with the completed rotation.
        let http = self.http.clone();
        let oidc = self.oidc.clone();
        let path = self.path.clone();
        let state = self.state.clone();
        let operation_state = state.clone();
        let Some(registration) = self.task_tracker.register() else {
            return Err(TokenManagerShuttingDown.into());
        };
        let result = match tokio::spawn(async move {
            let result = Self::access_token_owned(http, oidc, path, operation_state.clone()).await;
            if let Err(error) = &result {
                let mut state = operation_state.lock().await;
                if state.pending_persist.is_some() || state.refresh_outcome_unknown {
                    state.pending_error_reason = Some(refresh_persistence_retry_reason(error));
                }
            }
            drop(registration);
            result
        })
        .await
        {
            Ok(result) => result,
            Err(error) => {
                Err(anyhow::Error::new(error).context("トークン更新処理が中断されました"))
            }
        };
        if self.signed_out() {
            return Err(HttpError::Unauthorized.into());
        }
        match result {
            Err(error) => {
                let pending = state.lock().await.pending_persist.is_some();
                if pending && !is_refresh_persistence_pending(&error) {
                    Err(refresh_persistence_error(error))
                } else {
                    Err(error)
                }
            }
            result => result,
        }
    }

    async fn access_token_owned(
        http: reqwest::Client,
        oidc: OidcConfig,
        path: PathBuf,
        state: Arc<Mutex<State>>,
    ) -> Result<(String, i64)> {
        let mut state = state.lock().await;

        // Verify the on-disk session before returning even a still-fresh bearer.
        // This is what stops an old manager after logout or account replacement.
        let Some(file_lock) = token_file_lock_async(&path, false).await? else {
            state.pending_persist = None;
            state.session_changed = true;
            state.pending_error_reason = None;
            state.refresh_outcome_unknown = false;
            return Err(HttpError::Unauthorized.into());
        };
        let disk = load_unlocked(&path, &oidc, None)?;
        let disk = reconcile_disk_snapshot(&mut state, disk)?;
        if state.tokens.expires_at - REFRESH_SKEW_SECONDS > now() {
            retry_pending_persistence_unlocked(&mut state, &path, &disk)?;
            return Ok((state.tokens.access_token.clone(), state.tokens.expires_at));
        }
        drop(file_lock);

        if state.discovery.is_none() {
            state.discovery = Some(
                tokio::time::timeout(TOKEN_DISCOVERY_TIMEOUT, discover(&http, &oidc.issuer))
                    .await
                    .context("OIDC の設定取得が時間切れになりました")??,
            );
        }

        // Another process can rotate while discovery is fetched, so lock and
        // reread after discovery as well. Hold the lock across refresh+save.
        let Some(file_lock) = token_file_lock_async(&path, false).await? else {
            state.pending_persist = None;
            state.session_changed = true;
            state.pending_error_reason = None;
            state.refresh_outcome_unknown = false;
            return Err(HttpError::Unauthorized.into());
        };
        let disk = load_unlocked(&path, &oidc, None)?;
        let disk = reconcile_disk_snapshot(&mut state, disk)?;
        if state.tokens.expires_at - REFRESH_SKEW_SECONDS > now() {
            retry_pending_persistence_unlocked(&mut state, &path, &disk)?;
            return Ok((state.tokens.access_token.clone(), state.tokens.expires_at));
        }
        let expected_disk = state
            .pending_persist
            .clone()
            .unwrap_or_else(|| disk.clone());
        let Some(refresh_token) = state.tokens.refresh_token.clone() else {
            return Err(HttpError::Unauthorized.into());
        };
        let discovery = state.discovery.as_ref().expect("直前に設定した");
        let form = vec![
            ("grant_type", "refresh_token".to_string()),
            ("refresh_token", refresh_token.clone()),
        ];
        let result = tokio::time::timeout(
            TOKEN_REFRESH_TIMEOUT,
            token_request(&http, discovery, &oidc, form),
        )
        .await;
        let response = match result {
            Err(_) => {
                state.refresh_outcome_unknown = true;
                return Err(anyhow!("トークン更新が時間切れになりました"));
            }
            Ok(Err(error)) => {
                state.refresh_outcome_unknown = true;
                return Err(error.context("トークン更新の通信に失敗しました"));
            }
            Ok(Ok(TokenResult::Ok(response))) => response,
            // refresh_token やクライアントが使えなくなったときは、再ログインしてもらう。
            Ok(Ok(TokenResult::Err { error, .. })) if needs_login(&error) => {
                return Err(HttpError::Unauthorized.into());
            }
            // server_error などは一時的なものとして、呼び出し側に再試行させる。
            Ok(Ok(TokenResult::Err { error, description })) => {
                state.refresh_outcome_unknown = true;
                return Err(anyhow!(
                    "トークンを更新できませんでした: {}",
                    description.unwrap_or(error)
                ));
            }
        };
        state.refresh_outcome_unknown = false;
        let mut tokens = StoredTokens::new(&oidc, &state.tokens.api_base_url, response, now());
        tokens.session_id = state.tokens.session_id.clone();
        // ローテーションしない IdP では refresh_token が返らないので、前のものを使い続ける。
        tokens.refresh_token = tokens.refresh_token.or(Some(refresh_token));
        // 交換済みの refresh_token は IdP 側で失効しているので、保存に失敗しても新しい方を使い続ける。
        // 次のアクセスではこのスナップショットを照合して保存を再試行し、logout/別login後は書き戻さない。
        state.tokens = tokens;
        match save_unlocked(&path, &state.tokens) {
            Ok(()) => {
                state.pending_persist = None;
                state.session_changed = false;
                state.pending_error_reason = None;
            }
            Err(error) => {
                // Keep the original on-disk comparison point across further
                // in-memory rotations. A later retry may safely replace it.
                state.pending_persist = Some(expected_disk);
                return Err(refresh_persistence_error(error));
            }
        }
        drop(file_lock);
        Ok((state.tokens.access_token.clone(), state.tokens.expires_at))
    }
}

/// 同じ refresh_token で再試行しても回復しないエラー (RFC 6749 5.2)。
fn needs_login(error: &str) -> bool {
    matches!(
        error,
        "invalid_grant" | "invalid_client" | "unauthorized_client" | "unsupported_grant_type"
    )
}

impl BearerSource for TokenManager {
    fn bearer(&self) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>> {
        Box::pin(self.access_token())
    }

    fn persistence_report(
        &self,
    ) -> Pin<Box<dyn Future<Output = TokenPersistenceReport> + Send + '_>> {
        Box::pin(self.persistence_snapshot())
    }

    fn finish_token_tasks(
        &self,
    ) -> Pin<Box<dyn Future<Output = TokenPersistenceReport> + Send + '_>> {
        Box::pin(async move {
            self.task_tracker.shutdown_and_wait().await;
            self.persistence_snapshot().await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        process::{Command, Stdio},
        sync::Arc,
        time::Duration,
    };

    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, method, path},
    };

    use super::*;

    const API: &str = "https://api.example";

    fn oidc(issuer: &str) -> OidcConfig {
        OidcConfig {
            issuer: issuer.into(),
            client_id: "zoo2-tui".into(),
            resource: Some("https://zli.chat".into()),
            scope: "openid profile offline_access".into(),
        }
    }

    fn tokens(oidc: &OidcConfig, expires_at: i64, refresh: Option<&str>) -> StoredTokens {
        StoredTokens {
            session_id: "test-session".into(),
            issuer: oidc.issuer.clone(),
            client_id: oidc.client_id.clone(),
            resource: oidc.resource.clone(),
            api_base_url: API.into(),
            access_token: "old".into(),
            refresh_token: refresh.map(str::to_string),
            expires_at,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("zoo2-tui-test-{}-{name}", uuid::Uuid::new_v4()));
        token_path(&dir)
    }

    fn token_temp_artifacts(path: &Path) -> Vec<PathBuf> {
        let parent = path.parent().unwrap();
        let base = path.file_name().unwrap().to_string_lossy();
        let unique_prefix = format!("{base}.tmp-");
        let legacy_temp = path.with_extension("json.tmp");
        std::fs::read_dir(parent)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| {
                candidate == &legacy_temp
                    || candidate
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with(&unique_prefix))
            })
            .collect()
    }

    #[cfg(unix)]
    struct ReadOnlyDirectory {
        path: PathBuf,
        original_mode: u32,
    }

    #[cfg(unix)]
    impl ReadOnlyDirectory {
        fn new(path: &Path) -> Self {
            use std::os::unix::fs::PermissionsExt;

            let original_mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o7777;
            std::fs::set_permissions(
                path,
                std::fs::Permissions::from_mode(original_mode & !0o222),
            )
            .unwrap();
            Self {
                path: path.to_path_buf(),
                original_mode,
            }
        }

        fn allow_writes(&self) {
            use std::os::unix::fs::PermissionsExt;

            std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(self.original_mode),
            )
            .unwrap();
        }
    }

    #[cfg(unix)]
    impl Drop for ReadOnlyDirectory {
        fn drop(&mut self) {
            use std::os::unix::fs::PermissionsExt;

            let _ = std::fs::set_permissions(
                &self.path,
                std::fs::Permissions::from_mode(self.original_mode),
            );
        }
    }

    async fn issuer(server: &MockServer) -> String {
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer, "token_endpoint": format!("{}/api/auth/oauth2/token", server.uri())
            })))
            .mount(server)
            .await;
        issuer
    }

    #[test]
    fn saves_privately_and_ignores_tokens_of_other_clients() {
        let path = temp_path("save");
        let config = oidc("https://issuer");
        save(&path, &tokens(&config, 100, Some("rt"))).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let lock_mode = std::fs::metadata(path.with_extension("json.lock"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(lock_mode & 0o777, 0o600);
        }
        assert_eq!(
            load(&path, &config, Some(API))
                .unwrap()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("rt")
        );
        let other = OidcConfig {
            client_id: "other".into(),
            ..config.clone()
        };
        assert_eq!(load(&path, &other, Some(API)).unwrap(), None);
        assert!(delete(&path).unwrap());
        assert!(!delete(&path).unwrap());
        assert_eq!(load(&path, &config, Some(API)).unwrap(), None);
    }

    #[test]
    fn assigns_and_persists_a_session_id_for_legacy_token_files() {
        let path = temp_path("legacy-session-id");
        let config = oidc("https://issuer");
        let initial = tokens(&config, 100, Some("rt"));
        save(&path, &initial).unwrap();
        let mut legacy = serde_json::to_value(&initial).unwrap();
        legacy.as_object_mut().unwrap().remove("session_id");
        std::fs::write(&path, legacy.to_string()).unwrap();

        let migrated = load(&path, &config, Some(API)).unwrap().unwrap();
        assert!(!migrated.session_id.is_empty());
        assert_eq!(
            load(&path, &config, Some(API)).unwrap().unwrap().session_id,
            migrated.session_id
        );
        delete(&path).unwrap();
    }

    #[test]
    fn save_uses_a_private_unique_temp_and_leaves_legacy_temp_untouched() {
        let path = temp_path("existing-temp-permissions");
        let config = oidc("https://issuer");
        let temporary = path.with_extension("json.tmp");
        save(&path, &tokens(&config, 100, Some("rt"))).unwrap();
        std::fs::write(&temporary, "old temporary content").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644)).unwrap();
            std::fs::set_permissions(
                path.with_extension("json.lock"),
                std::fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }

        save(&path, &tokens(&config, 200, Some("rotated"))).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            assert_eq!(
                std::fs::metadata(&temporary).unwrap().permissions().mode() & 0o777,
                0o644,
                "the current writer must not change an old binary's temp file"
            );
            let lock_mode = std::fs::metadata(path.with_extension("json.lock"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(lock_mode & 0o777, 0o600);
        }
        assert_eq!(
            std::fs::read_to_string(&temporary).unwrap(),
            "old temporary content"
        );
        assert_eq!(token_temp_artifacts(&path), vec![temporary.clone()]);
        std::fs::remove_file(temporary).unwrap();
        delete(&path).unwrap();
    }

    #[test]
    fn save_removes_its_temp_and_reports_atomic_rename_failure() {
        let path = temp_path("atomic-rename-failure");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("sentinel"), "keep").unwrap();

        let error =
            save_unlocked(&path, &tokens(&oidc("https://issuer"), 100, Some("rt"))).unwrap_err();

        assert!(
            format!("{error:#}").contains(&path.display().to_string()),
            "rename error should identify the destination: {error:#}"
        );
        assert!(
            token_temp_artifacts(&path).is_empty(),
            "failed rename must clean the token temp"
        );
        assert_eq!(
            std::fs::read_to_string(path.join("sentinel")).unwrap(),
            "keep"
        );
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn write_sync_and_cleanup_failures_remove_temp_secrets_and_report_causes() {
        let path = temp_path("temp-write-failures");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let secret = b"rotated-refresh-secret";

        let write_temp = unique_temporary_path(&path);
        let write_error = write_atomic_file(
            &path,
            &write_temp,
            secret,
            |file, contents| {
                std::io::Write::write_all(file, &contents[..5])?;
                Err(std::io::Error::other("injected write failure"))
            },
            |file| file.sync_all(),
        )
        .unwrap_err();
        assert!(format!("{write_error:#}").contains("書き込めません"));
        assert!(!format!("{write_error:#}").contains("rotated-refresh-secret"));
        assert!(!write_temp.exists(), "partial write temp must be removed");

        let sync_temp = unique_temporary_path(&path);
        let sync_error =
            write_atomic_file(&path, &sync_temp, secret, std::io::Write::write_all, |_| {
                Err(std::io::Error::other("injected sync failure"))
            })
            .unwrap_err();
        assert!(format!("{sync_error:#}").contains("同期できません"));
        assert!(!format!("{sync_error:#}").contains("rotated-refresh-secret"));
        assert!(!sync_temp.exists(), "failed sync temp must be removed");

        let cleanup_temp = unique_temporary_path(&path);
        let path_to_replace = cleanup_temp.clone();
        let cleanup_error = write_atomic_file(
            &path,
            &cleanup_temp,
            secret,
            move |file, contents| {
                std::io::Write::write_all(file, contents)?;
                std::fs::remove_file(&path_to_replace)?;
                std::fs::create_dir(&path_to_replace)?;
                Err(std::io::Error::other("injected write failure"))
            },
            |file| file.sync_all(),
        )
        .unwrap_err();
        let report = format!("{cleanup_error:#}");
        assert!(report.contains("後始末にも失敗しました"), "{report}");
        assert!(cleanup_temp.is_dir());
        std::fs::remove_dir(cleanup_temp).unwrap();
        assert!(token_temp_artifacts(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn exclusive_temp_creation_refuses_a_symlink_without_following_it() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let path = temp_path("exclusive-temp-symlink");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let external = path.parent().unwrap().join("exclusive-temp-target");
        let temporary = path.with_file_name("token.json.tmp-fixed");
        std::fs::write(&external, "keep target").unwrap();
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&external, &temporary).unwrap();

        assert!(open_new_temp_file(&temporary).is_err());
        assert!(
            std::fs::symlink_metadata(&temporary)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&external).unwrap(), "keep target");
        assert_eq!(
            std::fs::metadata(&external).unwrap().permissions().mode() & 0o777,
            0o644
        );
        std::fs::remove_file(temporary).unwrap();
        std::fs::remove_file(external).unwrap();
    }

    #[tokio::test]
    async fn old_unlocked_save_can_resurrect_token_after_logout() {
        const WORKER_PATH: &str = "ZOO2_TUI_LEGACY_SAVER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_LEGACY_SAVER_READY").unwrap());
            let start = PathBuf::from(std::env::var_os("ZOO2_TUI_LEGACY_SAVER_START").unwrap());
            let config = oidc("https://issuer");
            let mut legacy = tokens(&config, now() + 900, None);
            legacy.session_id = "old-binary-session".into();
            let bytes = serde_json::to_vec_pretty(&legacy).unwrap();
            let split = bytes.len() / 2;
            let temporary = path.with_extension("json.tmp");
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&temporary).unwrap();
            std::io::Write::write_all(&mut file, &bytes[..split]).unwrap();
            file.sync_all().unwrap();
            let ready_temp = ready.with_extension("ready.tmp");
            std::fs::write(&ready_temp, &bytes[..split]).unwrap();
            std::fs::rename(&ready_temp, &ready).unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            std::io::Write::write_all(&mut file, &bytes[split..]).unwrap();
            file.sync_all().unwrap();
            drop(file);
            std::fs::rename(&temporary, &path).unwrap();
            return;
        }

        let config = oidc("https://issuer");
        let path = temp_path("legacy-saver-race");
        let mut current = tokens(&config, now() + 900, None);
        current.session_id = "new-binary-session".into();
        save(&path, &current).unwrap();
        let parent = path.parent().unwrap();
        let ready = parent.join("legacy-saver.ready");
        let start = parent.join("legacy-saver.start");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("auth::token::tests::old_unlocked_save_can_resurrect_token_after_logout")
            .arg("--nocapture")
            .env(WORKER_PATH, &path)
            .env("ZOO2_TUI_LEGACY_SAVER_READY", &ready)
            .env("ZOO2_TUI_LEGACY_SAVER_START", &start)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let partial = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if ready.exists() {
                    break std::fs::read(&ready).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("old saver opened and partially wrote its fixed temp");

        let new_save = tokens(&config, now() + 900, None);
        let mut new_save = new_save;
        new_save.session_id = "new-binary-session".into();
        save(&path, &new_save).unwrap();
        let intermediate = load(&path, &config, Some(API)).unwrap().unwrap();
        let legacy_temp_preserved =
            std::fs::read(path.with_extension("json.tmp")).ok() == Some(partial);
        assert_eq!(intermediate.session_id, "new-binary-session");
        assert!(
            legacy_temp_preserved,
            "the new save must preserve the old temp"
        );
        assert_eq!(
            logout(&path, &config, &reqwest::Client::new())
                .await
                .unwrap(),
            DeleteOutcome::Deleted
        );
        assert!(!path.exists(), "logout removes the current token file");
        std::fs::write(&start, "finish").unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "old saver exited with {status}");

        assert!(
            legacy_temp_preserved,
            "unique temp save must not truncate or unlink the old fixed temp"
        );
        let final_tokens = load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(
            final_tokens.session_id, "old-binary-session",
            "the old no-lock writer can recreate the file after logout"
        );
        assert_eq!(final_tokens.refresh_token, None);
        assert!(token_temp_artifacts(&path).is_empty());
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn killed_writer_temp_is_removed_on_restart_before_logout() {
        const WORKER_PATH: &str = "ZOO2_TUI_KILLED_TEMP_WORKER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_KILLED_TEMP_READY").unwrap());
            let _lock = token_file_lock(&path, false).unwrap().unwrap();
            let temporary = unique_temporary_path(&path);
            let mut file = open_new_temp_file(&temporary).unwrap();
            let mut interrupted = tokens(
                &oidc("https://issuer"),
                now() + 900,
                Some("secret-in-killed-temp"),
            );
            interrupted.access_token = "access-in-killed-temp".into();
            std::io::Write::write_all(&mut file, &serde_json::to_vec_pretty(&interrupted).unwrap())
                .unwrap();
            file.sync_all().unwrap();
            let ready_temp = ready.with_extension("ready.tmp");
            std::fs::write(&ready_temp, temporary.to_string_lossy().as_bytes()).unwrap();
            std::fs::rename(&ready_temp, ready).unwrap();
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }

        let config = oidc("https://issuer");
        let path = temp_path("killed-temp-restart-logout");
        let initial = tokens(&config, now() + 900, None);
        save(&path, &initial).unwrap();
        let ready = path.parent().unwrap().join("killed-temp.ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("auth::token::tests::killed_writer_temp_is_removed_on_restart_before_logout")
            .arg("--nocapture")
            .env(WORKER_PATH, &path)
            .env("ZOO2_TUI_KILLED_TEMP_READY", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let temporary = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if ready.exists() {
                    break PathBuf::from(std::fs::read_to_string(&ready).unwrap());
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child wrote and synced a private token temp");
        assert!(
            std::fs::read_to_string(&temporary)
                .unwrap()
                .contains("secret-in-killed-temp")
        );

        child.kill().unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(!status.success(), "worker must be killed before rename");

        let loaded = load_async(&path, &config, Some(API))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.access_token, "old");
        assert_eq!(loaded.session_id, initial.session_id);
        assert!(
            !temporary.exists(),
            "startup cleanup must remove token data left by a killed writer"
        );

        let logout_temp = unique_temporary_path(&path);
        let mut file = open_new_temp_file(&logout_temp).unwrap();
        std::io::Write::write_all(&mut file, b"another stale token temp").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(
            logout(&path, &config, &reqwest::Client::new())
                .await
                .unwrap(),
            DeleteOutcome::Deleted
        );
        assert!(!path.exists());
        assert!(
            !logout_temp.exists(),
            "logout also collects stale token temps"
        );
        assert!(token_temp_artifacts(&path).is_empty());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn startup_temp_cleanup_ignores_symlinks_directories_and_unrelated_files() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let config = oidc("https://issuer");
        let path = temp_path("temp-cleanup-targeting");
        let initial = tokens(&config, now() + 900, None);
        save(&path, &initial).unwrap();

        let stale = unique_temporary_path(&path);
        let mut stale_file = open_new_temp_file(&stale).unwrap();
        std::io::Write::write_all(&mut stale_file, b"private stale token temp").unwrap();
        stale_file.sync_all().unwrap();
        drop(stale_file);

        let parent = path.parent().unwrap();
        let symlink_target = parent.join("cleanup-symlink-target");
        std::fs::write(&symlink_target, "keep external target").unwrap();
        let symlink_path =
            path.with_file_name(format!("token.json.tmp-{}", uuid::Uuid::new_v4().simple()));
        symlink(&symlink_target, &symlink_path).unwrap();

        let directory_path =
            path.with_file_name(format!("token.json.tmp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&directory_path).unwrap();

        let malformed_path = path.with_extension("json.tmp-not-a-uuid");
        std::fs::write(&malformed_path, "keep unrelated file").unwrap();

        let loaded = load_async(&path, &config, Some(API))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.session_id, initial.session_id);
        assert!(!stale.exists(), "private UUID temp should be collected");
        assert!(
            std::fs::symlink_metadata(&symlink_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::read_to_string(&symlink_target).unwrap(),
            "keep external target"
        );
        assert!(directory_path.is_dir());
        assert!(malformed_path.exists());

        let public_file_path =
            path.with_file_name(format!("token.json.tmp-{}", uuid::Uuid::new_v4().simple()));
        std::fs::write(&public_file_path, "not an owned private temp").unwrap();
        std::fs::set_permissions(&public_file_path, std::fs::Permissions::from_mode(0o644))
            .unwrap();
        let error = load_async(&path, &config, Some(API)).await.unwrap_err();
        assert!(format!("{error:#}").contains("条件を満たさない"));
        assert!(
            public_file_path.exists(),
            "non-private matching files are not deleted"
        );

        std::fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn stale_temp_cleanup_reports_permission_denial_and_leaves_file() {
        let path = temp_path("temp-cleanup-denied");
        let config = oidc("https://issuer");
        save(&path, &tokens(&config, now() + 900, None)).unwrap();
        let stale = unique_temporary_path(&path);
        let mut file = open_new_temp_file(&stale).unwrap();
        std::io::Write::write_all(&mut file, b"private token data").unwrap();
        file.sync_all().unwrap();
        drop(file);
        let removable = unique_temporary_path(&path);
        let mut file = open_new_temp_file(&removable).unwrap();
        std::io::Write::write_all(&mut file, b"another private token temp").unwrap();
        file.sync_all().unwrap();
        drop(file);

        let _lock = token_file_lock(&path, false).unwrap().unwrap();
        let denied = stale.clone();
        let error = cleanup_stale_token_temps_unlocked_with(&path, |candidate| {
            if candidate == denied {
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected unlink denial",
                ))
            } else {
                std::fs::remove_file(candidate)
            }
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("削除できません"));
        assert!(stale.exists(), "denied unlink must leave the file in place");
        assert!(!removable.exists(), "other stale temps are still collected");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn sign_out_removes_token_when_stale_temp_is_unsafe() {
        use std::os::unix::fs::PermissionsExt;
        let config = oidc("https://issuer");
        let path = temp_path("signout-stale-temp");
        let initial = tokens(&config, now() + 900, Some("refresh"));
        save(&path, &initial).unwrap();
        let stale = unique_temporary_path(&path);
        std::fs::write(&stale, b"stale").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        let manager = TokenManager::new(reqwest::Client::new(), config, path.clone(), initial);
        let error = manager.sign_out().await.unwrap_err().to_string();
        assert!(
            !path.exists(),
            "token must be removed despite cleanup failure: {error}"
        );
        assert!(stale.exists());
        assert!(error.contains("Deleted"), "{error}");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn delete_unchanged_reports_cleanup_failure_after_removing_token() {
        use std::os::unix::fs::PermissionsExt;
        let config = oidc("https://issuer");
        let path = temp_path("delete-unchanged-stale-temp");
        let initial = tokens(&config, now() + 900, None);
        save(&path, &initial).unwrap();
        let stale = unique_temporary_path(&path);
        std::fs::write(&stale, b"stale").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = delete_if_unchanged(&path, Some(&initial))
            .unwrap_err()
            .to_string();
        assert!(
            !path.exists(),
            "token must be removed despite cleanup failure: {error}"
        );
        assert!(error.contains("Deleted"), "{error}");
        assert!(stale.exists());
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn delete_unchanged_preserves_replacement_when_cleanup_fails() {
        use std::os::unix::fs::PermissionsExt;
        let config = oidc("https://issuer");
        let path = temp_path("delete-replaced-stale-temp");
        let original = tokens(&config, now() + 900, None);
        let mut replacement = original.clone();
        replacement.session_id = uuid::Uuid::new_v4().to_string();
        save(&path, &replacement).unwrap();
        let stale = unique_temporary_path(&path);
        std::fs::write(&stale, b"stale").unwrap();
        std::fs::set_permissions(&stale, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = delete_if_unchanged(&path, Some(&original))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Changed"), "{error}");
        assert_eq!(read_tokens_unlocked(&path).unwrap(), Some(replacement));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[tokio::test]
    async fn logout_deletes_persisted_session_but_reports_an_unremovable_stale_temp() {
        let config = oidc("https://issuer");
        let path = temp_path("logout-stale-temp-denied");
        let initial = tokens(&config, now() + 900, None);
        save(&path, &initial).unwrap();
        let stale = unique_temporary_path(&path);
        let mut file = open_new_temp_file(&stale).unwrap();
        std::io::Write::write_all(&mut file, b"private stale token").unwrap();
        file.sync_all().unwrap();
        drop(file);

        let expected = read_tokens_unlocked(&path).unwrap();
        let lock = token_file_lock_async(&path, false).await.unwrap().unwrap();
        let error = logout_with_cleanup_result(
            &path,
            &config,
            &reqwest::Client::new(),
            expected,
            lock,
            Some(anyhow!("injected permission denied")),
        )
        .await
        .unwrap_err();
        let report = format!("{error:#}");
        assert!(report.contains("Deleted"), "{report}");
        assert!(report.contains("古い token 一時ファイルを削除できませんでした"));
        assert!(!path.exists(), "logout still removes the persisted session");
        assert!(stale.exists(), "the denied stale temp remains on disk");
        std::fs::remove_file(stale).unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn save_does_not_follow_a_temporary_path_symlink() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let path = temp_path("temporary-symlink");
        let config = oidc("https://issuer");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let external = parent.join("external-file");
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&external, "leave this file alone").unwrap();
        std::fs::set_permissions(&external, std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&external, &temporary).unwrap();

        save(&path, &tokens(&config, 100, Some("rt"))).unwrap();

        assert_eq!(
            std::fs::read_to_string(&external).unwrap(),
            "leave this file alone"
        );
        assert_eq!(
            std::fs::metadata(&external).unwrap().permissions().mode() & 0o777,
            0o644
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(
            std::fs::symlink_metadata(&temporary)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        delete(&path).unwrap();
        std::fs::remove_file(temporary).unwrap();
        std::fs::remove_file(external).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn save_refuses_a_lock_path_symlink_without_changing_its_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let path = temp_path("lock-symlink");
        let config = oidc("https://issuer");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let target = parent.join("external-lock-target");
        std::fs::write(&target, "leave lock target alone").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, path.with_extension("json.lock")).unwrap();

        assert!(save(&path, &tokens(&config, 100, Some("rt"))).is_err());
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "leave lock target alone"
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
        std::fs::remove_file(target).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn load_refuses_a_token_path_symlink_without_reading_its_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};

        let path = temp_path("token-symlink");
        let config = oidc("https://issuer");
        let parent = path.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        let target = parent.join("external-token-target");
        std::fs::write(
            &target,
            serde_json::to_vec(&tokens(&config, 100, Some("secret-refresh"))).unwrap(),
        )
        .unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, &path).unwrap();

        assert!(load(&path, &config, Some(API)).is_err());
        assert!(
            std::fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o644
        );
        delete(&path).unwrap();
        std::fs::remove_file(target).unwrap();
    }

    #[test]
    fn does_not_hand_tokens_to_another_api() {
        let path = temp_path("api");
        let config = oidc("https://issuer");
        save(&path, &tokens(&config, 100, Some("rt"))).unwrap();
        assert_eq!(
            load(&path, &config, Some("https://other.example")).unwrap(),
            None
        );
        // refresh_token の失効のように、API に送らない用途では URL を問わない。
        assert!(load(&path, &config, None).unwrap().is_some());

        // api_base_url を持たない古いファイルは、どの接続先とも一致しない。
        let mut old = serde_json::to_value(tokens(&config, 100, Some("rt"))).unwrap();
        old.as_object_mut().unwrap().remove("api_base_url");
        std::fs::write(&path, old.to_string()).unwrap();
        assert_eq!(load(&path, &config, Some(API)).unwrap(), None);
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn token_and_expiry_are_returned_from_one_snapshot() {
        let config = oidc("http://127.0.0.1:9");
        let path = temp_path("token-expiry-snapshot");
        let initial = tokens(&config, now() + 600, None);
        let expected = (initial.access_token.clone(), initial.expires_at);
        save(&path, &initial).unwrap();
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );
        let snapshot = TokenManager::access_token_owned(
            reqwest::Client::new(),
            config,
            path.clone(),
            manager.state.clone(),
        )
        .await
        .unwrap();
        {
            let mut state = manager.state.lock().await;
            state.tokens.access_token = "later".into();
            state.tokens.expires_at += 3600;
        }
        assert_eq!(snapshot, expected);
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn returns_fresh_token_without_network() {
        let config = oidc("http://127.0.0.1:9");
        let path = temp_path("fresh");
        let initial = tokens(&config, now() + 600, None);
        save(&path, &initial).unwrap();
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );
        assert_eq!(manager.access_token().await.unwrap(), "old");
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn refreshes_once_near_expiry_and_keeps_refresh_token() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("refresh_token=rt"))
            .and(body_string_contains("client_id=zoo2-tui"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "new", "expires_in": 300 })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let path = temp_path("refresh");
        let manager = TokenManager::new(reqwest::Client::new(), config.clone(), path.clone(), {
            let initial = tokens(&config, now() + 30, Some("rt"));
            save(&path, &initial).unwrap();
            initial
        });
        let (a, b) = tokio::join!(manager.access_token(), manager.access_token());
        assert_eq!(
            (a.unwrap(), b.unwrap()),
            ("new".to_string(), "new".to_string())
        );
        let saved = load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(saved.access_token, "new");
        assert_eq!(saved.refresh_token.as_deref(), Some("rt"));
        delete(&path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn retries_persisting_rotated_refresh_token_after_transient_save_failure() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=rt"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "new", "refresh_token": "rotated", "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;

        let path = temp_path("retry-rotation-save");
        let initial = tokens(&config, now() + 30, Some("rt"));
        save(&path, &initial).unwrap();
        let readonly_parent = ReadOnlyDirectory::new(path.parent().unwrap());
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );

        let error = manager.access_token().await.unwrap_err();
        assert!(error.to_string().contains("別 process の logout"));
        assert_eq!(
            load(&path, &config, Some(API))
                .unwrap()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("rt")
        );
        let (during_failure_a, during_failure_b) =
            tokio::join!(manager.access_token(), manager.access_token());
        assert!(
            during_failure_a
                .unwrap_err()
                .to_string()
                .contains("別 process の logout")
        );
        assert!(
            during_failure_b
                .unwrap_err()
                .to_string()
                .contains("別 process の logout")
        );
        assert!(token_temp_artifacts(&path).is_empty());
        readonly_parent.allow_writes();
        let (retry_a, retry_b) = tokio::join!(manager.access_token(), manager.access_token());
        assert_eq!(retry_a.unwrap(), "new");
        assert_eq!(retry_b.unwrap(), "new");

        let saved = load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(saved.access_token, "new");
        assert_eq!(saved.refresh_token.as_deref(), Some("rotated"));
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn caller_cancellation_after_refresh_post_still_persists_before_logout() {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/token", server.uri()),
                "revocation_endpoint": format!("{}/revoke", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=old-refresh"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(700))
                    .set_body_json(json!({
                        "access_token": "rotated-access",
                        "refresh_token": "rotated-refresh",
                        "expires_in": 300
                    })),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=rotated-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = temp_path("cancelled-refresh-persists");
        let initial = tokens(&config, now() + 30, Some("old-refresh"));
        save(&path, &initial).unwrap();
        let manager = std::sync::Arc::new(TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial.clone(),
        ));
        let caller = tokio::spawn({
            let manager = manager.clone();
            async move { manager.access_token().await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|request| request.url.path() == "/token")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("refresh POST has reached the controlled endpoint");
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());

        assert_eq!(
            logout(&path, &config, &reqwest::Client::new())
                .await
                .unwrap(),
            DeleteOutcome::Deleted
        );
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().any(|request| {
            request.method == "POST"
                && request.url.path() == "/revoke"
                && String::from_utf8_lossy(&request.body).contains("token=rotated-refresh")
        }));
        assert_eq!(load(&path, &config, Some(API)).unwrap(), None);
    }

    #[tokio::test]
    async fn logout_does_not_remove_a_new_session_that_wins_while_logout_waits_for_lock() {
        const WORKER_PATH: &str = "ZOO2_TUI_LOGOUT_REPLACEMENT_WORKER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let ready = PathBuf::from(
                std::env::var_os("ZOO2_TUI_LOGOUT_REPLACEMENT_WORKER_READY").unwrap(),
            );
            let start = PathBuf::from(
                std::env::var_os("ZOO2_TUI_LOGOUT_REPLACEMENT_WORKER_START").unwrap(),
            );
            let _lock = token_file_lock(&path, false).unwrap().unwrap();
            std::fs::write(&ready, "locked").unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let mut replacement = read_tokens_unlocked(&path).unwrap().unwrap();
            replacement.session_id = uuid::Uuid::new_v4().to_string();
            replacement.access_token = "replacement-session-access".into();
            replacement.refresh_token = Some("replacement-session-refresh".into());
            save_unlocked(&path, &replacement).unwrap();
            return;
        }

        let config = oidc("https://issuer");
        let path = temp_path("logout-waits-for-session-replacement");
        save(&path, &tokens(&config, now() + 600, None)).unwrap();
        let parent = path.parent().unwrap();
        let ready = parent.join("replacement-worker.ready");
        let start = parent.join("replacement-worker.start");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(
                "auth::token::tests::logout_does_not_remove_a_new_session_that_wins_while_logout_waits_for_lock",
            )
            .arg("--nocapture")
            .env(WORKER_PATH, &path)
            .env("ZOO2_TUI_LOGOUT_REPLACEMENT_WORKER_READY", &ready)
            .env("ZOO2_TUI_LOGOUT_REPLACEMENT_WORKER_START", &start)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("replacement process holds the session lock");

        let future_path = path.clone();
        let future_config = config.clone();
        let mut future =
            Box::pin(
                async move { logout(&future_path, &future_config, &reqwest::Client::new()).await },
            );
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(matches!(
            future.as_mut().poll(&mut context),
            std::task::Poll::Pending
        ));
        std::fs::write(&start, "replace").unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "replacement worker exited with {status}");

        assert_eq!(
            tokio::spawn(future).await.unwrap().unwrap(),
            DeleteOutcome::Changed
        );
        let retained = load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(retained.access_token, "replacement-session-access");
        assert_eq!(
            retained.refresh_token.as_deref(),
            Some("replacement-session-refresh")
        );
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn legacy_logout_recognizes_migration_refresh_but_preserves_replacements() {
        const WORKER_PATH: &str = "ZOO2_TUI_LEGACY_LOGOUT_WORKER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let mode = std::env::var("ZOO2_TUI_LEGACY_LOGOUT_WORKER_MODE").unwrap();
            let issuer = std::env::var("ZOO2_TUI_LEGACY_LOGOUT_WORKER_ISSUER").unwrap();
            let config = oidc(&issuer);
            let ready =
                PathBuf::from(std::env::var_os("ZOO2_TUI_LEGACY_LOGOUT_WORKER_READY").unwrap());
            let start =
                PathBuf::from(std::env::var_os("ZOO2_TUI_LEGACY_LOGOUT_WORKER_START").unwrap());
            let _lock = token_file_lock(&path, false).unwrap().unwrap();
            std::fs::write(&ready, "locked").unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            match mode.as_str() {
                "migrate-and-refresh" => {
                    let migrated = load_and_migrate_unlocked(&path, &config, None)
                        .unwrap()
                        .unwrap();
                    let http = reqwest::Client::new();
                    let discovery = discover(&http, &config.issuer).await.unwrap();
                    let response = token_request(
                        &http,
                        &discovery,
                        &config,
                        vec![
                            ("grant_type", "refresh_token".into()),
                            ("refresh_token", migrated.refresh_token.clone().unwrap()),
                        ],
                    )
                    .await
                    .unwrap();
                    let TokenResult::Ok(response) = response else {
                        panic!("controlled token endpoint must refresh the legacy session");
                    };
                    let mut refreshed =
                        StoredTokens::new(&config, &migrated.api_base_url, response, now());
                    refreshed.session_id = migrated.session_id;
                    save_unlocked(&path, &refreshed).unwrap();
                }
                "same-token-new-login" => {
                    let mut replacement = read_tokens_unlocked(&path).unwrap().unwrap();
                    replacement.session_id = uuid::Uuid::new_v4().to_string();
                    save_unlocked(&path, &replacement).unwrap();
                }
                "different-account" => {
                    let mut replacement = read_tokens_unlocked(&path).unwrap().unwrap();
                    replacement.session_id = uuid::Uuid::new_v4().to_string();
                    replacement.client_id = "replacement-account-client".into();
                    replacement.access_token = "replacement-account-access".into();
                    replacement.refresh_token = Some("replacement-account-refresh".into());
                    save_unlocked(&path, &replacement).unwrap();
                }
                other => panic!("unknown test mode: {other}"),
            }
            return;
        }

        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/token", server.uri()),
                "revocation_endpoint": format!("{}/revoke", server.uri())
            })))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .and(body_string_contains("refresh_token=legacy-refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "refreshed-access",
                "refresh_token": "refreshed-refresh",
                "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=refreshed-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        for mode in [
            "migrate-and-refresh",
            "same-token-new-login",
            "different-account",
        ] {
            let config = oidc(&issuer);
            let path = temp_path(&format!("legacy-logout-{mode}"));
            let initial = tokens(&config, now() + 300, Some("legacy-refresh"));
            save(&path, &initial).unwrap();
            let mut legacy = serde_json::to_value(&initial).unwrap();
            legacy.as_object_mut().unwrap().remove("session_id");
            std::fs::write(&path, legacy.to_string()).unwrap();

            let parent = path.parent().unwrap();
            let ready = parent.join("legacy-worker.ready");
            let start = parent.join("legacy-worker.start");
            let mut child = Command::new(std::env::current_exe().unwrap())
                .arg("--exact")
                .arg(
                    "auth::token::tests::legacy_logout_recognizes_migration_refresh_but_preserves_replacements",
                )
                .arg("--nocapture")
                .env(WORKER_PATH, &path)
                .env("ZOO2_TUI_LEGACY_LOGOUT_WORKER_MODE", mode)
                .env("ZOO2_TUI_LEGACY_LOGOUT_WORKER_ISSUER", &config.issuer)
                .env("ZOO2_TUI_LEGACY_LOGOUT_WORKER_READY", &ready)
                .env("ZOO2_TUI_LEGACY_LOGOUT_WORKER_START", &start)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !ready.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("legacy worker holds the token lock");

            let future_path = path.clone();
            let future_config = config.clone();
            let mut future = Box::pin(async move {
                logout(&future_path, &future_config, &reqwest::Client::new()).await
            });
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(matches!(
                future.as_mut().poll(&mut context),
                std::task::Poll::Pending
            ));
            std::fs::write(&start, "continue").unwrap();
            let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
                .await
                .unwrap();
            assert!(status.success(), "legacy worker exited with {status}");

            let outcome = tokio::spawn(future).await.unwrap().unwrap();
            match mode {
                "migrate-and-refresh" => {
                    assert_eq!(outcome, DeleteOutcome::Deleted);
                    assert_eq!(load(&path, &config, Some(API)).unwrap(), None);
                    let requests = server.received_requests().await.unwrap();
                    assert!(requests.iter().any(|request| {
                        request.method == "POST"
                            && request.url.path() == "/revoke"
                            && String::from_utf8_lossy(&request.body)
                                .contains("token=refreshed-refresh")
                    }));
                }
                "same-token-new-login" => {
                    assert_eq!(outcome, DeleteOutcome::Changed);
                    let replacement = read_tokens_unlocked(&path).unwrap().unwrap();
                    assert!(!replacement.session_id.is_empty());
                    assert_eq!(replacement.access_token, initial.access_token);
                    assert_eq!(replacement.refresh_token, initial.refresh_token);
                }
                "different-account" => {
                    assert_eq!(outcome, DeleteOutcome::Changed);
                    let replacement = read_tokens_unlocked(&path).unwrap().unwrap();
                    assert_eq!(replacement.client_id, "replacement-account-client");
                    assert_eq!(replacement.access_token, "replacement-account-access");
                }
                _ => unreachable!(),
            }
            if path.exists() {
                delete(&path).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn refresh_discovery_timeout_releases_state_for_a_later_retry() {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        let discovery_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = discovery_count.clone();
        let issuer_for_response = issuer.clone();
        let server_uri = server.uri();
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(move |_: &wiremock::Request| {
                let response = ResponseTemplate::new(200).set_body_json(json!({
                    "issuer": issuer_for_response,
                    "token_endpoint": format!("{server_uri}/token")
                }));
                if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    response.set_delay(Duration::from_secs(7))
                } else {
                    response
                }
            })
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "retried-access",
                "refresh_token": "retried-refresh",
                "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = temp_path("refresh-discovery-timeout");
        let initial = tokens(&config, now() - 1, Some("old-refresh"));
        save(&path, &initial).unwrap();
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );

        let error = manager.access_token().await.unwrap_err();
        assert!(error.to_string().contains("時間切れ"), "{error:#}");
        assert_eq!(manager.access_token().await.unwrap(), "retried-access");
        assert_eq!(
            load(&path, &config, Some(API))
                .unwrap()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("retried-refresh")
        );
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn logout_removes_unchanged_token_from_previous_oidc_configuration() {
        let old_config = oidc("https://old.example");
        let new_config = oidc("https://new.example");
        let path = temp_path("logout-previous-oidc");
        save(
            &path,
            &tokens(&old_config, now() + 300, Some("old-refresh")),
        )
        .unwrap();
        assert_eq!(
            logout(&path, &new_config, &reqwest::Client::new())
                .await
                .unwrap(),
            DeleteOutcome::Deleted
        );
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn logout_revokes_and_removes_legacy_file() {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        let config = oidc(&issuer);
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/token", server.uri()),
                "revocation_endpoint": format!("{}/revoke", server.uri())
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=legacy-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let path = temp_path("legacy-logout-migration-failure");
        let initial = tokens(&config, now() + 300, Some("legacy-refresh"));
        save(&path, &initial).unwrap();
        let mut legacy = serde_json::to_value(&initial).unwrap();
        legacy.as_object_mut().unwrap().remove("session_id");
        std::fs::write(&path, legacy.to_string()).unwrap();
        assert_eq!(
            logout(&path, &config, &reqwest::Client::new())
                .await
                .unwrap(),
            DeleteOutcome::Deleted
        );
        assert!(!path.exists(), "logout must unlink the legacy file itself");
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().any(|request| {
            request.method == "POST"
                && String::from_utf8_lossy(&request.body).contains("token=legacy-refresh")
        }));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_pending_persistence_snapshot_survives_multiple_refreshes() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        for (old_refresh, access, new_refresh) in [
            ("rt", "access-one", "refresh-one"),
            ("refresh-one", "access-two", "refresh-two"),
            ("refresh-two", "access-three", "refresh-three"),
        ] {
            Mock::given(method("POST"))
                .and(path("/api/auth/oauth2/token"))
                .and(body_string_contains(format!("refresh_token={old_refresh}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": access,
                    "refresh_token": new_refresh,
                    "expires_in": 300
                })))
                .expect(1)
                .mount(&server)
                .await;
        }

        let path = temp_path("pending-save-multiple-refreshes");
        let initial = tokens(&config, now() + 30, Some("rt"));
        save(&path, &initial).unwrap();
        let readonly_parent = ReadOnlyDirectory::new(path.parent().unwrap());
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );

        assert!(manager.access_token().await.is_err());
        {
            let mut state = manager.state.lock().await;
            state.tokens.expires_at = now() - 1;
        }
        assert!(manager.access_token().await.is_err());
        assert!(token_temp_artifacts(&path).is_empty());
        readonly_parent.allow_writes();
        assert_eq!(manager.access_token().await.unwrap(), "access-two");
        assert_eq!(
            load(&path, &config, Some(API))
                .unwrap()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("refresh-two")
        );

        {
            let mut state = manager.state.lock().await;
            state.tokens.expires_at = now() - 1;
            save(&path, &state.tokens).unwrap();
        }
        assert_eq!(manager.access_token().await.unwrap(), "access-three");
        let saved = load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(saved.access_token, "access-three");
        assert_eq!(saved.refresh_token.as_deref(), Some("refresh-three"));
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn independent_processes_share_one_refresh_rotation() {
        const WORKER_PATH: &str = "ZOO2_TUI_TOKEN_WORKER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let issuer = std::env::var("ZOO2_TUI_TOKEN_WORKER_ISSUER").unwrap();
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_TOKEN_WORKER_READY").unwrap());
            let start = PathBuf::from(std::env::var_os("ZOO2_TUI_TOKEN_WORKER_START").unwrap());
            let config = oidc(&issuer);
            let initial = load(&path, &config, Some(API)).unwrap().unwrap();
            std::fs::write(&ready, "ready").unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            let manager = TokenManager::new(reqwest::Client::new(), config, path, initial);
            assert_eq!(
                manager.access_token().await.unwrap(),
                "cross-process-access"
            );
            return;
        }

        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=rt"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(400))
                    .set_body_json(json!({
                        "access_token": "cross-process-access",
                        "refresh_token": "cross-process-refresh",
                        "expires_in": 300
                    })),
            )
            .mount(&server)
            .await;

        let path = temp_path("cross-process-refresh");
        let initial = tokens(&config, now() + 30, Some("rt"));
        save(&path, &initial).unwrap();
        let parent = path.parent().unwrap();
        let ready_paths = [parent.join("worker-a.ready"), parent.join("worker-b.ready")];
        let start = parent.join("worker.start");
        let executable = std::env::current_exe().unwrap();
        let mut children = Vec::new();
        for ready in &ready_paths {
            children.push(
                Command::new(&executable)
                    .arg("--exact")
                    .arg("auth::token::tests::independent_processes_share_one_refresh_rotation")
                    .arg("--nocapture")
                    .env(WORKER_PATH, &path)
                    .env("ZOO2_TUI_TOKEN_WORKER_ISSUER", &config.issuer)
                    .env("ZOO2_TUI_TOKEN_WORKER_READY", ready)
                    .env("ZOO2_TUI_TOKEN_WORKER_START", &start)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            while ready_paths.iter().any(|ready| !ready.exists()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both independent workers loaded the initial token snapshot");
        std::fs::write(&start, "go").unwrap();
        for child in children {
            let status = tokio::task::spawn_blocking(move || {
                let mut child = child;
                child.wait().unwrap()
            })
            .await
            .unwrap();
            assert!(status.success(), "refresh worker exited with {status}");
        }

        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.method == "POST")
                .count(),
            1,
            "only one process may consume the shared refresh token"
        );
        let saved = load(&path, &config, Some(API)).unwrap().unwrap();
        assert_eq!(
            saved.refresh_token.as_deref(),
            Some("cross-process-refresh")
        );
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn async_lock_wait_is_bounded_when_another_process_holds_the_lock() {
        const WORKER_PATH: &str = "ZOO2_TUI_LOCK_HOLDER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_LOCK_HOLDER_READY").unwrap());
            let _lock = token_file_lock(&path, false).unwrap().unwrap();
            std::fs::write(ready, "locked").unwrap();
            tokio::time::sleep(Duration::from_secs(4)).await;
            return;
        }

        let config = oidc("https://issuer");
        let path = temp_path("bounded-lock-wait");
        save(&path, &tokens(&config, now() + 600, None)).unwrap();
        let ready = path.parent().unwrap().join("lock-holder.ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("auth::token::tests::async_lock_wait_is_bounded_when_another_process_holds_the_lock")
            .arg("--nocapture")
            .env(WORKER_PATH, &path)
            .env("ZOO2_TUI_LOCK_HOLDER_READY", &ready)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("independent process acquired and holds the token-file lock");

        let lock_file = open_token_file_lock(&path, false).unwrap().unwrap();
        let started = tokio::time::Instant::now();
        let result =
            acquire_token_file_lock_async(lock_file, &path, Duration::from_millis(200)).await;
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            result.is_err(),
            "lock wait must return at its configured bound"
        );

        let waiter_path = path.clone();
        let waiter = tokio::spawn(async move {
            let lock_file = open_token_file_lock(&waiter_path, false)?.unwrap();
            acquire_token_file_lock_async(lock_file, &waiter_path, Duration::from_secs(3)).await
        });
        tokio::time::sleep(Duration::from_millis(60)).await;
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());

        child.kill().unwrap();
        let _status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(token_file_lock_async(&path, false).await.unwrap().is_some());
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn independent_process_logout_invalidates_a_rotated_token_held_only_in_memory() {
        const WORKER_PATH: &str = "ZOO2_TUI_LOGOUT_WORKER_PATH";
        if let Some(path) = std::env::var_os(WORKER_PATH) {
            let path = PathBuf::from(path);
            let ready = PathBuf::from(std::env::var_os("ZOO2_TUI_LOGOUT_WORKER_READY").unwrap());
            let start = PathBuf::from(std::env::var_os("ZOO2_TUI_LOGOUT_WORKER_START").unwrap());
            std::fs::write(&ready, "ready").unwrap();
            while !start.exists() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            assert!(delete(&path).unwrap());
            return;
        }

        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .and(body_string_contains("refresh_token=rt"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "rotated-access",
                "refresh_token": "rotated-refresh",
                "expires_in": 300
            })))
            .expect(1)
            .mount(&server)
            .await;

        let path = temp_path("memory-token-logout");
        let initial = tokens(&config, now() + 30, Some("rt"));
        save(&path, &initial).unwrap();
        let readonly_parent = ReadOnlyDirectory::new(path.parent().unwrap());
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );
        assert!(manager.access_token().await.is_err());
        readonly_parent.allow_writes();

        let parent = path.parent().unwrap();
        let ready = parent.join("logout-worker.ready");
        let start = parent.join("logout-worker.start");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg("auth::token::tests::independent_process_logout_invalidates_a_rotated_token_held_only_in_memory")
            .arg("--nocapture")
            .env(WORKER_PATH, &path)
            .env("ZOO2_TUI_LOGOUT_WORKER_READY", &ready)
            .env("ZOO2_TUI_LOGOUT_WORKER_START", &start)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("independent logout process is ready");
        std::fs::write(&start, "logout").unwrap();
        let status = tokio::task::spawn_blocking(move || child.wait().unwrap())
            .await
            .unwrap();
        assert!(status.success(), "logout worker exited with {status}");

        let error = manager.access_token().await.unwrap_err();
        assert!(matches!(
            error.downcast_ref::<HttpError>(),
            Some(HttpError::Unauthorized)
        ));
        assert_eq!(load(&path, &config, Some(API)).unwrap(), None);
    }

    #[tokio::test]
    async fn pending_refresh_persistence_does_not_restore_logout_or_replace_another_account() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "rotated-access",
                "refresh_token": "rotated-refresh",
                "expires_in": 300
            })))
            .expect(2)
            .mount(&server)
            .await;

        let logout_path = temp_path("retry-after-logout");
        let logout_initial = tokens(&config, now() + 30, Some("rt"));
        save(&logout_path, &logout_initial).unwrap();
        let logout_readonly = ReadOnlyDirectory::new(logout_path.parent().unwrap());
        let logout_manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            logout_path.clone(),
            logout_initial,
        );
        assert!(logout_manager.access_token().await.is_err());
        logout_readonly.allow_writes();
        assert!(delete(&logout_path).unwrap());
        let logout_error = logout_manager.access_token().await.unwrap_err();
        assert!(matches!(
            logout_error.downcast_ref::<HttpError>(),
            Some(HttpError::Unauthorized)
        ));
        assert_eq!(load(&logout_path, &config, Some(API)).unwrap(), None);

        let other_path = temp_path("retry-after-account-switch");
        let other_initial = tokens(&config, now() + 30, Some("rt"));
        save(&other_path, &other_initial).unwrap();
        let other_readonly = ReadOnlyDirectory::new(other_path.parent().unwrap());
        let other_manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            other_path.clone(),
            other_initial,
        );
        assert!(other_manager.access_token().await.is_err());
        other_readonly.allow_writes();
        let mut other_tokens = tokens(&config, now() + 600, Some("other-refresh"));
        other_tokens.session_id = uuid::Uuid::new_v4().to_string();
        other_tokens.access_token = "another-account-access".into();
        save(&other_path, &other_tokens).unwrap();
        let other_error = other_manager.access_token().await.unwrap_err();
        assert!(matches!(
            other_error.downcast_ref::<HttpError>(),
            Some(HttpError::Unauthorized)
        ));
        assert_eq!(
            load(&other_path, &config, Some(API)).unwrap(),
            Some(other_tokens)
        );
        delete(&other_path).unwrap();
    }

    #[tokio::test]
    async fn asks_for_login_when_refresh_is_impossible() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_grant" })),
            )
            .mount(&server)
            .await;

        for refresh in [None, Some("revoked")] {
            let path = temp_path("expired");
            let initial = tokens(&config, now() - 1, refresh);
            save(&path, &initial).unwrap();
            let manager = TokenManager::new(
                reqwest::Client::new(),
                config.clone(),
                path.clone(),
                initial,
            );
            let err = manager.access_token().await.unwrap_err();
            assert!(matches!(
                err.downcast_ref::<HttpError>(),
                Some(HttpError::Unauthorized)
            ));
            delete(&path).unwrap();
        }
    }

    #[tokio::test]
    async fn treats_server_error_as_transient() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(json!({ "error": "server_error" })),
            )
            .mount(&server)
            .await;

        let path = temp_path("transient");
        let initial = tokens(&config, now() - 1, Some("rt"));
        save(&path, &initial).unwrap();
        let manager = TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            initial,
        );
        let err = manager.access_token().await.unwrap_err();
        assert!(err.downcast_ref::<HttpError>().is_none(), "{err}");
        delete(&path).unwrap();
    }

    #[tokio::test]
    async fn sign_out_and_revoke_revokes_the_refresh_token_and_removes_the_file() {
        let server = MockServer::start().await;
        let issuer = format!("{}/api/auth", server.uri());
        Mock::given(method("GET"))
            .and(path("/api/auth/.well-known/openid-configuration"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "issuer": issuer,
                "token_endpoint": format!("{}/token", server.uri()),
                "revocation_endpoint": format!("{}/revoke", server.uri())
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/revoke"))
            .and(body_string_contains("token=old-refresh"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let config = oidc(&issuer);
        let path = temp_path("sign-out-and-revoke");
        let initial = tokens(&config, now() + 3600, Some("old-refresh"));
        save(&path, &initial).unwrap();
        let manager = TokenManager::new(reqwest::Client::new(), config, path.clone(), initial);

        assert!(manager.sign_out_and_revoke().await.unwrap());
        assert!(!path.exists());
        assert!(manager.access_token().await.is_err());
    }

    #[tokio::test]
    async fn refresh_finishing_after_sign_out_does_not_recreate_the_file() {
        let server = MockServer::start().await;
        let config = oidc(&issuer(&server).await);
        Mock::given(method("POST"))
            .and(path("/api/auth/oauth2/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "access_token": "new", "expires_in": 300 }))
                    .set_delay(std::time::Duration::from_millis(300)),
            )
            .mount(&server)
            .await;

        let path = temp_path("sign-out");
        save(&path, &tokens(&config, now() + 30, Some("rt"))).unwrap();
        let manager = std::sync::Arc::new(TokenManager::new(
            reqwest::Client::new(),
            config.clone(),
            path.clone(),
            tokens(&config, now() + 30, Some("rt")),
        ));
        let refreshing = tokio::spawn({
            let manager = manager.clone();
            async move { manager.access_token().await }
        });
        // 更新の要求が IdP に届くのを待ってからログアウトする。
        while server.received_requests().await.unwrap_or_default().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(manager.sign_out().await.unwrap());
        assert!(refreshing.await.unwrap().is_err());
        assert!(load(&path, &config, Some(API)).unwrap().is_none());
        assert!(manager.access_token().await.is_err());
    }
}
