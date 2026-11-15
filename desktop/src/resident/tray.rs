//! トレイアイコン。メニューは「開く」と「終了」の 2 つ (Tauri 版と同じ。文言は日本語で固定)。
//!
//! Linux は ksni (StatusNotifierItem)、macOS は tray-icon。macOS はビルドできる形までで、未検証。

use super::{ResidentEvent, send};

const ICON_PNG: &[u8] = include_bytes!("../../assets/tray.png");
const LABEL_OPEN: &str = "開く";
const LABEL_QUIT: &str = "終了";

/// トレイを起動する。トレイが使えない環境 (StatusNotifierWatcher がないなど) では、
/// メッセージを出して何もしない。アプリ自体は動く。
pub fn spawn() {
    #[cfg(target_os = "linux")]
    linux::spawn();
    #[cfg(target_os = "macos")]
    macos::spawn();
}

#[cfg(target_os = "linux")]
mod linux {
    use ksni::{Icon, MenuItem, Tray, TrayMethods, menu::StandardItem};

    use super::*;

    pub struct ZooTray {
        icon: Vec<Icon>,
    }

    /// PNG を StatusNotifierItem の ARGB32 (ネットワークバイトオーダー) にする。
    fn pixmap() -> Vec<Icon> {
        let Ok(image) = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        else {
            return Vec::new();
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let mut data = Vec::with_capacity(rgba.len());
        for pixel in rgba.pixels() {
            let [r, g, b, a] = pixel.0;
            data.extend_from_slice(&[a, r, g, b]);
        }
        vec![Icon {
            width: width as i32,
            height: height as i32,
            data,
        }]
    }

    impl Tray for ZooTray {
        fn id(&self) -> String {
            "zoo2-desktop".into()
        }
        fn title(&self) -> String {
            "zoo2".into()
        }
        fn icon_pixmap(&self) -> Vec<Icon> {
            self.icon.clone()
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            send(ResidentEvent::Show);
        }
        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                StandardItem {
                    label: LABEL_OPEN.into(),
                    activate: Box::new(|_| send(ResidentEvent::Show)),
                    ..Default::default()
                }
                .into(),
                StandardItem {
                    label: LABEL_QUIT.into(),
                    activate: Box::new(|_| send(ResidentEvent::Quit)),
                    ..Default::default()
                }
                .into(),
            ]
        }
    }

    pub fn spawn() {
        // ksni は自前のスレッドで動く。ハンドルは捨てず、プロセスの終わりまで持つ。
        std::thread::spawn(|| {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    eprintln!("トレイ用のランタイムを作れませんでした: {err}");
                    return;
                }
            };
            runtime.block_on(async {
                match (ZooTray { icon: pixmap() }).spawn().await {
                    Ok(_handle) => std::future::pending::<()>().await,
                    Err(err) => eprintln!("トレイを出せませんでした: {err}"),
                }
            });
        });
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{Menu, MenuEvent, MenuItem},
    };

    use super::*;

    /// tray-icon はメインスレッドで作る必要がある。gpui の `run` の中 (メインスレッド) から呼ぶ。
    pub fn spawn() {
        let open = MenuItem::new(LABEL_OPEN, true, None);
        let quit = MenuItem::new(LABEL_QUIT, true, None);
        let menu = Menu::new();
        if menu.append_items(&[&open, &quit]).is_err() {
            return;
        }
        let Ok(image) = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        else {
            return;
        };
        let rgba = image.to_rgba8();
        let (width, height) = rgba.dimensions();
        let Ok(icon) = Icon::from_rgba(rgba.into_raw(), width, height) else {
            return;
        };
        let (open_id, quit_id) = (open.id().clone(), quit.id().clone());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == open_id {
                send(ResidentEvent::Show);
            } else if event.id == quit_id {
                send(ResidentEvent::Quit);
            }
        }));
        match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip("zoo2")
            .build()
        {
            // 捨てるとアイコンが消える。プロセスの終わりまで持つ。
            Ok(tray) => std::mem::forget(tray),
            Err(err) => eprintln!("トレイを出せませんでした: {err}"),
        }
    }
}
