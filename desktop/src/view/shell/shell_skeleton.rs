//! 読み込み中の枠 (frontend の `layout/ShellSkeleton.tsx` に対応)。3 ペインの形だけをなぞる。

use gpui::{App, Div, ElementId, SharedString, Styled, div, prelude::*, px};

use crate::tokens::radius;
use crate::ui::pane_header::PaneHeader;
use crate::ui::skeleton::Skeleton;
use crate::ui::{palette, v_flex};
use crate::view::shell::left_rail::RAIL_WIDTH;
use crate::view::shell::sidebar::SIDEBAR_WIDTH;

/// サイドバーの行の数 (`SIDEBAR_ROWS`)。
pub const SIDEBAR_ROWS: usize = 6;

fn bar(id: &'static str) -> Skeleton {
    Skeleton::new(ElementId::Name(SharedString::from(id)))
}

/// `ShellSkeleton`。`animate` を false にすると静止して描く (撮影用)。
pub fn render_shell_skeleton(animate: bool, cx: &App) -> Div {
    let t = palette(cx).t;
    let sidebar_rows = (0..SIDEBAR_ROWS).map(|i| {
        Skeleton::new(ElementId::Name(SharedString::from(format!(
            "shell-skel-row-{i}"
        ))))
        .animate(animate)
        .h(px(36.0))
        .w_full()
        .rounded(radius::LG)
    });
    // `flex h-dvh w-full overflow-hidden bg-background`
    div()
        .flex()
        .size_full()
        .overflow_hidden()
        .bg(t.background)
        .text_color(t.foreground)
        // `w-14 shrink-0 border-r bg-sidebar md:w-16`
        .child(
            div()
                .flex_none()
                .w(px(RAIL_WIDTH))
                .h_full()
                .border_r_1()
                .border_color(t.border)
                .bg(t.sidebar),
        )
        // `w-72 shrink-0 flex-col gap-2 border-r p-3`
        .child(
            v_flex()
                .flex_none()
                .w(px(SIDEBAR_WIDTH))
                .h_full()
                .gap(px(8.0))
                .p(px(12.0))
                .border_r_1()
                .border_color(t.border)
                .child(
                    bar("shell-skel-title")
                        .animate(animate)
                        .h(px(24.0))
                        .w(px(96.0)),
                )
                .children(sidebar_rows),
        )
        // `flex min-w-0 flex-1 flex-col`
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    PaneHeader::new()
                        .child(
                            bar("shell-skel-icon")
                                .animate(animate)
                                .size(px(32.0))
                                .rounded(radius::LG),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .gap(px(6.0))
                                .child(
                                    bar("shell-skel-name")
                                        .animate(animate)
                                        .h(px(16.0))
                                        .w(px(160.0)),
                                )
                                .child(
                                    bar("shell-skel-desc")
                                        .animate(animate)
                                        .h(px(12.0))
                                        .w(px(96.0)),
                                ),
                        ),
                )
                // `flex flex-1 flex-col gap-3 p-4`
                .child(
                    v_flex()
                        .flex_1()
                        .gap(px(12.0))
                        .p(px(16.0))
                        .child(
                            bar("shell-skel-m1")
                                .animate(animate)
                                .h(px(64.0))
                                .w_full()
                                .rounded(radius::LG),
                        )
                        .child(
                            bar("shell-skel-m2")
                                .animate(animate)
                                .h(px(64.0))
                                .w_3_4()
                                .rounded(radius::LG),
                        )
                        .child(
                            bar("shell-skel-m3")
                                .animate(animate)
                                .h(px(64.0))
                                .w_1_2()
                                .rounded(radius::LG),
                        ),
                ),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_sidebar_rows_like_the_web_skeleton() {
        assert_eq!(SIDEBAR_ROWS, 6);
    }
}
