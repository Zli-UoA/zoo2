//! 添付の保存。OS の「名前を付けて保存」、保存先の設定、書き込み、中止。
//!
//! デスクトップ版では、WebView の既定のダウンロード（WebView2 などが保存先を決める）を使わない。
//! WebView は署名付き URL から本文を読み、`zoo_download_write` に octet-stream で渡す。Rust 側は
//! 保存先を決め、同じフォルダーの `.zoo-{id}.part` に書いてから、最後に本来の名前に移す。途中で
//! 中止されたら `.part` を消すので、壊れたファイルを残さない。
//!
//! JS から絶対パスを受け取らない。保存先を決めるのは保存ダイアログ（利用者の選択）と
//! `downloads.json` の設定だけで、`zoo_download_begin` が決めたパスにしか書けない。
//! 「保存のたびに確認する」をオフにするときも、Rust 側で OS の確認を出す（リモートの画面が
//! 勝手にオフにできないようにするため）。

use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use zoo2_client_types::ipc::{DownloadBegin, DownloadSettings, DownloadSettingsUpdate};

use crate::APP_NAME;
use zoo2_client_core::config::config_dir_for;

/// 一時ファイルに使う ID の連番。同じプロセスの中で一意にする。
static DOWNLOAD_SEQ: AtomicU64 = AtomicU64::new(0);

/// ファイルシステムが受け付ける名前の上限（バイト数）。macOS (APFS) と Linux (ext4) は
/// 255 バイト、Windows は 255 UTF-16 単位で、UTF-8 のバイト数はそれを超えない。
const MAX_NAME_BYTES: usize = 255;

/// 名前が長すぎるとファイルシステムの上限に当たるので、ファイル名をこのバイト数までにする。
/// 衝突したときの連番（最大 ` (9999)` の 7 バイト）などの分を残す。`.part` は名前を
/// 含まない（[`part_path`]）。
const MAX_FILE_NAME_BYTES: usize = MAX_NAME_BYTES - 17;

/// 同時に書き込める件数（保存ダイアログを待っているものも含む）。リモートの画面が `begin` を
/// 繰り返して、ファイルハンドルを使い切らないようにする。
const MAX_PENDING_DOWNLOADS: usize = 3;

/// 1 件の上限の既定値。添付の大きさ（[`DownloadBegin::size`]）が分からないときに使う。
/// バックエンドの `ATTACHMENT_MAX_FILE_SIZE`（既定 50 MiB）に合わせてある。サーバーの
/// 上限を上げるときは、この値も上げる（分かる添付は `size` で上限を広げられる）。
const DEFAULT_MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// 添付の大きさに足す余裕。`size` と実際の本文がわずかに違っても保存できるようにする。
const DOWNLOAD_SIZE_SLACK: u64 = 1024 * 1024;

/// どうしても超えない 1 件の上限。リモートの画面が大きすぎる `size` を渡しても、これ以上は書かせない。
const MAX_DOWNLOAD_BYTES_CEILING: u64 = 512 * 1024 * 1024;

/// `CON`、`NUL`、`COM1` など、Windows が予約している名前。予約名の後ろの空白は
/// Windows が無視するので、比べる前に落とす。`COM¹` などの上付き数字も予約名。
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³",
];

fn normalize_reserved_stem(stem: &str) -> String {
    let mut chars = stem.chars();
    let (Some(a), Some(b), Some(c), Some(d)) =
        (chars.next(), chars.next(), chars.next(), chars.next())
    else {
        return stem.to_string();
    };
    if chars.next().is_some() {
        return stem.to_string();
    }
    let digit = match d {
        '1' | '¹' => '1',
        '2' | '²' => '2',
        '3' | '³' => '3',
        '4' => '4',
        '5' => '5',
        '6' => '6',
        '7' => '7',
        '8' => '8',
        '9' => '9',
        _ => return stem.to_string(),
    };
    if (a == 'C' && b == 'O' && c == 'M') || (a == 'L' && b == 'P' && c == 'T') {
        return format!("{a}{b}{c}{digit}");
    }
    stem.to_string()
}

/// 確認なしで保存してよい拡張子。開いても、そのまま実行や読み込みに進まない形式だけを並べる。
/// ここに無い拡張子（`.exe`、`.py`、`.docm`、`.dll` など）と、拡張子の無い名前は、
/// 保存ダイアログに戻して利用者に確かめる。漏れの少ない許可リストにしておく。
/// 古い Office 文書（`.doc`、`.xls`、`.ppt`）は VBA を持てるので入れない。マクロを含められない
/// `.docx` などだけを許可する。OpenDocument（`.odt`、`.ods`、`.odp`）は Basic や Python の
/// マクロを埋め込めるので入れない。
const AUTO_SAVE_EXTENSIONS: &[&str] = &[
    // 画像（`.svg` はスクリプトを持てるので入れない）
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "heic", "heif", "tif", "tiff",
    // 音声・動画
    "mp3", "m4a", "ogg", "oga", "wav", "flac", "opus", "aac", "mp4", "m4v", "webm", "mov", "mkv",
    // 文書・テキスト（EPUB は HTML を描画するので入れない）
    "pdf", "txt", "md", "csv", "tsv", "json", "log", "docx", "xlsx", "pptx",
    // 圧縮（中身は開くときに利用者が選ぶ）
    "zip", "7z", "tar", "gz", "bz2", "xz", "zst",
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
    /// 書き込みに失敗した。途中までの内容がディスクに残っているかもしれないので、確定しない。
    failed: Arc<AtomicBool>,
}

/// 保存ダイアログで選ばれた名前を、確定時にどう扱うか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SaveDecision {
    /// 確定時に既存のファイルを置き換えない（番号を足した名前へ移す）。
    deduplicate: bool,
}

impl SaveDecision {
    /// 決まったフォルダーへの保存。始めた後にできた同名のファイルも置き換えない。
    const AUTO: Self = Self { deduplicate: true };

    /// 保存ダイアログで選ばれた名前。`existed` は、ダイアログが閉じた直後に確かめた
    /// 最終名の有無。あったなら、ダイアログが上書きを確認して利用者が承認している。
    /// 無かったなら、転送中にできた同名のファイルは置き換えない。
    fn dialog(existed: bool) -> Self {
        Self {
            deduplicate: !existed,
        }
    }
}

/// `begin` の結果。`prompted` は OS の保存ダイアログを出したかどうか。
#[derive(Debug, Clone)]
pub struct DownloadBeginResult {
    pub id: String,
    pub path: PathBuf,
    pub prompted: bool,
}

/// 保存先が、既存のファイルを上書きせずに保存する方法を持たない。`begin` で転送を
/// 始める前に確かめ、確定できない保存先では本文を読み始めない。
#[derive(Debug)]
pub struct UnsupportedDestination;

impl std::fmt::Display for UnsupportedDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "この保存先には保存できません。別のフォルダーを選ぶか、設定で保存先を変えてください",
        )
    }
}

impl std::error::Error for UnsupportedDestination {}

/// 同時に保存できる件数の上限に達している。待てば保存できるので、通信の失敗とは
/// 区別して画面に伝える。
#[derive(Debug)]
pub struct TooManyDownloads;

impl std::fmt::Display for TooManyDownloads {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "同時に保存できるのは {MAX_PENDING_DOWNLOADS} 件までです。ほかの保存が終わってから、もう一度お試しください"
        )
    }
}

impl std::error::Error for TooManyDownloads {}

/// 記録に残すパス。UTF-8 で表せないバイト（Unix）や、対になっていない UTF-16（Windows）も
/// 失わずに保存する。以前の版が書いた、パスだけの文字列も読める。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredPath {
    /// UTF-8 で表せるパス。以前の版の記録もこれとして読む。
    Text(String),
    /// Unix の、UTF-8 で表せないバイト列。
    #[cfg(unix)]
    Bytes { bytes: Vec<u8> },
    /// Windows の、対になっていない符号単位を含むパス。
    #[cfg(windows)]
    Wide { wide: Vec<u16> },
}

impl StoredPath {
    fn from_path(path: &Path) -> Self {
        if let Some(text) = path.to_str() {
            return Self::Text(text.to_owned());
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Self::Bytes {
                bytes: path.as_os_str().as_bytes().to_vec(),
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self::Wide {
                wide: path.as_os_str().encode_wide().collect(),
            }
        }
        #[cfg(not(any(unix, windows)))]
        Self::Text(path.to_string_lossy().into_owned())
    }

    fn into_path(self) -> PathBuf {
        match self {
            Self::Text(text) => PathBuf::from(text),
            #[cfg(unix)]
            Self::Bytes { bytes } => {
                use std::os::unix::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_vec(bytes))
            }
            #[cfg(windows)]
            Self::Wide { wide } => {
                use std::os::windows::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_wide(&wide))
            }
        }
    }
}

/// 記録したときの、保存先の親フォルダーの ID。外付けドライブが外れて、同じ名前の
/// マウントポイントだけが残った場合と、フォルダーが本当に消えた場合を区別する。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct ParentId {
    /// ボリューム（Unix は `st_dev`、Windows はボリュームのシリアル番号）。
    volume: u64,
    /// フォルダーの inode（Windows はファイル ID）。
    index: u64,
}

/// `path` の親フォルダーの ID。親が無い、または読めないときは `None`。
#[cfg(unix)]
fn parent_id(path: &Path) -> Option<ParentId> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path.parent()?).ok()?;
    Some(ParentId {
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

/// `path` の親フォルダーの ID。親が無い、または読めないときは `None`。
#[cfg(windows)]
fn parent_id(path: &Path) -> Option<ParentId> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
        },
    };

    let parent = path.parent()?;
    // ディレクトリを開くには `FILE_FLAG_BACKUP_SEMANTICS` が要る。
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(parent)
        .ok()?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `dir` は呼び出しの間生きていて有効なハンドルを持ち、`info` は出力先。
    if unsafe { GetFileInformationByHandle(HANDLE(dir.as_raw_handle()), &mut info) }.is_err() {
        return None;
    }
    Some(ParentId {
        volume: u64::from(info.dwVolumeSerialNumber),
        index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    })
}

#[cfg(not(any(unix, windows)))]
fn parent_id(_path: &Path) -> Option<ParentId> {
    None
}

/// 記録したときと同じフォルダーに、まだ届くか。ボリュームやフォルダーの ID が変わって
/// いたら（外付けドライブが外れてマウントポイントだけが残るなど）、記録したファイルには
/// 触れられないとみなす。旧形式で親 ID が無い記録は、以前どおり親の有無だけを見る。
fn reachable_parent(path: &Path, recorded: Option<ParentId>) -> bool {
    match recorded {
        Some(recorded) => parent_id(path) == Some(recorded),
        None => parent_id(path).is_some(),
    }
}

/// `download_parts.json` の 1 件。以前の版は、パスだけの文字列を書いていた。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredEntry {
    Legacy(String),
    Current {
        path: StoredPath,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<ParentId>,
    },
}

#[derive(Debug, Clone)]
struct JournalEntry {
    path: PathBuf,
    /// 記録したときの親フォルダー。読めなかったときは `None`。
    parent: Option<ParentId>,
}

/// クラッシュや強制終了で残る `.part` の記録。保存先がダイアログで選ばれたフォルダーでも、
/// 次の起動で消せるように、設定ディレクトリの `download_parts.json` に置く。
struct Journal {
    /// 記録のファイル。設定ディレクトリが分からないときは `None`（保存を始めない）。
    /// テストでは一時ディレクトリのパスを渡し、利用者の記録を読み書きしない。
    file: Option<PathBuf>,
    /// 書き込み中の `.part`。読み込みは [`Journal::load`]、以後はここが正本。
    entries: Mutex<Vec<JournalEntry>>,
    /// 読み込み失敗と回復の状態。
    state: Mutex<JournalState>,
    /// `download_parts.json` への書き込みを直列化する。スナップショットを作ってから
    /// 書くまでの間に別の更新が入ると、古い内容で上書きして記録が消えるため。
    write_lock: Mutex<()>,
}

#[derive(Clone, Debug, Default)]
struct JournalState {
    /// 既存の記録を読めなかった。空の記録で上書きすると旧 `.part` を消せなくなるため、
    /// 読み直せるまで新しい記録を書かない。
    load_failed: bool,
    /// 読めなかった記録を、退避または再読み込みで回復済みか。
    recovered: bool,
    /// 回復時に読み込んだ旧記録。呼び出し側が掃除に渡したら空にする。
    pending_cleanup: Vec<JournalEntry>,
}

impl Journal {
    fn load(file: Option<PathBuf>) -> Self {
        let mut load_failed = false;
        let entries = match file.as_deref() {
            None => Vec::new(),
            Some(path) => match std::fs::read(path) {
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(_) => {
                    load_failed = true;
                    Vec::new()
                }
                Ok(bytes) => match serde_json::from_slice::<Vec<StoredEntry>>(&bytes) {
                    Ok(entries) => entries
                        .into_iter()
                        .map(|entry| match entry {
                            StoredEntry::Legacy(text) => JournalEntry {
                                path: PathBuf::from(text),
                                parent: None,
                            },
                            StoredEntry::Current { path, parent } => JournalEntry {
                                path: path.into_path(),
                                parent,
                            },
                        })
                        .collect(),
                    Err(_) => {
                        load_failed = true;
                        Vec::new()
                    }
                },
            },
        };
        Self {
            file,
            entries: Mutex::new(entries),
            state: Mutex::new(JournalState {
                load_failed,
                recovered: false,
                pending_cleanup: Vec::new(),
            }),
            write_lock: Mutex::new(()),
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> Vec<PathBuf> {
        self.entries().into_iter().map(|entry| entry.path).collect()
    }

    /// 記録の内容。起動時の掃除で、保存先にまだ届くかを確かめるのに使う。
    fn entries(&self) -> Vec<JournalEntry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn quarantine_path(path: &Path) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        path.with_extension(format!("json.corrupt-{stamp}"))
    }

    fn read_entries(path: &Path) -> std::io::Result<Option<Vec<JournalEntry>>> {
        match std::fs::read(path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Some(Vec::new())),
            Err(err) => Err(err),
            Ok(bytes) => match serde_json::from_slice::<Vec<StoredEntry>>(&bytes) {
                Ok(entries) => Ok(Some(
                    entries
                        .into_iter()
                        .map(|entry| match entry {
                            StoredEntry::Legacy(text) => JournalEntry {
                                path: PathBuf::from(text),
                                parent: None,
                            },
                            StoredEntry::Current { path, parent } => JournalEntry {
                                path: path.into_path(),
                                parent,
                            },
                        })
                        .collect(),
                )),
                Err(_) => Ok(None),
            },
        }
    }

    /// 読めなかった記録を回復する。壊れた JSON は退避し、見られなかった記録は
    /// 読み直せたら再開する。退避した記録は残す（旧 `.part` の手がかりのため）。
    fn recover(&self) -> Result<()> {
        {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if !state.load_failed || state.recovered {
                return Ok(());
            }
        }
        let _write = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            if !state.load_failed || state.recovered {
                return Ok(());
            }
        }
        let Some(path) = self.file.as_deref() else {
            return Err(anyhow!(
                "前回の保存の記録を読めないため、保存を始められません"
            ));
        };
        match Self::read_entries(path) {
            Ok(Some(entries)) => {
                *self.entries.lock().unwrap_or_else(PoisonError::into_inner) = entries.clone();
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.load_failed = false;
                state.recovered = true;
                for entry in &entries {
                    if !state.pending_cleanup.iter().any(|e| e.path == entry.path) {
                        state.pending_cleanup.push(entry.clone());
                    }
                }
                Ok(())
            }
            Ok(None) => {
                let backup = Self::quarantine_path(path);
                std::fs::rename(path, &backup)?;
                *self.entries.lock().unwrap_or_else(PoisonError::into_inner) = Vec::new();
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                state.load_failed = false;
                state.recovered = true;
                Ok(())
            }
            Err(err) => Err(err.into()),
        }
    }

    fn take_pending_cleanup(&self) -> Vec<JournalEntry> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut state.pending_cleanup)
    }

    /// `.part` を作る前に記録する。書けなければ記録を戻してエラーにする
    /// （残骸を次の起動で消せないまま、書き込みを始めないため）。
    fn record(&self, part: &Path) -> Result<()> {
        self.recover()?;
        let _write = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let snapshot = {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            if entries.iter().any(|entry| entry.path == part) {
                return Ok(());
            }
            entries.push(JournalEntry {
                path: part.to_path_buf(),
                parent: parent_id(part),
            });
            entries.clone()
        };
        if let Err(err) = self.write(&snapshot) {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries.retain(|entry| entry.path != part);
            return Err(err);
        }
        Ok(())
    }

    fn forget(&self, part: &Path) {
        let _write = self
            .write_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let snapshot = {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            let before = entries.len();
            entries.retain(|entry| entry.path != part);
            if entries.len() == before {
                return;
            }
            entries.clone()
        };
        let _ = self.write(&snapshot);
    }

    /// 消せた（または既に無かった）ときだけ記録を外す。消せなかったときは残して、
    /// 次の起動でもう一度試す（外付けドライブを外した、ウイルス対策が掴んでいるなど）。
    fn remove_part(&self, part: &Path) {
        let parent = self
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|entry| entry.path == part)
            .and_then(|entry| entry.parent);
        let removed = match std::fs::remove_file(part) {
            Ok(()) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                reachable_parent(part, parent)
            }
            Err(_) => false,
        };
        if removed {
            self.forget(part);
        }
    }

    fn write(&self, entries: &[JournalEntry]) -> Result<()> {
        // 呼び出し側が `write_lock` で直列化する。ロックを二重に取らない。
        // 既存の記録を読めていないときに空の記録で上書きすると、旧 `.part` が記録から
        // 外れて次の起動で消せなくなる（E2EE では復号済みの内容が残る）。読み直せるまで
        // 書かない。
        let (load_failed, recovered) = {
            let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            (state.load_failed, state.recovered)
        };
        if load_failed && !recovered {
            return Err(anyhow!(
                "前回の保存の記録を読めないため、保存を始められません"
            ));
        }
        // 記録先が無いと、強制終了で残った `.part` を次の起動で消せない。メモリだけの
        // 記録で保存を始めない（`record` がエラーを返し、`begin` が保存を拒否する）。
        let Some(path) = self.file.as_deref() else {
            return Err(anyhow!("保存の記録を置く設定ディレクトリが見つかりません"));
        };
        if entries.is_empty() {
            // 記録が無くなったら、ファイルも消す。
            match std::fs::remove_file(path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(err) => return Err(err.into()),
            }
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let list: Vec<StoredEntry> = entries
            .iter()
            .map(|entry| StoredEntry::Current {
                path: StoredPath::from_path(&entry.path),
                parent: entry.parent,
            })
            .collect();
        let tmp = path.with_extension("json.tmp");
        let written = std::fs::write(&tmp, serde_json::to_vec(&list)?)
            .and_then(|()| std::fs::rename(&tmp, path));
        if let Err(err) = written {
            let _ = std::fs::remove_file(&tmp);
            return Err(err.into());
        }
        Ok(())
    }
}

/// 実行中の後始末（取消で始めた `.part` の削除）。終了時は、`pending` から既に
/// 取り出された分の削除も待つ。`cancel` は書き込み中のハンドルが閉じるのを待つことが
/// あるので、数だけを数える。
#[derive(Clone)]
struct Cleanups {
    state: Arc<(Mutex<usize>, Condvar)>,
}

impl Cleanups {
    fn new() -> Self {
        Self {
            state: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    /// 後始末を始める。返したガードを後始末のスレッドへ移す。スレッドが動き出す前に
    /// 終了が来ても、数に入っているので待てる。
    fn start(&self) -> CleanupGuard {
        let mut count = self.state.0.lock().unwrap_or_else(PoisonError::into_inner);
        *count += 1;
        CleanupGuard(self.clone())
    }

    fn finish(&self) {
        let mut count = self.state.0.lock().unwrap_or_else(PoisonError::into_inner);
        *count = count.saturating_sub(1);
        if *count == 0 {
            self.state.1.notify_all();
        }
    }

    #[cfg(test)]
    fn running(&self) -> usize {
        *self.state.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// 実行中の後始末が終わるまで、期限まで待つ。終わったら true。
    fn wait(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut count = self.state.0.lock().unwrap_or_else(PoisonError::into_inner);
        while *count > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }
            let (next, _) = self
                .state
                .1
                .wait_timeout(count, remaining)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            count = next;
        }
        true
    }
}

struct CleanupGuard(Cleanups);

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// 保存先の設定と、書き込み中のもの。
pub struct Downloads {
    settings: Mutex<DownloadSettings>,
    pending: Mutex<HashMap<String, Pending>>,
    /// 保存の枠。保存ダイアログを待っている間も数え、確定・中止の後始末が終わるまで返さない。
    slots: Arc<Mutex<usize>>,
    /// 画面の世代。`cancel_all` で進め、保存ダイアログを待っていた `begin` を無効にする。
    generation: AtomicU64,
    /// クラッシュで残る `.part` の記録。
    journal: Arc<Journal>,
    /// 実行中の後始末。終了時に、取消で始まっている削除も待つ。
    cleanups: Cleanups,
    /// テスト用の記録ディレクトリ。テスト終了時に自動で削除する。
    #[cfg(test)]
    test_dir: Option<PathBuf>,
}

impl Downloads {
    pub fn new() -> Self {
        Self::with_journal(journal_path())
    }

    fn with_journal(journal_file: Option<PathBuf>) -> Self {
        Self {
            settings: Mutex::new(load_settings()),
            pending: Mutex::new(HashMap::new()),
            slots: Arc::new(Mutex::new(0)),
            generation: AtomicU64::new(0),
            journal: Arc::new(Journal::load(journal_file)),
            cleanups: Cleanups::new(),
            #[cfg(test)]
            test_dir: None,
        }
    }

    /// 前回のクラッシュや強制終了で残った `.part` を消す。ウィンドウの作成を待たせないよう、
    /// 記録の読み出しだけをここで行い、ファイルの削除は裏のスレッドで行う。保存先が届かない
    /// 共有フォルダーでも、起動が止まらないようにするため。
    pub fn spawn_stale_part_cleanup(&self) {
        let stale = {
            // 読めなかった記録は、ここで回復を試みる。壊れた JSON は退避し、
            // 一時的な読み取り失敗は次の保存開始時にも再試行する。
            let _ = self.journal.recover();
            let mut entries = self.journal.entries();
            entries.extend(self.journal.take_pending_cleanup());
            entries
        };
        self.spawn_cleanup_for(stale);
    }

    fn spawn_cleanup_for(&self, stale: Vec<JournalEntry>) {
        if stale.is_empty() {
            return;
        }
        let journal = Arc::clone(&self.journal);
        let cleanups = self.cleanups.clone();
        let guard = cleanups.start();
        std::thread::spawn(move || {
            let _guard = guard;
            for entry in stale {
                if remove_stale_entry(&entry.path, entry.parent) {
                    journal.forget(&entry.path);
                }
            }
        });
    }

    pub fn settings(&self) -> DownloadSettings {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 確認の有無を変えて保存する。保存先は、`reset_directory` のときだけ
    /// 消す。確認なしで保存するにはフォルダーが必須で、無い設定にはしない。保存できなかったら、変えない。
    ///
    /// 保存先のパスは受け取らない。決められるのは OS のフォルダーの選択（[`Self::set_directory`]）
    /// だけで、リモートの画面から任意の場所へ書き込めないようにするため。
    ///
    /// 「保存のたびに確認する」をオフにするのは、確認なしで利用者のフォルダーへ書けるように
    /// なる操作なので、OS の確認を出す。承認されなければ、オンのままにする。
    pub async fn set_settings(
        &self,
        app: &AppHandle,
        update: DownloadSettingsUpdate,
    ) -> Result<DownloadSettings> {
        let current = self.settings();
        // 矛盾する更新は、確認ダイアログを出す前に拒否する。承認後に失敗したり、
        // キャンセル時に `reset_directory` だけが保存されたりしないようにする。
        let next_directory = if update.reset_directory {
            None
        } else {
            current.directory.as_deref()
        };
        if !update.ask_every_time && next_directory.is_none() {
            return Err(anyhow!(
                "確認なしで保存するには保存先のフォルダーを選んでください"
            ));
        }
        loop {
            // 確認が要るかは、ダイアログを出す直前の設定で決める。
            let ask_every_time = self.settings().ask_every_time;
            let directory = self.settings().directory;
            let approved = if ask_every_time && !update.ask_every_time {
                // 確認なしで利用者のフォルダーへ書けるようにする操作なので、OS の確認を出す。
                confirm_automatic_save(app, directory.as_deref()).await
            } else {
                true
            };
            // ダイアログを待っている間に保存先や確認の有無が変わっていることがある。
            // 待つ前の値を書き戻さず、その時点の設定に今回の変更だけを適用する。
            let mut stored = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
            if approved
                && ask_every_time
                && !update.ask_every_time
                && stored.directory.as_deref() != directory.as_deref()
            {
                // 確認したフォルダーと今の保存先が違う。別のフォルダーへの確認として使わない。
                drop(stored);
                continue;
            }
            let Some(next) = apply_settings_update(&stored, ask_every_time, approved, &update)
            else {
                // 確認の要否が変わったので、持っている承認は今の設定には使えない。
                // ロックを持ったままダイアログは出せないので、出し直す。
                drop(stored);
                continue;
            };
            automatic_save_needs_directory(&next)?;
            return store_settings(&mut stored, next);
        }
    }

    /// OS のフォルダーの選択で決まった保存先を、確かめて保存する。パスは JS から受け取らない。
    /// 選ばれたフォルダーが無いときは保存しない（作り直さない）。
    pub fn set_directory(&self, directory: &Path) -> Result<DownloadSettings> {
        let directory = check_directory(directory)?;
        let text = directory_text(&directory)?;
        let mut current = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = current.clone();
        next.directory = Some(text);
        store_settings(&mut current, next)
    }

    /// 保存先を決め、`.part` を開く。キャンセルされたら `None`。
    pub async fn begin(
        &self,
        app: &AppHandle,
        request: &DownloadBegin,
    ) -> Result<Option<DownloadBeginResult>> {
        // 枠は保存ダイアログを出す前に取る。4 件目にダイアログを出してから断らない。
        self.reserve_slot()?;
        let mut slot = SlotGuard {
            downloads: self,
            armed: true,
        };
        let generation = self.generation.load(Ordering::SeqCst);
        let name = sanitize_file_name(&request.file_name);
        let settings = self.settings();

        // 確認なしの保存でも、実行や読み込みに進む形式は保存ダイアログに戻す。
        let ask = settings.ask_every_time || needs_confirmation(&name);
        let (final_path, decision, prompted) = if ask {
            // ダイアログの初期フォルダーは、無ければダイアログ側が既定に戻す。
            match ask_save_decision(app, &name, settings.directory.as_deref().map(Path::new)).await
            {
                Some((path, decision)) => (path, decision, true),
                None => return Ok(None),
            }
        } else {
            // フォルダーの確認と空き名の探索はディスクを待つ。届かない共有フォルダーでも、
            // 非同期ランタイムのワーカーを塞がないように、ブロッキングのタスクへ移す。
            let configured = settings.directory.clone();
            let candidate = name.clone();
            let target = tokio::task::spawn_blocking(move || {
                auto_save_target(configured.as_deref().map(Path::new), &candidate)
            })
            .await
            .map_err(|err| anyhow!("保存先を決められません: {err}"))?;
            match target {
                Some(path) => (path, SaveDecision::AUTO, false),
                // 設定したフォルダーが消えている（外付けドライブや同期フォルダーが外れたなど）。
                // 作り直さず、利用者に決めてもらう。
                None => match ask_save_decision(app, &name, None).await {
                    Some((path, decision)) => (path, decision, true),
                    None => return Ok(None),
                },
            }
        };

        // ダイアログの後、生成の確認の前に後始末の待機対象に入れる。確認から登録までの間に
        // `shutdown` が来ても、待機対象が無いまま戻らないようにする。
        let guard = self.cleanups.start();
        // ダイアログを待っている間に、画面の読み込み直しやログアウトがあった。その画面には
        // もう ID を返せないので、ファイルを作らずに中止にする。
        if self.generation.load(Ordering::SeqCst) != generation {
            return Ok(None);
        }

        let id = new_download_id();
        let part = part_path(&final_path, &id);
        let journal = Arc::clone(&self.journal);
        let part_for_task = part.clone();
        let final_for_task = final_path.clone();
        let deduplicate = decision.deduplicate;
        let (file, recovered) = tokio::task::spawn_blocking(move || {
            prepare_part(&journal, &part_for_task, &final_for_task, deduplicate)
        })
        .await
        .map_err(|err| anyhow!("書き込み用のファイルを用意できません: {err}"))??;
        self.spawn_cleanup_for(recovered);

        let entry = Pending {
            file: Arc::new(Mutex::new(Some(file))),
            part: part.clone(),
            final_path: final_path.clone(),
            // 上書きの承認が無い保存は、確定時に既存のファイルを置き換えない。
            deduplicate: decision.deduplicate,
            written: Arc::new(AtomicU64::new(0)),
            limit: download_limit(request.size),
            cancelled: Arc::new(AtomicBool::new(false)),
            failed: Arc::new(AtomicBool::new(false)),
        };
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        // ファイルを作っている間にも、画面の読み込み直しやログアウトがありうる。
        // 登録と同じロックの中で確かめ、`cancel_all` が取りこぼさないようにする。
        if self.generation.load(Ordering::SeqCst) != generation {
            drop(pending);
            {
                let mut slot = entry.file.lock().unwrap_or_else(PoisonError::into_inner);
                slot.take();
            }
            let journal = Arc::clone(&self.journal);
            let part_for_task = part.clone();
            let _ = tokio::task::spawn_blocking(move || journal.remove_part(&part_for_task)).await;
            return Ok(None);
        }
        pending.insert(id.clone(), entry);
        drop(pending);
        // ここからは `pending` の分が後始末の対象なので、準備用のガードは手放す。
        drop(guard);
        // 枠は確定・中止の後始末まで、この保存が持つ。
        slot.armed = false;
        Ok(Some(DownloadBeginResult {
            id,
            path: final_path,
            prompted,
        }))
    }

    /// 途中まで書いたものを追記する。ディスクへの書き込みは `spawn_blocking` に移し、
    /// `pending` のロックを持ったまま同期 I/O をしない。
    pub async fn write(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let (file, written, limit, cancelled, failed) = {
            let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = pending
                .get(id)
                .ok_or_else(|| anyhow!("ダウンロードが見つかりません"))?;
            (
                Arc::clone(&entry.file),
                Arc::clone(&entry.written),
                entry.limit,
                Arc::clone(&entry.cancelled),
                Arc::clone(&entry.failed),
            )
        };
        if cancelled.load(Ordering::SeqCst) {
            return Err(anyhow!("ダウンロードは中止されました"));
        }
        if failed.load(Ordering::SeqCst) {
            return Err(anyhow!("書き込みに失敗したダウンロードです"));
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
        let failure = Arc::clone(&failed);
        let cancel_flag = Arc::clone(&cancelled);
        let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let slot_result = file.lock();
            let poisoned = slot_result.is_err();
            let mut slot = slot_result.unwrap_or_else(PoisonError::into_inner);
            if poisoned {
                failure.store(true, Ordering::SeqCst);
                return Err(std::io::Error::other("書き込みに失敗したダウンロードです"));
            }
            if cancel_flag.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("ダウンロードは中止されました"));
            }
            if failure.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("書き込みに失敗したダウンロードです"));
            }
            let file = slot
                .as_mut()
                .ok_or_else(|| std::io::Error::other("ダウンロードは中止されました"))?;
            let written_result = file.write_all(&data);
            if written_result.is_err() {
                // 一部だけ書けているかもしれない。同じ ID の確定はさせない（中止で消す）。
                failure.store(true, Ordering::SeqCst);
            }
            written_result
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => {
                written.fetch_sub(len, Ordering::SeqCst);
                Err(err.into())
            }
            Err(err) => {
                // ブロッキングのタスクが落ちた。書き込みの状態は分からないので、確定させない。
                failed.store(true, Ordering::SeqCst);
                written.fetch_sub(len, Ordering::SeqCst);
                Err(anyhow!("書き込みを実行できません: {err}"))
            }
        }
    }

    /// 書き終えたら、本来の名前に移す。保存した絶対パスを返す。
    pub async fn finish(&self, id: &str) -> Result<PathBuf> {
        let Some((entry, guard)) = self.take_pending_with_guard(id) else {
            return Err(anyhow!("ダウンロードが見つかりません"));
        };
        let slot = SlotGuard {
            downloads: self,
            armed: true,
        };
        let journal = Arc::clone(&self.journal);
        let part = entry.part.clone();
        // `sync_all`、名前の移動、記録の後始末はディスクを待つので、IPC のワーカーを塞がない。
        let result = tokio::task::spawn_blocking(move || {
            // `sync_all`、移動、記録の後始末が終わるまでガードを保持する（`shutdown` が待つ）。
            let _guard = guard;
            let result = finish_entry(entry);
            match result {
                // 移したので、もう `.part` は無い。消せたことを確かめてから記録を外す
                // （ハードリンクで確定したときに `.part` の削除だけ失敗した場合もここで消す）。
                Ok(saved) => {
                    journal.remove_part(&part);
                    Ok(saved)
                }
                // 消せたことを確かめてから記録を外す（`remove_part`）。
                Err(err) => {
                    journal.remove_part(&part);
                    Err(err)
                }
            }
        })
        .await;
        drop(slot);
        match result {
            Ok(result) => result,
            Err(err) => Err(anyhow!("保存を実行できません: {err}")),
        }
    }

    /// 中止する。途中までの `.part` を消す。見つからなくてもエラーにしない。
    pub async fn cancel(&self, id: &str) {
        let (entry, guard) = {
            let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(entry) = pending.remove(id) else {
                return;
            };
            // 取り出しと後始末の登録を同じロックで行う。`shutdown` がこの間に割り込んで
            // `pending` も後始末の数も 0 に見えると、`.part` を待たずに終了してしまう。
            (entry, self.cleanups.start())
        };
        let slot = SlotGuard {
            downloads: self,
            armed: true,
        };
        entry.cancelled.store(true, Ordering::SeqCst);
        let file = Arc::clone(&entry.file);
        let part = entry.part.clone();
        let journal = Arc::clone(&self.journal);
        // 書き込み中のハンドルが閉じるのを待ってから消す（Windows で開いたままのファイルは消せない）。
        let _ = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let taken = file.lock().unwrap_or_else(PoisonError::into_inner).take();
            drop(taken);
            // 消せなかった `.part` は記録に残し、次の起動で再試行する。
            journal.remove_part(&part);
        })
        .await;
        drop(slot);
    }

    /// 書き込み中のものをすべて中止する。画面の読み込み直しとログアウトで呼ぶ。
    /// アプリの終了時は [`Downloads::shutdown`] を使う。
    ///
    /// 呼び出し元はセッションのロックを持っていることがあるので、ここではファイルを閉じたり
    /// 消したりしない。記録から外し、ファイルの後始末は裏のスレッドに任せる。
    pub fn cancel_all(&self) {
        let (pending, guard) = {
            let mut current = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            // 世代更新と取り出しを同じロックで行う。`begin` が新しい世代を読んだあとに
            // 登録するまでの間に取り消さないようにする。
            self.generation.fetch_add(1, Ordering::SeqCst);
            let pending = std::mem::take(&mut *current);
            if pending.is_empty() {
                return;
            }
            // `cancel` と同じく、取り出しと後始末の登録を同じロックで行う。
            (pending, self.cleanups.start())
        };
        for entry in pending.values() {
            entry.cancelled.store(true, Ordering::SeqCst);
        }
        let journal = Arc::clone(&self.journal);
        let slots = Arc::clone(&self.slots);
        std::thread::spawn(move || {
            let _guard = guard;
            for (_, entry) in pending {
                let taken = entry
                    .file
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take();
                drop(taken);
                journal.remove_part(&entry.part);
                // 後始末が終わるまで枠を返さない。旧画面のファイルを閉じる前に、
                // 新画面で同じ数の保存を始められないようにする。
                let mut slots = slots.lock().unwrap_or_else(PoisonError::into_inner);
                *slots = slots.saturating_sub(1);
            }
        });
    }

    /// アプリの終了時に、書きかけの `.part` を消す。`tauri::App::run` は終了時に
    /// `std::process::exit` するので、managed state の `Drop` は呼ばれない。終了の
    /// イベント（`RunEvent::Exit`）から、上限時間まで待って呼ぶ。
    ///
    /// 削除は裏のスレッドで行う。共有フォルダーが応答しないときも、終了を待たせ続けない
    /// ため。既に `cancel` や `cancel_all` が始めた後始末も、同じ期限まで待つ。待ちきれなかった
    /// ものは記録に残し、次の起動で消す。
    pub fn shutdown(&self, timeout: Duration) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        let taken = {
            let mut current = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
            let pending = std::mem::take(&mut *current);
            if pending.is_empty() {
                None
            } else {
                // 取り出しと後始末の登録を同じロックで行い、`wait` が自分の分を
                // 取りこぼさないようにする。
                Some((pending, self.cleanups.start()))
            }
        };
        if let Some((pending, guard)) = taken {
            for entry in pending.values() {
                entry.cancelled.store(true, Ordering::SeqCst);
            }
            let journal = Arc::clone(&self.journal);
            // 書き込み中のハンドルが閉じるのを待ってから消す（Windows で開いたままのファイルは
            // 消せない）。記録から外れるのは、消せた（または既に無かった）ときだけ。
            std::thread::spawn(move || {
                let _guard = guard;
                for (_, entry) in pending {
                    let taken = entry
                        .file
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take();
                    drop(taken);
                    journal.remove_part(&entry.part);
                }
            });
        }
        // 取消が `pending` から取り出した分も、ここで終わるのを待つ。
        self.cleanups.wait(timeout);
    }

    /// 確定のために取り出す。取り出しと後始末の登録を同じロックで行い、`shutdown` が
    /// この間に割り込んで待ちを終らせないようにする。
    fn take_pending_with_guard(&self, id: &str) -> Option<(Pending, CleanupGuard)> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = pending.remove(id)?;
        Some((entry, self.cleanups.start()))
    }

    fn reserve_slot(&self) -> Result<()> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if *slots >= MAX_PENDING_DOWNLOADS {
            return Err(anyhow::Error::new(TooManyDownloads));
        }
        *slots += 1;
        Ok(())
    }

    fn release_slot(&self) {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        *slots = slots.saturating_sub(1);
    }
}

/// 確保した枠。保存が成立したら `armed` を false にして、確定・中止の側へ渡す。
struct SlotGuard<'a> {
    downloads: &'a Downloads,
    armed: bool,
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.downloads.release_slot();
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
        // 通常の終了では `tauri::App::run` が `std::process::exit` するので、この `Drop` は
        // 呼ばれない。終了時の後始末は `shutdown` を `RunEvent::Exit` から呼ぶ。
        // テストなど、プロセスが終わらない場面での取りこぼしを防ぐ。
        self.cancel_all();
        #[cfg(test)]
        if let Some(dir) = self.test_dir.take() {
            // `cancel_all` は裏スレッドで記録を書き換える。終わる前に消すと、そのスレッドが
            // ディレクトリを作り直してゴミが残る。終わるまで待ってから消す。
            if self.cleanups.wait(Duration::from_secs(5)) {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }
}

/// OS の「名前を付けて保存」を出し、選ばれたパスと、確定時の扱いを返す。閉じられたら `None`。
async fn ask_save_decision(
    app: &AppHandle,
    file_name: &str,
    start_dir: Option<&Path>,
) -> Option<(PathBuf, SaveDecision)> {
    let path = ask_save_path(app, file_name, start_dir).await?;
    // 上書きの承認は、ダイアログが閉じた直後の状態で決める。`journal.record`（ファイルへの
    // 書き込み）を待つと、その間に別のアプリが作った同名のファイルを「承認済み」と誤判定する。
    // 共有フォルダーでは `exists` が待つことがあるので、ブロッキングのタスクへ移す。
    let target = path.clone();
    let existed = tokio::task::spawn_blocking(move || target.exists())
        .await
        // 確かめられなかったときは、上書きしない側に倒す。
        .unwrap_or(false);
    Some((path, SaveDecision::dialog(existed)))
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

/// 「保存のたびに確認する」をオフにしてよいか、OS のダイアログで確かめる。
async fn confirm_automatic_save(app: &AppHandle, directory: Option<&str>) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let target = directory
        .filter(|dir| !dir.is_empty())
        .map(|dir| format!("選んだフォルダー（{dir}）へ、確認せずに保存します。"))
        .unwrap_or_else(|| "選んだフォルダーへ、確認せずに保存します。".to_string());
    let message = format!(
        "{target} フォルダーに届かないときは OS の保存ダイアログを出します。\
         実行や読み込みに進む形式のファイルは、これまでどおり保存先を尋ねます。"
    );
    let mut dialog = app
        .dialog()
        .message(message)
        .title("確認せずに保存しますか？")
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "確認せずに保存する".into(),
            "キャンセル".into(),
        ));
    if let Some(window) = app.get_webview_window(crate::window::MAIN) {
        dialog = dialog.parent(&window);
    }
    dialog.show(move |confirmed| {
        let _ = tx.send(confirmed);
    });
    rx.await.unwrap_or(false)
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

fn journal_path() -> Option<PathBuf> {
    config_dir_for(APP_NAME).map(|dir| dir.join("download_parts.json"))
}

/// 保存した設定。ないときや読めないときは、既定（保存のたびに確認する）。
fn load_settings() -> DownloadSettings {
    let settings = settings_path()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    usable_settings(settings)
}

/// `downloads.json` は手で書き換えられる。保存先に使ってよいのは、OS のフォルダーの選択で
/// 選んだ絶対パスだけなので、そうでない値は捨てて「保存のたびに確認する」に戻す。
/// フォルダーが実在するかは、起動を待たせないよう、保存のたびに確かめる（[`auto_save_target`]）。
fn usable_settings(settings: DownloadSettings) -> DownloadSettings {
    let Some(directory) = settings.directory.as_deref() else {
        // フォルダーが無いまま確認なしの設定だけが残っていても、自動保存はしない。
        return if settings.ask_every_time {
            settings
        } else {
            DownloadSettings::default()
        };
    };
    if Path::new(directory).is_absolute() {
        settings
    } else {
        DownloadSettings::default()
    }
}

/// 確認なしの保存には、利用者が選んだフォルダーが必須。フォルダーが無いまま
/// 自動保存を有効にしたり、自動保存の設定でフォルダーを消したりできないようにする。
fn automatic_save_needs_directory(settings: &DownloadSettings) -> Result<()> {
    if !settings.ask_every_time && settings.directory.is_none() {
        return Err(anyhow!(
            "確認なしで保存するには保存先のフォルダーを選んでください"
        ));
    }
    Ok(())
}

/// 待っていた確認の結果を、その時点の設定へ適用する。確認の要否が待っている間に変わって
/// いたら `None`（持っている承認は今の設定に使えないので、ダイアログを出し直す）。
fn apply_settings_update(
    current: &DownloadSettings,
    asked: bool,
    approved: bool,
    update: &DownloadSettingsUpdate,
) -> Option<DownloadSettings> {
    if current.ask_every_time != asked {
        return None;
    }
    let mut next = current.clone();
    // 承認されなければ、オンのままにする。
    next.ask_every_time = if approved {
        update.ask_every_time
    } else {
        true
    };
    if update.reset_directory {
        next.directory = None;
    }
    Some(next)
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

/// 設定ファイルに書ける保存先の文字列。UTF-8 で表せない名前は、別のパスへ変えてしまう
/// （置き換え文字が同名の別フォルダーに当たることがある）ため受け付けない。
fn directory_text(directory: &Path) -> Result<String> {
    directory
        .to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| anyhow!("このフォルダーの名前は保存先として扱えません"))
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

/// 同じフォルダーの、作業中のファイル。名前を含まない短い固定の名前にして、保存ダイアログで
/// 利用者が長い名前を選んでもファイルシステムの上限に当たらないようにする。
fn part_path(final_path: &Path, id: &str) -> PathBuf {
    let dir = final_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(".zoo-{id}.part"))
}

/// 起動時の掃除の対象か。`part_path` が作る `.zoo-{16 桁の ID}.part` と、保存先を確かめる
/// ときに作る `.probe` / `.probe-moved`。
fn is_part_file_name(name: &str) -> bool {
    let Some(stem) = name.strip_prefix(".zoo-") else {
        return false;
    };
    let Some((id, suffix)) = stem.rsplit_once('.') else {
        return false;
    };
    matches!(suffix, "part" | "probe" | "probe-moved")
        && id.len() == 16
        && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 検査用のファイルに書く目印。`.part` は中身が本体なので目印を書けないが、検査用の
/// ファイルには目印を書き、起動時の掃除はこの目印があるものだけを消す。`.part` の
/// 名前から ID を読んだ第三者が、同じ名前の `.probe` を先に置くことができるため。
const PROBE_MARKER: &[u8] = b"zoo2 probe\n";

/// 検査用の名前か。中身（目印）を確かめてから消す対象。
fn is_probe_name(name: &str) -> bool {
    matches!(
        name.rsplit_once('.').map(|(_, suffix)| suffix),
        Some("probe" | "probe-moved")
    )
}

/// 記録に残った名前を消す。消した、または触れる必要が無かったら `true`（記録から外す）。
fn remove_stale_entry(path: &Path, parent: Option<ParentId>) -> bool {
    remove_stale_entry_with(path, parent, |path| probe_state(path, parent))
}

/// [`remove_stale_entry`] の本体。テストで判断を差し替えられるよう、分類を受け取る。
fn remove_stale_entry_with(
    path: &Path,
    parent: Option<ParentId>,
    probe: impl Fn(&Path) -> ProbeState,
) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    if !is_part_file_name(name) {
        // 記録の形が違うものは、ファイルには触れずに記録から外す。
        return true;
    }
    // 事前照合はしない。`st_dev` は再マウントや再起動で変わり、実在する `.part` を
    // 回収できなくなるため。`.part` の名前に 16 桁の ID が入るので同名の衝突はほぼ無く、
    // `.probe` は削除前に目印を確認している。親の照合は `NotFound` のときだけ行う
    // （`Journal::remove_part` と同じ判定）。
    if is_probe_name(name) {
        match probe(path) {
            ProbeState::Ours => {}
            // 先にあった第三者のファイルかもしれない。自分が書いた目印が無いものには
            // 触れず、記録から外すだけにする。
            ProbeState::NotOurs => return true,
            // 一時的な I/O の失敗かもしれない。記録を残し、次の起動でやり直す。
            ProbeState::Unreadable => return false,
        }
    }
    // 目印を読んでから削除するまでの間に、同じ名前へ別のファイルが置かれると、
    // それを消してしまう。いまは名前を指定して削除しているため、この短い区間は
    // 残る（docs にも書いた）。
    match std::fs::remove_file(path) {
        Ok(()) => true,
        // 照合の後、削除の前にボリュームが外れて `NotFound` になることがある。
        // 削除の直後にも記録した親に届いているときだけ、消えたとみなす。
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => reachable_parent(path, parent),
        Err(_) => false,
    }
}

/// 記録に残った検査用の名前が、このアプリが作ったものか。
enum ProbeState {
    /// 目印がある。消してよい。
    Ours,
    /// 目印が無い、通常のファイルではない、0 バイト、または読む権限が無い。
    /// 触らずに記録から外す。
    NotOurs,
    /// 一時的な I/O の失敗で読めない。次の起動でやり直す。
    Unreadable,
}

/// Windows で再解析ポイント（シンボリックリンクなど）自体を開くためのフラグ。
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

/// 検査用のファイルを、種類を確かめながら開く。種類の確認と `open` を分けると、
/// その間に第三者が FIFO やリンクへ差し替えられる。FIFO を開くと書き手が現れるまで
/// 戻らず、1 件で掃除のスレッドが止まると、残りの `.part` も消せなくなる。
fn open_probe(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // FIFO を開いても止まらないようにし、リンクはたどらない。
        options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

fn probe_state_from_open(
    result: std::io::Result<File>,
    path: &Path,
    parent: Option<ParentId>,
) -> ProbeState {
    match result {
        Ok(file) => probe_state_from_file(file, true),
        Err(err)
            if err.kind() == std::io::ErrorKind::NotFound && !reachable_parent(path, parent) =>
        {
            ProbeState::Unreadable
        }
        Err(err) => classify_open_result(Err(err), true),
    }
}

fn probe_state(path: &Path, parent: Option<ParentId>) -> ProbeState {
    probe_state_from_open(open_probe(path), path, parent)
}

/// 開く操作の結果を、掃除の判断に変える。失敗を直接渡せるよう分けている。
fn classify_open_result(result: std::io::Result<File>, recorded_probe: bool) -> ProbeState {
    match result {
        Ok(file) => probe_state_from_file(file, recorded_probe),
        Err(err) if is_transient_error(&err) => ProbeState::Unreadable,
        // 無い、権限が無い、リンクをたどれない、ソケットのように種類が違う、など。
        // 何度やり直しても結果は変わらないので、記録から外す。
        Err(_) => ProbeState::NotOurs,
    }
}

/// 開く・読むときの失敗が、次の起動でやり直す価値のある一時的なものか。
///
/// 資源の不足や割り込みだけを一時的な失敗として扱う。権限不足や存在しない等は、
/// 繰り返しても同じ結果になることが多く、無関係なファイルを延々と記録に残さない。
fn is_transient_error(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::Interrupted {
        return true;
    }
    #[cfg(unix)]
    if matches!(
        err.raw_os_error(),
        Some(libc::EIO | libc::EMFILE | libc::ENFILE | libc::ENOMEM | libc::ENOBUFS)
    ) {
        return true;
    }
    #[cfg(windows)]
    if matches!(err.raw_os_error(), Some(8 | 14 | 1450)) {
        // ERROR_NOT_ENOUGH_MEMORY, ERROR_OUTOFMEMORY, ERROR_NO_SYSTEM_RESOURCES
        return true;
    }
    false
}

/// 開いた検査用のファイルが、このアプリが書いたものか。
/// `recorded_probe` が真のときは、ジャーナルに登録済みの検査名として扱い、
/// 目印が不完全でも 0 バイトなら自分が作成途中だった可能性があるため回収対象にする。
fn probe_state_from_file(file: File, recorded_probe: bool) -> ProbeState {
    // 開いたものの種類と長さを、パスではなく開いたものから読む（差し替えの影響を
    // 受けない）。FIFO は `O_NONBLOCK` で開けるが、通常のファイルではない。
    let metadata = match file.metadata() {
        Ok(metadata) => metadata,
        Err(err) if is_transient_error(&err) => return ProbeState::Unreadable,
        Err(_) => return ProbeState::NotOurs,
    };
    if !metadata.file_type().is_file() {
        return ProbeState::NotOurs;
    }
    if metadata.len() == 0 {
        return if recorded_probe {
            // 記録済みの検査名で 0 バイトなら、自分が作成後に目印を書く前に終了した可能性がある。
            ProbeState::Ours
        } else {
            ProbeState::NotOurs
        };
    }
    if metadata.len() > PROBE_MARKER.len() as u64 {
        return ProbeState::NotOurs;
    }
    let mut marker = Vec::with_capacity(PROBE_MARKER.len() + 1);
    match file
        .take(PROBE_MARKER.len() as u64 + 1)
        .read_to_end(&mut marker)
    {
        Ok(_) => {}
        Err(err) if is_transient_error(&err) => return ProbeState::Unreadable,
        Err(_) => return ProbeState::NotOurs,
    }
    if marker == PROBE_MARKER {
        return ProbeState::Ours;
    }
    // 記録済み検査名で、目印の途中まで書かれた自分のファイルだけを回収対象にする。
    if recorded_probe && PROBE_MARKER.starts_with(&marker) {
        return ProbeState::Ours;
    }
    ProbeState::NotOurs
}

/// 拡張子を残して、名前がファイルシステムの上限（バイト数）に収まるようにする。
/// 文字数ではなくバイト数で切るので、日本語などの名前でも収まる。
fn truncate_name(name: String) -> String {
    if name.len() <= MAX_FILE_NAME_BYTES {
        return name;
    }
    let extension = Path::new(&name)
        .extension()
        .map(|ext| ext.to_string_lossy().into_owned());
    let suffix = extension
        .as_deref()
        .map(|ext| format!(".{ext}"))
        .filter(|ext| ext.len() < 16)
        .filter(|ext| name.ends_with(ext.as_str()))
        .unwrap_or_default();
    // 拡張子が長すぎて残せないときは、名前をそのまま切ると、切った位置で
    // `.exe` のような別の拡張子が生まれることがある（`{234 文字}.exe{20 文字}` を
    // `.exe` の直後で切るなど）。長い拡張子ごと落としてから語幹を切る。
    let dropped_extension = suffix.is_empty() && extension.is_some();
    let stem_source = if let Some(ext) = extension.as_deref().filter(|_| dropped_extension) {
        &name[..name.len() - ext.len() - 1]
    } else {
        &name[..name.len() - suffix.len()]
    };
    let budget = MAX_FILE_NAME_BYTES.saturating_sub(suffix.len());
    let mut stem = String::with_capacity(budget);
    for c in stem_source.chars() {
        if stem.len() + c.len_utf8() > budget {
            break;
        }
        stem.push(c);
    }
    let stem = stem.trim_end_matches(['.', ' ']);
    // 長い拡張子を落とした後、語幹にドットが残ると `report.exe` のように別の
    // 拡張子が露出する。語幹のドットも `_` にして、拡張子を作らない。
    let stem = if dropped_extension {
        stem.replace('.', "_")
    } else {
        stem.to_string()
    };
    if stem.trim_matches(' ').is_empty() {
        // 語幹が無くなる切り方しかしないなら、拡張子だけの隠しファイルにしない。
        let extension = suffix.trim_start_matches('.');
        return if extension.is_empty() {
            "attachment".into()
        } else {
            format!("attachment.{extension}")
        };
    }
    format!("{stem}{suffix}")
}

/// 先頭がドットだけの名前（`.pdf` など）に、語幹を足す。隠しファイルを作らないため。
fn ensure_stem(name: String) -> String {
    let stem_len = name.find('.').unwrap_or(name.len());
    if name[..stem_len].trim_matches(' ').is_empty() {
        format!("attachment{name}")
    } else {
        name
    }
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
/// `%` も置き換える。Windows の保存ダイアログは、`%APPDATA%` のような名前を環境変数として
/// 展開し、別の場所へ保存しようとすることがあるため。
pub fn sanitize_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let mut cleaned: String = base
        .chars()
        .map(|c| {
            if is_unsafe_char(c) || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '%') {
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
    let stem = normalize_reserved_stem(&stem);
    if RESERVED_NAMES.contains(&stem.as_str()) {
        cleaned.insert(0, '_');
    }
    truncate_name(ensure_stem(cleaned))
}

/// 確認なしの保存で、保存ダイアログに戻すべき名前か。許可リストに無い拡張子と、
/// 拡張子の無い名前は戻す。`sanitize_file_name` が先に語幹を足すので、ここへ来る名前は
/// ドットで始まらない。
fn needs_confirmation(name: &str) -> bool {
    Path::new(name)
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase())
        .is_none_or(|ext| !AUTO_SAVE_EXTENSIONS.contains(&ext.as_str()))
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

/// 確認なしの保存の保存先。利用者が選んだ設定フォルダーだけを使う。
/// 無いか届かなければ `None`（呼び出し側が保存ダイアログを出す）。
fn auto_save_target(configured: Option<&Path>, name: &str) -> Option<PathBuf> {
    let dir = configured?;
    dir.is_dir().then(|| unique_path(dir, name))
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

/// 保存ダイアログで利用者が上書きを選んだ場合の確定。既存のファイルを置き換える。
/// Windows の `std::fs::rename` も Unix と同じく既存のファイルを置き換える。
fn replace_file(part: &Path, target: &Path) -> std::io::Result<()> {
    std::fs::rename(part, target)
}

/// 保存先が既にあると失敗する形で、`part` を `target` へ移す。Unix の `rename` は既存の
/// ファイルを置き換えるため、`exists` の確認と `rename` の間の競合で他のファイルを消さないよう、
/// OS の「既にあれば失敗する移動」を使う。
fn rename_unique(part: &Path, target: &Path) -> std::io::Result<()> {
    match rename_no_replace(part, target) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(err),
        // ファイルシステムが「既にあれば失敗する移動」を持たない（古いネットワーク共有など）。
        // 別のアプリが作ったファイルも置き換えないよう、名前を原子的に確保してから移す。
        Err(err) if is_unsupported(&err) => move_without_replace(part, target),
        Err(err) => Err(err),
    }
}

/// 「既にあれば失敗する移動」が使えないファイルシステム向けの、上書きしない移動。
///
/// 同じフォルダーなので、ハードリンクで最終名を原子的に確保する（先にあれば
/// `AlreadyExists` で失敗する）。ハードリンクも使えない保存先では、既存のファイルを
/// 置き換えずに移す方法がないため、保存を失敗させる。名前を `create_new` で確保してから
/// `rename` する方法は、確保したファイルを別のアプリが置き換えた場合にそのファイルを
/// 上書きし、強制終了では 0 バイトのファイルを最終名に残すので使わない。
fn move_without_replace(part: &Path, target: &Path) -> std::io::Result<()> {
    link_without_replace(part, target, |from, to| std::fs::hard_link(from, to))
}

/// [`move_without_replace`] の本体。テストでハードリンクの失敗を再現できるよう、
/// リンクを作る関数を受け取る。
fn link_without_replace(
    part: &Path,
    target: &Path,
    hard_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    match hard_link(part, target) {
        Ok(()) => {
            // 中身は `target` から読める。`.part` は消すだけ。消せなくても保存は成立しているので、
            // 記録に残し、`finish` の後始末と起動時の掃除に任せる。
            let _ = std::fs::remove_file(part);
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Err(err),
        Err(err) if is_unsupported_destination(&err) => Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("この保存先では、既存のファイルを上書きせずに移せません: {err}"),
        )),
        // 権限や一時的な I/O の失敗。保存先の性質にせず、元のエラーを返す。
        Err(err) => Err(err),
    }
}

/// `.part` を作り、確定できる保存先かを確かめる。`spawn_blocking` の中で動く。
fn prepare_part(
    journal: &Arc<Journal>,
    part: &Path,
    final_path: &Path,
    deduplicate: bool,
) -> Result<(File, Vec<JournalEntry>)> {
    prepare_part_with(
        journal,
        part,
        final_path,
        deduplicate,
        rename_no_replace,
        |from, to| std::fs::hard_link(from, to),
    )
}

/// [`prepare_part`] の本体。テストで保存先の非対応を再現できるよう、確かめに使う
/// 関数を受け取る。
fn prepare_part_with(
    journal: &Arc<Journal>,
    part: &Path,
    final_path: &Path,
    deduplicate: bool,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
    hard_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<(File, Vec<JournalEntry>)> {
    // 記録を先に保存する。書けなければ始めない（残骸を次の起動で消せなくしないため）。
    journal.record(part)?;
    let recovered = journal.take_pending_cleanup();
    let recovered = recovered
        .into_iter()
        .filter(|entry| entry.path != part)
        .collect();
    // ダイアログは既にあるフォルダーを返し、決まったフォルダーは上の分岐で存在を
    // 確かめてある。ここで `create_dir_all` はしない。
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(part) {
        Ok(file) => file,
        Err(err) => {
            journal.forget(part);
            return Err(err)
                .with_context(|| format!("書き込み用のファイルを作れません: {}", part.display()));
        }
    };
    // 上書きの承認が無い保存は、確定時に既存のファイルを置き換えない。置き換えない
    // 移動を持たない保存先なら、本文を読み始める前に断る。
    if deduplicate && let Err(err) = check_destination_with(part, journal, &rename, &hard_link) {
        drop(file);
        journal.remove_part(part);
        return Err(err.context(format!("保存できません: {}", final_path.display())));
    }
    Ok((file, recovered))
}

/// この保存先で、既存のファイルを置き換えずに `.part` を確定できるかを、本文を読み始める
/// 前に確かめる。目印を書いた短い検査用のファイルを作り、確定と同じ順（「既にあれば
/// 失敗する移動」、使えなければハードリンク）で別の名前へ移す。`.part` は動かさない。
fn check_destination_with(
    part: &Path,
    journal: &Arc<Journal>,
    rename: impl Fn(&Path, &Path) -> std::io::Result<()>,
    hard_link: impl Fn(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let probe = part.with_extension("probe");
    let moved = part.with_extension("probe-moved");
    // 作る前に記録する。作成の直後に終了しても、次の起動で掃除できるようにするため。
    // 記録だけが先に残っても、掃除は目印（[`PROBE_MARKER`]）のあるものしか消さない。
    journal.record(&probe)?;

    let (created, moved_created, outcome) =
        match OpenOptions::new().write(true).create_new(true).open(&probe) {
            Ok(mut file) => {
                let created = true;
                let write_result = file.write_all(PROBE_MARKER);
                drop(file);
                let (moved_created, outcome) = if let Err(err) = write_result {
                    // 書きかけのファイルは自分が作ったものなので、消せれば消す。
                    // 消せなければ記録に残し、起動時の掃除で目印またはジャーナル登録を頼りに回収する。
                    if let Err(remove_err) = std::fs::remove_file(&probe)
                        && remove_err.kind() != std::io::ErrorKind::NotFound
                    {
                        return Err(anyhow::Error::new(err)
                            .context(format!("検査用ファイルを片付けられません: {}", remove_err)));
                    }
                    (false, Err(err.into()))
                } else {
                    match rename(&probe, &moved) {
                        Ok(()) => {
                            // 移動先の名前は、この保存が実際に作った後にだけ記録する。
                            // 先に記録すると、第三者が同名を置いていた場合に起動時掃除で消しうる。
                            match journal.record(&moved) {
                                Ok(()) => (true, Ok(())),
                                Err(err) => {
                                    let _ = std::fs::remove_file(&moved);
                                    (false, Err(err))
                                }
                            }
                        }
                        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                            (false, Err(err.into()))
                        }
                        // ファイルシステムが「既にあれば失敗する移動」を持たない。確定
                        // （[`rename_unique`]）と同じ順で、ハードリンクで名前を確保できるかを見る。
                        Err(err) if is_unsupported(&err) => match hard_link(&probe, &moved) {
                            Ok(()) => match journal.record(&moved) {
                                Ok(()) => (true, Ok(())),
                                Err(err) => {
                                    let _ = std::fs::remove_file(&moved);
                                    (false, Err(err))
                                }
                            },
                            Err(link) => {
                                // 保存先の性質による失敗だけを `UnsupportedDestination` にする。
                                // 権限や一時的な I/O の失敗は、保存先を変えても直らないことがある。
                                let err = if is_unsupported_destination(&link) {
                                    anyhow::Error::new(UnsupportedDestination)
                                        .context(format!("ハードリンクも作れません: {link}"))
                                } else {
                                    link.into()
                                };
                                (false, Err(err))
                            }
                        },
                        Err(err) => (false, Err(err.into())),
                    }
                };
                (created, moved_created, outcome)
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                (false, false, Err(err.into()))
            }
            Err(err) => (false, false, Err(err.into())),
        };

    // 自分で作った検査用の名前だけを片付ける。先にあった名前は第三者のものかもしれない
    // ので消さない。片付けられないまま本文を転送すると保存先に検査用のファイルが残るため、
    // 消せなかったときは失敗にする（記録には残るので、次の起動で消す）。両方を試す。
    let probe_cleaned = discard_probe(journal, &probe, created);
    let moved_cleaned = discard_probe(journal, &moved, moved_created);
    match (outcome, probe_cleaned, moved_cleaned) {
        (Ok(()), Ok(()), Ok(())) => Ok(()),
        (Err(err), _, _) => Err(err),
        (Ok(()), Err(err), _) => Err(err),
        (Ok(()), Ok(()), Err(err)) => Err(err),
    }
}

/// 検査用の名前を消し、記録から外す。`ours` が偽なら、この保存が作った名前ではないので
/// 触らずに記録だけ外す。消せなければ記録に残し、次の起動で再試行する。
fn discard_probe(journal: &Arc<Journal>, path: &Path, ours: bool) -> Result<()> {
    if !ours {
        journal.forget(path);
        return Ok(());
    }
    match std::fs::remove_file(path) {
        Ok(()) => {
            journal.forget(path);
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            journal.forget(path);
            Ok(())
        }
        Err(err) => Err(err.into()),
    }
}

/// 保存先が、原子性を保った「上書きしない移動」に対応していないことを表すエラー。
/// 権限や一時的な I/O の失敗は含めない（保存先のせいにせず、元のエラーを返す）。
fn is_unsupported_destination(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::Unsupported | std::io::ErrorKind::InvalidInput
    ) || raw_not_supported(err.raw_os_error())
}

/// FAT32 や exFAT のハードリンクは `EPERM`、対応しない `renameat2` / `renamex_np` は
/// `ENOTSUP` / `EOPNOTSUPP` を返す。
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn raw_not_supported(code: Option<i32>) -> bool {
    matches!(
        code,
        Some(code)
            if code == libc::EPERM || code == libc::ENOTSUP || code == libc::EOPNOTSUPP
    )
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn raw_not_supported(_code: Option<i32>) -> bool {
    false
}

fn is_unsupported(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::Unsupported | std::io::ErrorKind::InvalidInput
    )
}

/// 既存のファイルを置き換えずに `part` を `target` へ移す。途中までの内容が最終名で見えることはない。
#[cfg(target_os = "linux")]
fn rename_no_replace(part: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    fn cstring(path: &Path) -> std::io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    }
    let from = cstring(part)?;
    let to = cstring(target)?;
    // SAFETY: 両方の `CString` はこの呼び出しの間生きており、NUL で終わっている。
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// 既存のファイルを置き換えずに `part` を `target` へ移す。
#[cfg(target_os = "macos")]
fn rename_no_replace(part: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    fn cstring(path: &Path) -> std::io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))
    }
    let from = cstring(part)?;
    let to = cstring(target)?;
    // SAFETY: 両方の `CString` はこの呼び出しの間生きており、NUL で終わっている。
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// 既存のファイルを置き換えずに `part` を `target` へ移す。`MOVEFILE_REPLACE_EXISTING` を
/// 付けない `MoveFileExW` は、移す先があると `ERROR_ALREADY_EXISTS` で失敗する。
/// 長いパスの設定が無効な環境でも 260 文字を超える保存先へ移せるよう、`\\?\` 付きの
/// 形にして渡す（`std::fs::rename` と同じ扱い）。
#[cfg(target_os = "windows")]
fn rename_no_replace(part: &Path, target: &Path) -> std::io::Result<()> {
    use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
    use windows::core::PCWSTR;

    let from = wide_extended_path(part);
    let to = wide_extended_path(target);
    // SAFETY: どちらも NUL で終わるバッファーで、呼び出しの間生きている。
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|err| std::io::Error::from_raw_os_error(err.code().0 & 0xFFFF))
}

/// `MoveFileExW` に渡す、長いパス対応（`\\?\`）の UTF-16 文字列（NUL 終端）。
/// `to_string_lossy()` は孤立サロゲートを U+FFFD に変えて別の名前を指してしまうので、
/// OS の文字列をそのまま使う。
#[cfg(target_os = "windows")]
fn wide_extended_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    const PREFIX: [u16; 4] = [b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const UNC_PREFIX: [u16; 8] = [
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];
    const DEVICE_PREFIX: [u16; 4] = [b'\\' as u16, b'?' as u16, b'?' as u16, b'\\' as u16];

    let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
    let wide: Vec<u16> = if raw.starts_with(&PREFIX) {
        raw
    } else if raw.starts_with(&[b'\\' as u16, b'\\' as u16]) {
        // UNC（`\\server\share`）は `\\?\UNC\server\share` にする。
        UNC_PREFIX
            .into_iter()
            .chain(raw[2..].iter().copied())
            .collect()
    } else if raw.starts_with(&DEVICE_PREFIX) {
        PREFIX.into_iter().chain(raw[4..].iter().copied()).collect()
    } else {
        PREFIX.into_iter().chain(raw).collect()
    };
    wide.into_iter().chain(std::iter::once(0)).collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn rename_no_replace(_part: &Path, _target: &Path) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
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
            dir.join(numbered_name(stem, n, extension))
        };
        match rename_unique(part, &candidate) {
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

/// 衝突したときの ` (n)` 付きの名前。語幹が長いときは、連番と拡張子の分を残して切る。
/// ファイルシステムの上限を超えると、確定できないまま `.part` を消すことになるため。
fn numbered_name(stem: &str, n: u32, extension: &str) -> String {
    let suffix = format!(" ({n})");
    let budget = MAX_NAME_BYTES.saturating_sub(suffix.len() + extension.len());
    let mut base = String::with_capacity(budget);
    for c in stem.chars() {
        if base.len() + c.len_utf8() > budget {
            break;
        }
        base.push(c);
    }
    let mut name = format!("{}{suffix}{extension}", base.trim_end_matches(['.', ' ']));
    if name.len() > MAX_NAME_BYTES {
        // 拡張子だけで上限に近く、語幹を削っても収まらない名前。末尾を切ってでも収める
        // （語幹が空なら連番は先頭に残る）。
        let mut end = MAX_NAME_BYTES;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

/// Windows で、保存したファイルに Mark of the Web（Zone 3）を付ける。ほかの利用者が送った
/// `.exe` やマクロ付きの文書を開いたときに、SmartScreen と Office の保護ビューが働くようにする
/// （WebView2 の既定のダウンロードと同じ扱い）。付けられなくても保存は成功させる。
#[cfg(target_os = "windows")]
fn mark_of_the_web(path: &Path) -> bool {
    let mut zone = path.as_os_str().to_os_string();
    zone.push(":Zone.Identifier");
    std::fs::write(PathBuf::from(zone), b"[ZoneTransfer]\r\nZoneId=3\r\n").is_ok()
}

#[cfg(not(target_os = "windows"))]
fn mark_of_the_web(_path: &Path) -> bool {
    true
}

fn same_saved_file(source: &std::fs::Metadata, saved: &Path) -> bool {
    same_file_as_path(source, saved)
}

#[cfg(unix)]
fn same_file_metadata(source: &std::fs::Metadata, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.dev() == source.dev() && meta.ino() == source.ino()
}

#[cfg(windows)]
fn same_file_metadata(source: &std::fs::Metadata, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    // Windows のファイルシステムトンネリングにより、削除・改名直後の同名ファイルの
    // 作成日時が元のファイルから引き継がれることがある。作成日時は比較せず、
    // サイズと更新時刻だけで確認する。
    meta.file_size() == source.file_size() && meta.last_write_time() == source.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file_metadata(_source: &std::fs::Metadata, _meta: &std::fs::Metadata) -> bool {
    true
}

#[cfg(unix)]
fn same_file_as_path(source: &std::fs::Metadata, path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| same_file_metadata(source, &meta))
        .unwrap_or(false)
}

#[cfg(windows)]
fn same_file_as_path(source: &std::fs::Metadata, path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| same_file_metadata(source, &meta))
        .unwrap_or(false)
}

#[cfg(not(any(unix, windows)))]
fn same_file_as_path(_source: &std::fs::Metadata, _path: &Path) -> bool {
    true
}

/// `finish` の実体。`spawn_blocking` の中で動く。
fn finish_entry(entry: Pending) -> Result<PathBuf> {
    finish_entry_with(entry, move_unique)
}

/// [`finish_entry`] の本体。テストで「上書きしない移動」の失敗を再現できるよう、
/// 確定に使う関数を受け取る。
fn finish_entry_with(
    entry: Pending,
    move_unique: impl Fn(&Path, &Path) -> std::io::Result<PathBuf>,
) -> Result<PathBuf> {
    // 中止と確定が同時に来ても、ファイルのロックの中で決める。中止はロックの前で
    // `cancelled` を立てるので、ここで見てから確定するまでの間に取り消されることはない。
    // ロックが poison されている（書き込みタスクがパニックした）場合も、不完全なファイルを
    // 確定しない。
    let lock_result = entry.file.lock();
    let poisoned = lock_result.is_err();
    let mut slot = lock_result.unwrap_or_else(PoisonError::into_inner);
    if poisoned {
        drop(slot.take());
        drop(slot);
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!("書き込みに失敗したダウンロードです"));
    }
    if entry.cancelled.load(Ordering::SeqCst) {
        drop(slot.take());
        drop(slot);
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!("ダウンロードは中止されました"));
    }
    if entry.failed.load(Ordering::SeqCst) {
        // 途中までの内容は確定しない。
        drop(slot.take());
        drop(slot);
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!("書き込みに失敗したダウンロードです"));
    }
    let Some(file) = slot.take() else {
        drop(slot);
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!("ダウンロードは中止されました"));
    };
    drop(slot);
    let source_meta = file.metadata().map_err(|err| {
        let _ = std::fs::remove_file(&entry.part);
        anyhow::Error::new(err).context("保存したファイルを確認できません")
    })?;
    let synced = file.sync_all();
    drop(file);
    if let Err(err) = synced {
        let _ = std::fs::remove_file(&entry.part);
        return Err(err.into());
    }
    let current_meta = std::fs::metadata(&entry.part).map_err(|err| {
        let _ = std::fs::remove_file(&entry.part);
        anyhow::Error::new(err).context("保存したファイルを確認できません")
    })?;
    if !same_file_metadata(&source_meta, &current_meta) {
        let _ = std::fs::remove_file(&entry.part);
        return Err(anyhow!(
            "保存先の一時ファイルを確認できません。もう一度お試しください: {}",
            entry.part.display()
        ));
    }
    let saved = if entry.deduplicate {
        move_unique(&entry.part, &entry.final_path)
    } else {
        replace_file(&entry.part, &entry.final_path).map(|()| entry.final_path.clone())
    };
    let saved = saved.map_err(|err| {
        let _ = std::fs::remove_file(&entry.part);
        if is_unsupported_destination(&err) {
            anyhow::Error::new(UnsupportedDestination)
                .context(format!("保存できません: {}", entry.final_path.display()))
        } else {
            anyhow::Error::new(err)
                .context(format!("保存できません: {}", entry.final_path.display()))
        }
    })?;
    if !same_saved_file(&source_meta, &saved) {
        let _ = std::fs::remove_file(&entry.part);
        // ファイルシステムトンネリングなどで不一致になることがあるが、確定したファイルは
        // 消さない（利用者が保存したファイルを削除しないため）。
        return Err(anyhow!(
            "保存先の確認に失敗しました。もう一度お試しください: {}",
            saved.display()
        ));
    }
    if !mark_of_the_web(&saved) {
        #[cfg(target_os = "windows")]
        eprintln!(
            "警告: Mark of the Web を付与できませんでした: {}",
            saved.display()
        );
    }
    Ok(saved)
}

/// 添付の大きさから、1 件の上限を決める。分からないときは既定値、極端に大きい値でも上限で抑える。
fn download_limit(size: Option<u64>) -> u64 {
    match size {
        Some(size) => size
            .saturating_add(DOWNLOAD_SIZE_SLACK)
            .clamp(DEFAULT_MAX_DOWNLOAD_BYTES, MAX_DOWNLOAD_BYTES_CEILING),
        None => DEFAULT_MAX_DOWNLOAD_BYTES,
    }
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
            failed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zoo2-downloads-{}", new_download_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// テスト用。利用者の `download_parts.json` を読み書きしないよう、記録も一時ディレクトリに置く。
    fn test_downloads() -> Downloads {
        let dir = temp_dir();
        let mut downloads = Downloads::with_journal(Some(dir.join("download_parts.json")));
        downloads.test_dir = Some(dir);
        downloads
    }

    #[test]
    fn sanitize_keeps_a_plain_name() {
        assert_eq!(sanitize_file_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_file_name("議事録 2026.docx"), "議事録 2026.docx");
    }

    #[test]
    fn numbered_names_fit_the_filesystem_limit() {
        // 保存ダイアログで 255 バイトの名前を選んでも、連番を足した候補が上限を超えない。
        let stem = "a".repeat(MAX_NAME_BYTES - 4);
        let candidate = numbered_name(&stem, 1, ".pdf");
        assert!(candidate.len() <= MAX_NAME_BYTES);
        assert!(candidate.ends_with(" (1).pdf"));
        assert_eq!(numbered_name("report", 2, ".pdf"), "report (2).pdf");

        // 拡張子だけで余裕が無い名前でも、同じ連番なら同じ長さに収まる。
        let long_extension = format!(".{}", "x".repeat(MAX_NAME_BYTES - 2));
        assert!(numbered_name("a", 9999, &long_extension).len() <= MAX_NAME_BYTES);
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
    fn sanitize_gives_a_stem_to_a_name_that_is_only_an_extension() {
        assert_eq!(sanitize_file_name(".pdf"), "attachment.pdf");
        assert_eq!(sanitize_file_name(".bashrc"), "attachment.bashrc");
    }

    #[test]
    fn sanitize_gives_a_stem_when_only_spaces_or_dots_remain() {
        assert_eq!(
            sanitize_file_name(&format!("{}.pdf", " ".repeat(300))),
            "attachment.pdf"
        );
        assert_eq!(
            sanitize_file_name(&format!("{}.pdf", ".".repeat(300))),
            "attachment.pdf"
        );
        assert_eq!(sanitize_file_name(&".".repeat(300)), "attachment");
    }

    #[test]
    fn unusable_settings_go_back_to_asking_every_time() {
        let dir = temp_dir();
        let picked = DownloadSettings {
            ask_every_time: false,
            directory: Some(dir.display().to_string()),
        };
        assert_eq!(usable_settings(picked.clone()), picked);
        // フォルダーが実在するかは、起動を待たせないよう、ここでは見ない。
        // 保存のたびに `auto_save_target` が確かめ、無ければダイアログに戻す。
        let missing = DownloadSettings {
            ask_every_time: false,
            directory: Some(dir.join("missing").display().to_string()),
        };
        assert_eq!(usable_settings(missing.clone()), missing);
        assert!(auto_save_target(missing.directory.as_deref().map(Path::new), "a.txt").is_none());
        // `downloads.json` を手で書き換えた場合。相対パスは保存先に使わない。
        assert_eq!(
            usable_settings(DownloadSettings {
                ask_every_time: false,
                directory: Some("Downloads".into()),
            }),
            DownloadSettings::default()
        );
        assert_eq!(
            auto_save_target(Some(&dir), "a.txt"),
            Some(dir.join("a.txt"))
        );
        // フォルダーが無い設定は、手で書き換えられていても都度確認へ戻す。
        assert_eq!(
            usable_settings(DownloadSettings {
                ask_every_time: false,
                directory: None,
            }),
            DownloadSettings::default()
        );
        assert!(auto_save_target(None, "a.txt").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
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

        // 重複したときの ` (9999)` を足しても収まる。
        assert!(format!("{cleaned} (9999)").len() <= 255);
    }

    #[test]
    fn truncation_does_not_create_a_new_extension() {
        // 語幹 234 文字 + `.exe` + 20 文字の拡張子。全体は上限を超え、拡張子は長すぎて残せない。
        // 単純に切ると `.exe` の直後で切れ、許可リストの検査をすり抜ける `.exe` が生まれる。
        let name = format!("{}.exe{}", "a".repeat(234), "x".repeat(20));
        assert!(name.len() > MAX_FILE_NAME_BYTES);
        let cleaned = sanitize_file_name(&name);
        assert!(cleaned.len() <= MAX_FILE_NAME_BYTES);
        assert!(!cleaned.to_ascii_lowercase().ends_with(".exe"), "{cleaned}");
        assert_eq!(cleaned, "a".repeat(234));

        // 語幹にもドットがあると、長い拡張子を落とした後に `report.exe` や
        // `report.pdf` が露出する。`report.pdf` は自動保存の対象になってしまう。
        assert_eq!(
            sanitize_file_name(&format!("report.exe.{}", "x".repeat(240))),
            "report_exe"
        );
        assert_eq!(
            sanitize_file_name(&format!("report.pdf.{}", "x".repeat(240))),
            "report_pdf"
        );
        // 拡張子が短ければ、これまでどおり保つ。
        assert_eq!(sanitize_file_name("report.pdf"), "report.pdf");
    }

    #[test]
    fn sanitize_replaces_percent_so_the_save_dialog_does_not_expand_it() {
        // Windows の保存ダイアログは `%APPDATA%` を環境変数として展開することがある。
        assert_eq!(sanitize_file_name("%APPDATA%.pdf"), "_APPDATA_.pdf");
        assert_eq!(sanitize_file_name("report%.pdf"), "report_.pdf");
    }

    #[test]
    fn part_path_does_not_include_the_final_name() {
        let dir = Path::new("C:\\Downloads");
        let target = dir.join(format!("{}.pdf", "a".repeat(250)));
        let part = part_path(&target, "0123456789abcdef");
        assert_eq!(part, dir.join(".zoo-0123456789abcdef.part"));
        // 保存ダイアログで長い名前を選んでも、`.part` はファイルシステムの上限に当たらない。
        assert!(part.file_name().unwrap().len() <= 255);
    }

    #[test]
    fn unique_path_adds_a_number_when_the_name_exists() {
        let dir = temp_dir();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (1).txt"));
        std::fs::write(dir.join("a (1).txt"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.txt"), dir.join("a (2).txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn is_part_file_name_matches_only_the_temporary_names() {
        assert!(is_part_file_name(".zoo-0123456789abcdef.part"));
        assert!(is_part_file_name(".zoo-0123456789abcdef.probe"));
        assert!(is_part_file_name(".zoo-0123456789abcdef.probe-moved"));
        assert!(!is_part_file_name("a.txt"));
        assert!(!is_part_file_name(".zoo-.part"));
        assert!(!is_part_file_name(".zoo-0123.part"));
        assert!(!is_part_file_name(".zoo-0123456789abcdeg.part"));
        // 完成したファイルの名前は消さない。
        assert!(!is_part_file_name("zoo-0123456789abcdef.part"));
        assert!(!is_part_file_name(".zoo-0123456789abcdef.txt"));
    }

    #[test]
    fn journal_keeps_a_part_it_could_not_remove() {
        let dir = temp_dir();
        let journal = Arc::new(Journal::load(Some(dir.join("download_parts.json"))));
        let part = dir.join(".zoo-0123456789abcdef.part");
        std::fs::write(&part, b"x").unwrap();
        journal.record(&part).unwrap();
        assert_eq!(journal.snapshot(), vec![part.clone()]);
        journal.remove_part(&part);
        assert!(journal.snapshot().is_empty());

        // 消せなかったものは記録に残し、次の起動で再試行する。
        let stuck = dir.join("stuck");
        std::fs::create_dir_all(&stuck).unwrap();
        journal.record(&stuck).unwrap();
        journal.remove_part(&stuck);
        assert_eq!(journal.snapshot(), vec![stuck.clone()]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_directory_is_not_saved_as_a_different_path() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let path = PathBuf::from(OsString::from_vec(b"caf\xe9".to_vec()));
        assert!(directory_text(&path).is_err());
        assert_eq!(
            directory_text(Path::new("/tmp/downloads")).unwrap(),
            "/tmp/downloads"
        );
    }

    #[test]
    fn journal_reads_the_previous_string_format() {
        let dir = temp_dir();
        let file = dir.join("download_parts.json");
        let part = dir.join(".zoo-0123456789abcdef.part");
        let old: Vec<String> = vec![part.to_string_lossy().into_owned()];
        std::fs::write(&file, serde_json::to_vec(&old).unwrap()).unwrap();

        let journal = Journal::load(Some(file));
        assert_eq!(journal.snapshot(), vec![part]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unreadable_journal_is_not_overwritten() {
        let dir = temp_dir();
        let file = dir.join("download_parts.json");
        let old = b"not json";
        std::fs::write(&file, old).unwrap();

        let journal = Journal::load(Some(file.clone()));
        let part = dir.join(".zoo-0123456789abcdef.part");
        assert!(journal.record(&part).is_ok());
        assert!(
            file.with_extension("json.corrupt-0").exists()
                || file.parent().unwrap().read_dir().unwrap().any(|entry| {
                    entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .contains("json.corrupt-")
                }),
            "壊れた記録は退避する"
        );

        // 読み直せるようになれば、新しい記録を書ける。
        let file2 = dir.join("download_parts2.json");
        std::fs::write(&file2, b"[]").unwrap();
        let journal = Arc::new(Journal::load(Some(file2)));
        assert!(journal.record(&part).is_ok());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn journal_keeps_non_utf8_bytes_in_a_path() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let dir = temp_dir();
        let file = dir.join("download_parts.json");
        let folder = dir.join(OsString::from_vec(b"caf\xe9".to_vec()));
        let part = folder.join(".zoo-0123456789abcdef.part");
        let journal = Arc::new(Journal::load(Some(file.clone())));
        // APFS は不正な UTF-8 の名前を作れないので、ディレクトリは作らずにパスだけを記録する。
        // このテストで確かめたいのはパスの往復で、`record` は親 ID を `None` として扱う。
        journal.record(&part).unwrap();

        // 再読み込みしても `?` に置き換わらず、元のバイト列で戻る。
        let reloaded = Journal::load(Some(file));
        assert_eq!(reloaded.snapshot(), vec![part]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_move_uses_extended_length_paths() {
        fn text(path: &Path) -> String {
            let wide = wide_extended_path(path);
            assert_eq!(wide.last(), Some(&0));
            String::from_utf16(&wide[..wide.len() - 1]).unwrap()
        }
        assert_eq!(
            text(Path::new(r"C:\Users\a\a very long file name.txt")),
            r"\\?\C:\Users\a\a very long file name.txt"
        );
        assert_eq!(
            text(Path::new(r"\\server\share\a.txt")),
            r"\\?\UNC\server\share\a.txt"
        );
        assert_eq!(text(Path::new(r"\\?\C:\a.txt")), r"\\?\C:\a.txt");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_move_keeps_unpaired_surrogates() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};

        let name = OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xD800]);
        let wide = wide_extended_path(&PathBuf::from(name));
        assert!(wide.starts_with(&[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16]));
        assert!(wide.contains(&0xD800));
        assert_eq!(wide.last(), Some(&0));
    }

    #[tokio::test]
    async fn write_finish_and_cancel_manage_the_part_file() {
        let dir = temp_dir();
        let downloads = test_downloads();
        let final_path = dir.join("saved.txt");

        let id = new_download_id();
        let entry = pending(&final_path, &id, false, DEFAULT_MAX_DOWNLOAD_BYTES);
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
            DEFAULT_MAX_DOWNLOAD_BYTES,
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
        let dir = temp_dir();
        let downloads = test_downloads();
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

    #[tokio::test]
    async fn a_failed_write_cannot_be_finished() {
        let dir = temp_dir();
        let downloads = test_downloads();
        let id = new_download_id();
        let entry = pending(
            &dir.join("broken.txt"),
            &id,
            false,
            DEFAULT_MAX_DOWNLOAD_BYTES,
        );
        let part = entry.part.clone();
        std::fs::write(&part, b"half").unwrap();
        entry.failed.store(true, Ordering::SeqCst);
        downloads.pending.lock().unwrap().insert(id.clone(), entry);
        assert!(downloads.finish(&id).await.is_err());
        assert!(!dir.join("broken.txt").exists());
        assert!(!part.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_fixed_folder_saves_do_not_overwrite_each_other() {
        let dir = temp_dir();
        let downloads = test_downloads();
        let target = dir.join("same.txt");

        // 同時に始めた 2 件は、どちらも同じ名前を選ぶ（まだファイルがないため）。
        let mut ids = Vec::new();
        for _ in 0..2 {
            let id = new_download_id();
            downloads.pending.lock().unwrap().insert(
                id.clone(),
                pending(&target, &id, true, DEFAULT_MAX_DOWNLOAD_BYTES),
            );
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
        let dir = temp_dir();
        let part = dir.join(".zoo-0123456789abcdef.part");
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

    #[test]
    fn shutdown_removes_pending_parts() {
        let dir = temp_dir();
        let downloads = test_downloads();
        let id = new_download_id();
        let target = dir.join("shutdown.txt");
        let entry = pending(&target, &id, true, DEFAULT_MAX_DOWNLOAD_BYTES);
        downloads.journal.record(&entry.part).unwrap();
        let part = entry.part.clone();
        downloads.pending.lock().unwrap().insert(id.clone(), entry);

        downloads.shutdown(Duration::from_secs(1));

        assert!(!part.exists());
        assert!(downloads.pending.lock().unwrap().is_empty());
        assert!(downloads.journal.snapshot().is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shutdown_waits_for_a_cancel_that_is_waiting_for_the_file() {
        let dir = temp_dir();
        let downloads = Arc::new(test_downloads());
        let id = new_download_id();
        let target = dir.join("cancel-wait.txt");
        let entry = pending(&target, &id, true, DEFAULT_MAX_DOWNLOAD_BYTES);
        downloads.journal.record(&entry.part).unwrap();
        let part = entry.part.clone();
        let file = Arc::clone(&entry.file);
        downloads.pending.lock().unwrap().insert(id.clone(), entry);

        // 書き込み中のハンドルを握ったまま、取消の後始末をファイルのロックで止める。
        let guard = file.lock().unwrap();
        let cancel = {
            let downloads = Arc::clone(&downloads);
            let id = id.clone();
            std::thread::spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(downloads.cancel(&id));
            })
        };
        // 取消が `pending` から取り出し、後始末のスレッドがロック待ちに入るまで待つ。
        let started = Instant::now();
        while downloads.cleanups.running() == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "取消の後始末が始まりません"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let shutdown = {
            let downloads = Arc::clone(&downloads);
            std::thread::spawn(move || downloads.shutdown(Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !shutdown.is_finished(),
            "取消の後始末を待たずに終了しました"
        );

        drop(guard);
        shutdown.join().unwrap();
        cancel.join().unwrap();

        assert!(!part.exists());
        assert!(downloads.journal.snapshot().is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_drop_removes_the_journal_dir_after_a_cancel_finishes() {
        let dir = temp_dir();
        let mut downloads = Downloads::with_journal(Some(dir.join("download_parts.json")));
        downloads.test_dir = Some(dir.clone());
        let id = new_download_id();
        let entry = pending(&dir.join("drop.txt"), &id, true, DEFAULT_MAX_DOWNLOAD_BYTES);
        downloads.journal.record(&entry.part).unwrap();
        let file = Arc::clone(&entry.file);
        downloads.pending.lock().unwrap().insert(id, entry);

        // 後始末スレッドをファイルのロック待ちで止め、その間に `Drop` を走らせる。
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = {
            let file = Arc::clone(&file);
            std::thread::spawn(move || {
                let _guard = file.lock().unwrap();
                locked_tx.send(()).unwrap();
                let _ = release_rx.recv();
            })
        };
        locked_rx.recv().unwrap();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(downloads);
            done_tx.send(()).unwrap();
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !dropper.is_finished(),
            "後始末を待たずにディレクトリを消した"
        );
        release_tx.send(()).unwrap();
        dropper.join().unwrap();
        holder.join().unwrap();
        done_rx.recv().unwrap();

        assert!(
            !dir.exists(),
            "後始末が記録を書いた後にディレクトリが残った"
        );
    }

    #[test]
    fn shutdown_waits_for_a_finish_that_took_the_entry() {
        let dir = temp_dir();
        let downloads = Arc::new(test_downloads());
        let id = new_download_id();
        let target = dir.join("finish-wait.txt");
        let entry = pending(&target, &id, true, DEFAULT_MAX_DOWNLOAD_BYTES);
        downloads.journal.record(&entry.part).unwrap();
        let part = entry.part.clone();
        let file = Arc::clone(&entry.file);
        downloads.pending.lock().unwrap().insert(id.clone(), entry);

        // 確定がファイルのロック待ちに入る前に、後始末の数へ入ることを確かめる。
        let guard = file.lock().unwrap();
        let finish = {
            let downloads = Arc::clone(&downloads);
            let id = id.clone();
            std::thread::spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(downloads.finish(&id))
            })
        };
        let started = Instant::now();
        while downloads.cleanups.running() == 0 {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "確定の後始末が始まりません"
            );
            std::thread::sleep(Duration::from_millis(5));
        }

        let shutdown = {
            let downloads = Arc::clone(&downloads);
            std::thread::spawn(move || downloads.shutdown(Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            !shutdown.is_finished(),
            "確定の後始末を待たずに終了しました"
        );

        drop(guard);
        shutdown.join().unwrap();
        let saved = finish.join().unwrap().unwrap();

        assert_eq!(saved, target);
        assert!(saved.exists());
        assert!(!part.exists());
        assert!(downloads.journal.snapshot().is_empty());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn cancelled_downloads_are_not_finished() {
        let dir = temp_dir();
        let downloads = test_downloads();
        let id = new_download_id();
        let entry = pending(
            &dir.join("cancelled.txt"),
            &id,
            true,
            DEFAULT_MAX_DOWNLOAD_BYTES,
        );
        downloads.pending.lock().unwrap().insert(id.clone(), entry);
        downloads.cancel_all();
        assert!(downloads.finish(&id).await.is_err());
        assert!(!dir.join("cancelled.txt").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn replace_file_overwrites_an_existing_file() {
        let dir = temp_dir();
        let part = dir.join(".zoo-0123456789abcdef.part");
        let target = dir.join("x.txt");
        std::fs::write(&part, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        replace_file(&part, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!part.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_safe_names_are_saved_without_the_dialog() {
        assert!(!needs_confirmation("report.pdf"));
        assert!(!needs_confirmation("photo.JPG"));
        assert!(!needs_confirmation("memo.txt"));
        assert!(!needs_confirmation("clip.mp4"));
        assert!(!needs_confirmation("report.docx"));
        // 開くとインタープリターや読み込みに進む形式は、保存ダイアログに戻す。
        for name in [
            "setup.exe",
            "setup.EXE",
            "script.bat",
            "autorun.desktop",
            "image.py",
            "macro.docm",
            "macro.xlsm",
            "legacy.doc",
            "legacy.xls",
            "legacy.ppt",
            // OpenDocument は Basic や Python のマクロを埋め込める。
            "invoice.odt",
            "invoice.ods",
            "invoice.odp",
            "lib.dll",
            "screen.scf",
            "app.AppImage",
            "disk.dmg",
            "page.html",
            "vector.svg",
            "book.epub",
            "note.rtf",
            "no-extension",
        ] {
            assert!(needs_confirmation(name), "{name}");
        }
        // `sanitize_file_name` が語幹を足すので、ここへ来る名前はドットで始まらない。
        assert!(needs_confirmation(&sanitize_file_name(".bashrc")));
    }

    #[test]
    fn the_dialog_overwrite_is_only_approved_when_the_name_existed_at_selection() {
        // 選んだ時点で既にあった: ダイアログが上書きを確認し、利用者が承認している。
        assert!(!SaveDecision::dialog(true).deduplicate);
        // 無かった: 転送中にできた同名のファイルは置き換えない。
        assert!(SaveDecision::dialog(false).deduplicate);
        // 決まったフォルダーへの保存: 常に置き換えない（始めた時点で同名は無い扱いと同じ）。
        assert_eq!(SaveDecision::AUTO, SaveDecision::dialog(false));
    }

    #[test]
    fn settings_update_uses_the_latest_settings() {
        let dir = temp_dir();
        let current = DownloadSettings {
            ask_every_time: true,
            directory: Some(dir.display().to_string()),
        };
        let off = DownloadSettingsUpdate {
            ask_every_time: false,
            reset_directory: false,
        };
        // 承認された: 待つ前ではなく、その時点の設定へ今回の変更だけを適用する。
        let next = apply_settings_update(&current, true, true, &off).unwrap();
        assert!(!next.ask_every_time);
        assert_eq!(next.directory, current.directory);
        // 承認されなかった: オンのまま。
        assert!(
            apply_settings_update(&current, true, false, &off)
                .unwrap()
                .ask_every_time
        );
        // 待っている間に確認の要否が変わった: 承認は今の設定に使えないので、出し直す。
        assert!(apply_settings_update(&current, false, true, &off).is_none());
        // 確認なしの設定でフォルダーを消すと、自動保存先が無くなるので拒否する。
        let saved = DownloadSettings {
            ask_every_time: false,
            directory: Some(dir.display().to_string()),
        };
        let reset = DownloadSettingsUpdate {
            ask_every_time: false,
            reset_directory: true,
        };
        let without_directory = apply_settings_update(&saved, false, true, &reset).unwrap();
        assert!(automatic_save_needs_directory(&without_directory).is_err());
        // 都度確認へ戻すなら、フォルダーを消してよい。
        let reset_to_default = DownloadSettingsUpdate {
            ask_every_time: true,
            reset_directory: true,
        };
        let default = apply_settings_update(&saved, false, true, &reset_to_default).unwrap();
        assert!(automatic_save_needs_directory(&default).is_ok());
        assert!(default.directory.is_none());
        // フォルダーが無いまま確認をオフにするのも拒否する。
        let off_without_directory = DownloadSettingsUpdate {
            ask_every_time: false,
            reset_directory: false,
        };
        assert!(
            automatic_save_needs_directory(
                &apply_settings_update(&default, true, true, &off_without_directory).unwrap()
            )
            .is_err()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn move_without_replace_does_not_remove_another_apps_file() {
        let dir = temp_dir();
        let part = dir.join(".zoo-0123456789abcdef.part");
        let target = dir.join("x.txt");
        std::fs::write(&part, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();

        // 既にある: 置き換えずに `AlreadyExists`。
        assert_eq!(
            move_without_replace(&part, &target).unwrap_err().kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(part.exists());

        // 無い: 移す。
        std::fs::remove_file(&target).unwrap();
        move_without_replace(&part, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!part.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn filesystems_without_hard_links_fail_instead_of_overwriting() {
        let dir = temp_dir();
        let part = dir.join(".zoo-0123456789abcdef.part");
        let target = dir.join("x.txt");
        std::fs::write(&part, b"new").unwrap();

        // ハードリンクを使えない保存先（古い共有フォルダーなど）を再現する。
        // 名前を確保してから移す方法は使わず、上書きしない保証がないので失敗させる。
        let err = link_without_replace(&part, &target, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
        })
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        assert!(!target.exists());
        assert_eq!(std::fs::read(&part).unwrap(), b"new");

        // 既にある場合の `AlreadyExists` は、番号付きの名前で再試行できるようそのまま返す。
        let err = link_without_replace(&part, &target, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists))
        })
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn raw_os_errors_of_unsupported_filesystems_are_recognized() {
        // `EPERM` は権限の失敗として `PermissionDenied` になるが、FAT32 のハードリンクの
        // ように、保存先が対応していない意味でも返る。`raw_os_error` も見る。
        for code in [libc::EPERM, libc::ENOTSUP, libc::EOPNOTSUPP] {
            let err = std::io::Error::from_raw_os_error(code);
            assert!(is_unsupported_destination(&err), "{code}");
        }
        assert!(!is_unsupported_destination(
            &std::io::Error::from_raw_os_error(libc::EIO)
        ));

        // ハードリンクの失敗も、この判定を通して保存先の非対応にする。
        let dir = temp_dir();
        let part = dir.join(".zoo-0123456789abcdef.part");
        let target = dir.join("x.txt");
        std::fs::write(&part, b"new").unwrap();
        let err = link_without_replace(&part, &target, |_, _| {
            Err(std::io::Error::from_raw_os_error(libc::EPERM))
        })
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
        assert!(!target.exists());
        assert_eq!(std::fs::read(&part).unwrap(), b"new");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finish_removes_the_part_when_the_destination_cannot_save() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let entry = pending(&target, "0123456789abcdef", true, 1024);
        let part = entry.part.clone();
        {
            use std::io::Write;
            let mut slot = entry.file.lock().unwrap();
            slot.as_mut().unwrap().write_all(b"new").unwrap();
        }

        // `renameat2` が使えず、ハードリンクも使えない保存先を再現する。確定の失敗で
        // `.part` を残さず、最終名も作らない。
        let err = finish_entry_with(entry, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
        })
        .unwrap_err();
        assert!(err.downcast_ref::<UnsupportedDestination>().is_some());
        assert!(!target.exists());
        assert!(!part.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn begin_refuses_a_destination_that_cannot_move_without_replacing() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let journal = Arc::new(Journal::load(Some(dir.join("download_parts.json"))));
        let part = part_path(&target, "0123456789abcdef");

        // `begin` と同じ準備を通す。`renameat2` もハードリンクも使えない保存先では、
        // `.part` も記録も残さず、`UnsupportedDestination` で断る。
        let err = prepare_part_with(
            &journal,
            &part,
            &target,
            true,
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<UnsupportedDestination>().is_some());
        assert!(!part.exists());
        assert!(!target.exists());
        assert!(journal.snapshot().is_empty());
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            0,
            "検査用のファイルを残さない"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn check_destination_uses_the_same_order_as_the_finish() {
        let dir = temp_dir();
        let journal = Arc::new(Journal::load(Some(dir.join("download_parts.json"))));
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        std::fs::write(&part, b"").unwrap();
        let probe = part.with_extension("probe");
        let moved = part.with_extension("probe-moved");

        // 「既にあれば失敗する移動」が使えないなら、ハードリンクで確かめる。
        let linked = std::cell::Cell::new(false);
        check_destination_with(
            &part,
            &journal,
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
            |from, to| {
                linked.set(true);
                std::fs::hard_link(from, to)
            },
        )
        .unwrap();
        assert!(linked.get());
        assert!(!probe.exists());
        assert!(!moved.exists());
        assert!(journal.snapshot().is_empty());
        assert_eq!(std::fs::read(&part).unwrap(), b"", "`.part` は動かさない");

        // ハードリンクも使えなければ、どちらの手段も無いので `UnsupportedDestination` にする。
        // 検査用のファイルは残さない。
        let err = check_destination_with(
            &part,
            &journal,
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<UnsupportedDestination>().is_some());
        assert!(!probe.exists());
        assert!(!moved.exists());

        // 権限や一時的な I/O の失敗は、保存先の性質ではないので、そのまま返す
        // （保存先を変えても直らないことがある）。
        let err = check_destination_with(
            &part,
            &journal,
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::Unsupported)),
            |_, _| Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        )
        .unwrap_err();
        assert!(err.downcast_ref::<UnsupportedDestination>().is_none());
        assert!(err.downcast_ref::<std::io::Error>().is_some());
        assert!(!probe.exists());
        assert!(!moved.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_pre_existing_probe_name_is_neither_written_through_nor_deleted() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir();
        let journal = Journal::load(Some(dir.join("download_parts.json")));
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();
        // 共有フォルダーで、第三者が同じ ID の `.probe` を別ファイルへのリンクとして
        // 先に置いた場合を再現する。
        symlink(&victim, &probe).unwrap();

        let journal = Arc::new(journal);
        let err = check_destination_with(&part, &journal, rename_no_replace, |from, to| {
            std::fs::hard_link(from, to)
        })
        .unwrap_err();
        assert!(err.downcast_ref::<UnsupportedDestination>().is_none());
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"keep",
            "リンク先を空にしない"
        );
        assert!(probe.symlink_metadata().is_ok(), "先にあった名前を消さない");
        assert!(journal.snapshot().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // 起動時の掃除は、記録に名前が残っていても、自分が書いた目印が無いものには触れない。
    // 記録してから検査用のファイルを作るので、終了のタイミングによっては第三者の名前が
    // 記録に残りうる。
    #[test]
    fn startup_cleanup_keeps_a_preexisting_probe_name() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        std::fs::write(&probe, b"keep").unwrap();

        assert!(remove_stale_entry(&probe, parent_id(&probe)));
        assert_eq!(std::fs::read(&probe).unwrap(), b"keep");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn startup_cleanup_keeps_a_preexisting_probe_link() {
        use std::os::unix::fs::symlink;

        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let moved = part.with_extension("probe-moved");
        let victim = dir.join("victim.txt");
        std::fs::write(&victim, b"keep").unwrap();
        symlink(&victim, &moved).unwrap();

        assert!(remove_stale_entry(&moved, parent_id(&moved)));
        assert!(moved.symlink_metadata().is_ok(), "リンクを消さない");
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn startup_cleanup_does_not_open_a_preexisting_probe_fifo() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        let path = std::ffi::CString::new(probe.to_str().unwrap()).unwrap();
        // 掃除が FIFO を開くと、書き手が現れるまで戻らず、残りの `.part` も消せなくなる。
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);

        assert!(remove_stale_entry(&probe, parent_id(&probe)));
        assert!(probe.symlink_metadata().is_ok(), "FIFO を消さない");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn startup_cleanup_keeps_a_preexisting_probe_socket() {
        // `sun_path` は 104 バイト（macOS）で、Rust は 103 バイトを超えるパスを
        // 拒否する。`TMPDIR` が長い環境でも入るよう、組み立てたパスの長さで
        // 判断し、収まらなければ短い `/tmp` に置く。
        let name = format!(".zoo-{}.probe", new_download_id());
        let mut probe = std::env::temp_dir().join(&name);
        if probe.as_os_str().len() >= 100 {
            probe = PathBuf::from("/tmp").join(&name);
        }
        assert!(
            probe.as_os_str().len() < 100,
            "ソケットのパスが長すぎます: {probe:?}"
        );
        // Unix ドメインソケットは、リンクをたどらず止まらない指定で開くと
        // ENXIO（macOS では EOPNOTSUPP）で失敗する。中身は無いので消さない。
        let listener = std::os::unix::net::UnixListener::bind(&probe).unwrap();

        assert!(
            remove_stale_entry(&probe, parent_id(&probe)),
            "記録からは外す"
        );
        assert!(probe.symlink_metadata().is_ok(), "ソケットを消さない");

        drop(listener);
        std::fs::remove_file(&probe).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn probe_state_classifies_open_failures_by_whether_retrying_helps() {
        for code in [
            libc::EACCES,
            libc::ENOENT,
            libc::ELOOP,
            libc::ENXIO,
            libc::EPERM,
        ] {
            let state = classify_open_result(Err(std::io::Error::from_raw_os_error(code)), true);
            assert!(
                matches!(state, ProbeState::NotOurs),
                "errno {code} はやり直しても変わらない"
            );
        }
        for code in [
            libc::EIO,
            libc::EMFILE,
            libc::ENFILE,
            libc::ENOMEM,
            libc::EINTR,
        ] {
            let state = classify_open_result(Err(std::io::Error::from_raw_os_error(code)), true);
            assert!(
                matches!(state, ProbeState::Unreadable),
                "errno {code} は次の起動でやり直す"
            );
        }
    }

    #[test]
    fn startup_cleanup_removes_an_empty_recorded_probe() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        // 記録済みの検査名で 0 バイトなら、目印を書く前に終了した自分のファイルの可能性がある。
        std::fs::write(&probe, b"").unwrap();

        assert!(remove_stale_entry(&probe, parent_id(&probe)));
        assert!(!probe.exists(), "記録済みの空 probe は掃除する");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn startup_cleanup_keeps_a_preexisting_probe_with_other_content() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        // 目印と同じ長さでも、中身が違えば自分が書いたものではない。
        std::fs::write(&probe, b"xxxxxxxxxxx").unwrap();

        assert!(remove_stale_entry(&probe, parent_id(&probe)));
        assert_eq!(std::fs::read(&probe).unwrap(), b"xxxxxxxxxxx");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleanup_keeps_the_record_when_the_parent_folder_is_gone() {
        let dir = temp_dir();
        // 外付けドライブが外れていると、親が無いために `NotFound` になる。
        let missing = part_path(&dir.join("unplugged").join("x.txt"), "0123456789abcdef");
        assert!(
            !remove_stale_entry(&missing, parent_id(&missing)),
            "親が無いときは、戻ってきた `.part` を消せるよう記録を残す"
        );

        // 親があるときは、既に無い `.part` の記録を外す。
        let gone = part_path(&dir.join("x.txt"), "0123456789abcdef");
        assert!(remove_stale_entry(&gone, parent_id(&gone)));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleanup_keeps_the_record_when_the_saved_volume_is_not_there() {
        let dir = temp_dir();
        let part = part_path(&dir.join("x.txt"), "0123456789abcdef");
        let probe = part.with_extension("probe");
        // 保存したボリュームが外れ、同じ場所に別の（空の）マウントポイントだけが残った
        // 状態を、記録した ID を変えて再現する。親の有無だけを見ると消したと誤判定する。
        let recorded = parent_id(&part).map(|id| ParentId {
            volume: id.volume.wrapping_add(1),
            ..id
        });
        assert!(recorded.is_some());
        assert!(
            !remove_stale_entry(&part, recorded),
            "元のボリュームに届かないなら `.part` の記録を残す"
        );
        assert!(
            !remove_stale_entry(&probe, recorded),
            "到達できない保存先に `.probe` が無いと断定しない"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleanup_keeps_the_record_when_the_parent_goes_away_before_remove() {
        use std::cell::Cell;

        let dir = temp_dir();
        let folder = dir.join("drive");
        std::fs::create_dir(&folder).unwrap();
        let probe = part_path(&folder.join("x.txt"), "0123456789abcdef").with_extension("probe");
        std::fs::write(&probe, PROBE_MARKER).unwrap();
        let recorded = parent_id(&probe);
        assert!(recorded.is_some());

        let removed = Cell::new(false);
        let result = remove_stale_entry_with(&probe, recorded, |_| {
            // 削除の直前にボリュームが外れた状況を作る。`remove_file` が `NotFound` を
            // 返すので、そのときの親照合で記録を残す分岐を通る。
            if !removed.get() {
                std::fs::remove_dir_all(&folder).unwrap();
                removed.set(true);
            }
            ProbeState::Ours
        });
        assert!(!result, "削除直前に親が消えたら記録を残す");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleanup_removes_a_part_even_when_the_volume_id_changed() {
        let dir = temp_dir();
        let part = part_path(&dir.join("x.txt"), "0123456789abcdef");
        std::fs::write(&part, b"ours").unwrap();
        let recorded = parent_id(&part).map(|id| ParentId {
            volume: id.volume.wrapping_add(1),
            ..id
        });
        assert!(recorded.is_some());

        assert!(
            remove_stale_entry(&part, recorded),
            "ファイルが実在するなら再マウントで ID が変わっても回収する"
        );
        assert!(!part.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleanup_removes_a_probe_even_when_the_volume_id_changed() {
        let dir = temp_dir();
        let probe = part_path(&dir.join("x.txt"), "0123456789abcdef").with_extension("probe");
        std::fs::write(&probe, PROBE_MARKER).unwrap();
        let recorded = parent_id(&probe).map(|id| ParentId {
            volume: id.volume.wrapping_add(1),
            ..id
        });
        assert!(recorded.is_some());

        assert!(
            remove_stale_entry(&probe, recorded),
            "目印を確認できた検査用ファイルは ID が変わっても回収する"
        );
        assert!(!probe.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleanup_deletes_a_legacy_record_by_name_when_the_parent_exists() {
        let dir = temp_dir();
        let part = part_path(&dir.join("x.txt"), "0123456789abcdef");
        std::fs::write(&part, b"ours").unwrap();

        assert!(
            remove_stale_entry(&part, None),
            "親 ID の無い古い記録は、以前どおり親の有無で判断する"
        );
        assert!(!part.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn startup_cleanup_keeps_the_record_when_the_probe_cannot_be_read() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        std::fs::write(&probe, PROBE_MARKER).unwrap();

        // 読めないだけなら、自分のファイルかもしれない。消さず、記録も残す。
        assert!(!remove_stale_entry_with(&probe, parent_id(&probe), |_| {
            ProbeState::Unreadable
        }));
        assert!(probe.exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn startup_cleanup_keeps_a_directory_with_a_probe_name() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        std::fs::create_dir(&probe).unwrap();

        assert!(remove_stale_entry(&probe, parent_id(&probe)));
        assert!(probe.is_dir());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn startup_cleanup_removes_a_probe_it_wrote() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");
        let probe = part.with_extension("probe");
        let moved = part.with_extension("probe-moved");

        // 検査の途中で終了した場合を再現する。目印があるものは消し、記録から外す。
        std::fs::write(&probe, PROBE_MARKER).unwrap();
        assert!(remove_stale_entry(&probe, parent_id(&probe)));
        assert!(!probe.exists());
        assert!(
            remove_stale_entry(&moved, parent_id(&moved)),
            "まだ無い名前は記録から外す"
        );
        assert!(
            remove_stale_entry(&part, parent_id(&part)),
            "無い名前は記録から外す"
        );
        assert!(
            remove_stale_entry(&dir.join("notes.txt"), parent_id(&dir)),
            "形の違う名前は触らずに記録から外す"
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prepare_part_accepts_a_destination_that_can_move_without_replacing() {
        let dir = temp_dir();
        let journal = Arc::new(Journal::load(Some(dir.join("download_parts.json"))));
        let target = dir.join("x.txt");
        let part = part_path(&target, "0123456789abcdef");

        // 実際の保存先で `.part` を作り、検査を通す。検査用の名前は記録に残らず、
        // 書き込み中の `.part` だけが確定まで記録される。
        let file = prepare_part(&journal, &part, &target, true).unwrap();
        drop(file);
        assert!(part.exists());
        assert_eq!(journal.snapshot(), vec![part.clone()]);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            2,
            "`.part` と記録だけ"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_write_error_is_not_blamed_on_the_destination() {
        let dir = temp_dir();
        let target = dir.join("x.txt");
        let entry = pending(&target, "0123456789abcdee", true, 1024);
        {
            use std::io::Write;
            let mut slot = entry.file.lock().unwrap();
            slot.as_mut().unwrap().write_all(b"new").unwrap();
        }

        // 権限や一時的な I/O の失敗は、保存先が対応していない扱いにしない。
        let err = finish_entry_with(entry, |_, _| {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert!(err.downcast_ref::<UnsupportedDestination>().is_none());
        assert!(!target.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn journal_without_a_path_refuses_to_start() {
        let dir = temp_dir();
        let journal = Journal::load(None);
        let part = dir.join(".zoo-0123456789abcdef.part");
        // 記録先が無いと、強制終了で残った `.part` を次の起動で消せない。開始を拒否する。
        assert!(journal.record(&part).is_err());
        assert!(journal.snapshot().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn download_limit_follows_the_attachment_size_within_a_ceiling() {
        assert_eq!(download_limit(None), DEFAULT_MAX_DOWNLOAD_BYTES);
        // 小さい添付でも、本文のわずかな違いを吸収できる既定値までは許す。
        assert_eq!(download_limit(Some(10)), DEFAULT_MAX_DOWNLOAD_BYTES);
        assert_eq!(download_limit(Some(60 * 1024 * 1024)), 61 * 1024 * 1024);
        assert_eq!(download_limit(Some(u64::MAX)), MAX_DOWNLOAD_BYTES_CEILING);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn mark_of_the_web_writes_zone_3() {
        let dir = temp_dir();
        let path = dir.join("report.pdf");
        std::fs::write(&path, b"x").unwrap();
        mark_of_the_web(&path);
        let mut zone = path.as_os_str().to_os_string();
        zone.push(":Zone.Identifier");
        assert_eq!(
            std::fs::read(PathBuf::from(zone)).unwrap(),
            b"[ZoneTransfer]\r\nZoneId=3\r\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
