//! ダイアログ・ポップアップ類 (frontend の `components/chat/*Dialog.tsx` などに対応)。
//!
//! 幕と枠は `ui/dialog.rs` の部品 (`overlay`、`panel_frame` など) で描く。
//! 開いているダイアログは 1 つだけで、優先順位は `render_open_dialog` が決める。

mod create_channel;
mod delete_confirm;
mod emoji_picker;
mod message_search;
mod quick_switcher;

use gpui::{Context, Div, FocusHandle, Focusable, Window, div, prelude::*, px};

use crate::icons::Icon;
use crate::ui::dialog::overlay;
use crate::ui::{Palette, alpha, icon_colored, ring};
use crate::view::WorkspaceView;

/// 並んだフォーカス先を 1 つ進める (`reverse` なら戻る)。ダイアログの中だけで Tab を回すのに使う。
pub(super) fn cycle_focus(window: &mut Window, handles: &[FocusHandle], reverse: bool) {
    if handles.is_empty() {
        return;
    }
    let current = handles.iter().position(|h| h.is_focused(window));
    let next = match (current, reverse) {
        (Some(i), false) => (i + 1) % handles.len(),
        (Some(i), true) => (i + handles.len() - 1) % handles.len(),
        (None, false) => 0,
        (None, true) => handles.len() - 1,
    };
    window.focus(&handles[next]);
}

/// チェックボックスの箱 (`<input type="checkbox">`)。16px の角丸の枠で、オンなら `primary` で塗って印を出す。
pub(super) fn checkbox_box(p: &Palette, checked: bool, focused: bool, disabled: bool) -> Div {
    let t = p.t;
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(16.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(if checked { t.primary } else { t.input })
        .bg(if checked {
            t.primary
        } else {
            p.pick(t.background, alpha(t.input, 0.3))
        })
        .opacity(if disabled { 0.5 } else { 1.0 })
        .when(focused, |d| d.shadow(vec![ring(alpha(t.ring, 0.5), 3.0)]))
        .when(checked, |d| {
            d.child(icon_colored(Icon::Check, 12.0, t.primary_foreground))
        })
}

impl WorkspaceView {
    /// 開いているダイアログを、幕に載せて返す。無ければ `None`。
    pub(in crate::view) fn render_open_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let s = self.state.read(cx);
        let (create, switcher, emoji, delete) = (
            s.create_channel_modal.is_some(),
            s.quick_switcher.is_some(),
            s.emoji_picker_open,
            s.confirm_delete_message.is_some(),
        );
        // 削除確認が閉じたとき (取り消し・削除のほか、別端末での削除やアーカイブで対象が消えたときも)、
        // フォーカスが描かれなくなったボタンに残らないよう Composer へ戻す。
        // ほかのダイアログが開いたときは、そのダイアログがフォーカスを決める。
        if !delete
            && self.delete_dialog_was_open.replace(false)
            && !(create || switcher || emoji)
            && (self.delete_cancel_focus.is_focused(window)
                || self.delete_confirm_focus.is_focused(window))
        {
            window.focus(&self.composer_input.focus_handle(cx));
        }
        if create {
            Some(overlay(self.render_create_channel_modal(window, cx)))
        } else if switcher {
            Some(overlay(self.render_quick_switcher_modal(window, cx)))
        } else if emoji {
            Some(overlay(self.render_emoji_picker(window, cx)))
        } else if delete {
            Some(self.render_delete_confirm_dialog(window, cx))
        } else {
            None
        }
    }
}
