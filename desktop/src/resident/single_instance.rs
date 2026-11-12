//! 単一インスタンス。Unix ソケットで、2 つ目の起動から既存のウィンドウへ「前に出す」を送る。
//!
//! ソケットは Linux では `$XDG_RUNTIME_DIR/zoo2-desktop.sock`、無ければ・macOS では設定ディレクトリ。
//! 接続に失敗したら、残っているソケットを消して作り直す。

use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::{
    os::unix::net::{UnixListener, UnixStream},
    sync::Mutex,
};

use super::{ResidentEvent, send};

const SOCKET_NAME: &str = "zoo2-desktop.sock";
const SHOW: &[u8] = b"show\n";

pub fn socket_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir).join(SOCKET_NAME);
    }
    crate::login::token_file()
        .ok()
        .and_then(|p| p.parent().map(|dir| dir.join(SOCKET_NAME)))
        .unwrap_or_else(|| std::env::temp_dir().join(SOCKET_NAME))
}

#[derive(Debug)]
pub enum Acquired {
    /// 自分が最初のインスタンス。
    Primary,
    /// 既存のインスタンスに「前に出す」を送った。終了してよい。
    Secondary,
}

#[cfg(unix)]
static LISTENER: Mutex<Option<UnixListener>> = Mutex::new(None);

/// 起動時に呼ぶ。最初のインスタンスなら、ソケットを作って持つ (`spawn_server` が使う)。
#[cfg(unix)]
pub fn acquire() -> Acquired {
    match acquire_at(&socket_path()) {
        Ok((acquired, listener)) => {
            if let Some(listener) = listener {
                *LISTENER.lock().unwrap() = Some(listener);
            }
            acquired
        }
        Err(err) => {
            // ソケットが作れなくてもアプリは動かす (単一インスタンスにならないだけ)。
            eprintln!("単一インスタンスのソケットを作れませんでした: {err}");
            Acquired::Primary
        }
    }
}

#[cfg(not(unix))]
pub fn acquire() -> Acquired {
    Acquired::Primary
}

#[cfg(unix)]
pub fn acquire_at(path: &Path) -> std::io::Result<(Acquired, Option<UnixListener>)> {
    for _ in 0..3 {
        match UnixStream::connect(path) {
            Ok(mut stream) => {
                stream.write_all(SHOW)?;
                return Ok((Acquired::Secondary, None));
            }
            Err(_) => {
                // 誰も聞いていない。残っているソケットを消して作り直す。
                let _ = std::fs::remove_file(path);
                match UnixListener::bind(path) {
                    Ok(listener) => return Ok((Acquired::Primary, Some(listener))),
                    // 同時に起動した別のインスタンスが先に作った。つなぎ直す。
                    Err(err) if err.kind() == std::io::ErrorKind::AddrInUse => continue,
                    Err(err) => return Err(err),
                }
            }
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AddrInUse,
        "ソケットを確保できません",
    ))
}

/// 2 つ目の起動からの「前に出す」を待つスレッドを起動する。
pub fn spawn_server() {
    #[cfg(unix)]
    if let Some(listener) = LISTENER.lock().unwrap().take() {
        std::thread::spawn(move || serve(listener, send));
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

    fn temp_socket(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zoo2-si-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(SOCKET_NAME)
    }

    #[test]
    fn first_is_primary_second_sends_show() {
        let path = temp_socket("a");
        let (first, listener) = acquire_at(&path).unwrap();
        assert!(matches!(first, Acquired::Primary));
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || serve(listener.unwrap(), move |e| tx.send(e).unwrap()));

        let (second, none) = acquire_at(&path).unwrap();
        assert!(matches!(second, Acquired::Secondary));
        assert!(none.is_none());
        let event = rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap();
        assert_eq!(event, ResidentEvent::Show);
    }

    #[test]
    fn stale_socket_is_replaced() {
        let path = temp_socket("b");
        // 開いたまま消えたソケットファイルを作る (listener を drop してもファイルは残る)。
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let (acquired, listener) = acquire_at(&path).unwrap();
        assert!(matches!(acquired, Acquired::Primary));
        assert!(listener.is_some());
    }
}
