//! `confirm-action-dialog.tsx`。取り消しと確定の 2 ボタンの確認。

use std::rc::Rc;

use gpui::{App, FocusHandle, SharedString, Window, div, prelude::*};

use super::alert_dialog::AlertDialog;
use super::button::{Button, ButtonVariant};
use super::{TextSpec, palette};

type Handler = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct ConfirmActionDialog {
    id: SharedString,
    title: SharedString,
    description: SharedString,
    confirm_label: SharedString,
    cancel_label: SharedString,
    executing_label: SharedString,
    pending: bool,
    error: Option<SharedString>,
    on_confirm: Option<Handler>,
    on_cancel: Option<Handler>,
    cancel_focus: Option<FocusHandle>,
    confirm_focus: Option<FocusHandle>,
}

impl ConfirmActionDialog {
    /// `id` はボタンの ID の元になる (`{id}-cancel`、`{id}-ok`)。
    /// 文言は呼び出し側が i18n から引いて渡す (`common.cancel`、`moderation.executing`)。
    pub fn new(
        id: impl Into<SharedString>,
        title: impl Into<SharedString>,
        description: impl Into<SharedString>,
        confirm_label: impl Into<SharedString>,
        cancel_label: impl Into<SharedString>,
        executing_label: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            confirm_label: confirm_label.into(),
            cancel_label: cancel_label.into(),
            executing_label: executing_label.into(),
            pending: false,
            error: None,
            on_confirm: None,
            on_cancel: None,
            cancel_focus: None,
            confirm_focus: None,
        }
    }

    /// 取り消しボタンのフォーカス。Tab と Enter / Space で操作するときに結ぶ。
    pub fn cancel_focus(mut self, handle: &FocusHandle) -> Self {
        self.cancel_focus = Some(handle.clone());
        self
    }

    /// 確定ボタンのフォーカス。
    pub fn confirm_focus(mut self, handle: &FocusHandle) -> Self {
        self.confirm_focus = Some(handle.clone());
        self
    }

    pub fn pending(mut self, pending: bool) -> Self {
        self.pending = pending;
        self
    }

    pub fn error(mut self, error: impl Into<SharedString>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn on_confirm(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_confirm = Some(Rc::new(handler));
        self
    }

    pub fn on_cancel(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(handler));
        self
    }
}

/// 確定ボタンの文言。実行中は「実行中…」になる。
pub fn confirm_text<'a>(
    pending: bool,
    confirm: &'a SharedString,
    executing: &'a SharedString,
) -> &'a SharedString {
    if pending { executing } else { confirm }
}

impl RenderOnce for ConfirmActionDialog {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = palette(cx);
        let label = confirm_text(self.pending, &self.confirm_label, &self.executing_label).clone();
        let mut cancel = Button::new(SharedString::from(format!("{}-cancel", self.id)))
            .variant(ButtonVariant::Outline)
            .label(self.cancel_label)
            .disabled(self.pending);
        if let Some(h) = self.on_cancel {
            cancel = cancel.on_click(move |_, w, cx| h(w, cx));
        }
        if let Some(handle) = self.cancel_focus.as_ref() {
            cancel = cancel.focus_handle(handle);
        }
        let mut confirm = Button::new(SharedString::from(format!("{}-ok", self.id)))
            .variant(ButtonVariant::Destructive)
            .label(label)
            .disabled(self.pending);
        if let Some(h) = self.on_confirm {
            confirm = confirm.on_click(move |_, w, cx| h(w, cx));
        }
        if let Some(handle) = self.confirm_focus.as_ref() {
            confirm = confirm.focus_handle(handle);
        }
        let destructive = p.t.destructive;
        let mut dialog = AlertDialog::new(self.title).description(self.description);
        if let Some(error) = self.error {
            // `<p role="alert" class="text-sm text-destructive">`
            dialog = dialog.child(
                div()
                    .text_size(TextSpec::SM.size())
                    .line_height(TextSpec::SM.line())
                    .text_color(destructive)
                    .child(error),
            );
        }
        dialog.footer(cancel).footer(confirm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_shows_executing_label() {
        let confirm: SharedString = "削除".into();
        let executing: SharedString = "実行中…".into();
        assert_eq!(confirm_text(false, &confirm, &executing), &confirm);
        assert_eq!(confirm_text(true, &confirm, &executing), &executing);
    }
}
