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
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use zoo2_client_types::ipc::{DownloadBegin, DownloadSettings, DownloadSettingsUpdate};

use crate::APP_NAME;
use zoo2_client_core::config::config_dir_for;

/// 一時ファイルに使う ID の連番。同じプロセスの中で一意にする。
static DOWNLOAD_SEQ: AtomicU64 = AtomicU64::new(0);

/// 名前が長すぎるとファイルシステムの上限に当たるので、ファイル名をこのバイト数までにする。
/// macOS (APFS) と Linux (ext4) の上限は UTF-8 で 255 バイト。重複したときの
/// `-{16 桁の ID}`（17 バイト）の分を引く。Windows の上限は 255 UTF-16 単位で、
/// UTF-8 のバイト数はそれを超えない。`.part` は名前を含まない（[`part_path`]）。
const MAX_FILE_NAME_BYTES: usize = 255 - 17;

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

/// 確認なしで保存してよい拡張子。開いても、そのまま実行や読み込みに進まない形式だけを並べる。
/// ここに無い拡張子（`.exe`、`.py`、`.docm`、`.dll` など）と、拡張子の無い名前は、
/// 保存ダイアログに戻して利用者に確かめる。漏れの少ない許可リストにしておく。
/// 古い Office 文書（`.doc`、`.xls`、`.ppt`）は VBA を持てるので入れない。マクロを含められない
/// `.docx` などだけを許可する。
const AUTO_SAVE_EXTENSIONS: &[&str] = &[
    // 画像（`.svg` はスクリプトを持てるので入れない）
    "png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "heic", "heif", "tif", "tiff",
    // 音声・動画
    "mp3", "m4a", "ogg", "oga", "wav", "flac", "opus", "aac", "mp4", "m4v", "webm", "mov", "mkv",
    // 文書・テキスト（EPUB は HTML を描画するので入れない）
    "pdf", "txt", "md", "csv", "tsv", "json", "log", "docx", "xlsx", "pptx", "odt", "ods", "odp",
    // 圧縮（中身は開くときに利用者が選ぶ）
    "zip", "7z", "tar", "gz", "bz2", "xz", "zst",
];

/// 「既にあれば失敗する移動」を OS が持たないファイルシステムで、確認と移動を直列化する。
/// このアプリの中の同時保存（同じ名前を選んだ 2 件）で上書きしないためのもの。
static MOVE_LOCK: Mutex<()> = Mutex::new(());

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

/// クラッシュや強制終了で残る `.part` の記録。保存先がダイアログで選ばれたフォルダーでも、
/// 次の起動で消せるように、設定ディレクトリの `download_parts.json` に置く。
struct Journal {
    /// 記録のファイル。設定ディレクトリが分からないときは `None`（記録はメモリだけ）。
    /// テストでは一時ディレクトリのパスを渡し、利用者の記録を読み書きしない。
    file: Option<PathBuf>,
    /// 書き込み中の `.part`。読み込みは [`Journal::load`]、以後はここが正本。
    paths: Mutex<Vec<PathBuf>>,
}

impl Journal {
    fn load(file: Option<PathBuf>) -> Self {
        let paths = file
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<Vec<String>>(&bytes).ok())
            .map(|paths| paths.into_iter().map(PathBuf::from).collect())
            .unwrap_or_default();
        Self {
            file,
            paths: Mutex::new(paths),
        }
    }

    fn snapshot(&self) -> Vec<PathBuf> {
        self.paths
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// `.part` を作る前に記録する。書けなければ記録を戻してエラーにする
    /// （残骸を次の起動で消せないまま、書き込みを始めないため）。
    fn record(&self, part: &Path) -> Result<()> {
        let mut paths = self.paths.lock().unwrap_or_else(PoisonError::into_inner);
        if paths.iter().any(|path| path == part) {
            return Ok(());
        }
        paths.push(part.to_path_buf());
        if let Err(err) = self.write(&paths) {
            paths.retain(|path| path != part);
            return Err(err);
        }
        Ok(())
    }

    fn forget(&self, part: &Path) {
        let mut paths = self.paths.lock().unwrap_or_else(PoisonError::into_inner);
        paths.retain(|path| path != part);
        let _ = self.write(&paths);
    }

    /// 消せた（または既に無かった）ときだけ記録を外す。消せなかったときは残して、
    /// 次の起動でもう一度試す（外付けドライブを外した、ウイルス対策が掴んでいるなど）。
    fn remove_part(&self, part: &Path) {
        match std::fs::remove_file(part) {
            Ok(()) => self.forget(part),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => self.forget(part),
            Err(_) => {}
        }
    }

    fn write(&self, paths: &[PathBuf]) -> Result<()> {
        let Some(path) = self.file.as_deref() else {
            return Ok(());
        };
        if paths.is_empty() {
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
        let list: Vec<String> = paths
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
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
        }
    }

    /// 前回のクラッシュや強制終了で残った `.part` を消す。ウィンドウの作成を待たせないよう、
    /// 記録の読み出しだけをここで行い、ファイルの削除は裏のスレッドで行う。保存先が届かない
    /// 共有フォルダーでも、起動が止まらないようにするため。
    pub fn spawn_stale_part_cleanup(&self) {
        let stale = self.journal.snapshot();
        if stale.is_empty() {
            return;
        }
        let journal = Arc::clone(&self.journal);
        std::thread::spawn(move || {
            let mut done = Vec::new();
            for path in stale {
                let is_part = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(is_part_file_name);
                if !is_part {
                    // 記録の形が違うものは、ファイルには触れずに記録から外す。
                    done.push(path);
                    continue;
                }
                match std::fs::remove_file(&path) {
                    Ok(()) => done.push(path),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => done.push(path),
                    // 消せなかったものは、次の起動でもう一度試す。
                    Err(_) => {}
                }
            }
            for path in done {
                journal.forget(&path);
            }
        });
    }

    pub fn settings(&self) -> DownloadSettings {
        self.settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 確認の有無を変えて保存する。保存先は、`reset_directory` のときだけ既定
    /// （OS のダウンロードフォルダー）に戻す。保存できなかったら、変えない。
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
        loop {
            // 確認が要るかは、ダイアログを出す直前の設定で決める。
            let ask_every_time = self.settings().ask_every_time;
            let approved = if ask_every_time && !update.ask_every_time {
                // 確認なしで利用者のフォルダーへ書けるようにする操作なので、OS の確認を出す。
                confirm_automatic_save(app).await
            } else {
                true
            };
            // ダイアログを待っている間に保存先や確認の有無が変わっていることがある。
            // 待つ前の値を書き戻さず、その時点の設定に今回の変更だけを適用する。
            let mut stored = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
            if stored.ask_every_time != ask_every_time {
                // 確認の要否が変わったので、持っている承認は今の設定には使えない。
                // ロックを持ったままダイアログは出せないので、出し直す。
                drop(stored);
                continue;
            }
            let mut next = stored.clone();
            // 承認されなければ、オンのままにする。
            next.ask_every_time = if approved {
                update.ask_every_time
            } else {
                true
            };
            if update.reset_directory {
                next.directory = None;
            }
            return store_settings(&mut stored, next);
        }
    }

    /// OS のフォルダーの選択で決まった保存先を、確かめて保存する。パスは JS から受け取らない。
    /// 選ばれたフォルダーが無いときは保存しない（作り直さない）。
    pub fn set_directory(&self, directory: &Path) -> Result<DownloadSettings> {
        let directory = check_directory(directory)?;
        let mut current = self.settings.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = current.clone();
        next.directory = Some(directory.display().to_string());
        store_settings(&mut current, next)
    }

    /// 保存先を決め、`.part` を開く。キャンセルされたら `None`。
    pub async fn begin(
        &self,
        app: &AppHandle,
        request: &DownloadBegin,
    ) -> Result<Option<(String, PathBuf)>> {
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
        let (final_path, deduplicate) = if ask {
            // ダイアログの初期フォルダーは、無ければダイアログ側が既定に戻す。
            match ask_save_path(app, &name, settings.directory.as_deref().map(Path::new)).await {
                Some(path) => (path, false),
                None => return Ok(None),
            }
        } else {
            // フォルダーの確認と空き名の探索はディスクを待つ。届かない共有フォルダーでも、
            // 非同期ランタイムのワーカーを塞がないように、ブロッキングのタスクへ移す。
            let configured = settings.directory.clone();
            let download_dir = app.path().download_dir().ok();
            let candidate = name.clone();
            let target = tokio::task::spawn_blocking(move || {
                auto_save_target(
                    configured.as_deref().map(Path::new),
                    download_dir.as_deref(),
                    &candidate,
                )
            })
            .await
            .map_err(|err| anyhow!("保存先を決められません: {err}"))?;
            match target {
                Some(path) => (path, true),
                // 設定したフォルダーが消えている（外付けドライブや同期フォルダーが外れたなど）か、
                // 既定のダウンロードフォルダーが分からない。作り直さず、利用者に決めてもらう。
                None => match ask_save_path(app, &name, None).await {
                    Some(path) => (path, false),
                    None => return Ok(None),
                },
            }
        };

        // ダイアログを待っている間に、画面の読み込み直しやログアウトがあった。その画面には
        // もう ID を返せないので、ファイルを作らずに中止にする。
        if self.generation.load(Ordering::SeqCst) != generation {
            return Ok(None);
        }

        let id = new_download_id();
        let part = part_path(&final_path, &id);
        let journal = Arc::clone(&self.journal);
        let part_for_task = part.clone();
        let target_for_task = final_path.clone();
        let (file, overwrite) = tokio::task::spawn_blocking(move || -> Result<(File, bool)> {
            // 記録を先に保存する。書けなければ始めない（残骸を次の起動で消せなくしないため）。
            journal.record(&part_for_task)?;
            // 保存ダイアログで選ばれた名前が、選んだ時点で既にあったか。あったなら、
            // ダイアログが上書きを確認して利用者が承認している。転送中にできた同名の
            // ファイルは、この値が false なので置き換えない（`finish` で別名にする）。
            let overwrite = !deduplicate && target_for_task.exists();
            // ダイアログは既にあるフォルダーを返し、決まったフォルダーは上の分岐で存在を
            // 確かめてある。ここで `create_dir_all` はしない。
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&part_for_task)
            {
                Ok(file) => Ok((file, overwrite)),
                Err(err) => {
                    journal.forget(&part_for_task);
                    Err(err).with_context(|| {
                        format!(
                            "書き込み用のファイルを作れません: {}",
                            part_for_task.display()
                        )
                    })
                }
            }
        })
        .await
        .map_err(|err| anyhow!("書き込み用のファイルを用意できません: {err}"))??;

        let entry = Pending {
            file: Arc::new(Mutex::new(Some(file))),
            part: part.clone(),
            final_path: final_path.clone(),
            // 上書きの承認が無い保存は、確定時に既存のファイルを置き換えない。
            deduplicate: deduplicate || !overwrite,
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
        // 枠は確定・中止の後始末まで、この保存が持つ。
        slot.armed = false;
        Ok(Some((id, final_path)))
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
        let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            let mut slot = file.lock().unwrap_or_else(PoisonError::into_inner);
            let file = slot
                .as_mut()
                .ok_or_else(|| std::io::Error::other("ダウンロードは中止されました"))?;
            let written = file.write_all(&data);
            if written.is_err() {
                // 一部だけ書けているかもしれない。同じ ID の確定はさせない（中止で消す）。
                failure.store(true, Ordering::SeqCst);
            }
            written
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
        let Some(entry) = self.take_pending(id) else {
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
            let result = finish_entry(entry);
            match result {
                // 移したので、もう `.part` は無い。記録を外す。
                Ok(saved) => {
                    journal.forget(&part);
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
        let Some(entry) = self.take_pending(id) else {
            return;
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
            let taken = file.lock().unwrap_or_else(PoisonError::into_inner).take();
            drop(taken);
            // 消せなかった `.part` は記録に残し、次の起動で再試行する。
            journal.remove_part(&part);
        })
        .await;
        drop(slot);
    }

    /// 書き込み中のものをすべて中止する。画面の読み込み直し、ログアウト、アプリの終了で呼ぶ。
    ///
    /// 呼び出し元はセッションのロックを持っていることがあるので、ここではファイルを閉じたり
    /// 消したりしない。記録から外し、ファイルの後始末は裏のスレッドに任せる。
    pub fn cancel_all(&self) {
        // 保存ダイアログを待っている `begin` が、後から登録しないようにする。
        self.generation.fetch_add(1, Ordering::SeqCst);
        let pending =
            std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        if pending.is_empty() {
            return;
        }
        for entry in pending.values() {
            entry.cancelled.store(true, Ordering::SeqCst);
        }
        let journal = Arc::clone(&self.journal);
        let slots = Arc::clone(&self.slots);
        std::thread::spawn(move || {
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

    fn take_pending(&self, id: &str) -> Option<Pending> {
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
    }

    fn reserve_slot(&self) -> Result<()> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        if *slots >= MAX_PENDING_DOWNLOADS {
            return Err(anyhow!(
                "同時に保存できるのは {MAX_PENDING_DOWNLOADS} 件までです"
            ));
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

/// 「保存のたびに確認する」をオフにしてよいか、OS のダイアログで確かめる。
async fn confirm_automatic_save(app: &AppHandle) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut dialog = app
        .dialog()
        .message(
            "これからは、確認せずに保存します。保存先は、設定したフォルダーか、OS のダウンロード\
             フォルダーです。実行できるファイルは、これまでどおり保存先を尋ねます。",
        )
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
        return settings;
    };
    if Path::new(directory).is_absolute() {
        settings
    } else {
        DownloadSettings::default()
    }
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

/// 同じフォルダーの、作業中のファイル。名前を含まない短い固定の名前にして、保存ダイアログで
/// 利用者が長い名前を選んでもファイルシステムの上限に当たらないようにする。
fn part_path(final_path: &Path, id: &str) -> PathBuf {
    let dir = final_path.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!(".zoo-{id}.part"))
}

/// 起動時の掃除の対象か。`part_path` が作る `.zoo-{16 桁の ID}.part` だけ。
fn is_part_file_name(name: &str) -> bool {
    let Some(id) = name
        .strip_prefix(".zoo-")
        .and_then(|name| name.strip_suffix(".part"))
    else {
        return false;
    };
    id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// 拡張子を残して、名前がファイルシステムの上限（バイト数）に収まるようにする。
/// 文字数ではなくバイト数で切るので、日本語などの名前でも収まる。
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

/// 確認なしの保存の保存先。設定のフォルダーか、OS のダウンロードフォルダーを使う。
/// どちらも使えなければ `None`（呼び出し側が保存ダイアログを出す）。
fn auto_save_target(
    configured: Option<&Path>,
    download_dir: Option<&Path>,
    name: &str,
) -> Option<PathBuf> {
    if let Some(dir) = configured {
        return dir.is_dir().then(|| unique_path(dir, name));
    }
    match download_dir {
        Some(dir) if dir.is_dir() => Some(unique_path(dir, name)),
        _ => None,
    }
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
        // このアプリの同時保存だけは、プロセスのロックで直列化して上書きしない。
        Err(err) if is_unsupported(&err) => {
            let _guard = MOVE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            if target.exists() {
                return Err(std::io::Error::from(std::io::ErrorKind::AlreadyExists));
            }
            std::fs::rename(part, target)
        }
        Err(err) => Err(err),
    }
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
#[cfg(target_os = "windows")]
fn wide_extended_path(path: &Path) -> Vec<u16> {
    let text = path.as_os_str().to_string_lossy();
    let text = if text.starts_with(r"\\?\") {
        text.into_owned()
    } else if let Some(rest) = text.strip_prefix(r"\\") {
        // UNC（`\\server\share`）は `\\?\UNC\server\share` にする。
        format!(r"\\?\UNC\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\??\") {
        format!(r"\\?\{rest}")
    } else {
        format!(r"\\?\{text}")
    };
    text.encode_utf16().chain(std::iter::once(0)).collect()
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
            dir.join(format!("{stem} ({n}){extension}"))
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

/// Windows で、保存したファイルに Mark of the Web（Zone 3）を付ける。ほかの利用者が送った
/// `.exe` やマクロ付きの文書を開いたときに、SmartScreen と Office の保護ビューが働くようにする
/// （WebView2 の既定のダウンロードと同じ扱い）。付けられなくても保存は成功させる。
#[cfg(target_os = "windows")]
fn mark_of_the_web(path: &Path) {
    let mut zone = path.as_os_str().to_os_string();
    zone.push(":Zone.Identifier");
    let _ = std::fs::write(PathBuf::from(zone), b"[ZoneTransfer]\r\nZoneId=3\r\n");
}

#[cfg(not(target_os = "windows"))]
fn mark_of_the_web(_path: &Path) {}

/// `finish` の実体。`spawn_blocking` の中で動く。
fn finish_entry(entry: Pending) -> Result<PathBuf> {
    // 中止と確定が同時に来ても、ファイルのロックの中で決める。中止はロックの前で
    // `cancelled` を立てるので、ここで見てから確定するまでの間に取り消されることはない。
    let mut slot = entry.file.lock().unwrap_or_else(PoisonError::into_inner);
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
    let saved = saved.map_err(|err| {
        let _ = std::fs::remove_file(&entry.part);
        anyhow::Error::new(err).context(format!("保存できません: {}", entry.final_path.display()))
    })?;
    mark_of_the_web(&saved);
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
        Downloads::with_journal(Some(temp_dir().join("download_parts.json")))
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
        assert!(
            auto_save_target(missing.directory.as_deref().map(Path::new), None, "a.txt").is_none()
        );
        // `downloads.json` を手で書き換えた場合。相対パスは保存先に使わない。
        assert_eq!(
            usable_settings(DownloadSettings {
                ask_every_time: false,
                directory: Some("Downloads".into()),
            }),
            DownloadSettings::default()
        );
        assert_eq!(
            auto_save_target(Some(&dir), None, "a.txt"),
            Some(dir.join("a.txt"))
        );
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
        let journal = Journal::load(Some(dir.join("download_parts.json")));
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
