//! メッセージ削除の確認 (frontend の `ui/confirm-action-dialog.tsx`)。
//! 文言は `ChannelScreen` の削除確認と同じ (`messages.deleteTitle` など)。

use gpui::{Context, Div, FocusHandle, KeyDownEvent, Window, prelude::*};

use crate::ui::confirm_action_dialog::ConfirmActionDialog;
use crate::ui::dialog::overlay;
use crate::view::WorkspaceView;

impl WorkspaceView {
    pub(in crate::view) fn render_delete_confirm_dialog(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let i18n = self.i18n;
        // 開いた直後は、取り消しにフォーカスを置く (Base UI の AlertDialog は最初のボタンへ)。
        if !self.delete_dialog_was_open.replace(true) {
            window.focus(&self.delete_cancel_focus);
        }
        let state_ok = self.state.clone();
        let state_cancel = self.state.clone();
        let handles: Vec<FocusHandle> = vec![
            self.delete_cancel_focus.clone(),
            self.delete_confirm_focus.clone(),
        ];
        let dialog = ConfirmActionDialog::new(
            "delete-message",
            i18n.t("messages.deleteTitle"),
            i18n.t("messages.deleteDescription"),
            i18n.t("messages.delete"),
            i18n.t("common.cancel"),
            i18n.t("moderation.executing"),
        )
        .cancel_focus(&self.delete_cancel_focus)
        .confirm_focus(&self.delete_confirm_focus)
        .on_confirm(move |_, cx| {
            state_ok.update(cx, |s, cx| {
                s.confirm_delete();
                s.pending_focus_composer = true;
                cx.notify();
            });
        })
        .on_cancel(move |_, cx| {
            state_cancel.update(cx, |s, cx| {
                s.cancel_delete();
                s.pending_focus_composer = true;
                cx.notify();
            });
        });
        overlay(dialog).on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
            match ev.keystroke.key.as_str() {
                "escape" => {
                    this.state.update(cx, |s, cx| {
                        s.cancel_delete();
                        s.pending_focus_composer = true;
                        cx.notify();
                    });
                }
                "tab" => {
                    super::cycle_focus(window, &handles, ev.keystroke.modifiers.shift);
                    cx.stop_propagation();
                }
                _ => {}
            }
        }))
    }
}
