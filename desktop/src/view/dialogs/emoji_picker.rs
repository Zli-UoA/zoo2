//! 絵文字ピッカー (frontend の `chat/messages/ReactionPicker.tsx` と `EmojiPicker.tsx` の見た目)。
//!
//! 写していないもの: 検索欄と肌の色 (desktop の絵文字は固定の一覧で、検索の入力がない)、
//! 最近使った絵文字、カテゴリの見出し、Emoji Kitchen とカスタム絵文字のタブ。
//! 選ぶ操作は入力欄の矢印キーと Enter が受ける (`view/mod.rs`)。

use gpui::{
    Context, Div, MouseButton, MouseUpEvent, ParentElement, Styled, Window, div, prelude::*, px,
};

use crate::app::AppState;
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::dialog::{description_text, panel_frame, title_text};
use crate::ui::scroll_area::ScrollArea;
use crate::ui::{TextSpec, h_flex, palette, shadow_sm, v_flex};
use crate::view::WorkspaceView;

/// `sm:max-w-md`
const WIDTH: f32 = 448.0;
/// `h-[min(36rem,calc(100dvh-2rem))]`
const HEIGHT: f32 = 576.0;
/// 余白 (`p-4`)。
const PADDING: f32 = 16.0;
/// 1 行の絵文字の数。矢印キーの移動 (`AppState::emoji_picker_*_row`) と同じ。
const COLUMNS: usize = AppState::EMOJI_PICKER_COLUMNS;

impl WorkspaceView {
    pub(in crate::view) fn render_emoji_picker(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let (selected_idx, should_scroll) = self.state.update(cx, |s, _| {
            (
                s.emoji_picker_selected,
                s.take_pending_emoji_picker_scroll(),
            )
        });
        if should_scroll {
            // 一覧は 1 行ずつを子に持つので、行の番号へ寄せる。
            self.emoji_picker_scroll
                .scroll_to_item(selected_idx / COLUMNS);
        }
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let sm = TextSpec::SM;
        let filtered = self.state.read(cx).filtered_emojis();
        let max_h = (f32::from(window.viewport_size().height) - 32.0).max(0.0);

        let rows: Vec<Div> = filtered
            .chunks(COLUMNS)
            .enumerate()
            .map(|(row_idx, chunk)| {
                // `flex w-full gap-1 px-1`
                h_flex()
                    .w_full()
                    .gap(px(4.0))
                    .px(px(4.0))
                    .children(chunk.iter().enumerate().map(|(col, e)| {
                        let idx = row_idx * COLUMNS + col;
                        let emoji_str = e.to_string();
                        let is_selected = idx == selected_idx;
                        // `flex min-w-0 flex-1 items-center justify-center rounded-md p-1 text-xl
                        // leading-none hover:bg-accent`、選択中は `bg-accent`
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .justify_center()
                            .p(px(4.0))
                            .h(px(36.0))
                            .rounded(radius::MD)
                            .text_size(px(20.0))
                            .line_height(px(20.0))
                            .when(is_selected, |d| d.bg(t.accent))
                            .hover(|s| s.bg(t.accent).cursor_pointer())
                            .child(*e)
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                    let em = emoji_str.clone();
                                    let range = this.composer_input.read(cx).selected_range();
                                    this.state.update(cx, |s, cx| {
                                        s.insert_emoji(&em, range);
                                        cx.notify();
                                    });
                                }),
                            )
                    }))
                    // 最後の行は、足りない分を空けて、ほかの行と同じ幅の升に揃える。
                    .children((chunk.len()..COLUMNS).map(|_| div().flex_1().min_w_0()))
            })
            .collect();

        // `Tabs.List`: `flex shrink-0 gap-1 rounded-lg bg-muted p-1`。desktop にあるのは絵文字のタブだけ。
        let tabs = h_flex()
            .flex_none()
            .gap(px(4.0))
            .rounded(radius::LG)
            .bg(t.muted)
            .p(px(4.0))
            .child(
                // `flex-1 rounded-md px-3 py-1.5 text-sm font-medium`、選択中は `bg-background text-foreground shadow-sm`
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .rounded(radius::MD)
                    .px(px(12.0))
                    .py(px(6.0))
                    .text_size(sm.size())
                    .line_height(sm.line())
                    .font(fonts::sans(500.0))
                    .text_color(t.foreground)
                    .bg(t.background)
                    .shadow(shadow_sm())
                    .child(i18n.t("reactions.tabEmoji")),
            );

        let list = div().flex_1().min_h_0().child(if rows.is_empty() {
            // `Frimousse.Empty`: `p-3 text-center text-xs text-muted-foreground`
            div()
                .p(px(12.0))
                .text_center()
                .text_size(px(12.0))
                .text_color(t.muted_foreground)
                .child(i18n.t("reactions.noSearchResults"))
                .into_any_element()
        } else {
            ScrollArea::new("emoji-picker-list")
                .scroll_handle(&self.emoji_picker_scroll)
                .size_full()
                .child(v_flex().gap(px(4.0)).py(px(4.0)).children(rows))
                .into_any_element()
        });

        panel_frame(&p, WIDTH)
            .gap(px(12.0))
            .h(px(HEIGHT))
            .max_h(px(max_h))
            .px(px(PADDING))
            .pb(px(PADDING))
            .overflow_hidden()
            .child(
                // `DialogHeader className="shrink-0 pr-8"`
                v_flex()
                    .flex_none()
                    .gap(px(8.0))
                    .pr(px(32.0))
                    .child(title_text(i18n.t("reactions.add"), 16.0))
                    .child(description_text(&p, i18n.t("reactions.emojiDescription"))),
            )
            .child(tabs)
            .child(
                // `Frimousse.Viewport`: `min-h-0 flex-1 rounded-lg border border-border`
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded(radius::LG)
                    .border_1()
                    .border_color(t.border)
                    .overflow_hidden()
                    .child(list),
            )
            .child(
                // `absolute top-2 right-2` の閉じるボタン。
                Button::new("emoji-picker-close")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .icon_start(Icon::X)
                    .absolute()
                    .top(px(8.0))
                    .right(px(8.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| {
                            s.close_emoji_picker();
                            cx.notify();
                        });
                    })),
            )
    }
}
