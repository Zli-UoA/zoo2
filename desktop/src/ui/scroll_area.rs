//! `scroll-area.tsx`。縦にスクロールする領域で、右に細いスクロールバーが付く。
//! 親の `className` に当たる大きさや枠は `Styled` で渡す。

use gpui::{
    AnyElement, App, ElementId, ScrollHandle, StyleRefinement, Window, div, prelude::*, px,
    relative,
};

use super::palette;

/// スクロールバーの幅 (`w-2.5`)。
pub const BAR_WIDTH: f32 = 10.0;

/// つまみの位置。どちらも 0〜1 の割合。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thumb {
    /// つまみの長さ / トラックの長さ。
    pub ratio: f32,
    /// スクロール位置 (0 = 先頭、1 = 末尾)。
    pub position: f32,
}

impl Thumb {
    /// 見えている高さ、全体の高さ、スクロール量から作る。内容が収まるなら `None`。
    pub fn from_extent(viewport: f32, content: f32, offset: f32) -> Option<Self> {
        if content <= viewport || viewport <= 0.0 {
            return None;
        }
        let max = content - viewport;
        Some(Self {
            ratio: viewport / content,
            position: (offset / max).clamp(0.0, 1.0),
        })
    }

    /// トラックの上端からつまみの上端までの割合 (トラックの長さに対して)。
    pub fn top(self) -> f32 {
        (1.0 - self.ratio) * self.position
    }
}

#[derive(IntoElement)]
pub struct ScrollArea {
    id: ElementId,
    handle: Option<ScrollHandle>,
    thumb: Option<Thumb>,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl ScrollArea {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            handle: None,
            thumb: None,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    /// スクロール位置を共有するハンドル。つまみはここから求める。
    pub fn scroll_handle(mut self, handle: &ScrollHandle) -> Self {
        self.handle = Some(handle.clone());
        self
    }

    /// つまみを直接指定する (静止画用)。
    pub fn thumb(mut self, thumb: Option<Thumb>) -> Self {
        self.thumb = thumb;
        self
    }
}

impl ParentElement for ScrollArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for ScrollArea {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ScrollArea {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let thumb = self.thumb.or_else(|| {
            let h = self.handle.as_ref()?;
            let viewport = f32::from(h.bounds().size.height);
            let max = f32::from(h.max_offset().height);
            let offset = -f32::from(h.offset().y);
            Thumb::from_extent(viewport, viewport + max, offset)
        });
        let mut viewport = div()
            .id(self.id)
            .size_full()
            .overflow_y_scroll()
            .children(self.children);
        if let Some(h) = self.handle.as_ref() {
            viewport = viewport.track_scroll(h);
        }
        // `relative`。つまみは `rounded-full bg-border`。
        let mut root = div().relative().overflow_hidden().child(viewport);
        if let Some(thumb) = thumb {
            root = root.child(
                // `flex p-px border-l border-l-transparent`、`w-2.5 h-full`
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .right_0()
                    .w(px(BAR_WIDTH))
                    .child(
                        div()
                            .absolute()
                            .top(px(1.0))
                            .bottom(px(1.0))
                            .left(px(2.0))
                            .right(px(1.0))
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .right_0()
                                    .top(relative(thumb.top()))
                                    .h(relative(thumb.ratio))
                                    .rounded_full()
                                    .bg(p.t.border),
                            ),
                    ),
            );
        }
        root.style().refine(&self.style);
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_thumb_when_content_fits() {
        assert_eq!(Thumb::from_extent(200.0, 200.0, 0.0), None);
        assert_eq!(Thumb::from_extent(200.0, 100.0, 0.0), None);
    }

    #[test]
    fn thumb_ratio_and_position() {
        let t = Thumb::from_extent(100.0, 400.0, 0.0).unwrap();
        assert_eq!(t.ratio, 0.25);
        assert_eq!(t.top(), 0.0);
        let end = Thumb::from_extent(100.0, 400.0, 300.0).unwrap();
        assert_eq!(end.position, 1.0);
        assert!((end.top() - 0.75).abs() < 1e-6);
        let over = Thumb::from_extent(100.0, 400.0, 999.0).unwrap();
        assert_eq!(over.position, 1.0);
    }
}
