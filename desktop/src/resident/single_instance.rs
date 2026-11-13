//! 単一インスタンス。Unix ソケットで、2 つ目の起動から既存のウィンドウへ「前に出す」を送る。
//!
//! 誰が最初かは、ソケットではなくロックファイル (`flock`) で決める。ソケットの接続の成否で決めると、
//! 同時に起動した 2 つが互いの生きたソケットを消して、両方が最初になってしまうため。
//!
//! - ロックが取れた: 最初のインスタンス。ロックはプロセスの終わりまで持つ (死ぬと OS が外す)。
//!   古いソケットは無条件に消して作り直す。
//! - ロックが取れない: 2 つ目。最初のインスタンスがソケットを作るまで短い間隔でつなぎ直し、
//!   「前に出す」を送って終わる。
//!
//! 置き場所は、Linux では `$XDG_RUNTIME_DIR`、無ければ・macOS では設定ディレクトリ。
//! ファイル名には設定ディレクトリの識別子を入れ、設定の違う版 (開発用など) が同じ名前を取り合わないようにする。
//! ソケットとロックは 0600。

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
use std::{
    fs::File,
    os::unix::{
        fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    sync::Mutex,
};

use super::{ResidentEvent, send};

const SHOW: &[u8] = b"show\n";
/// 2 つ目が、最初のインスタンスのソケットを待つ回数と間隔 (合わせて 5 秒)。
const CONNECT_RETRIES: u32 = 50;
const CONNECT_INTERVAL: Duration = Duration::from_millis(100);

/// ソケットとロックファイルのパス。親ディレクトリは作らない (`acquire_at` が作る)。
pub fn paths() -> (PathBuf, PathBuf) {
    let app = crate::login::APP_NAME;
    let config_dir = zoo2_client_core::config::config_dir_for(app);
    #[cfg(target_os = "linux")]
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        // 共有の実行時ディレクトリでは、設定ディレクトリごとに名前を分ける。
        let id = config_dir
            .as_deref()
            .map(|dir| short_hash(&dir.to_string_lossy()))
            .unwrap_or_default();
        let base = PathBuf::from(dir);
        return (
            base.join(format!("{app}-{id}.sock")),
            base.join(format!("{app}-{id}.lock")),
        );
    }
    let base = config_dir.unwrap_or_else(std::env::temp_dir);
    (
        base.join(format!("{app}.sock")),
        base.join(format!("{app}.lock")),
    )
}

/// 名前に入れるための、固定の短いハッシュ (FNV-1a)。`DefaultHasher` は版によって変わるので使わない。
fn short_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08x}", hash & 0xffff_ffff)
}

#[derive(Debug)]
pub enum Acquired {
    /// 自分が最初のインスタンス。
    Primary,
    /// 既存のインスタンスに「前に出す」を送った。終了してよい。
    Secondary,
}

/// 最初のインスタンスが持つもの。`lock` はプロセスの終わりまで手放さない。
#[cfg(unix)]
pub struct Held {
    pub listener: UnixListener,
    pub lock: File,
}

#[cfg(unix)]
static HELD: Mutex<Option<Held>> = Mutex::new(None);

/// 起動時に呼ぶ。最初のインスタンスなら、ソケットとロックを持つ (`spawn_server` が使う)。
#[cfg(unix)]
pub fn acquire() -> Acquired {
    let (socket, lock) = paths();
    match acquire_at(&socket, &lock) {
        Ok((acquired, held)) => {
            if let Some(held) = held {
                *HELD.lock().unwrap() = Some(held);
            }
            acquired
        }
        Err(err) => {
            // 単一インスタンスの仕組みそのものが使えない (ディレクトリやロックファイルの I/O エラー、
            // または他のインスタンスがロックを持ったままソケットに応答しない)。
            // 起動を止めないために最初として動かすが、ロックは持たないので単一インスタンスにはならない。
            eprintln!("単一インスタンスの確保に失敗しました (ロックなしで起動します): {err}");
            Acquired::Primary
        }
    }
}

#[cfg(not(unix))]
pub fn acquire() -> Acquired {
    Acquired::Primary
}

#[cfg(unix)]
pub fn acquire_at(socket: &Path, lock: &Path) -> std::io::Result<(Acquired, Option<Held>)> {
    acquire_with(socket, lock, CONNECT_RETRIES, CONNECT_INTERVAL, LOCK_ROUNDS)
}

/// ロックを取り直す回数。2 つ目がつなぎ直しに失敗したとき (最初のインスタンスが終わった直後など) に、
/// ロックを取り直して自分が最初になるため。ロックを持たないまま最初になることは、ここでは決してしない。
#[cfg(unix)]
const LOCK_ROUNDS: u32 = 3;

#[cfg(unix)]
fn acquire_with(
    socket: &Path,
    lock: &Path,
    retries: u32,
    interval: Duration,
    rounds: u32,
) -> std::io::Result<(Acquired, Option<Held>)> {
    for path in [socket, lock] {
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)?;
        }
    }
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(lock)?;
    let mut last = None;
    for _ in 0..rounds.max(1) {
        match lock_file.try_lock() {
            Ok(()) => {
                // 最初のインスタンス。ソケットが残っていても持ち主はいないので、無条件に消す。
                match std::fs::remove_file(socket) {
                    Ok(()) => {}
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err),
                }
                let listener = UnixListener::bind(socket)?;
                std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
                return Ok((
                    Acquired::Primary,
                    Some(Held {
                        listener,
                        lock: lock_file,
                    }),
                ));
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                // 2 つ目。最初のインスタンスがソケットを作るまで待ってつなぐ。
                // つなげないまま終わったら、最初のインスタンスが終わったのかもしれないので、ロックを取り直す。
                for _ in 0..retries {
                    match UnixStream::connect(socket) {
                        Ok(mut stream) => {
                            stream.write_all(SHOW)?;
                            return Ok((Acquired::Secondary, None));
                        }
                        Err(err) => last = Some(err),
                    }
                    std::thread::sleep(interval);
                }
            }
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("ソケットに接続できません")))
}

/// 2 つ目の起動からの「前に出す」を待つスレッドを起動する。
pub fn spawn_server() {
    #[cfg(unix)]
    if let Some(held) = HELD.lock().unwrap().take() {
        std::thread::spawn(move || {
            // ロックはサーバーのスレッドが持ち続ける (プロセスの終わりまで)。
            let Held {
                listener,
                lock: _lock,
            } = held;
            serve(listener, send)
        });
    }
}

#[cfg(unix)]
fn serve(listener: UnixListener, on_event: impl Fn(ResidentEvent)) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let mut buf = Vec::new();
        let _ = stream
            .set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .and_then(|_| stream.take(64).read_to_end(&mut buf).map(|_| ()));
        if buf.starts_with(b"show") {
            on_event(ResidentEvent::Show);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::sync::mpsc;

    /// テスト用の一時ディレクトリ。drop で片付ける。
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("zoo2-si-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn paths(&self) -> (PathBuf, PathBuf) {
            (self.0.join("a.sock"), self.0.join("a.lock"))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn first_is_primary_second_sends_show() {
        let dir = TempDir::new("a");
        let (socket, lock) = dir.paths();
        let (first, held) = acquire_at(&socket, &lock).unwrap();
        assert!(matches!(first, Acquired::Primary));
        let held = held.unwrap();
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let Held {
                listener,
                lock: _lock,
            } = held;
            serve(listener, move |e| tx.send(e).unwrap())
        });

        let (second, none) = acquire_at(&socket, &lock).unwrap();
        assert!(matches!(second, Acquired::Secondary));
        assert!(none.is_none());
        let event = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(event, ResidentEvent::Show);
    }

    #[test]
    fn stale_socket_is_replaced() {
        let dir = TempDir::new("b");
        let (socket, lock) = dir.paths();
        // 持ち主のいないソケットファイルを作る (listener を drop してもファイルは残る)。
        drop(UnixListener::bind(&socket).unwrap());
        assert!(socket.exists());
        let (acquired, held) = acquire_at(&socket, &lock).unwrap();
        assert!(matches!(acquired, Acquired::Primary));
        assert!(held.is_some());
    }

    #[test]
    fn creates_missing_directory() {
        let dir = TempDir::new("c");
        let socket = dir.0.join("nested/dir/a.sock");
        let lock = dir.0.join("nested/dir/a.lock");
        let (acquired, _held) = acquire_at(&socket, &lock).unwrap();
        assert!(matches!(acquired, Acquired::Primary));
        let mode = std::fs::metadata(&lock).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    /// 同時に起動しても、最初になるのは 1 つだけで、残りは 2 つ目として終わる。
    #[test]
    fn simultaneous_acquire_yields_exactly_one_primary() {
        for round in 0..20 {
            let dir = TempDir::new(&format!("race{round}"));
            let (socket, lock) = dir.paths();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let handles: Vec<_> = (0..2)
                .map(|_| {
                    let (socket, lock, barrier) = (socket.clone(), lock.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        barrier.wait();
                        acquire_at(&socket, &lock).unwrap()
                    })
                })
                .collect();
            let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
            let primaries = results
                .iter()
                .filter(|(acquired, _)| matches!(acquired, Acquired::Primary))
                .count();
            assert_eq!(primaries, 1, "round {round}");
        }
    }

    /// 最初のインスタンスが、2 つ目のつなぎ直しの途中で終わっても、
    /// 2 つ目はロックを取り直して最初になり、その後の起動は 2 つ目になる (最初が 2 つにならない)。
    #[test]
    fn lock_is_retaken_when_primary_exits_during_connect_retries() {
        let dir = TempDir::new("retake");
        let (socket, lock) = dir.paths();
        let (first, held) = acquire_at(&socket, &lock).unwrap();
        assert!(matches!(first, Acquired::Primary));
        let Held {
            listener,
            lock: a_lock,
        } = held.unwrap();
        // 最初のインスタンスが終わる途中: ソケットは先に閉じ (ファイルは残る)、ロックは少し後に手放す。
        drop(listener);
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(a_lock);
        });
        let (acquired, held_b) = acquire_with(&socket, &lock, 5, Duration::from_millis(20), 3)
            .expect("ロックを取り直せる");
        releaser.join().unwrap();
        assert!(matches!(acquired, Acquired::Primary));
        let held_b = held_b.expect("最初になるときはロックを持つ");

        let (c, held_c) = acquire_with(&socket, &lock, 1, Duration::from_millis(1), 1)
            .map(|(a, h)| (Some(a), h))
            .unwrap_or((None, None));
        assert!(!matches!(c, Some(Acquired::Primary)), "最初が 2 つになった");
        assert!(held_c.is_none());
        drop(held_b);
    }

    #[test]
    fn hash_is_stable_and_differs_by_directory() {
        assert_eq!(short_hash("/a"), short_hash("/a"));
        assert_ne!(short_hash("/a"), short_hash("/b"));
        assert_eq!(short_hash("").len(), 8);
    }
}
