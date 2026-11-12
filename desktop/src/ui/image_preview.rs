//! `image-preview.tsx`。読み込み中は blurhash を引き伸ばして見せ、読めたら画像を出す。

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use gpui::{
    App, Corners, ImageSource, ObjectFit, RenderImage, StyleRefinement, StyledImage as _, Window,
    canvas, div, img, prelude::*,
};
use image::{Frame, RgbaImage};

use super::palette;
use crate::tokens::radius;

/// blurhash を描く画素数 (`<canvas width=32 height=32>`)。
pub const BLUR_SIZE: u32 = 32;
/// これより長い blurhash は使わない。
pub const MAX_HASH_LEN: usize = 166;

/// blurhash の画像を覚えておく数の上限。
pub const CACHE_CAPACITY: usize = 256;

struct Slot {
    /// 読めなかった blurhash は `None` の印だけを置く。上限には数える。
    image: Option<Arc<RenderImage>>,
    used: u64,
}

/// 使った順に捨てる blurhash の入れ物。キーは相手 (他のユーザー) から来る文字列なので、数を抑える。
pub struct BlurCache {
    capacity: usize,
    tick: u64,
    map: HashMap<String, Slot>,
}

impl BlurCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            tick: 0,
            map: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// 画像と、入れたせいで追い出された画像 (アトラスから外すため) を返す。
    /// 先に `get` で引くので、既にあるキーでは文字列を作らない。
    pub fn get_or_decode(
        &mut self,
        hash: &str,
        decode: impl FnOnce(&str) -> Option<Arc<RenderImage>>,
    ) -> (Option<Arc<RenderImage>>, Option<Arc<RenderImage>>) {
        self.tick += 1;
        if let Some(slot) = self.map.get_mut(hash) {
            slot.used = self.tick;
            return (slot.image.clone(), None);
        }
        let mut evicted = None;
        if self.map.len() >= self.capacity {
            let oldest = self
                .map
                .iter()
                .min_by_key(|(_, slot)| slot.used)
                .map(|(key, _)| key.clone());
            if let Some(key) = oldest {
                evicted = self.map.remove(&key).and_then(|slot| slot.image);
            }
        }
        let image = decode(hash);
        self.map.insert(
            hash.to_string(),
            Slot {
                image: image.clone(),
                used: self.tick,
            },
        );
        (image, evicted)
    }
}

thread_local! {
    static CACHE: RefCell<BlurCache> = RefCell::new(BlurCache::new(CACHE_CAPACITY));
}

/// blurhash を 32x32 の画像にする。壊れた値や長すぎる値は `None`。
/// 追い出された画像は、このウィンドウのアトラスから外す。
pub fn blur_image(hash: &str, window: &mut Window, cx: &mut App) -> Option<Arc<RenderImage>> {
    if hash.len() > MAX_HASH_LEN {
        return None;
    }
    let (image, evicted) = CACHE.with(|c| c.borrow_mut().get_or_decode(hash, decode));
    if let Some(old) = evicted {
        cx.drop_image(old, Some(window));
    }
    image
}

fn decode(hash: &str) -> Option<Arc<RenderImage>> {
    let mut pixels = blurhash::decode(hash, BLUR_SIZE, BLUR_SIZE, 1.0).ok()?;
    // GPUI の画像は BGRA。
    for px in pixels.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let buffer = RgbaImage::from_raw(BLUR_SIZE, BLUR_SIZE, pixels)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(buffer)])))
}

/// 箱いっぱいに blurhash の画像を引き伸ばして描く。角丸は箱に合わせる。
/// `img` の要素は元の縦横比で並ぶので使わず、`paint_image` に箱の大きさを渡す。
fn blur_layer(image: Arc<RenderImage>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let _ = window.paint_image(bounds, Corners::all(radius::LG), image.clone(), 0, false);
        },
    )
    .size_full()
}

#[derive(IntoElement)]
pub struct ImagePreview {
    src: Option<ImageSource>,
    blurhash: Option<String>,
    fit: ObjectFit,
    style: StyleRefinement,
}

impl ImagePreview {
    pub fn new() -> Self {
        Self {
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
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        // `relative overflow-hidden rounded-lg bg-muted`
        let mut root = div()
            .relative()
            .overflow_hidden()
            .rounded(radius::LG)
            .bg(p.t.muted);
        root.style().refine(&self.style);
        let blur = self
            .blurhash
            .as_deref()
            .and_then(|hash| blur_image(hash, window, cx));
        let fill = || div().absolute().top_0().left_0().size_full();
        match self.src {
            // 画像が無いうちは blurhash を見せる。
            None => root.children(blur.map(|image| fill().child(blur_layer(image)))),
            // 画像が読めるまでの間だけ blurhash を出し、読めたら画像に置き換わる (Web 版の `!loaded`)。
            // 読み込みが `LOADING_DELAY` (GPUI 側の待ち) より短ければ、blurhash は出ない。
            Some(src) => {
                let mut photo = img(src)
                    .size_full()
                    .rounded(radius::LG)
                    .object_fit(self.fit);
                if let Some(image) = blur {
                    photo =
                        photo.with_loading(move || blur_layer(image.clone()).into_any_element());
                }
                root.child(fill().child(photo))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy() -> Arc<RenderImage> {
        let buf = RgbaImage::new(1, 1);
        Arc::new(RenderImage::new(vec![Frame::new(buf)]))
    }

    #[test]
    fn decodes_a_valid_hash() {
        assert!(decode("LEHV6nWB2yk8pyo0adR*.7kCMdnj").is_some());
    }

    #[test]
    fn rejects_invalid_hashes() {
        assert!(decode("invalid").is_none());
    }

    #[test]
    fn hit_does_not_decode_again() {
        let mut cache = BlurCache::new(4);
        let mut calls = 0;
        for _ in 0..3 {
            cache.get_or_decode("a", |_| {
                calls += 1;
                Some(dummy())
            });
        }
        assert_eq!(calls, 1);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn evicts_the_least_recently_used() {
        let mut cache = BlurCache::new(2);
        cache.get_or_decode("a", |_| Some(dummy()));
        cache.get_or_decode("b", |_| Some(dummy()));
        // a を使い直したので、次に追い出されるのは b。
        cache.get_or_decode("a", |_| unreachable!());
        let (_, evicted) = cache.get_or_decode("c", |_| Some(dummy()));
        assert!(evicted.is_some());
        assert_eq!(cache.len(), 2);
        let mut decoded_b = false;
        cache.get_or_decode("b", |_| {
            decoded_b = true;
            Some(dummy())
        });
        assert!(decoded_b);
    }

    #[test]
    fn failures_are_cached_and_count_toward_the_limit() {
        let mut cache = BlurCache::new(2);
        let mut calls = 0;
        for _ in 0..3 {
            let (image, _) = cache.get_or_decode("bad", |_| {
                calls += 1;
                None
            });
            assert!(image.is_none());
        }
        assert_eq!(calls, 1);
        cache.get_or_decode("x", |_| Some(dummy()));
        // 上限 2 に 3 つ目を入れると、失敗の印が追い出される (画像は無いので `None`)。
        let (_, evicted) = cache.get_or_decode("y", |_| Some(dummy()));
        assert!(evicted.is_none());
        assert_eq!(cache.len(), 2);
    }
}
