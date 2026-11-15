//! TUI の E2EE（docs/e2ee.md）。#359 は暗号ストアと端末の登録、#360 は初回設定と解錠
//! （リカバリーキーと、ループバックのリダイレクトによる再認証）、#361 は暗号化チャンネルと DM の
//! 送受信（`send` と `store` の Megolm の部分）を持つ。

pub mod manager;
pub mod reauth;
pub mod send;
pub mod setup;
pub mod store;
