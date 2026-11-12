//! 同梱フォント (Geist と Noto Sans JP) の登録。
//!
//! 可変フォントは Linux でウェイトが変わらなかったので、どちらも静的な TTF を 4 本ずつ入れている。
//! Linux の GPUI (cosmic-text) は `Font.fallbacks` を無視し、Geist にない文字は登録済みのフォントから
//! ウェイトが合うものを選ぶ。Noto Sans JP の 4 ウェイトを登録しておくと、かなと漢字も同じ太さで出る。
//! 同梱しないと OS のフォールバックになり、かなが常に細字になる。

use anyhow::{Context as _, Result};
use gpui::{App, AssetSource, Font, FontFallbacks, FontStyle, FontWeight, font};

use crate::assets::Assets;

/// Web 版の `--font-sans` に当たるファミリー名。
pub const FAMILY: &str = "Geist";

/// 同梱する TTF。Geist と Noto Sans JP のそれぞれ Regular 400 / Medium 500 / SemiBold 600 / Bold 700。
pub const FILES: &[&str] = &[
    "fonts/Geist-Regular.ttf",
    "fonts/Geist-Medium.ttf",
    "fonts/Geist-SemiBold.ttf",
    "fonts/Geist-Bold.ttf",
    "fonts/NotoSansJP-Regular.ttf",
    "fonts/NotoSansJP-Medium.ttf",
    "fonts/NotoSansJP-SemiBold.ttf",
    "fonts/NotoSansJP-Bold.ttf",
];

/// 日本語など CJK を描くときのフォールバック順。frontend と同じ Noto Sans JP (同梱) を先にし、
/// それでも足りない文字だけ macOS の Hiragino Sans に回す。Linux ではこの指定は使われない (上の説明)。
pub const CJK_FALLBACKS: &[&str] = &["Noto Sans JP", "Hiragino Sans"];

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
            ["Noto Sans JP", "Hiragino Sans"]
        );
    }
}
