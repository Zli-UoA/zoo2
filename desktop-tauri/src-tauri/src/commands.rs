//! WebView から呼ぶコマンド。名前は `zoo2_client_types::ipc::command` と同じ。
//!
//! 呼べるウィンドウは capabilities で絞っているが、二重の防御として、ここでも呼び出し元の
//! URL を確かめる。同梱の画面からトークンを取り出せないようにするため。

use std::sync::Arc;

use tauri::{AppHandle, State, Webview};
use zoo2_client_types::{
    ipc::{
        AccessToken, DOWNLOAD_ID_HEADER, DeviceLogin, DownloadBegin, DownloadSettings,
        DownloadSettingsUpdate, DownloadStart, ErrorCode, FocusState, InvokeAppAction, IpcError,
        NotificationSettings, ObservedNotifications, OpenChannel, SendMessage, SendResult,
        SessionInfo, UiMode,
    },
    shape::ShapeId,
};

use crate::{
    core::{Core, to_ipc},
    window::{Origin, classify},
};

type Core_<'a> = State<'a, Arc<Core>>;

/// 呼び出し元が、今の画面の案で許された場所かを確かめる。
fn require(webview: &Webview, core: &Core, allowed: &[Origin]) -> Result<(), IpcError> {
    let url = webview
        .url()
        .map_err(|err| IpcError::new(ErrorCode::Internal, err.to_string()))?;
    let origin = classify(&url, core);
    if allowed.contains(&origin) {
        Ok(())
    } else {
        Err(IpcError::new(
            ErrorCode::WrongMode,
            "この画面からは使えないコマンドです",
        ))
    }
}

/// 同梱の画面と、(b) のリモートの画面の両方から呼べるもの。
const ANY: &[Origin] = &[Origin::Bundled, Origin::Remote];
const BUNDLED: &[Origin] = &[Origin::Bundled];
const REMOTE: &[Origin] = &[Origin::Remote];

#[tauri::command]
pub async fn zoo_session(webview: Webview, core: Core_<'_>) -> Result<SessionInfo, IpcError> {
    require(&webview, &core, ANY)?;
    Ok(core.session().await)
}

#[tauri::command]
pub async fn zoo_start_login(webview: Webview, core: Core_<'_>) -> Result<DeviceLogin, IpcError> {
    require(&webview, &core, ANY)?;
    core.inner().start_login().await
}

#[tauri::command]
pub async fn zoo_cancel_login(webview: Webview, core: Core_<'_>) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    core.cancel_login().await;
    Ok(())
}

#[tauri::command]
pub async fn zoo_logout(webview: Webview, core: Core_<'_>) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    core.logout().await
}

#[tauri::command]
pub async fn zoo_set_focus(
    webview: Webview,
    core: Core_<'_>,
    focus: FocusState,
) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    core.set_focus(focus).await;
    Ok(())
}

/// Dock やランチャーのアイコンの未読数。`count <= 0` で消す。
/// 同期のコマンドにして、届いた順に適用する。IPC の到着順は保証されないので、WebView 側が
/// 前の呼び出しの完了を待ってから次を送る（`desktopSetBadge`）。
/// 数は React が計算済みのもの（ActivityPub を含む）を、そのまま使う。
#[tauri::command]
pub fn zoo_set_badge(webview: Webview, core: Core_<'_>, count: i64) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    let badge = (count > 0).then_some(count);
    // Windows など、未対応の環境ではエラーになるが、表示が出ないだけなので無視する。
    let _ = webview.window().set_badge_count(badge);
    Ok(())
}

#[tauri::command]
pub async fn zoo_notification_settings(
    webview: Webview,
    core: Core_<'_>,
) -> Result<NotificationSettings, IpcError> {
    require(&webview, &core, ANY)?;
    Ok(core.notification_settings())
}

#[tauri::command]
pub async fn zoo_set_notifications(
    webview: Webview,
    core: Core_<'_>,
    settings: NotificationSettings,
) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    core.set_notification_settings(settings)
        .map_err(|err| IpcError::new(ErrorCode::Internal, format!("{err:#}")))
}

#[tauri::command]
pub async fn zoo_open_channel(
    webview: Webview,
    core: Core_<'_>,
    open: OpenChannel,
) -> Result<(), IpcError> {
    require(&webview, &core, BUNDLED)?;
    leptos_only(&core)?;
    core.open_channel(open.channel_id).await
}

#[tauri::command]
pub async fn zoo_resync(webview: Webview, core: Core_<'_>) -> Result<(), IpcError> {
    require(&webview, &core, BUNDLED)?;
    leptos_only(&core)?;
    core.resync().await
}

#[tauri::command]
pub async fn zoo_send_message(
    webview: Webview,
    core: Core_<'_>,
    message: SendMessage,
) -> Result<SendResult, IpcError> {
    require(&webview, &core, BUNDLED)?;
    leptos_only(&core)?;
    let api = core.api().await?;
    let txid = api
        .send_message(
            &message.id,
            &message.channel_id,
            &message.content,
            message.parent_id.as_deref(),
        )
        .await
        .map_err(to_ipc)?;
    Ok(SendResult { txid })
}

#[tauri::command]
pub async fn zoo_invoke_app_action(
    webview: Webview,
    core: Core_<'_>,
    action: InvokeAppAction,
) -> Result<(), IpcError> {
    require(&webview, &core, BUNDLED)?;
    leptos_only(&core)?;
    let api = core.api().await?;
    api.invoke_app_action(
        &action.app_id,
        &action.channel_id,
        &action.message_id,
        &action.custom_id,
        action.value,
    )
    .await
    .map_err(to_ipc)?;
    Ok(())
}

#[tauri::command]
pub async fn zoo_access_token(webview: Webview, core: Core_<'_>) -> Result<AccessToken, IpcError> {
    require(&webview, &core, REMOTE)?;
    let (access_token, expires_at) = core.access_token().await?;
    Ok(AccessToken {
        access_token,
        expires_at,
    })
}

#[tauri::command]
pub async fn zoo_notifications_observed(
    webview: Webview,
    core: Core_<'_>,
    observed: ObservedNotifications,
) -> Result<(), IpcError> {
    require(&webview, &core, REMOTE)?;
    let generation = core.generation().await;
    core.handle_batch(generation, ShapeId::Notifications, observed.batch)
        .await;
    Ok(())
}

/// E2EE の再認証。既定のブラウザーで AuthZ にログインし直してもらい、ID トークンを返す。
/// (b) の画面だけが呼べる。
#[tauri::command]
pub async fn zoo_reauthenticate(webview: Webview, core: Core_<'_>) -> Result<String, IpcError> {
    require(&webview, &core, REMOTE)?;
    core.inner().reauthenticate().await
}

/// トーストの「応答」を押した招待の ID を、一度だけ返す（なければ `None`）。(b) の画面だけが呼べる。
/// 応答するのは、これを取れた画面だけ。外から偽装できない、Rust 側だけが残す印で、イベントや URL
/// だけでは応答しない。
#[tauri::command]
pub async fn zoo_take_call_accept(
    webview: Webview,
    core: Core_<'_>,
) -> Result<Option<String>, IpcError> {
    require(&webview, &core, REMOTE)?;
    Ok(core.take_call_accept().await)
}

fn leptos_only(core: &Core) -> Result<(), IpcError> {
    if core.ui() == UiMode::Leptos {
        Ok(())
    } else {
        Err(IpcError::new(
            ErrorCode::WrongMode,
            "(b) React の画面では使えないコマンドです",
        ))
    }
}

#[tauri::command]
pub async fn zoo_account_profiles(
    webview: Webview,
    core: Core_<'_>,
) -> Result<zoo2_client_types::accounts::AccountProfiles, IpcError> {
    require(&webview, &core, BUNDLED)?;
    leptos_only(&core)?;
    core.api().await?.account_profiles().await.map_err(to_ipc)
}

/// 添付の保存先の設定。古い版のアプリにはない。
#[tauri::command]
pub async fn zoo_download_settings(
    webview: Webview,
    core: Core_<'_>,
) -> Result<DownloadSettings, IpcError> {
    require(&webview, &core, ANY)?;
    Ok(core.downloads().settings())
}

#[tauri::command]
pub async fn zoo_set_download_settings(
    webview: Webview,
    core: Core_<'_>,
    settings: DownloadSettingsUpdate,
) -> Result<DownloadSettings, IpcError> {
    require(&webview, &core, ANY)?;
    core.downloads().set_settings(settings).map_err(to_ipc)
}

/// OS のフォルダーの選択。選ばれたら、Rust 側で保存先として保存し、新しい設定を返す。
/// 閉じたら `None`。パスは JS から指定できない。
#[tauri::command]
pub async fn zoo_pick_download_directory(
    webview: Webview,
    core: Core_<'_>,
    app: AppHandle,
) -> Result<Option<DownloadSettings>, IpcError> {
    require(&webview, &core, ANY)?;
    let Some(path) = crate::downloads::pick_directory(&app).await else {
        return Ok(None);
    };
    core.downloads()
        .set_directory(&path)
        .map(Some)
        .map_err(to_ipc)
}

/// 保存ダイアログを出し、書き込み先を決める。利用者が閉じたら `Cancelled`。
#[tauri::command]
pub async fn zoo_download_begin(
    webview: Webview,
    core: Core_<'_>,
    app: AppHandle,
    begin: DownloadBegin,
) -> Result<DownloadStart, IpcError> {
    require(&webview, &core, ANY)?;
    match core
        .downloads()
        .begin(&app, &begin.file_name)
        .await
        .map_err(to_ipc)?
    {
        Some((id, path)) => Ok(DownloadStart::Started {
            id,
            path: path.display().to_string(),
        }),
        None => Ok(DownloadStart::Cancelled),
    }
}

/// 本文（octet-stream）を、`x-zoo-download-id` のダウンロードに追記する。
#[tauri::command]
pub async fn zoo_download_write(
    webview: Webview,
    core: Core_<'_>,
    request: tauri::ipc::Request<'_>,
) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    let id = request
        .headers()
        .get(DOWNLOAD_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| IpcError::new(ErrorCode::Internal, "ダウンロードの ID がありません"))?;
    let bytes = match request.body() {
        tauri::ipc::InvokeBody::Raw(bytes) => bytes,
        tauri::ipc::InvokeBody::Json(_) => {
            return Err(IpcError::new(
                ErrorCode::Internal,
                "本文がバイト列ではありません",
            ));
        }
    };
    core.downloads().write(id, bytes).await.map_err(to_ipc)
}

/// 書き終えたら、`.part` を本来の名前に置き換える。
#[tauri::command]
pub async fn zoo_download_finish(
    webview: Webview,
    core: Core_<'_>,
    id: String,
) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    core.downloads().finish(&id).await.map_err(to_ipc)?;
    Ok(())
}

/// 中止して、途中までのファイルを消す。
#[tauri::command]
pub async fn zoo_download_cancel(
    webview: Webview,
    core: Core_<'_>,
    id: String,
) -> Result<(), IpcError> {
    require(&webview, &core, ANY)?;
    core.downloads().cancel(&id).await;
    Ok(())
}
