//! コンポーザー (frontend の `chat/messages/Composer.tsx` / `ComposerField.tsx` に対応)。

use gpui::{
    Context, Div, ElementId, Focusable, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    PathPromptOptions, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*,
    px,
};
use zoo2_client_core::electric::ShapeId;

use crate::app::{MentionCandidate, Target};
use crate::fonts;
use crate::icons::Icon;
use crate::tokens::radius;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::spinner::Spinner;
use crate::ui::tooltip::tooltip_view;
use crate::ui::{TextSpec, alpha, h_flex, icon_colored, palette, ring, shadow_sm, v_flex};
use crate::view::format_file_size;
use crate::view::{WorkspaceView, is_activate_key};

/// `ComposerField` の外枠の寸法 (`rounded-xl p-1.5`)。
/// 送信キーのヒント (`composerSendHint`)。送信キーは `input.rs` の `MODIFIER_SUBMIT_KEY`。
#[cfg(target_os = "macos")]
const SEND_HINT_KEY: &str = "composer.mac";
#[cfg(not(target_os = "macos"))]
const SEND_HINT_KEY: &str = "composer.ctrl";

const FIELD_PADDING: f32 = 6.0;
/// 入力欄の左右・上下の余白 (`px-2 py-2.5`) と、1 行の高さ (`text-sm leading-6`)。
const INPUT_PADDING_X: f32 = 8.0;
const INPUT_PADDING_Y: f32 = 10.0;
const INPUT_LINE: f32 = 24.0;
/// 補完の一覧の最大の高さ (`max-h-48`)。
const SUGGESTIONS_MAX_HEIGHT: f32 = 192.0;

/// 補完の 1 行に出す (名前, 説明)。名前は `font-mono`、説明は `text-muted-foreground`。
/// Web 版の `@{name}` と、その横の表示名に当たる。
/// `display_name` は `表示名 (@name)` か `@here (説明)` の形で来る。
pub fn mention_option_parts(candidate: &MentionCandidate) -> (String, String) {
    let name = format!("@{}", candidate.name);
    let label = if candidate.is_special {
        candidate
            .display_name
            .split_once(" (")
            .map(|(_, rest)| rest.trim_end_matches(')').to_string())
            .unwrap_or_default()
    } else {
        candidate
            .display_name
            .rsplit_once(" (@")
            .map(|(display, _)| display.to_string())
            .unwrap_or_else(|| candidate.display_name.clone())
    };
    (name, label)
}

/// `canSubmitComposer`: `value.trim().length > 0` か添付があれば送れる (空白や改行だけでは送れない)。
/// アップロード中は送れない。
pub fn can_submit_composer(draft: &str, attachment_count: usize, uploading: bool) -> bool {
    !uploading && (!draft.trim().is_empty() || attachment_count > 0)
}

/// `ComposerField`: `flex items-end gap-2 rounded-xl border border-input bg-background p-1.5
/// focus-within:border-ring focus-within:ring-3 focus-within:ring-ring/30 dark:bg-input/30`。
/// `focus_within` は、枠の中 (入力欄、添付、絵文字、送信) のどれかにフォーカスがあるか。
/// 呼び出し側が render で調べて渡す。枠は FocusHandle を持たない (入力欄の FocusHandle を
/// 枠でも `track_focus` すると、フォーカスの経路が二重になる)。
pub fn composer_field_frame(p: &crate::ui::Palette, focus_within: bool) -> Div {
    let t = p.t;
    let focus_ring = ring(alpha(t.ring, 0.3), 3.0);
    div()
        .flex()
        .flex_row()
        .items_end()
        .gap(px(8.0))
        .p(px(FIELD_PADDING))
        .rounded(radius::XL)
        .border_1()
        .border_color(t.input)
        .bg(p.pick(t.background, alpha(t.input, 0.3)))
        .when(focus_within, |d| {
            d.border_color(t.ring).shadow(vec![focus_ring])
        })
}

impl WorkspaceView {
    pub(in crate::view) fn submit_composer(&mut self, cx: &mut Context<Self>) {
        self.composer_input
            .update(cx, |input, cx| input.request_submit(cx));
    }

    pub(in crate::view) fn prompt_and_add_attachments(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(channel_id) = state.current_channel.as_deref() else {
            return;
        };
        if !state.can_post_in_channel(channel_id) {
            return;
        }
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = prompt.await {
                let _ = this.update(cx, |this, cx| {
                    this.state.update(cx, |s, cx| {
                        for p in paths {
                            s.add_attachment_from_path(p);
                        }
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    pub(in crate::view) fn render_composer_view(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let xs = TextSpec::XS;
        let sm = TextSpec::SM;
        let composer_focus = self.composer_input.focus_handle(cx);
        // `focus-within`: 枠の中のどれかにフォーカスがあれば、枠の色と輪を出す。
        let focus_within = composer_focus.is_focused(window)
            || self.composer_attach_focus.is_focused(window)
            || self.composer_emoji_focus.is_focused(window)
            || self.composer_send_focus.is_focused(window);
        let state = self.state.read(cx);
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let is_synced = state.store.is_synced(&ShapeId::Channels)
            && state.store.is_synced(&ShapeId::ChannelUsers);
        let has_channel = state.current_channel.is_some();
        let is_archived = state
            .current_channel
            .as_deref()
            .and_then(|id| state.store.channel(id))
            .is_some_and(|c| c.archive_at.is_some());
        let can_post = state
            .current_channel
            .as_deref()
            .is_some_and(|id| state.can_post_in_channel(id));
        let _ = me_id;

        let (target_label, show_cancel, typing_scope) = match &state.target {
            Target::Channel => (None, false, None),
            Target::Reply(root_id) => {
                (Some(i18n.t("threads.compose")), true, Some(root_id.clone()))
            }
            Target::Edit(_) => (Some(i18n.t("messages.editLabel")), true, None),
        };

        let is_editing = matches!(state.target, Target::Edit(_));
        // 暗号化チャンネルはタイムラインに「表示できない」と出すので、入力欄も制限の案内も出さない。
        let is_encrypted = state
            .current_channel
            .as_deref()
            .is_some_and(|id| state.is_encrypted_channel(id));
        let is_posting_restricted =
            is_synced && has_channel && !is_archived && !is_encrypted && !can_post && !is_editing;
        let show_composer = !is_archived && !is_encrypted && (can_post || is_editing);
        // `TypingIndicator`: `messages.typing` に名前を `messages.typingSeparator` でつなぐ。
        let typing_text = state
            .typing
            .get(&typing_scope)
            .filter(|users| !users.is_empty())
            .map(|users| {
                let names: Vec<&str> = users.iter().map(|u| u.display_name.as_str()).collect();
                i18n.t_with(
                    "messages.typing",
                    &[("names", &names.join(&i18n.t("messages.typingSeparator")))],
                )
            });
        let notice = state.notice.as_ref().map(|(msg, _)| msg.clone());
        let invalid_rows = state.store.invalid_rows;
        let update_available = state.update_available.clone();
        let (attachment_count, uploading) = if let Target::Edit(msg_id) = &state.target {
            // 編集では、元の投稿の添付が残るので本文が空でも保存できる (`submit_message` と同じ判定)。
            let existing = state
                .current_channel
                .as_ref()
                .map(|ch| state.store.attachments_for_message(ch, msg_id).len())
                .unwrap_or(0);
            (existing, false)
        } else {
            let atts = state.current_channel_attachments();
            (
                atts.iter()
                    .filter(|a| !a.is_uploading && a.error.is_none())
                    .count(),
                atts.iter().any(|a| a.is_uploading),
            )
        };
        let can_submit = can_submit_composer(
            self.composer_input.read(cx).text(),
            attachment_count,
            uploading,
        );

        let pending_attachments_bar = self.render_pending_attachments_bar(cx);
        let suggestions = self.render_mention_autocomplete(cx);

        // 入力欄の左右・上下の余白は、ボタンの 36px と 1 行 24px の入力が同じ高さに並ぶ値 (`min-h-11`)。
        let input_cell = div()
            .flex_1()
            .min_w_0()
            .px(px(INPUT_PADDING_X))
            .py(px(INPUT_PADDING_Y))
            .text_size(sm.size())
            .line_height(px(INPUT_LINE))
            .font(fonts::sans(400.0))
            .text_color(t.foreground)
            .child(self.composer_input.clone());

        let notice_box = |text: String| {
            // `rounded-lg border bg-muted/50 px-3 py-2 text-sm text-muted-foreground`
            div()
                .w_full()
                .rounded(radius::LG)
                .border_1()
                .border_color(t.border)
                .bg(alpha(t.muted, 0.5))
                .px(px(12.0))
                .py(px(8.0))
                .text_size(sm.size())
                .line_height(sm.line())
                .text_color(t.muted_foreground)
                .child(text)
        };

        let form = v_flex()
            .w_full()
            .flex_none()
            .border_t_1()
            .border_color(t.border)
            .px(px(12.0))
            .py(px(12.0))
            .when_some(pending_attachments_bar, |d, v| d.child(v.mb(px(8.0))))
            .when_some(suggestions, |d, v| d.child(v.mb(px(8.0))))
            .when(is_archived, |d| {
                d.child(notice_box(i18n.t("moderation.archivedReadOnly")))
            })
            .when(is_posting_restricted, |d| {
                d.child(notice_box(
                    "このチャンネルは管理者のみ投稿できます".to_string(),
                ))
            })
            .when(show_composer, |d| {
                d
                    // 返信・編集の対象と取り消し。
                    .when_some(target_label, |d, label| {
                        d.child(
                            h_flex()
                                .gap(px(8.0))
                                .mb(px(8.0))
                                .px(px(4.0))
                                .child(
                                    div()
                                        .text_size(xs.size())
                                        .line_height(xs.line())
                                        .font(fonts::sans(500.0))
                                        .text_color(t.foreground)
                                        .child(label),
                                )
                                .when(show_cancel, |d| {
                                    d.child(
                                        Button::new("btn-cancel-target")
                                            .focus_handle(&self.composer_cancel_focus)
                                            .variant(ButtonVariant::Ghost)
                                            .size(ButtonSize::Xs)
                                            .label(format!("{} (Esc)", i18n.t("common.cancel")))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.cancel_target();
                                                    cx.notify();
                                                });
                                            })),
                                    )
                                }),
                        )
                    })
                    .child(
                        composer_field_frame(&p, focus_within)
                            .when(!is_editing, |d| {
                                d.child(
                                    Button::new("btn-attach-file")
                                        .focus_handle(&self.composer_attach_focus)
                                        .variant(ButtonVariant::Ghost)
                                        .size(ButtonSize::IconLg)
                                        .icon_start(Icon::Paperclip)
                                        .mb(px(4.0))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.prompt_and_add_attachments(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("btn-open-emoji-picker")
                                    .focus_handle(&self.composer_emoji_focus)
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::IconLg)
                                    .icon_start(Icon::SmilePlus)
                                    .mb(px(4.0))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            if s.emoji_picker_open {
                                                s.close_emoji_picker();
                                            } else {
                                                s.open_emoji_picker();
                                            }
                                            cx.notify();
                                        });
                                    })),
                            )
                            .child(input_cell)
                            .child({
                                let send = Button::new("btn-send-message")
                                    .focus_handle(&self.composer_send_focus)
                                    .mb(px(4.0))
                                    .disabled(!can_submit);
                                let send = if is_editing {
                                    send.size(ButtonSize::Lg).label(i18n.t("common.save"))
                                } else {
                                    send.size(ButtonSize::IconLg).icon_start(Icon::Send)
                                };
                                send.on_click(cx.listener(|this, _, _, cx| {
                                    this.submit_composer(cx);
                                }))
                            }),
                    )
                    // `mt-1.5 px-1 text-[11px] text-muted-foreground`
                    .child(
                        div()
                            .mt(px(6.0))
                            .px(px(4.0))
                            .text_size(px(11.0))
                            .line_height(xs.line())
                            .text_color(t.muted_foreground)
                            .child(i18n.t(SEND_HINT_KEY)),
                    )
            })
            // ステータス通知行
            .when(
                notice.is_some() || invalid_rows > 0 || update_available.is_some(),
                |d| {
                    d.child(
                        h_flex()
                            .justify_between()
                            .mt(px(6.0))
                            .px(px(4.0))
                            .text_size(xs.size())
                            .line_height(xs.line())
                            .child(
                                div()
                                    .text_color(t.muted_foreground)
                                    .child(notice.unwrap_or_default()),
                            )
                            .child(
                                h_flex()
                                    .gap(px(12.0))
                                    .when(invalid_rows > 0, |d| {
                                        d.child(
                                            div()
                                                .text_color(t.destructive)
                                                .child(format!("読めない行: {invalid_rows}")),
                                        )
                                    })
                                    .when_some(update_available, |d, ver| {
                                        d.child(
                                            div()
                                                .text_color(t.foreground)
                                                .child(format!("新しい版 v{ver} があります")),
                                        )
                                    }),
                            ),
                    )
                },
            );

        v_flex()
            .w_full()
            .flex_none()
            // `TypingIndicator`: `min-h-6 px-4 text-xs text-muted-foreground`
            .when(show_composer, |d| {
                d.child(
                    div()
                        .min_h(px(24.0))
                        .px(px(16.0))
                        .text_size(xs.size())
                        .line_height(xs.line())
                        .text_color(t.muted_foreground)
                        .children(typing_text),
                )
            })
            .child(form)
    }

    fn render_pending_attachments_bar(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.state.read(cx);
        if matches!(state.target, Target::Edit(_)) {
            return None;
        }
        let attachments = state.current_channel_attachments();
        if attachments.is_empty() {
            return None;
        }
        let p = palette(cx);
        let t = p.t;
        let i18n = self.i18n;
        let pills = attachments.into_iter().map(|att| {
            let local_id = att.local_id.clone();
            let size_str = format_file_size(att.file_size);
            let remove_label = i18n.t_with("attachments.remove", &[("name", &att.file_name)]);

            let del_focus = self
                .attachment_focus_handles
                .borrow_mut()
                .entry(format!("pending_del:{}", att.local_id))
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            let lid_kd = local_id.clone();
            let focus_ring = ring(alpha(t.ring, 0.5), 3.0);

            // 画像以外の下書き: `relative flex h-20 w-56 items-center gap-2 overflow-hidden rounded-lg
            // border bg-muted/30 p-2 pr-9`
            div()
                .relative()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .h(px(80.0))
                .w(px(224.0))
                .overflow_hidden()
                .rounded(radius::LG)
                .border_1()
                .border_color(t.border)
                .bg(alpha(t.muted, 0.3))
                .p(px(8.0))
                .pr(px(36.0))
                .child(icon_colored(Icon::Paperclip, 20.0, t.muted_foreground))
                .child(
                    v_flex()
                        .min_w_0()
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(12.0))
                                .line_height(px(16.0))
                                .font(fonts::sans(500.0))
                                .text_color(t.foreground)
                                .child(att.file_name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .line_height(px(16.0))
                                .text_color(if att.error.is_some() {
                                    t.destructive
                                } else {
                                    t.muted_foreground
                                })
                                .child(if att.error.is_some() {
                                    i18n.t("attachments.uploadError")
                                } else {
                                    size_str
                                }),
                        ),
                )
                .when(att.is_uploading, |d| {
                    // `Spinner className="absolute inset-0 bg-background/60"`
                    d.child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(alpha(t.background, 0.6))
                            .child(Spinner::new(ElementId::Name(
                                format!("pending-spinner-{}", att.local_id).into(),
                            ))),
                    )
                })
                .child(
                    // `absolute right-1 top-1 grid size-6 place-items-center rounded-full
                    // bg-background/90 text-foreground shadow-sm hover:bg-background`
                    div()
                        .id(SharedString::from(format!("pending-del-{}", att.local_id)))
                        .track_focus(&del_focus)
                        .focus(move |s| s.shadow(vec![focus_ring]))
                        .absolute()
                        .top(px(4.0))
                        .right(px(4.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.0))
                        .rounded_full()
                        .bg(alpha(t.background, 0.9))
                        .shadow(shadow_sm())
                        .hover(|s| s.bg(t.background).cursor_pointer())
                        .child(icon_colored(Icon::X, 14.0, t.foreground))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let lid = local_id.clone();
                                this.state.update(cx, |s, cx| {
                                    s.remove_pending_attachment(&lid);
                                    cx.notify();
                                });
                            }),
                        )
                        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                            if is_activate_key(&ev.keystroke.key) {
                                cx.stop_propagation();
                                let lid = lid_kd.clone();
                                this.state.update(cx, |s, cx| {
                                    s.remove_pending_attachment(&lid);
                                    cx.notify();
                                });
                            }
                        }))
                        .tooltip(tooltip_view(remove_label)),
                )
        });

        // `<ul className="mb-2 flex flex-wrap gap-2">` (`mb` は呼び出し側で付ける)
        Some(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .children(pills),
        )
    }

    /// 補完の一覧 (`role="listbox"`): `max-h-48 overflow-y-auto rounded-lg border bg-popover p-1`。
    /// メンション・チャンネル・コマンド・MFM の補完のうち、desktop にあるのはメンションだけ。
    fn render_mention_autocomplete(&self, cx: &mut Context<Self>) -> Option<gpui::Stateful<Div>> {
        let p = palette(cx);
        let t = p.t;
        let sm = TextSpec::SM;
        let state = self.state.read(cx);
        // 補完が閉じたら記録を消し、次に開いたとき選択中の行へスクロールし直す。
        let Some(autocomplete) = state
            .mention_autocomplete
            .as_ref()
            .filter(|a| !a.candidates.is_empty())
        else {
            self.mention_scrolled_to.set(None);
            return None;
        };
        let selected_index = autocomplete
            .selected_index
            .min(autocomplete.candidates.len() - 1);
        // 選択が変わったときだけ、その行を見える位置にする (`scrollIntoView({ block: "nearest" })`)。
        if self.mention_scrolled_to.get() != Some(selected_index) {
            self.mention_scrolled_to.set(Some(selected_index));
            self.mention_scroll.scroll_to_item(selected_index);
        }

        let items: Vec<_> = autocomplete
            .candidates
            .iter()
            .enumerate()
            .map(|(i, candidate)| {
                let is_selected = i == selected_index;
                let (name, label) = mention_option_parts(candidate);
                let idx = i;
                // `flex cursor-pointer gap-2 rounded px-2 py-1 text-left text-sm`、選択中は `bg-accent`
                h_flex()
                    .w_full()
                    .gap(px(8.0))
                    .rounded(px(4.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .text_size(sm.size())
                    .line_height(sm.line())
                    .text_color(t.popover_foreground)
                    .when(is_selected, |d| d.bg(t.accent))
                    .hover(|s| s.cursor_pointer())
                    .child(div().flex_none().child(name))
                    .child(
                        div()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_color(t.muted_foreground)
                            .child(label),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                            this.state.update(cx, |s, cx| {
                                if let Some(ac) = &mut s.mention_autocomplete {
                                    ac.selected_index = idx;
                                }
                                s.mention_confirm();
                                cx.notify();
                            });
                        }),
                    )
            })
            .collect();

        Some(
            div()
                .id("mention-autocomplete-list")
                .track_scroll(&self.mention_scroll)
                .w_full()
                .max_h(px(SUGGESTIONS_MAX_HEIGHT))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .rounded(radius::LG)
                .border_1()
                .border_color(t.border)
                .bg(t.popover)
                .p(px(4.0))
                .children(items),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(name: &str, display: &str, special: bool) -> MentionCandidate {
        MentionCandidate {
            name: name.into(),
            display_name: display.into(),
            is_special: special,
        }
    }

    #[test]
    fn mention_option_splits_name_and_label() {
        assert_eq!(
            mention_option_parts(&candidate("alice", "Alice (@alice)", false)),
            ("@alice".to_string(), "Alice".to_string())
        );
        assert_eq!(
            mention_option_parts(&candidate("here", "@here (全員へ通知)", true)),
            ("@here".to_string(), "全員へ通知".to_string())
        );
        // 表示名に括弧があっても、最後の ` (@name)` だけを外す。
        assert_eq!(
            mention_option_parts(&candidate("bob", "Bob (営業) (@bob)", false)),
            ("@bob".to_string(), "Bob (営業)".to_string())
        );
        assert_eq!(
            mention_option_parts(&candidate("x", "Alice", false)),
            ("@x".to_string(), "Alice".to_string())
        );
    }

    #[test]
    fn submit_needs_text_or_attachment_and_no_upload() {
        assert!(!can_submit_composer("", 0, false));
        assert!(can_submit_composer("a", 0, false));
        assert!(can_submit_composer("", 1, false));
        assert!(!can_submit_composer("a", 1, true));
    }

    #[test]
    fn whitespace_only_draft_cannot_submit() {
        // `value.trim().length > 0`: 空白、改行、全角空白だけでは送れない。
        assert!(!can_submit_composer("   ", 0, false));
        assert!(!can_submit_composer("\n\n", 0, false));
        assert!(!can_submit_composer(" \u{3000}\n\t", 0, false));
        assert!(can_submit_composer(" a \n", 0, false));
        // 添付があれば、本文が空白だけでも送れる。
        assert!(can_submit_composer("  ", 1, false));
    }
}
