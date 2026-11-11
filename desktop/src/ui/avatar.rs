//! `avatar.tsx`。丸いアイコン。画像が無い・読めないときは頭文字を出す。

use gpui::{
    AnyElement, App, ImageSource, ObjectFit, SharedString, StyledImage as _, Window, div, img,
    prelude::*, px,
};

use super::{Palette, TextSpec, ring};
use crate::fonts;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarSize {
    /// `data-[size=sm]` (24px)
    Sm,
    /// 既定 (32px)
    Default,
    /// `data-[size=lg]` (40px)
    Lg,
}

impl AvatarSize {
    pub fn px(self) -> f32 {
        match self {
            Self::Sm => 24.0,
            Self::Default => 32.0,
            Self::Lg => 40.0,
        }
    }

    /// 頭文字の大きさ。sm だけ `text-xs`。
    pub fn text(self) -> TextSpec {
        match self {
            Self::Sm => TextSpec::XS,
            _ => TextSpec::SM,
        }
    }

    /// `AvatarBadge` の直径。
    pub fn badge(self) -> f32 {
        match self {
            Self::Sm => 8.0,
            Self::Default => 10.0,
            Self::Lg => 12.0,
        }
    }
}

#[derive(IntoElement)]
pub struct Avatar {
    p: Palette,
    size: AvatarSize,
    initials: Option<SharedString>,
    image: Option<ImageSource>,
    badge: bool,
    /// `AvatarGroup` の中で付く `ring-2 ring-background`。
    ringed: bool,
}

impl Avatar {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            size: AvatarSize::Default,
            initials: None,
            image: None,
            badge: false,
            ringed: false,
        }
    }

    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }

    pub fn initials(mut self, initials: impl Into<SharedString>) -> Self {
        self.initials = Some(initials.into());
        self
    }

    /// 画像。URL (`https://…`) は `gpui::img` が読む。
    pub fn image(mut self, source: impl Into<ImageSource>) -> Self {
        self.image = Some(source.into());
        self
    }

    pub fn badge(mut self, badge: bool) -> Self {
        self.badge = badge;
        self
    }

    pub fn ringed(mut self, ringed: bool) -> Self {
        self.ringed = ringed;
        self
    }
}

fn fallback(p: &Palette, size: AvatarSize, initials: Option<SharedString>) -> AnyElement {
    let text = size.text();
    // `flex size-full items-center justify-center rounded-full bg-muted text-sm text-muted-foreground`
    div()
        .flex()
        .items_center()
        .justify_center()
        .size_full()
        .rounded_full()
        .bg(p.t.muted)
        .text_color(p.t.muted_foreground)
        .text_size(text.size())
        .line_height(text.line())
        .font(fonts::sans(400.0))
        .children(initials)
        .into_any_element()
}

impl RenderOnce for Avatar {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.p.t;
        let side = self.size.px();
        let size = self.size;
        let content: AnyElement = match self.image {
            Some(source) => {
                let p = self.p;
                let initials = self.initials.clone();
                img(source)
                    .size_full()
                    .rounded_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(move || fallback(&p, size, initials.clone()))
                    .into_any_element()
            }
            None => fallback(&self.p, size, self.initials),
        };
        // `relative flex size-8 shrink-0 rounded-full`。
        // `after:border-border` の枠は画像の上に重ねる (`mix-blend` は GPUI に無い)。
        div()
            .relative()
            .flex()
            .flex_none()
            .size(px(side))
            .rounded_full()
            .when(self.ringed, |d| d.shadow(vec![ring(t.background, 2.0)]))
            .child(content)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .rounded_full()
                    .border_1()
                    .border_color(t.border),
            )
            .when(self.badge, |d| {
                // `absolute right-0 bottom-0 z-10 rounded-full bg-primary ring-2 ring-background`
                d.child(
                    div()
                        .absolute()
                        .right_0()
                        .bottom_0()
                        .size(px(size.badge()))
                        .rounded_full()
                        .bg(t.primary)
                        .shadow(vec![ring(t.background, 2.0)]),
                )
            })
    }
}

/// `AvatarGroup`。`-space-x-2` で 8px ずつ重ね、各アイコンに `ring-2 ring-background` を付ける。
#[derive(IntoElement)]
pub struct AvatarGroup {
    items: Vec<Avatar>,
    count: Option<(Palette, AvatarSize, SharedString)>,
}

impl AvatarGroup {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            count: None,
        }
    }

    pub fn avatar(mut self, avatar: Avatar) -> Self {
        self.items.push(avatar);
        self
    }

    /// 末尾の「+5」。大きさは群のサイズにそろえる。
    pub fn count(mut self, p: &Palette, size: AvatarSize, label: impl Into<SharedString>) -> Self {
        self.count = Some((*p, size, label.into()));
        self
    }
}

impl Default for AvatarGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for AvatarGroup {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut group = div().flex();
        let mut first = true;
        for item in self.items {
            let item = item.ringed(true);
            group = group.child(div().when(!first, |d| d.ml(px(-8.0))).child(item));
            first = false;
        }
        if let Some((p, size, label)) = self.count {
            let text = size.text();
            // `size-8 rounded-full bg-muted text-sm text-muted-foreground ring-2 ring-background`
            group = group.child(
                div()
                    .when(!first, |d| d.ml(px(-8.0)))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(size.px()))
                    .rounded_full()
                    .bg(p.t.muted)
                    .text_color(p.t.muted_foreground)
                    .text_size(text.size())
                    .line_height(text.line())
                    .shadow(vec![ring(p.t.background, 2.0)])
                    .child(label),
            );
        }
        group
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_are_24_32_40() {
        assert_eq!(AvatarSize::Sm.px(), 24.0);
        assert_eq!(AvatarSize::Default.px(), 32.0);
        assert_eq!(AvatarSize::Lg.px(), 40.0);
    }

    #[test]
    fn badge_grows_with_size() {
        assert_eq!(AvatarSize::Sm.badge(), 8.0);
        assert_eq!(AvatarSize::Default.badge(), 10.0);
        assert_eq!(AvatarSize::Lg.badge(), 12.0);
    }

    #[test]
    fn small_avatar_uses_text_xs() {
        assert_eq!(AvatarSize::Sm.text(), TextSpec::XS);
        assert_eq!(AvatarSize::Lg.text(), TextSpec::SM);
    }
}
