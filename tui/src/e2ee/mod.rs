//! TUI の E2EE（docs/e2ee.md）。#359 は暗号ストアと端末の登録、#360 は初回設定と解錠
//! （リカバリーキーと、ループバックのリダイレクトによる再認証）を持つ。

pub mod manager;
pub mod reauth;
pub mod setup;
pub mod store;
