//! WebView から呼ぶコマンド。名前は `zoo2_client_types::ipc::command` と同じ。
//!
//! 呼べるウィンドウは capabilities で絞っているが、二重の防御として、ここでも呼び出し元の
//! URL を確かめる。同梱の画面からトークンを取り出せないようにするため。

use std::sync::Arc;

use tauri::{State, Webview};
use zoo2_client_types::{
    ipc::{
        AccessToken, DeviceLogin, ErrorCode, FocusState, IpcError, NotificationSettings,
        ObservedNotifications, OpenChannel, SendMessage, SendResult, SessionInfo, UiMode,
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
