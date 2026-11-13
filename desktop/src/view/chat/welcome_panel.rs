//! チャンネル未選択のときのメイン面 (frontend の `chat/WelcomePanel.tsx` に対応)。

use gpui::{Context, Div, ParentElement, Styled, div, px};

use crate::assets::logo;
use crate::fonts;
use crate::tokens::radius;
use crate::ui::{TextSpec, palette, v_flex};
use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn render_welcome_panel(&self, cx: &mut Context<Self>) -> Div {
        let t = palette(cx).t;
        let i18n = self.i18n;
        let sm = TextSpec::SM;
        // `flex h-full flex-1 items-center justify-center p-6`
        div()
            .flex()
            .flex_1()
            .h_full()
            .items_center()
            .justify_center()
            .p(px(24.0))
            .child(
                // `flex max-w-md flex-col items-center gap-3 text-center`
                v_flex()
                    .max_w(px(448.0))
                    .items_center()
                    .gap(px(12.0))
                    .text_center()
                    .child(logo(px(48.0)).rounded(radius::XXL))
                    .child(
                        // `font-heading text-xl font-semibold tracking-tight`
                        div()
                            .text_size(px(20.0))
                            .line_height(px(28.0))
                            .font(fonts::sans(600.0))
                            .child(i18n.t("welcome.title")),
                    )
                    .child(
                        div()
                            .text_size(sm.size())
                            .line_height(sm.line())
                            .text_color(t.muted_foreground)
                            .child(i18n.t("welcome.description")),
                    ),
            )
    }
}
