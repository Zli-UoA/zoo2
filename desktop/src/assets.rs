//! `assets/` を実行ファイルに埋め込み、GPUI のアセットソースにする。
//! フォント (`fonts/`)、アイコン (`icons/`)、ロゴ (`zoo.svg`) が入っている。

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, ImageSource, Img, Pixels, Resource, SharedString, img, prelude::*};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets"]
struct Embedded;

#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

impl Assets {
    /// ロゴのパス。ロゴは色を持つので `svg()` でなく `img()` で描く。
    pub const LOGO: &'static str = "zoo.svg";
}

/// ロゴの `img()`。`svg()` は透明度しか使わず色が出ないので、ロゴは `img()` で描く。
/// `img("zoo.svg")` と書くと、文字列が URI として読めてしまい HTTP で取りに行って失敗する。
/// そのため、埋め込みのリソースだと明示して渡す。
pub fn logo(size: Pixels) -> Img {
    img(ImageSource::Resource(Resource::Embedded(
        Assets::LOGO.into(),
    )))
    .w(size)
    .h(size)
    .flex_none()
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(Embedded::get(path).map(|file| file.data))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let prefix = if path.is_empty() || path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };
        Ok(Embedded::iter()
            .filter(|name| name.starts_with(&prefix))
            .map(|name| SharedString::from(name.to_string()))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_embedded_files() {
        assert!(Assets.load(Assets::LOGO).unwrap().is_some());
        assert!(Assets.load("nope.svg").unwrap().is_none());
        let fonts = Assets.list("fonts").unwrap();
        for name in ["Regular", "Medium", "SemiBold", "Bold"] {
            let path = format!("fonts/Geist-{name}.ttf");
            assert!(
                fonts.iter().any(|f| f.as_ref() == path),
                "{path} が埋め込まれていません"
            );
        }
    }
}
