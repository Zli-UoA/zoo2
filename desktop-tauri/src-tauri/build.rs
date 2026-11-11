use zoo2_build_commands::COMMANDS;

fn main() {
    // コマンドを 1 つずつ宣言すると、`allow-<名前>` の許可が作られ、capabilities で
    // ウィンドウごとに許せる。app の manifest があると、Tauri は宣言のないコマンドを
    // どのウィンドウからも呼べないものとして拒否する (docs/desktop-tauri.md)。
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("tauri-build に失敗しました");
}

mod zoo2_build_commands {
    include!("src/commands_list.rs");
}
