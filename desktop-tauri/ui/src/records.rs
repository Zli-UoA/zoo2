//! 一度 `encrypted = true` を見たチャンネルの記録（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
//! この画面は暗号ストアを持たないので、WebView の `localStorage` にチャンネルごとに置く
//! （Web 版と同じキー。ID はサーバーも知っている値なので暗号化しない）。ログアウトでは消さない。

const PREFIX: &str = "zoo.e2ee.encryptedChannel:";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// 以前に `encrypted = true` を見たか。記録を読めなければ `None`。
pub fn recorded(channel_id: &str) -> Option<bool> {
    let item = storage()?.get_item(&format!("{PREFIX}{channel_id}")).ok()?;
    Some(item.is_some())
}

/// `encrypted = true` のチャンネルを記録する。
pub fn remember<'a>(channel_ids: impl IntoIterator<Item = &'a str>) {
    let Some(storage) = storage() else {
        return;
    };
    for id in channel_ids {
        if recorded(id) == Some(false) {
            // 保存できなければ、次にチャンネルの一覧が届いたときにまた試す。
            let _ = storage.set_item(&format!("{PREFIX}{id}"), "1");
        }
    }
}
