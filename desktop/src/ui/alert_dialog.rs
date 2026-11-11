//! `alert-dialog.tsx`。閉じるボタンは無く、操作はフッターのボタンで行う。

use gpui::{AnyElement, App, SharedString, StyleRefinement, Window, div, prelude::*, px};

use super::dialog::{body, description_text, footer_frame, overlay, panel_frame, title_text};
use super::{Palette, icon_colored};
use crate::icons::Icon;
use crate::tokens::radius;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertDialogSize {
    /// `max-w-xs sm:max-w-sm` (デスクトップでは 384px)。ヘッダーは左寄せ。
    Default,
    /// `max-w-xs` (320px)。ヘッダーは中央寄せで、フッターは 2 列。
    Sm,
}

impl AlertDialogSize {
    pub fn width(self) -> f32 {
        match self {
            Self::Default => 384.0,
            Self::Sm => 320.0,
        }
    }
}

#[derive(IntoElement)]
pub struct AlertDialog {
    p: Palette,
    size: AlertDialogSize,
    media: Option<Icon>,
    title: SharedString,
    description: Option<SharedString>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    style: StyleRefinement,
}

impl AlertDialog {
    pub fn new(p: &Palette, title: impl Into<SharedString>) -> Self {
        Self {
            p: *p,
            size: AlertDialogSize::Default,
            media: None,
            title: title.into(),
            description: None,
            body: Vec::new(),
            footer: Vec::new(),
            style: StyleRefinement::default(),
        }
    }

    pub fn size(mut self, size: AlertDialogSize) -> Self {
        self.size = size;
        self
    }

    /// `AlertDialogMedia`: 40px の角丸の枠に入れたアイコン。
    pub fn media(mut self, icon: Icon) -> Self {
        self.media = Some(icon);
        self
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn footer(mut self, child: impl IntoElement) -> Self {
        self.footer.push(child.into_any_element());
        self
    }

    /// 幕とあわせて、親いっぱいに中央寄せで出す。
    pub fn into_overlay(self) -> gpui::Div {
        overlay(self)
    }
}

impl ParentElement for AlertDialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl Styled for AlertDialog {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for AlertDialog {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.p.t;
        let centered = self.size == AlertDialogSize::Sm;
        // 本文: タイトルは `text-base font-medium` (行の高さ 24px)、説明は `text-sm`。gap は `gap-1.5`。
        let text = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap(px(6.0))
            .when(centered, |d| d.items_center().text_center())
            .child(title_text(self.title, 24.0))
            .children(self.description.map(|d| description_text(&self.p, d)));
        let media = self.media.map(|icon| {
            // `inline-flex size-10 items-center justify-center rounded-md bg-muted`
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(40.0))
                .rounded(radius::MD)
                .bg(t.muted)
                .when(centered, |d| d.mb(px(8.0)))
                .child(icon_colored(icon, 24.0, t.foreground))
        });
        let header = if centered {
            div()
                .flex()
                .flex_col()
                .items_center()
                .children(media)
                .child(text)
        } else {
            // 既定は、メディアを左に置いて 2 行にまたがせる (`gap-x-4`)。
            div().flex().gap(px(16.0)).children(media).child(text)
        };
        let has_footer = !self.footer.is_empty();
        let mut panel = panel_frame(&self.p, self.size.width())
            .when(!has_footer, |d| d.pb(px(16.0)))
            .child(body([header.into_any_element()]))
            .children(self.body.into_iter().map(|c| body([c])))
            .when(has_footer, |d| {
                let footer = footer_frame(&self.p);
                let footer = if centered {
                    // `group-data-[size=sm]:grid grid-cols-2`
                    footer.children(
                        self.footer
                            .into_iter()
                            .map(|c| div().flex_1().flex().child(c).into_any_element()),
                    )
                } else {
                    footer.children(self.footer)
                };
                d.child(footer)
            });
        panel.style().refine(&self.style);
        panel
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(AlertDialogSize::Default.width(), 384.0);
        assert_eq!(AlertDialogSize::Sm.width(), 320.0);
    }
}
