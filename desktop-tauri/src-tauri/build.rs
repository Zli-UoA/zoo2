use zoo2_build_commands::COMMANDS;

fn main() {
    // コマンドを 1 つずつ宣言すると、`allow-<名前>` の許可が作られる。宣言しないと
    // 許可の識別子がなく、すべてのウィンドウから呼べてしまう (docs/desktop-tauri.md)。
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri-build に失敗しました");
}

mod zoo2_build_commands {
    include!("src/commands_list.rs");
}
