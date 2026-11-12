//! Tauri の IPC (`window.__TAURI__`) を呼ぶ薄い層。値は JSON の文字列を経由して受け渡す。

use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::prelude::*;
use zoo2_client_types::ipc::{ErrorCode, IpcError};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen)]
    async fn tauri_listen(event: &str, handler: &Closure<dyn FnMut(JsValue)>) -> JsValue;
}

fn to_js<T: Serialize>(value: &T) -> JsValue {
    let text = serde_json::to_string(value).expect("IPC の引数を JSON にできません");
    js_sys::JSON::parse(&text).expect("IPC の引数を JS の値にできません")
}

fn from_js<T: DeserializeOwned>(value: &JsValue) -> Result<T, IpcError> {
    let text: String = js_sys::JSON::stringify(value)
        .map(Into::into)
        .unwrap_or_else(|_| "null".into());
    serde_json::from_str(&text).map_err(|err| IpcError::new(ErrorCode::Internal, err.to_string()))
}

/// コマンドを呼ぶ。`args` は `{ 引数名: 値 }` の形のもの。
pub async fn invoke<R: DeserializeOwned>(cmd: &str, args: impl Serialize) -> Result<R, IpcError> {
    match tauri_invoke(cmd, to_js(&args)).await {
        Ok(value) => from_js(&value),
        Err(err) => Err(from_js(&err).unwrap_or_else(|_| {
            IpcError::new(
                ErrorCode::Internal,
                err.as_string().unwrap_or_else(|| format!("{err:?}")),
            )
        })),
    }
}

/// 引数のないコマンド。
pub async fn invoke0<R: DeserializeOwned>(cmd: &str) -> Result<R, IpcError> {
    invoke(cmd, serde_json::json!({})).await
}

/// イベントを購読する。画面と同じだけ生きるので、解除はしない。
pub async fn listen<T: DeserializeOwned + 'static>(
    event: &str,
    mut handler: impl FnMut(T) + 'static,
) {
    let closure = Closure::<dyn FnMut(JsValue)>::new(move |event: JsValue| {
        let payload = js_sys::Reflect::get(&event, &"payload".into()).unwrap_or(JsValue::NULL);
        match from_js::<T>(&payload) {
            Ok(value) => handler(value),
            Err(err) => web_sys::console::error_1(&err.message.into()),
        }
    });
    tauri_listen(event, &closure).await;
    closure.forget();
}

/// ウィンドウが前面にあるか。
pub fn has_focus() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.has_focus().ok())
        .unwrap_or(true)
}
