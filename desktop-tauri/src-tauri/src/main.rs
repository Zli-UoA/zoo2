// release ビルドの Windows でコンソールを出さない。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    zoo2_desktop_tauri::run();
}
