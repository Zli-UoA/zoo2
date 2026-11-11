//! `image-preview.tsx`。読み込み中は blurhash を引き伸ばして見せ、読めたら画像を出す。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, ImageSource, ObjectFit, RenderImage, StyleRefinement, StyledImage as _, Window, div, img,
    prelude::*,
};
use image::{Frame, RgbaImage};

use super::Palette;
use crate::tokens::radius;

/// blurhash を描く画素数 (`<canvas width=32 height=32>`)。
pub const BLUR_SIZE: u32 = 32;
/// これより長い blurhash は使わない。
pub const MAX_HASH_LEN: usize = 166;

thread_local! {
    static CACHE: RefCell<HashMap<String, Option<Arc<RenderImage>>>> = RefCell::new(HashMap::new());
}

/// blurhash を 32x32 の画像にする。壊れた値や長すぎる値は `None`。
pub fn blur_image(hash: &str) -> Option<Arc<RenderImage>> {
    if hash.len() > MAX_HASH_LEN {
        return None;
    }
    CACHE.with(|c| {
        c.borrow_mut()
            .entry(hash.to_string())
            .or_insert_with(|| decode(hash))
            .clone()
    })
}

fn decode(hash: &str) -> Option<Arc<RenderImage>> {
    let mut pixels = blurhash::decode(hash, BLUR_SIZE, BLUR_SIZE, 1.0).ok()?;
    // GPUI の画像は BGRA。
    for px in pixels.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(BLUR_SIZE, BLUR_SIZE, pixels)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

#[derive(IntoElement)]
pub struct ImagePreview {
    p: Palette,
    src: Option<ImageSource>,
    blurhash: Option<String>,
    fit: ObjectFit,
    style: StyleRefinement,
}

impl ImagePreview {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            src: None,
            blurhash: None,
            fit: ObjectFit::Contain,
            style: StyleRefinement::default(),
        }
    }

    pub fn src(mut self, src: impl Into<ImageSource>) -> Self {
        self.src = Some(src.into());
        self
    }

    pub fn blurhash(mut self, blurhash: Option<&str>) -> Self {
        self.blurhash = blurhash.map(str::to_string);
        self
    }

    /// `imageClassName` (既定は `object-contain`)。
    pub fn object_fit(mut self, fit: ObjectFit) -> Self {
        self.fit = fit;
        self
    }
}

impl Styled for ImagePreview {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ImagePreview {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        // `relative overflow-hidden rounded-lg bg-muted`
        let mut root = div()
            .relative()
            .overflow_hidden()
            .rounded(radius::LG)
            .bg(self.p.t.muted);
        root.style().refine(&self.style);
        let blur = self
            .blurhash
            .as_deref()
            .and_then(blur_image)
            .map(|image| {
                div().absolute().top_0().left_0().size_full().child(
                    img(ImageSource::Render(image))
                        .size_full()
                        .rounded(radius::LG)
                        .object_fit(ObjectFit::Fill),
                )
            });
        let photo = self.src.map(|src| {
            div().absolute().top_0().left_0().size_full().child(
                img(src)
                    .size_full()
                    .rounded(radius::LG)
                    .object_fit(self.fit),
            )
        });
        // 画像が読めるまでは blurhash が下にあり、読めた画像がその上に重なる。
        root.children(blur).children(photo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_valid_hash() {
        assert!(blur_image("LEHV6nWB2yk8pyo0adR*.7kCMdnj").is_some());
    }

    #[test]
    fn rejects_invalid_and_long_hashes() {
        assert!(blur_image("invalid").is_none());
        assert!(blur_image(&"L".repeat(MAX_HASH_LEN + 1)).is_none());
    }
}
