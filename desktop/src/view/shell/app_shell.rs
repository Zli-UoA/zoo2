//! アプリシェル (frontend の `components/layout/**` に対応)。サイドバー | タイムライン | スレッドの横並びと、全体のショートカット。

use gpui::{
    Context, Div, ExternalPaths, Focusable, KeyDownEvent, ParentElement, Styled, Window, div,
    prelude::*,
};

use crate::app::Target;
use crate::fonts;
use crate::ui::{palette, v_flex};
use crate::view::{WorkspaceView, is_activate_key};

impl WorkspaceView {
    pub(in crate::view) fn render_app_shell(
        &self,
        timeline_div: Div,
        thread_div: Option<Div>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = palette(cx).t;
        // `fixed inset-x-0 top-0 flex h-dvh w-full overflow-hidden bg-background text-foreground`
        div()
            .flex()
            .flex_row()
            .size_full()
            .overflow_hidden()
            .bg(t.background)
            .text_color(t.foreground)
            .font(fonts::sans(400.0))
            .track_focus(&self.focus_handle(cx))
            .on_drop::<ExternalPaths>(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                for path in paths.paths() {
                    this.state.update(cx, |s, cx| {
                        s.add_attachment_from_path(path.clone());
                        cx.notify();
                    });
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_global_key_down(ev, window, cx);
            }))
            .child(self.render_left_rail(cx))
            .child(self.render_sidebar_view(cx))
            // `main`: `min-h-0 min-w-0 flex-1 flex-col`
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .child(timeline_div),
            )
            .when_some(thread_div, |d, v| d.child(v))
    }

    fn handle_global_key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = ev.keystroke.key.as_str();
        let cmd_or_ctrl = ev.keystroke.modifiers.platform || ev.keystroke.modifiers.control;
        let is_modal_open = self.state.read(cx).create_channel_modal.is_some();

        // チャンネル作成モーダル表示中は背面の検索・スイッチャー等のショートカットを無効化
        if is_modal_open {
            if key == "escape" {
                self.create_channel_name_input
                    .update(cx, |i, cx| i.clear(cx));
                self.create_channel_desc_input
                    .update(cx, |i, cx| i.clear(cx));
                self.state.update(cx, |s, cx| {
                    s.close_create_channel_modal();
                    cx.notify();
                });
                window.focus(&self.composer_input.focus_handle(cx));
            }
            return;
        }

        // グローバルショートカット
        if cmd_or_ctrl && key == "k" {
            self.open_quick_switcher(window, cx);
            return;
        }
        if cmd_or_ctrl && key == "n" {
            if self.state.read(cx).is_current_user_guest() {
                return;
            }
            self.open_create_channel_modal(window, cx);
            return;
        }
        if cmd_or_ctrl && key == "f" {
            self.toggle_message_search(window, cx);
            return;
        }
        if cmd_or_ctrl && key == "u" {
            if matches!(self.state.read(cx).target, Target::Edit(_)) {
                return;
            }
            self.prompt_and_add_attachments(cx);
            return;
        }
        if cmd_or_ctrl && key == "e" {
            self.state.update(cx, |s, cx| {
                if s.emoji_picker_open {
                    s.close_emoji_picker();
                } else {
                    s.open_emoji_picker();
                }
                cx.notify();
            });
            return;
        }
        if cmd_or_ctrl && (key == "t" || key == "T") {
            self.state.update(cx, |s, cx| {
                s.toggle_theme();
                cx.notify();
            });
            return;
        }
        if self.state.read(cx).emoji_picker_open {
            if is_activate_key(key) {
                let range = self.composer_input.read(cx).selected_range();
                self.state.update(cx, |s, cx| {
                    s.insert_selected_emoji(range);
                    cx.notify();
                });
                window.focus(&self.composer_input.focus_handle(cx));
                return;
            }
            if key == "left" || key == "arrow_left" {
                self.state.update(cx, |s, cx| {
                    s.emoji_picker_prev();
                    cx.notify();
                });
                return;
            }
            if key == "right" || key == "arrow_right" || key == "tab" {
                self.state.update(cx, |s, cx| {
                    s.emoji_picker_next();
                    cx.notify();
                });
                return;
            }
            if key == "up" || key == "arrow_up" {
                self.state.update(cx, |s, cx| {
                    s.emoji_picker_prev_row();
                    cx.notify();
                });
                return;
            }
            if key == "down" || key == "arrow_down" {
                self.state.update(cx, |s, cx| {
                    s.emoji_picker_next_row();
                    cx.notify();
                });
                return;
            }
        }
        if key == "tab" {
            if ev.keystroke.modifiers.shift {
                window.focus_prev();
            } else {
                window.focus_next();
            }
            cx.notify();
            return;
        }
        if key == "escape" {
            self.create_channel_name_input
                .update(cx, |i, cx| i.clear(cx));
            self.create_channel_desc_input
                .update(cx, |i, cx| i.clear(cx));
            self.quick_switcher_input.update(cx, |i, cx| i.clear(cx));
            self.message_search_input.update(cx, |i, cx| i.clear(cx));
            self.state.update(cx, |s, cx| {
                if s.create_channel_modal.is_some() {
                    s.close_create_channel_modal();
                } else if s.quick_switcher.is_some() {
                    s.close_quick_switcher();
                } else if s.emoji_picker_open {
                    s.close_emoji_picker();
                } else if s.message_search.is_some() {
                    s.close_message_search();
                } else if s.thread.is_some() {
                    s.close_thread();
                }
                cx.notify();
            });
            window.focus(&self.composer_input.focus_handle(cx));
        }
    }
}
