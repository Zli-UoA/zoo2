//! Tauri 版の Rust 側と WebView 側の間の IPC。
//!
//! 画面は 2 案あり、同期 (Electric の Shape の購読) を持つ側が違う。
//! 詳しくは `docs/desktop-tauri.md` の「同期の境界」を参照。
//!
//! | | (a) Leptos | (b) React (`https://zli.chat` を読み込む) |
//! |---|---|---|
//! | Shape の購読 | Rust 側。差分を [`event::SHAPE`] で流す | WebView 側 (既存の TanStack DB / Electric) |
//! | 書き込み | [`command::SEND_MESSAGE`] で Rust 側が REST を呼ぶ | WebView 側が REST を直接呼ぶ |
//! | トークン | Rust 側だけが持つ | [`command::ACCESS_TOKEN`] で受け取る |
//! | 通知の判定 | Rust 側 ([`crate::notify`]) | Rust 側。行は [`command::NOTIFICATIONS_OBSERVED`] で受け取る |
//!
//! 同じ Shape を Rust 側と WebView 側の両方で購読しない。(b) では通知の Shape も React が
//! 購読している (通知の画面で使う) ため、Rust 側は購読せず、React が受け取った変化を渡してもらう。
//!
//! 名前は Tauri のコマンド名・イベント名にそのまま使う。(b) は別にデプロイされる Web から
//! 呼ばれるので、互換を崩す変更をしたら [`PROTOCOL_VERSION`] を上げる。

use serde::{Deserialize, Serialize};

use crate::{
    api::Me,
    shape::{Batch, ShapeId},
};

/// 互換を崩す変更をしたら上げる。(b) の Web 側は [`SessionInfo::protocol`] を見て、
/// 知らない版なら IPC を使わずに通常の Web として動く。
pub const PROTOCOL_VERSION: u32 = 1;

/// WebView から Rust 側を呼ぶコマンドの名前 (`invoke` に渡す文字列)。
pub mod command {
    // (a)・(b) 共通
    /// 引数なし。[`super::SessionInfo`] を返す。
    pub const SESSION: &str = "zoo_session";
    /// 引数なし。デバイスコードのログインを始め、[`super::DeviceLogin`] を返す。
    /// 承認を待つ間は [`super::event::SESSION`] で状態を知らせる。
    pub const START_LOGIN: &str = "zoo_start_login";
    /// 引数なし。承認待ちをやめる。
    pub const CANCEL_LOGIN: &str = "zoo_cancel_login";
    /// 引数なし。トークンを消し、購読を止める。
    pub const LOGOUT: &str = "zoo_logout";
    /// [`super::FocusState`]。通知を抑えるために、前面かどうかと表示中のチャンネルを知らせる。
    pub const SET_FOCUS: &str = "zoo_set_focus";
    /// 引数なし。[`super::NotificationSettings`] を返す。アプリが OS の通知を出すかどうか。
    /// 古い版のアプリにはないので、呼べなかったら設定を出さない (`PROTOCOL_VERSION` は上げない)。
    pub const NOTIFICATION_SETTINGS: &str = "zoo_notification_settings";
    /// `{ settings: NotificationSettings }`。OS の通知を出すかどうかを変えて、保存する。
    pub const SET_NOTIFICATIONS: &str = "zoo_set_notifications";

    // (a) Leptos だけ
    /// [`super::OpenChannel`]。そのチャンネルのメッセージの Shape に購読を切り替える。
    pub const OPEN_CHANNEL: &str = "zoo_open_channel";
    /// 引数なし。Rust 側が持っている行を `reset` 付きの [`super::event::SHAPE`] で送り直す。
    /// WebView を読み込み直したときに使う。
    pub const RESYNC: &str = "zoo_resync";
    pub const ACCOUNT_PROFILES: &str = "zoo_account_profiles";
    /// [`super::SendMessage`]。[`super::SendResult`] を返す。
    pub const SEND_MESSAGE: &str = "zoo_send_message";

    // (b) React だけ
    /// 引数なし。[`super::AccessToken`] を返す。期限が近ければ Rust 側で更新してから返す。
    pub const ACCESS_TOKEN: &str = "zoo_access_token";
    /// [`super::ObservedNotifications`]。React が受け取った通知の Shape の変化を渡す。
    pub const NOTIFICATIONS_OBSERVED: &str = "zoo_notifications_observed";
    /// 引数なし。トーストの「応答」を押した招待の ID を、一度だけ返す（なければ `null`）。
    /// 外から偽装できない、Rust 側だけが残す印。古い版のアプリにはないので、呼べなかったら何もしない
    /// (`PROTOCOL_VERSION` は上げない)。
    pub const TAKE_CALL_ACCEPT: &str = "zoo_take_call_accept";
}

/// Rust 側から WebView へ送るイベントの名前。
pub mod event {
    /// [`super::SessionInfo`]。ログイン状態が変わったとき。
    pub const SESSION: &str = "zoo://session";
    /// [`super::ShapeBatch`]。(a) だけ。
    pub const SHAPE: &str = "zoo://shape";
    /// [`super::ShapeFailed`]。(a) だけ。回復できない購読のエラー。
    pub const SHAPE_FAILED: &str = "zoo://shape-failed";
    /// [`super::OpenTarget`]。通知やトレイ、ディープリンクから、あるメッセージを開くとき。
    pub const OPEN: &str = "zoo://open";
    /// 中身なし。着信のトーストの「応答」が押された。`zoo_take_call_accept` で印を取りに来る合図で、
    /// このイベントだけでは応答しない。
    pub const CALL_ACCEPT: &str = "zoo://call-accept";
}

/// どちらの画面で動いているか。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiMode {
    /// (a) 同梱の Leptos (CSR)。
    Leptos,
    /// (b) `https://zli.chat` などをリモートで読み込む。
    Remote,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub protocol: u32,
    pub ui: UiMode,
    /// backend の起点 (`https://zli.chat` など)。
    pub api_base_url: String,
    pub state: SessionState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionState {
    /// 保存済みのトークンを確かめている間。
    Starting,
    LoggedOut,
    /// デバイスコードの承認を待っている。
    LoggingIn {
        login: DeviceLogin,
    },
    Ready {
        me: Me,
    },
    /// 設定の誤りなど、ログインし直しても直らないもの。
    Failed {
        message: String,
    },
}

/// デバイスコード (RFC 8628) の、画面に出す部分。`device_code` は Rust 側だけが持つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLogin {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    /// UNIX 秒。これを過ぎたらやり直す。
    pub expires_at: i64,
}

/// アプリが OS の通知 (投稿の通知) を出すかどうか。既定は出す。更新の知らせは、これに関係なく出す。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSettings {
    pub enabled: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FocusState {
    /// ウィンドウが前面にあるか。
    pub focused: bool,
    /// 表示中のチャンネル。
    pub channel_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenChannel {
    pub channel_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMessage {
    /// クライアントで作る id (UUID v4)。送り直しても二重にならないようにする。
    pub id: String,
    pub channel_id: String,
    pub content: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendResult {
    /// Shape でこの書き込みが届いたかを確かめるための txid。
    pub txid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessToken {
    pub access_token: String,
    /// UNIX 秒。
    pub expires_at: i64,
}

/// React の通知のコレクションで起きた変化。初回の読み込みが終わったときは、
/// その時点の全行を `reset: true, up_to_date: true` で送る。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObservedNotifications {
    pub batch: Batch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapeBatch {
    pub shape: ShapeId,
    pub batch: Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeFailed {
    pub shape: ShapeId,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenTarget {
    pub channel_id: String,
    pub message_id: Option<String>,
}

/// コマンドの失敗。WebView 側は `code` で振る舞いを変え、`message` はそのまま表示してよい。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// ログインしていない、またはログインし直しが要る。
    NotLoggedIn,
    /// 今の画面ではそのコマンドを使えない ((b) で `zoo_send_message` を呼んだなど)。
    WrongMode,
    /// 通信の失敗。再試行してよい。
    Network,
    /// backend が 4xx を返した。
    Rejected,
    Internal,
}

impl IpcError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IpcError {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// (b) の Web 側は TypeScript で同じ形を読むため、JSON の形を固定しておく。
    #[test]
    fn session_info_json_is_stable() {
        let info = SessionInfo {
            protocol: PROTOCOL_VERSION,
            ui: UiMode::Remote,
            api_base_url: "https://zli.chat".into(),
            state: SessionState::Ready {
                me: Me {
                    id: "u1".into(),
                    user_name: "alice".into(),
                    display_name: "Alice".into(),
                },
            },
        };
        assert_eq!(
            serde_json::to_value(&info).unwrap(),
            json!({
                "protocol": 1,
                "ui": "remote",
                "api_base_url": "https://zli.chat",
                "state": {
                    "state": "ready",
                    "me": { "id": "u1", "user_name": "alice", "display_name": "Alice" }
                }
            })
        );
        let logged_out: SessionState =
            serde_json::from_value(json!({ "state": "logged_out" })).unwrap();
        assert_eq!(logged_out, SessionState::LoggedOut);
    }

    #[test]
    fn shape_batch_and_error_json_is_stable() {
        let batch = ShapeBatch {
            shape: ShapeId::Messages("c1".into()),
            batch: Batch {
                reset: true,
                changes: vec![],
                up_to_date: true,
            },
        };
        assert_eq!(
            serde_json::to_value(&batch).unwrap(),
            json!({
                "shape": { "shape": "messages", "channel_id": "c1" },
                "batch": { "reset": true, "changes": [], "up_to_date": true }
            })
        );
        assert_eq!(
            serde_json::to_value(IpcError::new(ErrorCode::NotLoggedIn, "x")).unwrap(),
            json!({ "code": "not_logged_in", "message": "x" })
        );
    }

    #[test]
    fn names_follow_tauri_rules() {
        // コマンド名は Rust の関数名に、イベント名は英数字と `-/:_` に限られる。
        for name in [
            command::SESSION,
            command::START_LOGIN,
            command::CANCEL_LOGIN,
            command::LOGOUT,
            command::SET_FOCUS,
            command::OPEN_CHANNEL,
            command::RESYNC,
            command::SEND_MESSAGE,
            command::ACCESS_TOKEN,
            command::NOTIFICATIONS_OBSERVED,
            command::TAKE_CALL_ACCEPT,
        ] {
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{name}"
            );
        }
        for name in [
            event::SESSION,
            event::SHAPE,
            event::SHAPE_FAILED,
            event::OPEN,
            event::CALL_ACCEPT,
        ] {
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-/:_".contains(c)),
                "{name}"
            );
        }
    }
}
