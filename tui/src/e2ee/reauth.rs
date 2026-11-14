//! TUI の直前の再認証（docs/e2ee.md の「クロス署名」の API：TUI はブラウザーで開き、ループバックの
//! リダイレクトで受け取る。#360）。
//!
//! 認可コードフローとリダイレクトの受け取りは `client-core` の [`LoopbackReauth`] が行う。ここは、
//! ブラウザーを開くことと、SSH の先で動かしているときの案内を持つ。

use std::process::Stdio;

use anyhow::Result;

use crate::auth::{self, reauth::LoopbackReauth};
use crate::config::OidcConfig;

/// SSH の先で動かしているか（ブラウザーが別のマシンにあり、ループバックの URL に届かないかもしれない）。
pub fn over_ssh() -> bool {
    ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
}

/// SSH の先で動かしているときの案内。ポートを転送するか、初回設定を Web か Android で行う。
pub fn ssh_guidance(port: u16) -> Vec<String> {
    vec![
        "SSH の先で動かしているため、手元のブラウザーからこの URL に届かないことがあります。"
            .into(),
        "次のどちらかで進めてください。".into(),
        format!(
            "・手元のマシンで `ssh -L {port}:127.0.0.1:{port} <このホスト>` を実行してから、URL を開く"
        ),
        "・初回設定を Web か Android で行う（解錠は、このあとリカバリーキーだけでできます）".into(),
    ]
}

/// URL を既定のブラウザーで開く（開けなくても、画面に出した URL を利用者が開ける）。
pub fn open_browser(url: &str) {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(command)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// 再認証の待ち受けを始める。OIDC の設定とディスカバリーの結果も返す。
pub async fn start(
    http: &reqwest::Client,
    oidc: &OidcConfig,
) -> Result<(LoopbackReauth, auth::Discovery)> {
    let discovery = auth::discover(http, &oidc.issuer).await?;
    let reauth = LoopbackReauth::start(&discovery, oidc).await?;
    Ok((reauth, discovery))
}
