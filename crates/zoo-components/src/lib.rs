//! zoo2 のメッセージ部品（Discord Components V2 形式）。
//!
//! - `model`: 部品の木と JSON の読み書き
//! - `validate`: 配置のルールと上限の検証
//! - `text`: 本文の代わりの文字
//! - `legacy`: 旧形式（MessageCard と Block 一覧）からの変換
//!
//! backend・client-types（Tauri の WebView、TUI）が同じ定義を使う。
//! Web と Android は `fixtures/` の JSON で同じ結果になることを確かめる。

pub mod legacy;
pub mod model;
pub mod text;
pub mod validate;

pub use model::*;
pub use validate::{Context, ValidationError, limits};
