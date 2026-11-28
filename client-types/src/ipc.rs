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
    api::{Me, NotificationLevel},
    links::LinkedFetch,
    shape::{Batch, ShapeId},
};

/// WebView 側も同じ型で通知レベルを読む。
pub use crate::api::ChannelNotificationSettings;

/// 互換を崩す変更をしたら上げる。(b) の Web 側は [`SessionInfo::protocol`] を見て、
/// 知らない版なら IPC を使わずに通常の Web として動く。
pub const PROTOCOL_VERSION: u32 = 1;

/// WebView から Rust 側を呼ぶコマンドの名前 (`invoke` に渡す文字列)。
pub mod command {
    // (a)・(b) 共通
    /// 引数なし。[`super::SessionInfo`] を返す。
    pub const SESSION: &str = "zoo_session";
    /// `{ method?: LoginMethod }`。省略すると `browser`。ログインを始め、[`super::LoginPrompt`] を返す。
    /// `browser` は認可コード + PKCE（ループバックのリダイレクト）、`device` はデバイスコード
    /// (RFC 8628)。承認を待つ間は [`super::event::SESSION`] で状態を知らせる。
    pub const START_LOGIN: &str = "zoo_start_login";
    /// 引数なし。承認待ちをやめる。
    pub const CANCEL_LOGIN: &str = "zoo_cancel_login";
    /// 引数なし。トークンを消し、購読を止める。
    pub const LOGOUT: &str = "zoo_logout";
    /// [`super::FocusState`]。通知を抑えるために、前面かどうかと表示中のチャンネルを知らせる。
    pub const SET_FOCUS: &str = "zoo_set_focus";
    /// `{ count: i64 }`。Dock やランチャーのアイコンに出す未読数。`0` 以下で消す。
    /// 古い版のアプリにはないので、呼べなかったら何もしない (`PROTOCOL_VERSION` は上げない)。
    pub const SET_BADGE: &str = "zoo_set_badge";
    /// 引数なし。[`super::NotificationSettings`] を返す。アプリが OS の通知を出すかどうか。
    /// 古い版のアプリにはないので、呼べなかったら設定を出さない (`PROTOCOL_VERSION` は上げない)。
    pub const NOTIFICATION_SETTINGS: &str = "zoo_notification_settings";
    /// `{ settings: NotificationSettings }`。OS の通知を出すかどうかを変えて、保存する。
    pub const SET_NOTIFICATIONS: &str = "zoo_set_notifications";
    /// `{ channel_id: String }`。[`super::ChannelNotificationSettings`] を返す。
    /// 古い版のアプリにはないので、呼べなかったらレベルを出さない (`PROTOCOL_VERSION` は上げない)。
    pub const CHANNEL_NOTIFICATION_SETTINGS: &str = "zoo_channel_notification_settings";
    /// [`super::SetChannelNotificationLevel`]。変更後の [`super::ChannelNotificationSettings`] を返す。
    pub const SET_CHANNEL_NOTIFICATION_LEVEL: &str = "zoo_set_channel_notification_level";

    // (a) Leptos だけ
    /// [`super::OpenChannel`]。そのチャンネルのメッセージの Shape に購読を切り替える。
    pub const OPEN_CHANNEL: &str = "zoo_open_channel";
    /// 引数なし。Rust 側が持っている行を `reset` 付きの [`super::event::SHAPE`] で送り直す。
    /// WebView を読み込み直したときに使う。
    pub const RESYNC: &str = "zoo_resync";
    pub const ACCOUNT_PROFILES: &str = "zoo_account_profiles";
    /// [`super::SendMessage`]。[`super::SendResult`] を返す。
    pub const SEND_MESSAGE: &str = "zoo_send_message";
    /// [`super::InvokeAppAction`]。メッセージ部品のボタン・選択肢を押したことをアプリへ送る。
    /// 古い版のアプリにはないので、呼べなかったら知らせる (`PROTOCOL_VERSION` は上げない)。
    pub const INVOKE_APP_ACTION: &str = "zoo_invoke_app_action";
    /// [`super::WatchLinks`]。画面の近くにある本文のリンクの、取得が要るリンク先を入れ替える。
    /// 結果は [`super::event::LINKED`] で返す。
    pub const WATCH_LINKS: &str = "zoo_watch_links";

    // (b) React だけ
    /// 引数なし。[`super::AccessToken`] を返す。期限が近ければ Rust 側で更新してから返す。
    pub const ACCESS_TOKEN: &str = "zoo_access_token";
    /// [`super::ObservedNotifications`]。React が受け取った通知の Shape の変化を渡す。
    pub const NOTIFICATIONS_OBSERVED: &str = "zoo_notifications_observed";
    /// 引数なし。トーストの「応答」を押した招待の ID を、一度だけ返す（なければ `null`）。
    /// 外から偽装できない、Rust 側だけが残す印。古い版のアプリにはないので、呼べなかったら何もしない
    /// (`PROTOCOL_VERSION` は上げない)。
    pub const TAKE_CALL_ACCEPT: &str = "zoo_take_call_accept";
    /// 引数なし。E2EE の再認証の ID トークン（文字列）を返す。既定のブラウザーで AuthZ に
    /// ログインし直してもらい、ループバックのリダイレクトで受け取る。古い版のアプリにはないので、
    /// 呼べなかったら Web 側はアプリの更新を案内する (`PROTOCOL_VERSION` は上げない)。
    pub const REAUTHENTICATE: &str = "zoo_reauthenticate";
    /// 引数なし。[`super::DownloadSettings`] を返す。添付の保存先の設定。
    /// 古い版のアプリにはないので、呼べなかったら Web 側はブラウザーの保存に戻す
    /// (`PROTOCOL_VERSION` は上げない)。
    pub const DOWNLOAD_SETTINGS: &str = "zoo_download_settings";
    /// [`super::DownloadSettingsUpdate`]。確認の有無を変えて、保存する。新しい設定を返す。
    pub const SET_DOWNLOAD_SETTINGS: &str = "zoo_set_download_settings";
    /// 引数なし。OS のフォルダーの選択を出し、選ばれた保存先を Rust 側で保存して、新しい
    /// [`super::DownloadSettings`] を返す。閉じたら `null`。パスは JS に決めさせない。
    pub const PICK_DOWNLOAD_DIRECTORY: &str = "zoo_pick_download_directory";
    /// [`super::DownloadBegin`]。保存先を決め、書き込み先の ID を返す。確認なし保存が
    /// 有効で選択済みフォルダーを使え、対象形式が許可リストに含まれる場合はダイアログを
    /// 出さない。それ以外は OS の保存ダイアログを出す。添付の大きさを受け取り、1 件の
    /// 上限に使う。
    pub const DOWNLOAD_BEGIN: &str = "zoo_download_begin";
    /// 本文は octet-stream。`x-zoo-download-id` ヘッダーの ID に追記する。
    /// x-zoo-download-offset を付けると、その位置への書き込みだけを受け付ける。
    pub const DOWNLOAD_WRITE: &str = "zoo_download_write";
    /// `{ id: String }`。`.part` を本来の名前へ置き換える。
    pub const DOWNLOAD_FINISH: &str = "zoo_download_finish";
    /// `{ id: String }`。途中までのファイルを消す。
    pub const DOWNLOAD_CANCEL: &str = "zoo_download_cancel";
}

/// [`command::DOWNLOAD_WRITE`] の、書き込み先の ID を渡すヘッダー。
pub const DOWNLOAD_ID_HEADER: &str = "x-zoo-download-id";
/// DOWNLOAD_WRITE の本文が、保存先のどの位置に対応するか（10進のバイト数）。
/// ヘッダーがある場合は、その位置にだけ書く。期待位置と異なる場合は拒否するが、
/// 直前に書いた同一チャンク（offset + len == 現在位置）の再送は no-op で成功を返す。
/// ヘッダーを省略した場合は位置検査を行わず、現在位置へ追記する。
pub const DOWNLOAD_OFFSET_HEADER: &str = "x-zoo-download-offset";

/// Rust 側から WebView へ送るイベントの名前。
pub mod event {
    /// [`super::SessionInfo`]。ログイン状態が変わったとき。
    pub const SESSION: &str = "zoo://session";
    /// [`super::ShapeBatch`]。(a) だけ。
    pub const SHAPE: &str = "zoo://shape";
    /// [`super::LinkedUpdate`]。(a) だけ。[`super::command::WATCH_LINKS`] のリンク先で分かったこと。
    pub const LINKED: &str = "zoo://linked";
    /// [`super::ShapeFailed`]。(a) だけ。回復できない購読のエラー。
    pub const SHAPE_FAILED: &str = "zoo://shape-failed";
    /// [`super::LoginFailed`]。(a) だけ。ブラウザーでのログインが、始まったあとで失敗したとき。
    pub const LOGIN_FAILED: &str = "zoo://login-failed";
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
    /// ブラウザーでの承認を待っている。
    LoggingIn {
        login: LoginPrompt,
    },
    Ready {
        me: Me,
    },
    /// 設定の誤りなど、ログインし直しても直らないもの。
    Failed {
        message: String,
    },
}

/// ログインの方式。ブラウザーを開ける環境では認可コード + PKCE を使い（RFC 8252）、ブラウザーを
/// 開けない・ループバックで受けられない環境のために、デバイスコードを残す。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginMethod {
    #[default]
    Browser,
    Device,
}

/// 承認待ちの画面に出す内容。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LoginPrompt {
    Browser(BrowserLogin),
    Device(DeviceLogin),
}

/// 認可コード + PKCE のログインの、画面に出す部分。code verifier、`state`、`nonce` は Rust 側だけが持つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserLogin {
    /// 既定のブラウザーで開く認可リクエストの URL（開けなかったときに手動で開く）。
    pub url: String,
    /// UNIX 秒。これを過ぎたらやり直す。
    pub expires_at: i64,
}

/// デバイスコード (RFC 8628) の、画面に出す部分。`device_code` は Rust 側だけが持つ。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceLogin {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    /// ブラウザーを自動で開く対象かどうか。実際に開けたかどうかは表さない。
    pub auto_open: bool,
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

/// [`command::CHANNEL_NOTIFICATION_SETTINGS`] の引数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelNotificationSettingsRequest {
    pub channel_id: String,
}

/// [`command::SET_CHANNEL_NOTIFICATION_LEVEL`] の引数。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetChannelNotificationLevel {
    pub channel_id: String,
    pub level: NotificationLevel,
}

/// 添付を保存するときの、デスクトップ版の設定。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSettings {
    /// 保存のたびに OS の「名前を付けて保存」を出すか。既定は出す。
    #[serde(default = "ask_every_time_default")]
    pub ask_every_time: bool,
    /// 「都度確認」がオフのときの保存先（絶対パス）。確認なしで保存するには必須。
    /// 画面に出すためだけに返す。JS からは変えられず、OS のフォルダーの選択でだけ決まる。
    #[serde(default)]
    pub directory: Option<String>,
}

/// [`command::SET_DOWNLOAD_SETTINGS`] の引数。
///
/// 保存先のパスは受け取らない。保存先を決められるのは OS のフォルダーの選択
/// （[`command::PICK_DOWNLOAD_DIRECTORY`]）だけで、リモートの画面から任意の場所へ
/// 書き込めないようにするため。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadSettingsUpdate {
    /// 保存のたびに OS の「名前を付けて保存」を出すか。
    pub ask_every_time: bool,
    /// 選んだ保存先を消して、保存のたびに確認する設定へ戻すか。
    #[serde(default)]
    pub reset_directory: bool,
}

fn ask_every_time_default() -> bool {
    true
}

impl Default for DownloadSettings {
    fn default() -> Self {
        Self {
            ask_every_time: true,
            directory: None,
        }
    }
}

/// [`command::DOWNLOAD_BEGIN`] の引数。保存するファイルの名前（拡張子を含む）。
/// Rust 側で使えない文字を除くので、そのまま渡してよい。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadBegin {
    pub file_name: String,
    /// 添付の大きさ（平文のバイト数）。分からなければ `None`。Rust 側はこの値から
    /// 1 件の上限を決める（サーバーの添付の上限を上げても、ここが連動する）。
    /// 上限は 512 MiB で頭打ちになる。サーバーの添付の上限をそれより大きくするときは、
    /// `desktop-tauri/src-tauri/src/downloads.rs` の天井も上げること。
    #[serde(default)]
    pub size: Option<u64>,
}

/// [`command::DOWNLOAD_BEGIN`] の結果。保存先が決まったか、利用者がダイアログを閉じたか。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DownloadStart {
    /// 書き込み先の ID と、保存するファイルのパス（画面に出すためだけ）。
    /// `prompted` はこの保存で OS の保存ダイアログを出したか。
    Started {
        id: String,
        path: String,
        prompted: bool,
    },
    /// 利用者が保存ダイアログをキャンセルした。
    Cancelled,
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

/// 本文のリンクが指すメッセージ。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LinkedMessage {
    pub channel_id: String,
    pub message_id: String,
}

/// [`command::WATCH_LINKS`] の引数。画面の近くにある、取得が要るリンク先のすべて（空なら、なし）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchLinks {
    pub targets: Vec<LinkedMessage>,
    /// 移動先を確かめている間の、確認の番号。`targets` は移動先で、確認ごとに取り直す。
    pub check: Option<u64>,
}

/// [`event::LINKED`] の内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkedUpdate {
    /// 状態が変わったリンク先。
    pub linked: Vec<Linked>,
    /// 暗号文の行が届いたことのあるチャンネルのすべて。画面は覚えて、暗号化として扱う。
    pub ciphertext: Vec<String>,
}

/// [`LinkedUpdate`] の 1 件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Linked {
    pub target: LinkedMessage,
    /// `None` は、分かっていたことがなくなった（捨てた）。画面も捨てる。
    pub fetch: Option<LinkedFetch>,
    /// [`WatchLinks::check`] の確認のあとに始めた取得で届いた結果なら、その番号。移動先の判定には、
    /// いまの確認の番号の結果だけを使う（ほかは、削除などを見落としているかもしれない。埋め込みには出す）。
    pub checked: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SendMessage {
    /// クライアントで作る id (UUID v4)。送り直しても二重にならないようにする。
    pub id: String,
    pub channel_id: String,
    pub content: String,
    pub parent_id: Option<String>,
}

/// メッセージ部品の操作 ([`command::INVOKE_APP_ACTION`])。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvokeAppAction {
    pub app_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub custom_id: String,
    /// ボタンの `value`、選んだ値 (select は配列)
    pub value: serde_json::Value,
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

/// ブラウザーでのログインが失敗した理由。`START_LOGIN` はもう成功を返しているので、イベントで知らせる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginFailed {
    pub message: String,
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
    /// backend が 4xx を返した。再認証では、AuthZ が拒んだ、または ID トークンを受け付けられない。
    Rejected,
    /// 時間内に終わらなかった (再認証で、利用者がログインし直すのを待ちきれなかった)。
    Timeout,
    /// 新しい操作を始めたので、前の操作を取り消した。
    Cancelled,
    /// 保存先が、既存のファイルを上書きせずに保存する操作に対応していない。
    UnsupportedDestination,
    /// 同時に保存できる件数の上限に達した。ほかの保存が終われば保存できる。
    TooManyDownloads,
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
        assert_eq!(
            serde_json::to_value(IpcError::new(ErrorCode::TooManyDownloads, "x")).unwrap(),
            json!({ "code": "too_many_downloads", "message": "x" })
        );
    }

    #[test]
    fn download_json_is_stable() {
        assert_eq!(
            serde_json::to_value(DownloadStart::Started {
                id: "abc".into(),
                path: "C:\\Users\\a\\file.pdf".into(),
                prompted: true,
            })
            .unwrap(),
            json!({
                "status": "started",
                "id": "abc",
                "path": "C:\\Users\\a\\file.pdf",
                "prompted": true
            })
        );
        assert_eq!(
            serde_json::to_value(DownloadStart::Cancelled).unwrap(),
            json!({ "status": "cancelled" })
        );
        // 添付の大きさは任意。古い画面が大きさを送らなくても読める。
        assert_eq!(
            serde_json::from_value::<DownloadBegin>(json!({ "file_name": "a.pdf" })).unwrap(),
            DownloadBegin {
                file_name: "a.pdf".into(),
                size: None
            }
        );
        assert_eq!(
            serde_json::from_value::<DownloadBegin>(json!({ "file_name": "a.pdf", "size": 1234 }))
                .unwrap(),
            DownloadBegin {
                file_name: "a.pdf".into(),
                size: Some(1234)
            }
        );
        // 古い設定ファイルに項目がなくても、既定 (都度確認する) で読める。
        assert_eq!(
            serde_json::from_value::<DownloadSettings>(json!({})).unwrap(),
            DownloadSettings::default()
        );
        assert_eq!(
            serde_json::from_value::<DownloadSettings>(json!({ "directory": null })).unwrap(),
            DownloadSettings {
                ask_every_time: true,
                directory: None
            }
        );
        // 更新の引数は保存先のパスを受け取らない（`directory` を足しても無視される）。
        assert_eq!(
            serde_json::from_value::<DownloadSettingsUpdate>(
                json!({ "ask_every_time": false, "directory": "C:\\Windows" })
            )
            .unwrap(),
            DownloadSettingsUpdate {
                ask_every_time: false,
                reset_directory: false
            }
        );
    }

    #[test]
    fn channel_notification_settings_json_is_stable() {
        assert_eq!(
            serde_json::to_value(SetChannelNotificationLevel {
                channel_id: "c1".into(),
                level: NotificationLevel::Mute,
            })
            .unwrap(),
            json!({ "channel_id": "c1", "level": "mute" })
        );
        assert_eq!(
            serde_json::from_value::<ChannelNotificationSettings>(json!({ "level": "all" }))
                .unwrap(),
            ChannelNotificationSettings {
                level: NotificationLevel::All
            }
        );
    }

    #[test]
    fn login_prompt_is_tagged_and_the_method_defaults_to_browser() {
        let browser = LoginPrompt::Browser(BrowserLogin {
            url: "https://issuer/authorize?x=1".into(),
            expires_at: 10,
        });
        let value = serde_json::to_value(&browser).unwrap();
        assert_eq!(value["kind"], "browser");
        assert_eq!(value["url"], "https://issuer/authorize?x=1");
        assert_eq!(
            serde_json::from_value::<LoginPrompt>(value).unwrap(),
            browser
        );
        let device = LoginPrompt::Device(DeviceLogin {
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://issuer/device".into(),
            verification_uri_complete: None,
            auto_open: true,
            expires_at: 10,
        });
        assert_eq!(serde_json::to_value(&device).unwrap()["kind"], "device");
        assert_eq!(LoginMethod::default(), LoginMethod::Browser);
        assert_eq!(
            serde_json::from_value::<LoginMethod>(json!("device")).unwrap(),
            LoginMethod::Device
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
            command::SET_BADGE,
            command::CHANNEL_NOTIFICATION_SETTINGS,
            command::SET_CHANNEL_NOTIFICATION_LEVEL,
            command::OPEN_CHANNEL,
            command::RESYNC,
            command::SEND_MESSAGE,
            command::ACCESS_TOKEN,
            command::NOTIFICATIONS_OBSERVED,
            command::TAKE_CALL_ACCEPT,
            command::REAUTHENTICATE,
            command::DOWNLOAD_SETTINGS,
            command::SET_DOWNLOAD_SETTINGS,
            command::PICK_DOWNLOAD_DIRECTORY,
            command::DOWNLOAD_BEGIN,
            command::DOWNLOAD_WRITE,
            command::DOWNLOAD_FINISH,
            command::DOWNLOAD_CANCEL,
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
