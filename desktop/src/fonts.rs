//! 同梱フォント (Geist) の登録。
//!
//! 可変フォントは Linux でウェイトが変わらなかったので、静的な TTF を 4 本入れている。

use anyhow::{Context as _, Result};
use gpui::{App, AssetSource, Font, FontFallbacks, FontStyle, FontWeight, font};

use crate::assets::Assets;

/// Web 版の `--font-sans` に当たるファミリー名。
pub const FAMILY: &str = "Geist";

/// 同梱する TTF (Regular 400 / Medium 500 / SemiBold 600 / Bold 700)。
pub const FILES: &[&str] = &[
    "fonts/Geist-Regular.ttf",
    "fonts/Geist-Medium.ttf",
    "fonts/Geist-SemiBold.ttf",
    "fonts/Geist-Bold.ttf",
];

/// 日本語など CJK を描くときのフォールバック順。
/// 同梱はしない。環境にあるものを使う (macOS は Hiragino Sans)。
pub const CJK_FALLBACKS: &[&str] = &["Noto Sans CJK JP", "Noto Sans JP", "Hiragino Sans"];

/// 同梱フォントを GPUI のテキストシステムに登録する。起動時に 1 回呼ぶ。
pub fn register(cx: &App) -> Result<()> {
    let mut data = Vec::with_capacity(FILES.len());
    for path in FILES {
        let bytes = Assets
            .load(path)?
            .with_context(|| format!("フォント {path} が埋め込まれていません"))?;
        data.push(bytes);
    }
    cx.text_system().add_fonts(data)
}

pub fn cjk_fallbacks() -> FontFallbacks {
    FontFallbacks::from_fonts(CJK_FALLBACKS.iter().map(|s| s.to_string()).collect())
}

/// Geist の指定ウェイトのフォント。CJK のフォールバック付き。
pub fn sans(weight: f32) -> Font {
    let mut f = font(FAMILY);
    f.weight = FontWeight(weight);
    f.style = FontStyle::Normal;
    f.fallbacks = Some(cjk_fallbacks());
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_font_files_are_embedded_ttf() {
        for path in FILES {
            let bytes = Assets.load(path).unwrap().expect(path);
            // TrueType のシグネチャ (0x00010000)。
            assert_eq!(&bytes[..4], &[0, 1, 0, 0], "{path}");
        }
    }

    #[test]
    fn sans_has_fallbacks_in_order() {
        let f = sans(600.0);
        assert_eq!(f.weight, FontWeight(600.0));
        assert_eq!(
            f.fallbacks.unwrap().fallback_list(),
            ["Noto Sans CJK JP", "Noto Sans JP", "Hiragino Sans"]
        );
    }
}
