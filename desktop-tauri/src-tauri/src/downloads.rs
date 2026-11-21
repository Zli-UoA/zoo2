//! 添付の保存。OS の「名前を付けて保存」、保存先の設定、書き込み、中止。
//!
//! デスクトップ版では、WebView の既定のダウンロード（WebView2 などが保存先を決める）を使わない。
//! WebView は署名付き URL から本文を読み、`zoo_download_write` に octet-stream で渡す。Rust 側は
//! 保存先を決め、同じフォルダーの `.part` に書いてから、最後に本来の名前に置き換える。途中で
//! 中止されたら `.part` を消すので、壊れたファイルを残さない。
//!
//! JS から絶対パスを受け取らない。保存先を決めるのは保存ダイアログ（利用者の選択）と
//! `downloads.json` の設定だけで、`zoo_download_begin` が決めたパスにしか書けない。

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use zoo2_client_types::ipc::{DownloadSettings, DownloadSettingsUpdate};

use crate::APP_NAME;
use zoo2_client_core::config::config_dir_for;

/// 一時ファイルに使う ID の連番。同じプロセスの中で一意にする。
static DOWNLOAD_SEQ: AtomicU64 = AtomicU64::new(0);

/// 名前が長すぎるとファイルシステムの上限に当たるので、ファイル名をこのバイト数までにする。
/// macOS (APFS) と Linux (ext4) の上限は UTF-8 で 255 バイト。`.part` に付く
/// `.{16 桁の ID}.part`（23 バイト）と、重複したときの `-{16 桁の ID}`（17 バイト）の分を引く。
/// Windows の上限は 255 UTF-16 単位で、UTF-8 のバイト数はそれを超えない。
const MAX_FILE_NAME_BYTES: usize = 255 - 23 - 17;

/// 同時に書き込める件数。リモートの画面が `begin` を繰り返して、ファイルハンドルを
/// 使い切らないようにする。
const MAX_PENDING_DOWNLOADS: usize = 3;

/// 1 件の上限。添付の上限（`MAX_ATTACHMENT_FILE_SIZE`、50 MiB）に合わせる。
const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// `CON`、`NUL`、`COM1` など、Windows が予約している名前。予約名の後ろの空白は
/// Windows が無視するので、比べる前に落とす。`COM¹` などの上付き数字も予約名。
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// 書き込み中の 1 件。`file` は `.part` を開いたもの。`None` は中止済みで、
/// 書き込みも終了処理もしない。
struct Pending {
    file: Arc<Mutex<Option<File>>>,
    part: PathBuf,
    final_path: PathBuf,
    /// 決まったフォルダーへの保存か（保存ダイアログで利用者が選んだのではない）。
    /// 同時に始めた同じ名前の保存で、先に完成したファイルを消さないようにする。
    deduplicate: bool,
    /// この保存で書いた合計バイト数。
    written: Arc<AtomicU64>,
    /// 1 件あたりの上限。テストで小さくできるよう、`Pending` に持たせる。
    limit: u64,
    cancelled: Arc<AtomicBool>,
}

/// 保存先の設定と、書き込み中のもの。
pub struct Downloads {
    settings: std::sync::Mutex<DownloadSettings>,
    pending: std::sync::Mutex<HashMap<String, Pending>>,
}

impl Downloads {
    pub fn new() -> Self {
        Self {
            settings: std::sync::Mutex::new(load_settings()),
            pending: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// 前回のクラッシュや強制終了で残った `.part` を、起動時に消す。
    /// 保存先にしているフォルダーと、既定のダウンロードフォルダーだけを見る。
    pub fn cleanup_stale_parts(&self, app: &AppHandle) {
        let settings = self.settings();
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(dir) = settings.directory.as_deref() {
            dirs.push(PathBuf::from(dir));
        }
        if let Ok(dir) = app.path().download_dir()
            && !dirs.contains(&dir)
        {
            dirs.push(dir);
        }
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let name = entry.file_name();
                if is_part_file_name(&name.to_string_lossy()) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }

    pub fn settings(&self) -> DownloadSettings {
        self.settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// 確認の有無を変えて保存する。保存先は、`reset_directory` のときだけ既定
    /// （OS のダウンロードフォルダー）に戻す。保存できなかったら、変えない。
    ///
    /// 保存先のパスは受け取らない。決められるのは OS のフォルダーの選択（[`Self::set_directory`]）
    /// だけで、リモートの画面から任意の場所へ書き込めないようにするため。
    pub fn set_settings(&self, update: DownloadSettingsUpdate) -> Result<DownloadSettings> {
        let mut current = self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = current.clone();
        next.ask_every_time = update.ask_every_time;
        if update.reset_directory {
            next.directory = None;
        }
        store_settings(&mut current, next)
    }

    /// OS のフォルダーの選択で決まった保存先を、確かめて保存する。パスは JS から受け取らない。
    /// 選ばれたフォルダーが無いときは保存しない（作り直さない）。
    pub fn set_directory(&self, directory: &Path) -> Result<DownloadSettings> {
        let directory = check_directory(directory)?;
        let mut current = self
            .settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut next = current.clone();
        next.directory = Some(directory.display().to_string());
        store_settings(&mut current, next)
    }

    /// 保存先を決め、`.part` を開く。キャンセルされたら `None`。
    pub async fn begin(
        &self,
        app: &AppHandle,
        file_name: &str,
    ) -> Result<Option<(String, PathBuf)>> {
        let name = sanitize_file_name(file_name);
        let settings = self.settings();
        let configured = settings.directory.as_deref().map(Path::new);

        let (final_path, deduplicate) = if settings.ask_every_time {
            match ask_save_path(app, &name, configured.filter(|dir| dir.is_dir())).await {
                Some(path) => (path, false),
                None => return Ok(None),
            }
        } else {
            match configured {
                // 設定したフォルダーが消えている（外付けドライブや同期フォルダーが外れたなど）。
                // 作り直さず、保存ダイアログに戻して利用者に決めてもらう。
                Some(dir) if dir.is_dir() => (unique_path(dir, &name), true),
                Some(_) => match ask_save_path(app, &name, None).await {
                    Some(path) => (path, false),
                    None => return Ok(None),
                },
                None => {
                    let dir = app
                        .path()
                        .download_dir()
                        .map_err(|err| anyhow!("ダウンロードフォルダーが見つかりません: {err}"))?;
                    if dir.is_dir() {
                        (unique_path(&dir, &name), true)
                    } else {
                        match ask_save_path(app, &name, None).await {
                            Some(path) => (path, false),
                            None => return Ok(None),
                        }
                    }
                }
            }
        };

        // 保存ダイアログは既にあるフォルダーを返す。決まったフォルダーの保存も、上の分岐で
        // フォルダーの存在を確かめてある。ここで `create_dir_all` はしない。
        let id = new_download_id();
        let part = part_path(&final_path, &id);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .with_context(|| format!("書き込み用のファイルを作れません: {}", part.display()))?;
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if pending.len() >= MAX_PENDING_DOWNLOADS {
            drop(pending);
            drop(file);
            let _ = std::fs::remove_file(&part);
            return Err(anyhow!(
                "同時に保存できるのは {MAX_PENDING_DOWNLOADS} 件までです"
            ));
        }
        pending.insert(
            id.clone(),
            Pending {
                file: Arc::new(Mutex::new(Some(file))),
                part,
                final_path: final_path.clone(),
                deduplicate,
                written: Arc::new(AtomicU64::new(0)),
                limit: MAX_DOWNLOAD_BYTES,
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        Ok(Some((id, final_path)))
    }

    /// 途中まで書いたものを追記する。ディスクへの書き込みは `spawn_blocking` に移し、
    /// `pending` のロックを持ったまま同期 I/O をしない。
    pub async fn write(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let (file, written, limit, cancelled) = {
            let pending = self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let entry = pending
                .get(id)
                .ok_or_else(|| anyhow!("ダウンロードが見つかりません"))?;
            (
                Arc::clone(&entry.file),
                Arc::clone(&entry.written),
                entry.limit,
                Arc::clone(&entry.cancelled),
            )
        };
        if cancelled.load(Ordering::SeqCst) {
            return Err(anyhow!("ダウンロードは中止されました"));
        }
        let len = bytes.len() as u64;
        let before = written.fetch_add(len, Ordering::SeqCst);
        if before.saturating_add(len) > limit {
            written.fetch_sub(len, Ordering::SeqCst);
            return Err(anyhow!(
                "添付が大きすぎます（上限は {} MiB）",
                limit / (1024 * 1024)
            ));
        }
        let data = bytes.to_vec();
        let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut slot = file
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let file = slot
                .as_mut()
                .ok_or_else(|| std::io::Error::other("ダウンロードは中止されました"))?;
            file.write_all(&data)
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => {
                written.fetch_sub(len, Ordering::SeqCst);
                Err(err.into())
            }
            Err(err) => {
                written.fetch_sub(len, Ordering::SeqCst);
                Err(anyhow!("書き込みを実行できません: {err}"))
            }
        }
    }

    /// 書き終えたら、本来の名前に置き換える。保存した絶対パスを返す。
    pub async fn finish(&self, id: &str) -> Result<PathBuf> {
        let entry = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
            .ok_or_else(|| anyhow!("ダウンロードが見つかりません"))?;
        // `sync_all` と置き換えはディスクを待つので、IPC のワーカーを塞がない。
        tokio::task::spawn_blocking(move || finish_entry(entry))
            .await
            .map_err(|err| anyhow!("保存を実行できません: {err}"))?
    }

    /// 中止する。途中までの `.part` を消す。見つからなくてもエラーにしない。
    pub async fn cancel(&self, id: &str) {
        let entry = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        let Some(entry) = entry else {
            return;
        };
        entry.cancelled.store(true, Ordering::SeqCst);
        let file = Arc::clone(&entry.file);
        let part = entry.part;
        // 書き込み中のハンドルが閉じるのを待ってから消す（Windows で開いたままのファイルは消せない）。
        let _ = tokio::task::spawn_blocking(move || {
            let taken = file
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            drop(taken);
            let _ = std::fs::remove_file(&part);
        })
        .await;
    }

    /// 書き込み中のものをすべて中止する。画面の読み込み直し、ログアウト、アプリの終了で呼ぶ。
    pub fn cancel_all(&self) {
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, entry) in pending {
            entry.cancelled.store(true, Ordering::SeqCst);
            if let Ok(mut slot) = entry.file.lock() {
                drop(slot.take());
            }
            let _ = std::fs::remove_file(&entry.part);
        }
    }
}

impl Default for Downloads {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Downloads {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

/// OS の「名前を付けて保存」を出す。選ばれた絶対パス。閉じられたら `None`。
async fn ask_save_path(
    app: &AppHandle,
    file_name: &str,
    start_dir: Option<&Path>,
) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file().set_file_name(file_name);
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }
    if let Some(window) = app.get_webview_window(crate::window::MAIN) {
        dialog = dialog.set_parent(&window);
    }
    dialog.save_file(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx.await.ok().flatten()?;
    picked.simplified().into_path().ok()
}

/// OS のフォルダーの選択を出す。選ばれた絶対パス。閉じられたら `None`。
pub async fn pick_directory(app: &AppHandle) -> Option<PathBuf> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app.dialog().file();
    if let Some(window) = app.get_webview_window(crate::window::MAIN) {
        dialog = dialog.set_parent(&window);
    }
    dialog.pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx.await.ok().flatten()?;
    picked.simplified().into_path().ok()
}

fn settings_path() -> Result<PathBuf> {
    config_dir_for(APP_NAME)
        .map(|dir| dir.join("downloads.json"))
        .ok_or_else(|| anyhow!("設定ディレクトリが見つかりません"))
}

/// 保存した設定。ないときや読めないときは、既定（保存のたびに確認する）。
fn load_settings() -> DownloadSettings {
    settings_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// 設定をファイルへ書き、覚えている値も入れ替える。書き出せなかったら、覚えている値は変えない。
fn store_settings(
    current: &mut DownloadSettings,
    next: DownloadSettings,
) -> Result<DownloadSettings> {
    let path = settings_path()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // 通知の設定と同じく、同じディレクトリの一時ファイルから置き換える。
    let tmp = path.with_extension("json.tmp");
    let written = std::fs::write(&tmp, serde_json::to_vec(&next)?)
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(err.into());
    }
    *current = next.clone();
    Ok(next)
}

/// 保存先として使えるかを確かめる。無いときは作り直さず、エラーにする。
fn check_directory(directory: &Path) -> Result<PathBuf> {
    if !directory.is_absolute() {
        return Err(anyhow!("保存先は絶対パスで指定してください"));
    }
    if !directory.is_dir() {
        return Err(anyhow!(
            "保存先のフォルダーが見つかりません: {}",
            directory.display()
        ));
    }
    Ok(directory.to_path_buf())
}

fn new_download_id() -> String {
    use std::hash::{BuildHasher, Hasher};

    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    hasher.write_u128(nanos);
    hasher.write_u64(DOWNLOAD_SEQ.fetch_add(1, Ordering::Relaxed));
    format!("{:016x}", hasher.finish())
}

/// 同じフォルダーの、作業中のファイル。完成した名前とは別にする。
fn part_path(final_path: &Path, id: &str) -> PathBuf {
    let name = final_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into());
    final_path.with_file_name(format!(".{name}.{id}.part"))
}

/// 起動時の掃除の対象か。`part_path` が作る `.{名前}.{16 桁の ID}.part` だけ。
fn is_part_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".part")) else {
        return false;
    };
    let Some((_, id)) = stem.rsplit_once('.') else {
        return false;
    };
    id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 拡張子を残して、名前がファイルシステムの上限（バイト数）に収まるようにする。
/// 文字数ではなくバイト数で切るので、日本語などの名前でも `.part` を含めて 255 バイトに収まる。
fn truncate_name(name: String) -> String {
    if name.len() <= MAX_FILE_NAME_BYTES {
        return name;
    }
    let suffix = Path::new(&name)
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .filter(|ext| ext.len() < 16)
        .filter(|ext| name.ends_with(ext.as_str()))
        .unwrap_or_default();
    let stem_source = &name[..name.len() - suffix.len()];
    let budget = MAX_FILE_NAME_BYTES.saturating_sub(suffix.len());
    let mut stem = String::with_capacity(budget);
    for c in stem_source.chars() {
        if stem.len() + c.len_utf8() > budget {
            break;
        }
        stem.push(c);
    }
    let stem = stem.trim_end_matches(['.', ' ']);
    format!("{stem}{suffix}")
}

/// 危険な制御文字（双方向の制御文字を含む）か。
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
        )
}

/// 保存に使えるファイル名にする。パスの区切りは落とし、Windows が使えない文字を置き換える。
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut cleaned: String = base
        .chars()
        .map(|c| {
            if is_unsafe_char(c) || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    cleaned = cleaned.trim().trim_end_matches(['.', ' ']).to_string();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        cleaned = "attachment".into();
    }
    // Windows は予約名の後ろの空白を無視するので、比べる前に落とす。
    let stem = cleaned
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        cleaned.insert(0, '_');
    }
    truncate_name(cleaned)
}

/// 名前を、拡張子を残したまま `(n)` を足せる形に分ける。
fn split_name(name: &str) -> (&str, &str) {
    let path = Path::new(name);
    let extension = path
        .extension()
        .map(|ext| &name[name.len() - ext.len() - 1..])
        .unwrap_or("");
    let stem = &name[..name.len() - extension.len()];
    (stem, extension)
}

/// 決まったフォルダーに保存するとき、既にある名前を避ける（`name (1).pdf` など）。
/// これは始めの候補を選ぶだけで、確定は [`move_unique`] が上書きせずに行う。
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, extension) = split_name(name);
    for n in 1..=9999 {
        let candidate = dir.join(format!("{stem} ({n}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}{extension}", new_download_id()))
}

/// `rename` で置き換える。Windows では既にあるファイルに `rename` できないので、消してから置き換える。
/// 保存ダイアログは上書きを確かめてからパスを返すので、ここでの削除は利用者の選択に沿う。
fn replace_file(part: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::rename(part, target) {
        Ok(()) => Ok(()),
        Err(_) if target.exists() => {
            std::fs::remove_file(target)?;
            std::fs::rename(part, target)
        }
        Err(err) => Err(err),
    }
}

/// 保存先が既にあると失敗する形で、`part` を `target` へ移す。Unix の `rename` は既存の
/// ファイルを置き換えるため、`exists` の確認と `rename` の間の競合で他のファイルを消さないよう、
/// ハードリンク（既にあれば `AlreadyExists`）を使う。
fn link_no_replace(part: &Path, target: &Path) -> std::io::Result<()> {
    match std::fs::hard_link(part, target) {
        Ok(()) => {
            let _ = std::fs::remove_file(part);
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(err),
        // ハードリンクを使えないファイルシステム（FAT など）や、ポリシーで禁じられている環境では、
        // 空のファイルを `create_new` で先に確保してから写す。これも既にあるファイルを置き換えない。
        Err(_) => {
            let mut dest = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            let copied = std::fs::File::open(part).and_then(|mut src| {
                std::io::copy(&mut src, &mut dest)?;
                dest.sync_all()
            });
            if let Err(err) = copied {
                drop(dest);
                let _ = std::fs::remove_file(target);
                return Err(err);
            }
            drop(dest);
            let _ = std::fs::remove_file(part);
            Ok(())
        }
    }
}

/// 決まったフォルダーへ保存するときは上書きしない。`begin` の後に同じ名前のファイルができたら、
/// 番号を足した名前へ移す（同時に始めた保存や、別のアプリが作ったファイルを消さない）。
fn move_unique(part: &Path, target: &Path) -> std::io::Result<PathBuf> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into());
    let (stem, extension) = split_name(&name);
    for n in 0..=9999 {
        let candidate = if n == 0 {
            target.to_path_buf()
        } else {
            dir.join(format!("{stem} ({n}){extension}"))
        };
        match link_no_replace(part, &candidate) {
            Ok(()) => return Ok(candidate),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "空いているファイル名が見つかりません",
    ))
}

/// `finish` の実体。`spawn_blocking` の中で動く。
fn finish_entry(entry: Pending) -> Result<PathBuf> {
    if entry.cancelled.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!("ダウンロードは中止されました"));
    }
    let file = entry
        .file
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let Some(file) = file else {
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!("ダウンロードは中止されました"));
    };
    let synced = file.sync_all();
    drop(file);
    if let Err(err) = synced {
        let _ = std::fs::remove_file(&entry.part);
        return Err(err.into());
    }
    let saved = if entry.deduplicate {
        move_unique(&entry.part, &entry.final_path)
    } else {
        replace_file(&entry.part, &entry.final_path).map(|()| entry.final_path.clone())
    };
    saved.map_err(|err| {
        let _ = std::fs::remove_file(&entry.part);
        anyhow::Error::new(err).context(format!("保存できません: {}", entry.final_path.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(target: &Path, id: &str, deduplicate: bool, limit: u64) -> Pending {
        let part = part_path(target, id);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .unwrap();
        Pending {
            file: Arc::new(Mutex::new(Some(file))),
            part,
            final_path: target.to_path_buf(),
            deduplicate,
            written: Arc::new(AtomicU64::new(0)),
            limit,
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    #[test]
    fn sanitize_keeps_a_plain_name() {
        assert_eq!(sanitize_file_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_file_name("議事録 2026.docx"), "議事録 2026.docx");
    }

    #[test]
    fn sanitize_drops_directories_and_unsafe_characters() {
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\Users\\a\\x.txt"), "x.txt");
        assert_eq!(
            sanitize_file_name("a<b>c:d\"e|f?g*h.txt"),
            "a_b_c_d_e_f_g_h.txt"
        );
        assert_eq!(sanitize_file_name("evil\u{202e}txt.exe"), "evil_txt.exe");
        assert_eq!(sanitize_file_name("line\nbreak.txt"), "line_break.txt");
    }

    #[test]
    fn sanitize_handles_empty_reserved_and_trailing_dots() {
        assert_eq!(sanitize_file_name(""), "attachment");
        assert_eq!(sanitize_file_name(".."), "attachment");
        assert_eq!(sanitize_file_name("name..."), "name");
        assert_eq!(sanitize_file_name("CON"), "_CON");
        assert_eq!(sanitize_file_name("con.txt"), "_con.txt");
        // 予約名の後ろの空白は Windows が無視する。上付き数字も予約名。
        assert_eq!(sanitize_file_name("CON .txt"), "_CON .txt");
        assert_eq!(sanitize_file_name("COM¹.txt"), "_COM¹.txt");
        assert_eq!(sanitize_file_name("LPT².log"), "_LPT².log");
    }

    #[test]
    fn sanitize_truncates_long_names_by_bytes_and_keeps_the_extension() {
        let name = format!("{}.pdf", "a".repeat(300));
        let cleaned = sanitize_file_name(&name);
        assert_eq!(cleaned.len(), MAX_FILE_NAME_BYTES);
        assert!(cleaned.ends_with(".pdf"));

        // 日本語は 1 文字 3 バイト。文字数で切ると macOS / Linux の 255 バイトを超える。
        let cleaned = sanitize_file_name(&"あ".repeat(300));
        assert!(cleaned.len() <= MAX_FILE_NAME_BYTES);
        assert!(cleaned.ends_with('あ'));
        assert_eq!(cleaned.len() % 3, 0);

        // `.part` に付く分を含めても、ファイル名の上限（255 バイト）に収まる。
        let part = part_path(&PathBuf::from(&cleaned), "0123456789abcdef");
        assert!(part.file_name().unwrap().len() <= 255);
        // 重複したときの `(9999)` を足しても収まる。
        assert!(format!("{cleaned} (9999)").len() <= 255);
    }

    #[test]
    fn unique_path_adds_a_number_when_the_name_exists() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn is_part_file_name_matches_only_the_temporary_names() {
        assert!(is_part_file_name(".a.txt.0123456789abcdef.part"));
        assert!(is_part_file_name(".報告.0123456789abcdef.part"));
        assert!(!is_part_file_name("a.txt"));
        assert!(!is_part_file_name(".a.txt.part"));
        assert!(!is_part_file_name(".a.txt.0123.part"));
        assert!(!is_part_file_name(".a.txt.0123456789abcdeg.part"));
    }

    #[tokio::test]
    async fn write_finish_and_cancel_manage_the_part_file() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let downloads = Downloads::new();
        let final_path = dir.join("saved.txt");

        let id = new_download_id();
        let entry = pending(&final_path, &id, false, MAX_DOWNLOAD_BYTES);
        let part = entry.part.clone();
        downloads.pending.lock().unwrap().insert(id.clone(), entry);
        downloads.write(&id, b"hello ").await.unwrap();
        downloads.write(&id, b"world").await.unwrap();
        assert!(part.exists());
        let saved = downloads.finish(&id).await.unwrap();
        assert_eq!(saved, final_path);
        assert_eq!(std::fs::read(&final_path).unwrap(), b"hello world");
        assert!(!part.exists());

        let cancel_id = new_download_id();
        let entry = pending(
            &dir.join("cancelled.txt"),
            &cancel_id,
            false,
            MAX_DOWNLOAD_BYTES,
        );
        let cancel_part = entry.part.clone();
        downloads
            .pending
            .lock()
            .unwrap()
            .insert(cancel_id.clone(), entry);
        downloads.write(&cancel_id, b"partial").await.unwrap();
        downloads.cancel(&cancel_id).await;
        assert!(!cancel_part.exists());
        assert!(downloads.write(&cancel_id, b"x").await.is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn write_rejects_more_than_the_limit() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let downloads = Downloads::new();
        let id = new_download_id();
        downloads
            .pending
            .lock()
            .unwrap()
            .insert(id.clone(), pending(&dir.join("limit.bin"), &id, true, 4));
        downloads.write(&id, b"1234").await.unwrap();
        assert!(downloads.write(&id, b"5").await.is_err());
        downloads.cancel(&id).await;
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_fixed_folder_saves_do_not_overwrite_each_other() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let downloads = Downloads::new();
        let target = dir.join("same.txt");

        // 同時に始めた 2 件は、どちらも同じ名前を選ぶ（まだファイルがないため）。
        let mut ids = Vec::new();
        for _ in 0..2 {
            let id = new_download_id();
            downloads
                .pending
                .lock()
                .unwrap()
                .insert(id.clone(), pending(&target, &id, true, MAX_DOWNLOAD_BYTES));
            downloads.write(&id, id.as_bytes()).await.unwrap();
            ids.push(id);
        }

        // 同時に確定しても、後から確定した方が番号を足した名前になり、どちらも消えない。
        let (first, second) = tokio::join!(downloads.finish(&ids[0]), downloads.finish(&ids[1]));
        let saved = [first.unwrap(), second.unwrap()];
        assert!(saved.contains(&target));
        assert!(saved.contains(&dir.join("same (1).txt")));
        for (id, path) in ids.iter().zip(&saved) {
            assert_eq!(std::fs::read(path).unwrap(), id.as_bytes());
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_unique_does_not_replace_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let part = dir.join(".x.part");
        let target = dir.join("x.txt");
        std::fs::write(&part, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();

        let saved = move_unique(&part, &target).unwrap();
        assert_eq!(saved, dir.join("x (1).txt"));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read(&saved).unwrap(), b"new");
        assert!(!part.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn cancelled_downloads_are_not_finished() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let downloads = Downloads::new();
        let id = new_download_id();
        let entry = pending(&dir.join("cancelled.txt"), &id, true, MAX_DOWNLOAD_BYTES);
        downloads.pending.lock().unwrap().insert(id.clone(), entry);
        downloads.cancel_all();
        assert!(downloads.finish(&id).await.is_err());
        assert!(!dir.join("cancelled.txt").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_file_overwrites_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let part = dir.join(".x.part");
        let target = dir.join("x.txt");
        std::fs::write(&part, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        replace_file(&part, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!part.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
