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
    content: ContentLayout,
}

/// 子を並べる viewport の中身の指定。何も指定しなければ、子は縦に積むだけ (block)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ContentLayout {
    gap: Option<f32>,
    pr: Option<f32>,
    py: Option<f32>,
}

impl ContentLayout {
    fn is_default(self) -> bool {
        self == Self::default()
    }
}

impl ScrollArea {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            handle: None,
            thumb: None,
            children: Vec::new(),
            style: StyleRefinement::default(),
            content: ContentLayout::default(),
        }
    }

    /// 子の間隔 (`gap-*`)。指定すると、子は縦に並べる (flex-col)。
    ///
    /// `ScrollHandle::scroll_to_item(ix)` は viewport の直下の子を ix で引くので、行を 1 つの
    /// コンテナにまとめて渡さず、行そのものを `child` / `children` に渡す。間隔や余白は、
    /// コンテナではなく、このメソッドと `content_pr` / `content_py` で指定する。
    pub fn content_gap(mut self, gap: f32) -> Self {
        self.content.gap = Some(gap);
        self
    }

    /// viewport の右の余白 (スクロールバーとの間)。
    pub fn content_pr(mut self, pr: f32) -> Self {
        self.content.pr = Some(pr);
        self
    }

    /// viewport の上下の余白。
    pub fn content_py(mut self, py: f32) -> Self {
        self.content.py = Some(py);
        self
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
        let layout = self.content;
        let mut viewport = div()
            .id(self.id)
            .size_full()
            .overflow_y_scroll()
            .when(!layout.is_default(), |d| d.flex().flex_col())
            .when_some(layout.gap, |d, v| d.gap(px(v)))
            .when_some(layout.pr, |d, v| d.pr(px(v)))
            .when_some(layout.py, |d, v| d.py(px(v)))
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

    /// `scroll_to_item(ix)` が効くのは、viewport の直下に行が並んでいるときだけ。
    mod scroll_to_item {
        use gpui::{Context, TestAppContext, Window};

        use super::super::*;
        use crate::tokens::ThemeMode;
        use crate::ui::Palette;

        struct Root {
            handle: ScrollHandle,
            /// true なら、行を 1 つのコンテナにまとめて渡す (効かない渡し方)。
            wrapped: bool,
        }

        impl Render for Root {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let rows = (0..20).map(|i| {
                    div()
                        .id(("row", i as usize))
                        .flex_none()
                        .w_full()
                        .h(px(50.0))
                        .child(format!("{i}"))
                });
                let area = ScrollArea::new("list")
                    .scroll_handle(&self.handle)
                    .size_full();
                let area = if self.wrapped {
                    area.child(div().flex().flex_col().children(rows))
                } else {
                    area.content_gap(2.0).content_pr(8.0).children(rows)
                };
                div().w(px(200.0)).h(px(200.0)).child(area)
            }
        }

        fn offset_after_scrolling_to_end(wrapped: bool) -> f32 {
            let cx = &mut TestAppContext::single();
            cx.update(|cx| cx.set_global(Palette::new(ThemeMode::Light)));
            let handle = ScrollHandle::new();
            let h = handle.clone();
            let (_root, cx) = cx.add_window_view(move |_, _| Root { handle: h, wrapped });
            cx.run_until_parked();
            assert_eq!(f32::from(handle.offset().y), 0.0);
            handle.scroll_to_item(19);
            cx.update(|window, _| window.refresh());
            cx.run_until_parked();
            f32::from(handle.offset().y)
        }

        #[test]
        fn rows_as_direct_children_follow_scroll_to_item() {
            // 末尾の行 (19 番目) を表示するので、offset は負 (下へスクロール) になる。
            assert!(offset_after_scrolling_to_end(false) < -100.0);
        }

        #[test]
        fn rows_wrapped_in_one_container_do_not() {
            // 直下の子が 1 つだけなので、19 番目の子はなく、動かない。
            assert_eq!(offset_after_scrolling_to_end(true), 0.0);
        }
    }
}
