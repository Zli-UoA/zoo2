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
    sync::atomic::{AtomicU64, Ordering},
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

/// `CON`、`NUL`、`COM1` など、Windows が予約している名前。
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 書き込み中の 1 件。`file` は `.part` を開いたもの。
struct Pending {
    file: File,
    part: PathBuf,
    final_path: PathBuf,
    /// 決まったフォルダーへの保存か（保存ダイアログで利用者が選んだのではない）。
    /// 同時に始めた同じ名前の保存で、先に完成したファイルを消さないようにする。
    deduplicate: bool,
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
        let start_dir = settings
            .directory
            .as_deref()
            .map(Path::new)
            .filter(|dir| dir.is_dir());

        let (final_path, deduplicate) = if settings.ask_every_time {
            match ask_save_path(app, &name, start_dir).await {
                Some(path) => (path, false),
                None => return Ok(None),
            }
        } else {
            let dir = match settings.directory.as_deref() {
                Some(dir) => PathBuf::from(dir),
                None => app
                    .path()
                    .download_dir()
                    .map_err(|err| anyhow!("ダウンロードフォルダーが見つかりません: {err}"))?,
            };
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("保存先を作れません: {}", dir.display()))?;
            (unique_path(&dir, &name), true)
        };

        if let Some(dir) = final_path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("保存先を作れません: {}", dir.display()))?;
        }
        let id = new_download_id();
        let part = part_path(&final_path, &id);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .with_context(|| format!("書き込み用のファイルを作れません: {}", part.display()))?;
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.clone(),
                Pending {
                    file,
                    part,
                    final_path: final_path.clone(),
                    deduplicate,
                },
            );
        Ok(Some((id, final_path)))
    }

    /// 途中まで書いたものを追記する。
    pub fn write(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = pending
            .get_mut(id)
            .ok_or_else(|| anyhow!("ダウンロードが見つかりません"))?;
        entry.file.write_all(bytes)?;
        Ok(())
    }

    /// 書き終えたら、本来の名前に置き換える。保存した絶対パスを返す。
    pub fn finish(&self, id: &str) -> Result<PathBuf> {
        let entry = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id)
            .ok_or_else(|| anyhow!("ダウンロードが見つかりません"))?;
        let synced = entry.file.sync_all();
        drop(entry.file);
        if let Err(err) = synced {
            let _ = std::fs::remove_file(&entry.part);
            return Err(err.into());
        }
        let saved = if entry.deduplicate {
            // `begin` の後に同じ名前のファイルができても（同時に始めた保存など）、消さずに番号を足す。
            rename_unique(&entry.part, &entry.final_path)
        } else {
            replace_file(&entry.part, &entry.final_path).map(|()| entry.final_path.clone())
        };
        saved.map_err(|err| {
            let _ = std::fs::remove_file(&entry.part);
            anyhow::Error::new(err)
                .context(format!("保存できません: {}", entry.final_path.display()))
        })
    }

    /// 中止する。途中までの `.part` を消す。見つからなくてもエラーにしない。
    pub fn cancel(&self, id: &str) {
        let entry = self
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        if let Some(entry) = entry {
            drop(entry.file);
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
        let pending = std::mem::take(
            &mut *self
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for (_, entry) in pending {
            drop(entry.file);
            let _ = std::fs::remove_file(&entry.part);
        }
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

/// 保存先として使えるかを確かめる。無ければ作る。
fn check_directory(directory: &Path) -> Result<PathBuf> {
    if !directory.is_absolute() {
        return Err(anyhow!("保存先は絶対パスで指定してください"));
    }
    std::fs::create_dir_all(directory)
        .with_context(|| format!("保存先を作れません: {}", directory.display()))?;
    if !directory.is_dir() {
        return Err(anyhow!(
            "保存先がフォルダーではありません: {}",
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
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    if RESERVED_NAMES.contains(&stem.as_str()) {
        cleaned.insert(0, '_');
    }
    truncate_name(cleaned)
}

/// 決まったフォルダーに保存するとき、既にある名前を避ける（`name (1).pdf` など）。
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    let stem = &name[..name.len().saturating_sub(extension.len())];
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

/// 決まったフォルダーへ保存するときは上書きしない。`begin` の後に同じ名前のファイルができたら、
/// 番号を足した名前へ移す（同時に始めた保存や、別のアプリが作ったファイルを消さない）。
fn rename_unique(part: &Path, target: &Path) -> std::io::Result<PathBuf> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "attachment".into());
    let chosen = if target.exists() {
        unique_path(dir, &name)
    } else {
        target.to_path_buf()
    };
    match std::fs::rename(part, &chosen) {
        Ok(()) => Ok(chosen),
        // Unix の `rename` は既存のファイルを置き換えるので、Windows でだけ起きる。
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let next = unique_path(dir, &name);
            std::fs::rename(part, &next)?;
            Ok(next)
        }
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // 重複したときの `-{ID}` を足しても収まる。
        assert!(format!("{cleaned}-0123456789abcdef").len() <= 255);
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
    fn write_finish_and_cancel_manage_the_part_file() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let downloads = Downloads::new();
        let final_path = dir.join("saved.txt");

        let id = new_download_id();
        let part = part_path(&final_path, &id);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part)
            .unwrap();
        downloads.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                file,
                part: part.clone(),
                final_path: final_path.clone(),
                deduplicate: false,
            },
        );
        downloads.write(&id, b"hello ").unwrap();
        downloads.write(&id, b"world").unwrap();
        assert!(part.exists());
        let saved = downloads.finish(&id).unwrap();
        assert_eq!(saved, final_path);
        assert_eq!(std::fs::read(&final_path).unwrap(), b"hello world");
        assert!(!part.exists());

        let cancel_id = new_download_id();
        let cancel_part = part_path(&dir.join("cancelled.txt"), &cancel_id);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&cancel_part)
            .unwrap();
        downloads.pending.lock().unwrap().insert(
            cancel_id.clone(),
            Pending {
                file,
                part: cancel_part.clone(),
                final_path: dir.join("cancelled.txt"),
                deduplicate: false,
            },
        );
        downloads.write(&cancel_id, b"partial").unwrap();
        downloads.cancel(&cancel_id);
        assert!(!cancel_part.exists());
        assert!(downloads.write(&cancel_id, b"x").is_err());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fixed_folder_saves_do_not_overwrite_each_other() {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let downloads = Downloads::new();
        let target = dir.join("same.txt");

        // 同時に始めた 2 件は、どちらも同じ名前を選ぶ（まだファイルがないため）。
        let mut ids = Vec::new();
        for _ in 0..2 {
            let id = new_download_id();
            let part = part_path(&target, &id);
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part)
                .unwrap();
            downloads.pending.lock().unwrap().insert(
                id.clone(),
                Pending {
                    file,
                    part,
                    final_path: target.clone(),
                    deduplicate: true,
                },
            );
            downloads.write(&id, id.as_bytes()).unwrap();
            ids.push(id);
        }

        // 先に終わった方が本来の名前、後から終わった方は番号を足した名前になる。どちらも消えない。
        let first = downloads.finish(&ids[0]).unwrap();
        let second = downloads.finish(&ids[1]).unwrap();
        assert_eq!(first, target);
        assert_eq!(second, dir.join("same (1).txt"));
        assert_eq!(std::fs::read(&first).unwrap(), ids[0].as_bytes());
        assert_eq!(std::fs::read(&second).unwrap(), ids[1].as_bytes());

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
