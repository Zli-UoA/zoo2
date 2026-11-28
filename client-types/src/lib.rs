//! zoo2 のクライアントで共有する型と純粋関数。
//!
//! `zoo2-client-core` (TUI、Tauri 版の Rust 側) と、Tauri 版の WebView 側
//! (Leptos、wasm32) の両方から使う。通信、ファイル、時計には触れない。

pub mod accounts;
/// メッセージ部品（`messages.components`）。backend と同じ crate を使う。
pub use zoo_components as components;
pub mod api;
pub mod ipc;
pub mod link_store;
pub mod links;
pub mod mentions;
pub mod mfm;
pub mod notify;
pub mod rows;
pub mod shape;
pub mod store;
pub mod text;
