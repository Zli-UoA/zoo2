//! `collapsible-section.tsx`。サイドバーの折りたたみ見出し。

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, SharedString, StyleRefinement, Transformation, Window, div,
    percentage, prelude::*, px,
};

use super::{TextSpec, alpha, icon_colored, palette, ring};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;

type ToggleHandler = Rc<dyn Fn(bool, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct CollapsibleSection {
    id: ElementId,
    title: SharedString,
    icon: Icon,
    count: Option<SharedString>,
    actions: Vec<AnyElement>,
    open: bool,
    hovered: bool,
    on_toggle: Option<ToggleHandler>,
    children: Vec<AnyElement>,
    style: StyleRefinement,
}

impl CollapsibleSection {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, icon: Icon) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon,
            count: None,
            actions: Vec::new(),
            open: true,
            hovered: false,
            on_toggle: None,
            children: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn count(mut self, count: impl Into<SharedString>) -> Self {
        self.count = Some(count.into());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// 見出しの hover を常に出す。
    pub fn hovered(mut self, hovered: bool) -> Self {
        self.hovered = hovered;
        self
    }

    /// 見出しを押したとき。引数は押した後の開閉。
    pub fn on_toggle(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }
}

impl ParentElement for CollapsibleSection {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl Styled for CollapsibleSection {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for CollapsibleSection {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let t = p.t;
        let xs = TextSpec::XS;
        let open = self.open;
        let hover_bg = t.sidebar_accent;
        let hover_fg = t.sidebar_accent_foreground;
        let (bg, fg) = if self.hovered {
            (hover_bg, hover_fg)
        } else {
            (super::TRANSPARENT, t.muted_foreground)
        };
        let chevron = icon_colored(Icon::ChevronRight, 12.0, fg).when(open, |s| {
            s.with_transformation(Transformation::rotate(percentage(0.25)))
        });
        let toggle = self.on_toggle.clone();
        // `flex min-h-9 w-full min-w-0 items-center gap-1.5 rounded-md px-1 text-left text-xs font-semibold`
        let button = div()
            .id(self.id)
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(6.0))
            .min_h(px(36.0))
            .px(px(4.0))
            .rounded(radius::MD)
            .bg(bg)
            .text_color(fg)
            .text_size(xs.size())
            .line_height(xs.line())
            .font(fonts::sans(600.0))
            .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
            .focus(move |s| s.shadow(vec![ring(alpha(t.ring, 0.5), 2.0)]))
            .child(chevron)
            .child(
                // `flex size-4 shrink-0 items-center justify-center`
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(16.0))
                    .child(icon_colored(self.icon, 16.0, fg)),
            )
            .child(div().truncate().child(self.title))
            .when_some(self.count, |d, count| {
                // `ml-auto shrink-0 px-1 font-normal tabular-nums`
                d.child(
                    div()
                        .ml_auto()
                        .flex_none()
                        .px(px(4.0))
                        .font(fonts::sans(400.0))
                        .child(count),
                )
            })
            .when_some(toggle, |d, handler| {
                d.on_click(move |_, w, cx| handler(!open, w, cx))
            });
        let mut section = div()
            .child(
                // `flex min-h-9 items-center gap-1 px-2`
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .min_h(px(36.0))
                    .px(px(8.0))
                    .child(button)
                    .when(!self.actions.is_empty(), |d| {
                        d.child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(2.0))
                                .children(self.actions),
                        )
                    }),
            )
            .when(open, |d| d.children(self.children));
        section.style().refine(&self.style);
        section
    }
}
