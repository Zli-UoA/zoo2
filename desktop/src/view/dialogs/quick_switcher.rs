//! クイックスイッチャー。

use gpui::{
    Context, Div, Focusable, MouseButton, MouseUpEvent, ParentElement, Styled, Window, div,
    prelude::*, px,
};

use crate::icons::Icon;
use crate::tokens::radius;
use crate::ui::dialog::Dialog;
use crate::ui::input::Input;
use crate::ui::scroll_area::ScrollArea;
use crate::ui::{TextSpec, h_flex, icon_colored, palette, v_flex};
use crate::view::WorkspaceView;
use crate::view::chat::channel_header::header_icon;

/// `sm:max-w-md`
const WIDTH: f32 = 448.0;
/// 一覧の高さ (`h-80`) と、1 行の最小の高さ (`min-h-14`)。
const LIST_HEIGHT: f32 = 320.0;
const ROW_HEIGHT: f32 = 56.0;

impl WorkspaceView {
    pub(in crate::view) fn open_quick_switcher(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.state.read(cx).create_channel_modal.is_some() {
            return;
        }
        if self.state.read(cx).quick_switcher.is_some() {
            window.focus(&self.quick_switcher_input.focus_handle(cx));
            return;
        }
        self.quick_switcher_input
            .update(cx, |input, cx| input.clear(cx));
        self.state.update(cx, |s, cx| {
            if s.create_channel_modal.is_some() {
                s.close_create_channel_modal();
            }
            if s.emoji_picker_open {
                s.close_emoji_picker();
            }
            s.open_quick_switcher();
            cx.notify();
        });
        window.focus(&self.quick_switcher_input.focus_handle(cx));
    }

    /// ⌘K のスイッチャー。Web 版にはなく、見た目は `ChannelBrowserDialog.tsx` に合わせる。
    pub(in crate::view) fn render_quick_switcher_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let should_scroll = self
            .state
            .update(cx, |s, _| s.take_pending_quick_switcher_scroll());
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let sm = TextSpec::SM;
        let xs = TextSpec::XS;
        let state = self.state.read(cx);
        let qs = state.quick_switcher.as_ref().unwrap();
        let candidates = state.quick_switcher_candidates();
        let selected_index = qs.selected_index;
        let has_query = !qs.query.trim().is_empty();
        if should_scroll {
            self.quick_switcher_scroll.scroll_to_item(selected_index);
        }

        let items: Vec<_> = candidates
            .iter()
            .enumerate()
            .map(|(i, (ch, label))| {
                let is_selected = i == selected_index;
                let has_unread = state.unread.contains(&ch.id);
                let icon = header_icon(
                    ch.is_dm,
                    ch.is_dm && ch.channel_name.starts_with("dm:group:"),
                    ch.is_private,
                );
                let description = if ch.is_dm {
                    None
                } else {
                    ch.description
                        .as_deref()
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                        .map(str::to_string)
                };
                let ch_id = ch.id.clone();
                // `flex min-h-14 items-center gap-3 rounded-lg py-2 pl-3 pr-3 hover:bg-muted`
                h_flex()
                    .w_full()
                    .flex_none()
                    .min_h(px(ROW_HEIGHT))
                    .gap(px(12.0))
                    .rounded(radius::LG)
                    .py(px(8.0))
                    .pl(px(12.0))
                    .pr(px(12.0))
                    .when(is_selected, |d| d.bg(t.muted))
                    .hover(|s| s.bg(t.muted).cursor_pointer())
                    .child(
                        // `size-4 shrink-0 opacity-60`
                        div().flex_none().opacity(0.6).child(icon_colored(
                            icon,
                            16.0,
                            t.foreground,
                        )),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .text_size(sm.size())
                                    .line_height(sm.line())
                                    .text_color(t.foreground)
                                    .child(label.clone()),
                            )
                            .when_some(description, |d, desc| {
                                d.child(
                                    div()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .text_size(xs.size())
                                        .line_height(xs.line())
                                        .text_color(t.muted_foreground)
                                        .child(desc),
                                )
                            }),
                    )
                    .when(has_unread, |d| {
                        // サイドバーと同じ未読の点 (`size-2 rounded-full bg-brand`)
                        d.child(div().flex_none().size(px(8.0)).rounded_full().bg(t.brand))
                    })
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                            let target = ch_id.clone();
                            this.state.update(cx, |s, cx| {
                                s.close_quick_switcher();
                                s.open_channel(target);
                                cx.notify();
                            });
                        }),
                    )
            })
            .collect();

        let search = div()
            .relative()
            .child(
                Input::new()
                    .focused(
                        self.quick_switcher_input
                            .focus_handle(cx)
                            .is_focused(window),
                    )
                    .pl(px(32.0))
                    .child(self.quick_switcher_input.clone()),
            )
            .child(
                // `pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground`
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(10.0))
                    .flex()
                    .items_center()
                    .child(icon_colored(Icon::Search, 16.0, t.muted_foreground)),
            );

        let list = div()
            .h(px(LIST_HEIGHT))
            .min_h_0()
            .min_w_0()
            .child(if items.is_empty() {
                // `flex h-full items-center justify-center px-2 text-center text-sm text-muted-foreground`
                div()
                    .flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .px(px(8.0))
                    .text_center()
                    .text_size(sm.size())
                    .line_height(sm.line())
                    .text_color(t.muted_foreground)
                    .child(i18n.t(if has_query {
                        "channels.searchEmpty"
                    } else {
                        "channels.browseEmpty"
                    }))
                    .into_any_element()
            } else {
                ScrollArea::new("quick-switcher-list")
                    .scroll_handle(&self.quick_switcher_scroll)
                    .size_full()
                    // 行を直接の子にする (`scroll_to_item` は直下の子を番号で引く)。
                    .content_gap(2.0)
                    .content_pr(8.0)
                    .children(items)
                    .into_any_element()
            });

        div().child(
            Dialog::new("quick-switcher", i18n.t("channels.browseTitle"))
                .w(px(WIDTH))
                .on_close(cx.listener(|this, _, _, cx| {
                    this.state.update(cx, |s, cx| {
                        s.close_quick_switcher();
                        cx.notify();
                    });
                }))
                .child(search)
                .child(list)
                .child(
                    div()
                        .text_size(xs.size())
                        .line_height(xs.line())
                        .text_color(t.muted_foreground)
                        .child("↑↓ で移動、Enter で決定、Esc で閉じる"),
                ),
        )
    }
}
