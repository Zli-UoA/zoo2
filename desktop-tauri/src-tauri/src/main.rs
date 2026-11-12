// release ビルドの Windows でコンソールを出さない (今は Windows を配布しないが、ひな形どおり)。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    zoo2_desktop_tauri::run();
}
