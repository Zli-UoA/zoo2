//! 一度 `encrypted = true` を見たチャンネルの記録（docs/e2ee.md の「暗号化の格下げを防ぐ」）。
//! この画面は暗号ストアを持たないので、WebView の `localStorage` にチャンネルごとに置く
//! （Web 版と同じキー。ID はサーバーも知っている値なので暗号化しない）。保存できなくても、画面を
//! 開いている間は覚える。ログアウトでは消さない。

use std::{cell::RefCell, collections::HashSet};

const PREFIX: &str = "zoo.e2ee.encryptedChannel:";

thread_local! {
    /// この画面で `encrypted = true` を見たチャンネル。
    static SEEN: RefCell<HashSet<String>> = RefCell::default();
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// 以前に `encrypted = true` を見たか。記録を読めなければ `None`。
pub fn recorded(channel_id: &str) -> Option<bool> {
    if SEEN.with_borrow(|seen| seen.contains(channel_id)) {
        return Some(true);
    }
    let item = storage()?.get_item(&format!("{PREFIX}{channel_id}")).ok()?;
    Some(item.is_some())
}

/// `encrypted = true` のチャンネルを記録する。
pub fn remember<'a>(channel_ids: impl IntoIterator<Item = &'a str>) {
    let storage = storage();
    for id in channel_ids {
        SEEN.with_borrow_mut(|seen| {
            if !seen.contains(id) {
                seen.insert(id.to_owned());
            }
        });
        let Some(storage) = &storage else {
            continue;
        };
        let key = format!("{PREFIX}{id}");
        if storage.get_item(&key).is_ok_and(|item| item.is_none()) {
            // 保存できなければ、次にチャンネルの一覧が届いたときにまた試す。
            let _ = storage.set_item(&key, "1");
        }
    }
}
