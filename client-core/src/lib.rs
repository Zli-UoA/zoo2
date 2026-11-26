pub mod api;
pub mod auth;
pub mod config;
pub mod electric;
pub mod http;
pub mod poll;
pub mod update;

// 型と純粋関数は zoo2-client-types にある。TUI はこれまでどおり
// `zoo2_client_core::{mfm, store}` から使える。
pub use zoo2_client_types::{mentions, mfm, notify, store, text};

/// メッセージ部品（backend と同じ定義）。
pub use zoo2_client_types::components;
