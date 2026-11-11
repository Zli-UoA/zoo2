//! `sonner.tsx`。トースト (sonner 2.0.8) の積み重ね。
//! 色は `--normal-bg: popover`、`--normal-text: popover-foreground`、`--normal-border: border`。
//! 寸法は `node_modules/sonner/dist/styles.css` と `index.mjs` から読んだ値
//! (幅 356px、padding 16px、角丸 8px、文字 13px、影 `0 4px 12px`、gap 6px、積み重ねの gap 14px)。

use gpui::{AnyElement, App, SharedString, Window, div, prelude::*, px};

use super::{Palette, black, icon_colored};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::hex;

pub const WIDTH: f32 = 356.0;
/// 画面の端からの距離 (`--offset`、デスクトップ)。
pub const OFFSET: f32 = 24.0;
/// 積み重ねたときの間隔と、重なったときに見える幅。
pub const GAP: f32 = 14.0;
pub const TEXT_SIZE: f32 = 13.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Normal,
    Success,
    Error,
    Info,
    Warning,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub kind: ToastKind,
    pub title: SharedString,
    pub description: Option<SharedString>,
}

impl Toast {
    pub fn new(kind: ToastKind, title: impl Into<SharedString>) -> Self {
        Self {
            kind,
            title: title.into(),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// 画面に出る最大枚数 (`VISIBLE_TOASTS_AMOUNT`)。
pub const VISIBLE: usize = 3;

/// 手前のトーストの高さの見積もり。
/// 枠 1px×2 + padding 16px×2 + 行 (題 `13*1.5`、説明 `13*1.4` と間 2px)。
pub fn toast_height(toast: &Toast) -> f32 {
    let title = TEXT_SIZE * 1.5;
    let body = match toast.description {
        Some(_) => title + 2.0 + TEXT_SIZE * 1.4,
        None => title,
    };
    // アイコンは 20px で、題だけのときは行より高い。
    34.0 + body.max(20.0)
}

/// 折りたたんだときの、後ろの n 枚目の倍率。CSS は `scale(calc(-1 * var(--toasts-before) * 0.05 + 1))`、つまり `1 - n * 0.05`。
pub fn stack_scale(index: usize) -> f32 {
    1.0 - index as f32 * 0.05
}

/// 折りたたんだときの、後ろの n 枚目を上へ上げる量 (`--lift-amount * n`)。
pub fn stack_peek(index: usize) -> f32 {
    index as f32 * GAP
}

fn icon_view(p: &Palette, kind: ToastKind) -> Option<AnyElement> {
    let t = p.t;
    let (glyph, mark): (Option<Icon>, Option<&'static str>) = match kind {
        ToastKind::Normal => return None,
        ToastKind::Success => (Some(Icon::Check), None),
        ToastKind::Error => (Some(Icon::X), None),
        ToastKind::Info => (None, Some("i")),
        ToastKind::Warning => (None, Some("!")),
    };
    // sonner の絵柄は `currentColor` で塗った 20px の円。中の記号は背景色で抜く。
    Some(
        div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(20.0))
            // 16px の枠からはみ出す分 (`margin-left: -3px; margin-right: 4px`)。
            .ml(px(-3.0))
            .mr(px(4.0))
            .rounded_full()
            .bg(t.popover_foreground)
            .text_color(t.popover)
            .text_size(px(12.0))
            .font(fonts::sans(700.0))
            .children(glyph.map(|g| icon_colored(g, 12.0, t.popover)))
            .children(mark)
            .into_any_element(),
    )
}

fn toast_box(p: &Palette) -> gpui::Div {
    let t = p.t;
    div()
        .w(px(WIDTH))
        .rounded(px(8.0))
        .border_1()
        .border_color(t.border)
        .bg(t.popover)
        .shadow(vec![gpui::BoxShadow {
            color: black(0.1).into(),
            offset: gpui::point(px(0.0), px(4.0)),
            blur_radius: px(12.0),
            spread_radius: px(0.0),
        }])
}

fn toast_view(p: &Palette, toast: &Toast) -> gpui::Div {
    let t = p.t;
    // 説明の文字色は sonner の既定 (`#3f3f3f` / `#e8e8e8`)。
    let description = p.pick(hex(0x3f3f3f), hex(0xe8e8e8));
    toast_box(p)
        .flex()
        .items_center()
        .gap(px(6.0))
        .p(px(16.0))
        .text_color(t.popover_foreground)
        .text_size(px(TEXT_SIZE))
        .font(fonts::sans(400.0))
        .children(icon_view(p, toast.kind))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .line_height(px(TEXT_SIZE * 1.5))
                        .font(fonts::sans(500.0))
                        .child(toast.title.clone()),
                )
                .children(toast.description.clone().map(|d| {
                    div()
                        .line_height(px(TEXT_SIZE * 1.4))
                        .text_color(description)
                        .child(d)
                })),
        )
}

/// 右下に出る積み重ね (`position="bottom-right"`)。`relative` な親の中に置く。
/// `toasts` は新しい順 (先頭が手前)。
#[derive(IntoElement)]
pub struct Toaster {
    p: Palette,
    toasts: Vec<Toast>,
    expanded: bool,
}

impl Toaster {
    pub fn new(p: &Palette) -> Self {
        Self {
            p: *p,
            toasts: Vec::new(),
            expanded: false,
        }
    }

    pub fn toast(mut self, toast: Toast) -> Self {
        self.toasts.push(toast);
        self
    }

    /// ホバーで広げた状態。false は重ねた状態。
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = expanded;
        self
    }
}

impl RenderOnce for Toaster {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap(px(GAP));
        if self.expanded || self.toasts.len() <= 1 {
            // 手前を下に置く。
            for toast in self.toasts.iter().rev() {
                list = list.child(toast_view(&self.p, toast));
            }
        } else {
            // 重ねた状態。後ろの紙は、手前と同じ高さを `scale(1 - n * 0.05)` で中心から縮め、
            // `translateY(-14px * n)` で上へ上げる。
            let front = &self.toasts[0];
            let h = toast_height(front);
            let shown = self.toasts.len().min(VISIBLE);
            let mut extra = 0.0f32;
            let mut backs = Vec::new();
            for i in (1..shown).rev() {
                let sc = stack_scale(i);
                let bottom = stack_peek(i) + h * (1.0 - sc) / 2.0;
                extra = extra.max(bottom + h * sc - h);
                backs.push((i, sc, bottom));
            }
            let mut stack = div().relative().w(px(WIDTH)).h(px(h + extra));
            for (_, sc, bottom) in backs {
                let width = WIDTH * sc;
                stack = stack.child(
                    div()
                        .absolute()
                        .bottom(px(bottom))
                        .left(px((WIDTH - width) / 2.0))
                        .w(px(width))
                        .h(px(h * sc))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(self.p.t.border)
                        .bg(self.p.t.popover),
                );
            }
            stack = stack.child(
                div()
                    .absolute()
                    .bottom(px(0.0))
                    .left_0()
                    .child(toast_view(&self.p, front)),
            );
            list = list.child(stack);
        }
        div()
            .absolute()
            .right(px(OFFSET))
            .bottom(px(OFFSET))
            .child(list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_geometry() {
        assert_eq!(stack_scale(0), 1.0);
        assert!((stack_scale(1) - 0.95).abs() < 1e-6);
        assert!((stack_scale(2) - 0.9).abs() < 1e-6);
        assert_eq!(stack_peek(2), 28.0);
    }
}
