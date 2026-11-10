//! GPUI による画面描画 (`WorkspaceView`、`LoginView`、`SidebarView`、`TimelineView`、`ComposerView`、`ThreadView`)。

use gpui::{
    ClipboardItem, Context, Div, ElementId, Entity, ExternalPaths, FocusHandle, Focusable,
    FontWeight, IntoElement, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    PathPromptOptions, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, px, rgba,
};
use jiff::{Zoned, civil::DateTime, tz::TimeZone};
use zoo2_client_core::{
    config::AuthConfig,
    electric::{
        ShapeId,
        rows::{CardData, Message, parse_message_cards},
    },
    store::ReactionGroup,
};

use crate::{
    app::{AppState, QUICK_EMOJIS, Target, is_image_content_type, reaction_label},
    input::{InputEvent, TextInput},
    login::{self, LoginPhase},
    mfm_view, theme,
};

pub struct WorkspaceView {
    pub state: Entity<AppState>,
    pub search_input: Entity<TextInput>,
    pub composer_input: Entity<TextInput>,
    pub reaction_input: Entity<TextInput>,
    pub create_channel_name_input: Entity<TextInput>,
    pub create_channel_desc_input: Entity<TextInput>,
    pub quick_switcher_input: Entity<TextInput>,
    pub message_search_input: Entity<TextInput>,
    pub timeline_scroll: ScrollHandle,
    pub thread_scroll: ScrollHandle,
    pub quick_switcher_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    login_primary_focus: FocusHandle,
    login_secondary_focus: FocusHandle,
    login_cancel_focus: FocusHandle,
}

impl Focusable for WorkspaceView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl WorkspaceView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(|cx| TextInput::new_single_line(cx, "チャンネル・DM を検索…"));
        let composer_input = cx.new(|cx| {
            TextInput::new_multiline(cx, "メッセージを入力 (Enter で送信、Shift+Enter で改行)…")
        });
        let reaction_input = cx.new(|cx| {
            TextInput::new_single_line(cx, "絵文字またはショートコード (例: thumbsup, 🎉)")
        });
        let create_channel_name_input =
            cx.new(|cx| TextInput::new_single_line(cx, "例: general, random"));
        let create_channel_desc_input =
            cx.new(|cx| TextInput::new_single_line(cx, "チャンネルのトピックや説明…"));
        let quick_switcher_input = cx.new(|cx| {
            let mut input = TextInput::new_single_line(cx, "チャンネル名またはメンバー名…");
            input.intercept_vertical_arrows = true;
            input
        });
        let message_search_input = cx.new(|cx| {
            let mut input = TextInput::new_single_line(cx, "チャンネル内メッセージを検索…");
            input.intercept_vertical_arrows = true;
            input
        });

        cx.subscribe(
            &search_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        state.search_query = text.clone();
                        cx.notify();
                    });
                }
                InputEvent::Submit(_) => {
                    this.state.update(cx, |state, cx| {
                        if let Some(first) = state.filtered_channels().first().map(|c| c.id.clone())
                        {
                            state.open_channel(first);
                            cx.notify();
                        }
                    });
                }
                InputEvent::Cancel => {
                    this.search_input.update(cx, |input, cx| input.clear(cx));
                    this.state.update(cx, |state, cx| {
                        state.search_query.clear();
                        cx.notify();
                    });
                }
                InputEvent::Up
                | InputEvent::Down
                | InputEvent::Tab
                | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.subscribe(
            &composer_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    let caret = this.composer_input.read(cx).cursor_offset();
                    this.state.update(cx, |state, cx| {
                        state.on_composer_changed(text.clone());
                        state.update_mention_autocomplete(text, caret);
                        cx.notify();
                    });
                }
                InputEvent::Up => {
                    let handled = this.state.update(cx, |state, cx| {
                        if state.mention_autocomplete.is_some() {
                            state.mention_select_prev();
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    });
                    if handled {
                        cx.notify();
                    }
                }
                InputEvent::Down => {
                    let handled = this.state.update(cx, |state, cx| {
                        if state.mention_autocomplete.is_some() {
                            state.mention_select_next();
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    });
                    if handled {
                        cx.notify();
                    }
                }
                InputEvent::Tab => {
                    let sel = this.composer_input.read(cx).selected_range();
                    let mention_handled = this.state.update(cx, |state, cx| {
                        if state.mention_autocomplete.is_some() {
                            if !sel.is_empty() {
                                state.close_mention_autocomplete();
                                cx.notify();
                                return false;
                            }
                            state.mention_confirm();
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    });
                    if mention_handled {
                        cx.notify();
                    }
                }
                InputEvent::Submit(text) => {
                    let sel = this.composer_input.read(cx).selected_range();
                    let mention_handled = this.state.update(cx, |state, cx| {
                        if state.mention_autocomplete.is_some() {
                            if !sel.is_empty() {
                                state.close_mention_autocomplete();
                                cx.notify();
                                return false;
                            }
                            state.mention_confirm();
                            cx.notify();
                            true
                        } else {
                            false
                        }
                    });
                    if mention_handled {
                        return;
                    }
                    let submitted = this.state.update(cx, |state, cx| {
                        let ok = state.submit_message(text.clone());
                        cx.notify();
                        ok
                    });
                    if submitted {
                        this.composer_input.update(cx, |input, cx| input.clear(cx));
                    }
                }
                InputEvent::Cancel => {
                    this.state.update(cx, |state, cx| {
                        if state.mention_autocomplete.is_some() {
                            state.close_mention_autocomplete();
                        } else {
                            state.cancel_target();
                        }
                        cx.notify();
                    });
                }
                InputEvent::CursorMoved(caret) => {
                    let sel = this.composer_input.read(cx).selected_range();
                    this.state.update(cx, |state, cx| {
                        if let Some(ac) = &state.mention_autocomplete
                            && (!sel.is_empty() || *caret != ac.end_byte)
                        {
                            state.close_mention_autocomplete();
                            cx.notify();
                        }
                    });
                }
            },
        )
        .detach();

        cx.subscribe(
            &reaction_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(_) => {}
                InputEvent::Submit(text) => {
                    let ok = this.state.update(cx, |state, cx| {
                        let Some((_, msg_id)) = state.active_reaction_message.clone() else {
                            return false;
                        };
                        let res = state.submit_reaction_input(&msg_id, text);
                        cx.notify();
                        res
                    });
                    if ok {
                        this.reaction_input.update(cx, |input, cx| input.clear(cx));
                    }
                }
                InputEvent::Cancel => {
                    this.reaction_input.update(cx, |input, cx| input.clear(cx));
                    this.state.update(cx, |state, cx| {
                        state.active_reaction_message = None;
                        cx.notify();
                    });
                }
                InputEvent::Up
                | InputEvent::Down
                | InputEvent::Tab
                | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.subscribe(
            &create_channel_name_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        if let Some(m) = &mut state.create_channel_modal {
                            m.channel_name = text.clone();
                            m.error = None;
                        }
                        cx.notify();
                    });
                }
                InputEvent::Submit(_) => {
                    this.state.update(cx, |state, cx| {
                        state.submit_create_channel();
                        cx.notify();
                    });
                }
                InputEvent::Cancel => {
                    this.state.update(cx, |state, cx| {
                        state.close_create_channel_modal();
                        cx.notify();
                    });
                }
                InputEvent::Up
                | InputEvent::Down
                | InputEvent::Tab
                | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.subscribe(
            &create_channel_desc_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        if let Some(m) = &mut state.create_channel_modal {
                            m.description = text.clone();
                        }
                        cx.notify();
                    });
                }
                InputEvent::Submit(_) => {
                    this.state.update(cx, |state, cx| {
                        state.submit_create_channel();
                        cx.notify();
                    });
                }
                InputEvent::Cancel => {
                    this.state.update(cx, |state, cx| {
                        state.close_create_channel_modal();
                        cx.notify();
                    });
                }
                InputEvent::Up
                | InputEvent::Down
                | InputEvent::Tab
                | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.subscribe(
            &quick_switcher_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        if let Some(qs) = &mut state.quick_switcher {
                            qs.query = text.clone();
                            qs.selected_index = 0;
                        }
                        cx.notify();
                    });
                }
                InputEvent::Up => {
                    this.state.update(cx, |state, cx| {
                        state.quick_switcher_select_prev();
                        cx.notify();
                    });
                }
                InputEvent::Down => {
                    this.state.update(cx, |state, cx| {
                        state.quick_switcher_select_next();
                        cx.notify();
                    });
                }
                InputEvent::Submit(_) => {
                    this.state.update(cx, |state, cx| {
                        state.quick_switcher_confirm();
                        cx.notify();
                    });
                }
                InputEvent::Cancel => {
                    this.state.update(cx, |state, cx| {
                        state.close_quick_switcher();
                        cx.notify();
                    });
                }
                InputEvent::Tab | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.subscribe(
            &message_search_input,
            |this: &mut Self, _, event: &InputEvent, cx| match event {
                InputEvent::Changed(text) => {
                    this.state.update(cx, |state, cx| {
                        state.update_message_search_query(text.clone());
                        cx.notify();
                    });
                }
                InputEvent::Up => {
                    this.state.update(cx, |state, cx| {
                        state.search_select_prev();
                        cx.notify();
                    });
                }
                InputEvent::Down | InputEvent::Submit(_) => {
                    this.state.update(cx, |state, cx| {
                        state.search_select_next();
                        cx.notify();
                    });
                }
                InputEvent::Cancel => {
                    this.state.update(cx, |state, cx| {
                        state.close_message_search();
                        cx.notify();
                    });
                }
                InputEvent::Tab | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.observe(&state, |this, state, cx| {
            let (pending_composer, pending_search, pending_open_url, is_mention_active, theme) =
                state.update(cx, |s, _| {
                    (
                        s.pending_composer_text.take(),
                        s.pending_search_text.take(),
                        s.pending_open_url.take(),
                        s.mention_autocomplete.is_some(),
                        s.theme(),
                    )
                });
            if let Some((text, cursor)) = pending_composer {
                this.composer_input.update(cx, |input, cx| {
                    if let Some(pos) = cursor {
                        input.set_text_and_cursor(text, pos, cx);
                    } else {
                        input.set_text(text, cx);
                    }
                });
            }
            if let Some(text) = pending_search {
                this.search_input
                    .update(cx, |input, cx| input.set_text(text, cx));
            }
            if let Some(url) = pending_open_url {
                cx.open_url(&url);
            }
            this.composer_input.update(cx, |input, _| {
                input.intercept_vertical_arrows = is_mention_active;
                input.intercept_tab = is_mention_active;
            });
            this.search_input.update(cx, |i, _| i.theme = theme);
            this.composer_input.update(cx, |i, _| i.theme = theme);
            this.reaction_input.update(cx, |i, _| i.theme = theme);
            this.create_channel_name_input
                .update(cx, |i, _| i.theme = theme);
            this.create_channel_desc_input
                .update(cx, |i, _| i.theme = theme);
            this.quick_switcher_input.update(cx, |i, _| i.theme = theme);
            this.message_search_input.update(cx, |i, _| i.theme = theme);
            cx.notify();
        })
        .detach();

        Self {
            state,
            search_input,
            composer_input,
            reaction_input,
            create_channel_name_input,
            create_channel_desc_input,
            quick_switcher_input,
            message_search_input,
            timeline_scroll: ScrollHandle::new(),
            thread_scroll: ScrollHandle::new(),
            quick_switcher_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            login_primary_focus: cx.focus_handle(),
            login_secondary_focus: cx.focus_handle(),
            login_cancel_focus: cx.focus_handle(),
        }
    }

    fn submit_composer(&mut self, cx: &mut Context<Self>) {
        self.composer_input
            .update(cx, |input, cx| input.request_submit(cx));
    }

    fn open_create_channel_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).create_channel_modal.is_some() {
            window.focus(&self.create_channel_name_input.focus_handle(cx));
            return;
        }
        self.create_channel_name_input
            .update(cx, |input, cx| input.clear(cx));
        self.create_channel_desc_input
            .update(cx, |input, cx| input.clear(cx));
        self.state.update(cx, |s, cx| {
            if s.quick_switcher.is_some() {
                s.close_quick_switcher();
            }
            if s.emoji_picker_open {
                s.close_emoji_picker();
            }
            s.open_create_channel_modal();
            cx.notify();
        });
        window.focus(&self.create_channel_name_input.focus_handle(cx));
    }

    fn open_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn toggle_message_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let is_open = self.state.read(cx).message_search.is_some();
        if is_open {
            self.message_search_input
                .update(cx, |input, cx| input.clear(cx));
            self.state.update(cx, |s, cx| {
                s.close_message_search();
                cx.notify();
            });
        } else {
            self.open_message_search(window, cx);
        }
    }

    fn open_message_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.message_search_input
            .update(cx, |input, cx| input.clear(cx));
        self.state.update(cx, |s, cx| {
            if s.quick_switcher.is_some() {
                s.close_quick_switcher();
            }
            if s.emoji_picker_open {
                s.close_emoji_picker();
            }
            s.open_message_search();
            s.pending_focus_composer = false;
            cx.notify();
        });
        window.focus(&self.message_search_input.focus_handle(cx));
    }

    fn render_login_view(
        &self,
        phase: &LoginPhase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let api_url = state.api_base_url.clone();
        let auth_summary = state.auth_summary.clone();
        let is_oidc = matches!(state.config.auth, AuthConfig::Oidc(_));

        if !self.login_primary_focus.is_focused(window)
            && !self.login_secondary_focus.is_focused(window)
            && !self.login_cancel_focus.is_focused(window)
            && !matches!(phase, LoginPhase::Checking)
        {
            window.focus(&self.login_primary_focus);
        }

        let body = match phase {
            LoginPhase::Checking => div().flex().flex_col().items_center().gap_3().child(
                div()
                    .text_base()
                    .text_color(theme.accent)
                    .child("接続とセッションを確認しています…"),
            ),
            LoginPhase::Ready { error } => {
                let mut col = div().flex().flex_col().gap_3().w_full();
                if let Some(err) = error {
                    col = col.child(
                        div()
                            .w_full()
                            .p_3()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.danger)
                            .text_sm()
                            .text_color(theme.warning)
                            .child(err.clone()),
                    );
                }
                if is_oidc {
                    let primary_focused = self.login_primary_focus.is_focused(window);
                    let secondary_focused = self.login_secondary_focus.is_focused(window);
                    col = col
                        .child(
                            div()
                                .id("btn-start-device-login")
                                .track_focus(&self.login_primary_focus)
                                .w_full()
                                .py_2()
                                .px_4()
                                .rounded_md()
                                .bg(theme.accent_button)
                                .border_1()
                                .border_color(if primary_focused {
                                    theme.text_primary
                                } else {
                                    theme.accent_button
                                })
                                .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_on_accent)
                                .child("ブラウザでログイン (Enter)")
                                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                    if is_activate_key(&ev.keystroke.key) {
                                        this.state.update(cx, |s, cx| {
                                            s.start_device_login();
                                            cx.notify();
                                        });
                                    }
                                }))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.start_device_login();
                                            cx.notify();
                                        });
                                    }),
                                ),
                        )
                        .child(
                            div()
                                .id("btn-retry-initial-auth")
                                .track_focus(&self.login_secondary_focus)
                                .w_full()
                                .py_2()
                                .px_4()
                                .rounded_md()
                                .bg(theme.bg_surface)
                                .border_1()
                                .border_color(if secondary_focused {
                                    theme.border_accent
                                } else {
                                    theme.border
                                })
                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                .text_center()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child("保存済みトークンで再試行")
                                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                    if is_activate_key(&ev.keystroke.key) {
                                        this.state.update(cx, |s, cx| {
                                            s.start_initial_auth();
                                            cx.notify();
                                        });
                                    }
                                }))
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.start_initial_auth();
                                            cx.notify();
                                        });
                                    }),
                                ),
                        );
                } else {
                    let primary_focused = self.login_primary_focus.is_focused(window);
                    col = col.child(
                        div()
                            .id("btn-retry-dev-auth")
                            .track_focus(&self.login_primary_focus)
                            .w_full()
                            .py_2()
                            .px_4()
                            .rounded_md()
                            .bg(theme.accent_button)
                            .border_1()
                            .border_color(if primary_focused {
                                theme.text_primary
                            } else {
                                theme.accent_button
                            })
                            .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                            .text_center()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_on_accent)
                            .child("再接続する (Enter)")
                            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                if is_activate_key(&ev.keystroke.key) {
                                    this.state.update(cx, |s, cx| {
                                        s.start_initial_auth();
                                        cx.notify();
                                    });
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.start_initial_auth();
                                        cx.notify();
                                    });
                                }),
                            ),
                    );
                }
                col
            }
            LoginPhase::AwaitingApproval {
                verification_uri,
                user_code,
            } => {
                let uri_for_open = verification_uri.clone();
                let uri_for_key = verification_uri.clone();
                let code_for_copy = user_code.clone();
                let code_for_key = user_code.clone();
                let primary_focused = self.login_primary_focus.is_focused(window);
                let secondary_focused = self.login_secondary_focus.is_focused(window);
                let cancel_focused = self.login_cancel_focus.is_focused(window);
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.text_primary)
                            .child("ブラウザで次のコードを確認し、ログインを承認してください。"),
                    )
                    .child(
                        div()
                            .w_full()
                            .p_4()
                            .rounded_md()
                            .bg(theme.bg_primary)
                            .border_1()
                            .border_color(theme.border_accent)
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("確認コード"),
                            )
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.accent)
                                    .child(user_code.clone()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(verification_uri.clone()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .w_full()
                            .child(
                                div()
                                    .id("btn-open-browser")
                                    .track_focus(&self.login_primary_focus)
                                    .flex_1()
                                    .py_2()
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme.accent_button)
                                    .border_1()
                                    .border_color(if primary_focused {
                                        theme.text_primary
                                    } else {
                                        theme.accent_button
                                    })
                                    .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_on_accent)
                                    .child("ブラウザを開く")
                                    .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, _| {
                                        if is_activate_key(&ev.keystroke.key) {
                                            login::open_browser(&uri_for_key);
                                        }
                                    }))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |_, _: &MouseUpEvent, _, _| {
                                            login::open_browser(&uri_for_open);
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("btn-copy-user-code")
                                    .track_focus(&self.login_secondary_focus)
                                    .flex_1()
                                    .py_2()
                                    .px_3()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(if secondary_focused {
                                        theme.border_accent
                                    } else {
                                        theme.border
                                    })
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_center()
                                    .text_sm()
                                    .text_color(theme.text_primary)
                                    .child("コードをコピー")
                                    .on_key_down(cx.listener(move |_, ev: &KeyDownEvent, _, cx| {
                                        if is_activate_key(&ev.keystroke.key) {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                code_for_key.clone(),
                                            ));
                                        }
                                    }))
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                code_for_copy.clone(),
                                            ));
                                        }),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .id("btn-cancel-device-login")
                            .track_focus(&self.login_cancel_focus)
                            .w_full()
                            .py_1p5()
                            .px_3()
                            .rounded_md()
                            .border_1()
                            .border_color(if cancel_focused {
                                theme.border_accent
                            } else {
                                theme.bg_secondary
                            })
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_center()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("キャンセル (Esc)")
                            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                                if is_activate_key(&ev.keystroke.key)
                                    || ev.keystroke.key == "escape"
                                {
                                    this.state.update(cx, |s, cx| {
                                        s.cancel_device_login();
                                        cx.notify();
                                    });
                                }
                            }))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.cancel_device_login();
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
            }
        };

        let is_awaiting = matches!(phase, LoginPhase::AwaitingApproval { .. });
        let is_ready_oidc = matches!(phase, LoginPhase::Ready { .. }) && is_oidc;

        div()
            .flex()
            .size_full()
            .bg(theme.bg_primary)
            .text_color(theme.text_primary)
            .justify_center()
            .items_center()
            .p_6()
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                if key == "escape" && is_awaiting {
                    this.state.update(cx, |s, cx| {
                        s.cancel_device_login();
                        cx.notify();
                    });
                    return;
                }
                let forward =
                    matches!(key, "tab" | "down" | "right") && !ev.keystroke.modifiers.shift;
                let backward =
                    (key == "tab" && ev.keystroke.modifiers.shift) || matches!(key, "up" | "left");
                if !forward && !backward {
                    return;
                }
                if is_awaiting {
                    let handles = [
                        &this.login_primary_focus,
                        &this.login_secondary_focus,
                        &this.login_cancel_focus,
                    ];
                    let current = handles
                        .iter()
                        .position(|h| h.is_focused(window))
                        .unwrap_or(0);
                    let next = if forward {
                        (current + 1) % handles.len()
                    } else {
                        (current + handles.len() - 1) % handles.len()
                    };
                    window.focus(handles[next]);
                    cx.notify();
                } else if is_ready_oidc {
                    let handles = [&this.login_primary_focus, &this.login_secondary_focus];
                    let current = handles
                        .iter()
                        .position(|h| h.is_focused(window))
                        .unwrap_or(0);
                    let next = (current + 1) % handles.len();
                    window.focus(handles[next]);
                    cx.notify();
                }
            }))
            .child(
                div()
                    .w(px(460.0))
                    .p_6()
                    .rounded_lg()
                    .bg(theme.bg_secondary)
                    .border_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div()
                                    .text_xl()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child("zoo2 Desktop"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(format!("接続先: {api_url}")),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child(format!("認証方式: {auth_summary}")),
                            ),
                    )
                    .child(body),
            )
    }

    fn render_sidebar_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let me_summary = state
            .me
            .as_ref()
            .map(|m| format!("{} (@{})", m.display_name, m.user_name))
            .unwrap_or_default();
        let is_oidc = matches!(state.config.auth, AuthConfig::Oidc(_));
        let channels_synced = state.store.is_synced(&ShapeId::Channels);
        let sync_error = state.sync_error();
        let notice = state.notice.as_ref().map(|(m, _)| m.clone());
        let current_channel = state.current_channel.clone();
        let unread = state.unread.clone();

        let filtered = state.filtered_channels();
        let mut normal_rows = Vec::new();
        let mut dm_rows = Vec::new();

        let theme = state.theme();

        for ch in filtered {
            let ch_id = ch.id.clone();
            let ch_id_click = ch_id.clone();
            let is_dm = ch.is_dm;
            let is_open = current_channel.as_deref() == Some(ch_id.as_str());
            let is_unread = unread.contains(&ch_id);
            let is_member = state.store.is_member(&ch_id, &me_id);
            let mut label = state.store.channel_label(ch, &me_id);
            if ch.is_private && !ch.is_dm {
                label.push_str(" 🔒");
            }
            if ch.archive_at.is_some() {
                label.push_str(" (アーカイブ)");
            }

            let row_bg = if is_open {
                theme.bg_selected
            } else {
                theme.bg_secondary
            };
            let text_color = if is_open || is_unread {
                theme.text_primary
            } else if is_member {
                theme.text_muted
            } else {
                theme.text_dim
            };
            let weight = if is_open || is_unread {
                FontWeight::BOLD
            } else {
                FontWeight::NORMAL
            };

            let row = div()
                .id(ElementId::Name(SharedString::from(format!(
                    "sidebar-ch-{ch_id}"
                ))))
                .w_full()
                .px_2p5()
                .py_1p5()
                .rounded_md()
                .bg(row_bg)
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .flex()
                .flex_row()
                .items_center()
                .gap_1p5()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        let id = ch_id_click.clone();
                        this.state.update(cx, |s, cx| {
                            s.open_channel(id);
                            cx.notify();
                        });
                    }),
                )
                .child(
                    div()
                        .w(px(10.0))
                        .text_xs()
                        .text_color(theme.accent)
                        .child(if is_unread { "●" } else { "" }),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_sm()
                        .font_weight(weight)
                        .text_color(text_color)
                        .child(label),
                );

            if is_dm {
                dm_rows.push(row);
            } else {
                normal_rows.push(row);
            }
        }

        div()
            .flex()
            .flex_col()
            .w(px(260.0))
            .h_full()
            .bg(theme.bg_secondary)
            .border_r_1()
            .border_color(theme.border)
            // サイドバーヘッダー
            .child(
                div()
                    .p_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child("zoo2"),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_1p5()
                                    .child(
                                        div()
                                            .id("btn-open-create-channel")
                                            .px_2()
                                            .py_0p5()
                                            .rounded_sm()
                                            .bg(theme.bg_surface)
                                            .border_1()
                                            .border_color(theme.border)
                                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                            .text_xs()
                                            .text_color(theme.text_primary)
                                            .child("+ 作成")
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(
                                                    |this, _: &MouseUpEvent, window, cx| {
                                                        this.open_create_channel_modal(window, cx);
                                                    },
                                                ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .id("btn-toggle-theme")
                                            .px_1p5()
                                            .py_0p5()
                                            .rounded_sm()
                                            .bg(theme.bg_surface)
                                            .border_1()
                                            .border_color(theme.border)
                                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                            .text_xs()
                                            .child(if state.theme_mode == theme::ThemeMode::Dark {
                                                "🌙"
                                            } else {
                                                "☀️"
                                            })
                                            .on_mouse_up(
                                                MouseButton::Left,
                                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                    this.state.update(cx, |s, cx| {
                                                        s.toggle_theme();
                                                        cx.notify();
                                                    });
                                                }),
                                            ),
                                    )
                                    .when(is_oidc, |d| {
                                        d.child(
                                            div()
                                                .id("btn-logout")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(theme.bg_surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                                .text_xs()
                                                .text_color(theme.text_muted)
                                                .child("ログアウト")
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                        this.state.update(cx, |s, cx| {
                                                            s.logout();
                                                            cx.notify();
                                                        });
                                                    }),
                                                ),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(me_summary),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .child(div().flex_1().child(self.search_input.clone()))
                            .child(
                                div()
                                    .id("btn-open-quick-switcher")
                                    .px_2()
                                    .py_1p5()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("⌘K")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, window, cx| {
                                            this.open_quick_switcher(window, cx);
                                        }),
                                    ),
                            ),
                    ),
            )
            // チャンネル・DM 一覧
            .child(
                div()
                    .id("sidebar-scroll-list")
                    .flex_1()
                    .overflow_y_scroll()
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_muted)
                            .child(if channels_synced {
                                "チャンネル"
                            } else if sync_error.is_some() {
                                "チャンネル (同期停止)"
                            } else {
                                "チャンネル (同期中…)"
                            }),
                    )
                    .when_some(sync_error, |d, err| {
                        d.child(
                            div()
                                .mx_2()
                                .my_1()
                                .p_2()
                                .rounded_md()
                                .bg(theme.bg_surface)
                                .border_1()
                                .border_color(theme.danger)
                                .flex()
                                .flex_col()
                                .gap_1p5()
                                .child(div().text_xs().text_color(theme.warning).child(err))
                                .child(
                                    div()
                                        .id("btn-sidebar-retry-sync")
                                        .px_2()
                                        .py_1()
                                        .rounded_sm()
                                        .bg(theme.accent_button)
                                        .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                        .text_center()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_on_accent)
                                        .child("再同期する")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.retry_sync();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                ),
                        )
                    })
                    .when(current_channel.is_none(), |d| {
                        d.when_some(notice, |d, msg| {
                            d.child(
                                div()
                                    .mx_2()
                                    .my_1()
                                    .p_2()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.warning)
                                    .text_xs()
                                    .text_color(theme.warning)
                                    .child(msg),
                            )
                        })
                    })
                    .children(normal_rows)
                    .when(!dm_rows.is_empty(), |d| {
                        d.child(
                            div()
                                .mt_2()
                                .px_2()
                                .py_1()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_muted)
                                .child("ダイレクトメッセージ"),
                        )
                        .children(dm_rows)
                    }),
            )
    }

    fn render_timeline_view(
        &self,
        pending_scroll_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> (Div, Option<usize>) {
        let (
            channel_id,
            me_id,
            me_user_name,
            ch_label,
            ch_desc,
            is_archived,
            is_member,
            can_join,
            can_leave,
            failed_draft_count,
            confirm_delete_id,
            synced,
            timeline_at_bottom,
            all_root_messages,
            reply_counts,
            reactions_by_msg,
            is_searching,
            start_idx,
            end_idx,
            theme,
        ) = {
            let state = self.state.read(cx);
            let theme = state.theme();
            let Some(channel_id) = state.current_channel.clone() else {
                return (
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .h_full()
                        .justify_center()
                        .items_center()
                        .p_6()
                        .gap_2()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_primary)
                                .child("チャンネルを選択してください"),
                        )
                        .child(div().text_sm().text_color(theme.text_muted).child(
                            "左のサイドバーからチャンネルまたはダイレクトメッセージを開けます。",
                        )),
                    None,
                );
            };

            let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
            let me_user_name = state
                .me
                .as_ref()
                .map(|m| m.user_name.clone())
                .unwrap_or_default();
            let channel = state.store.channel(&channel_id);
            let ch_label = channel
                .map(|c| state.store.channel_label(c, &me_id))
                .unwrap_or_else(|| format!("#{channel_id}"));
            let ch_desc = channel
                .and_then(|c| c.description.clone())
                .filter(|d| !d.is_empty());
            let is_archived = channel.is_some_and(|c| c.archive_at.is_some());
            let is_member = state.store.is_member(&channel_id, &me_id);
            let can_join = channel.is_some_and(|c| state.can_join_channel(c));
            let can_leave = channel.is_some_and(|c| state.can_leave_channel(c));
            let failed_draft_count = state.failed_drafts.len();
            let confirm_delete_id = state.confirm_delete_message.clone();
            let synced = state
                .store
                .is_synced(&ShapeId::Messages(channel_id.clone()));
            let timeline_at_bottom = state.timeline_at_bottom;

            let all_root_messages = state.channel_cache.root_messages.clone();
            let (start_idx, end_idx) = state.visible_timeline_bounds(&all_root_messages);
            let reply_counts = state.channel_cache.reply_counts.clone();
            let reactions_by_msg = state.channel_cache.reactions_by_message.clone();
            let is_searching = state.message_search.is_some();

            (
                channel_id,
                me_id,
                me_user_name,
                ch_label,
                ch_desc,
                is_archived,
                is_member,
                can_join,
                can_leave,
                failed_draft_count,
                confirm_delete_id,
                synced,
                timeline_at_bottom,
                all_root_messages,
                reply_counts,
                reactions_by_msg,
                is_searching,
                start_idx,
                end_idx,
                theme,
            )
        };

        let hidden_before = start_idx;
        let hidden_after = all_root_messages.len().saturating_sub(end_idx);
        let visible_roots = &all_root_messages[start_idx..end_idx];

        let mut timeline_items: Vec<Div> = Vec::new();
        if hidden_before > 0 {
            timeline_items.push(
                div().w_full().flex().justify_center().py_1().child(
                    div()
                        .id("btn-load-older-messages")
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .border_1()
                        .border_color(theme.border)
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_xs()
                        .text_color(theme.accent)
                        .child(format!("過去のメッセージを表示 (残り {hidden_before} 件)"))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                this.state.update(cx, |s, cx| {
                                    s.load_older_messages();
                                    cx.notify();
                                });
                            }),
                        ),
                ),
            );
        }
        let mut target_scroll_item_index = None;
        let mut last_date = None;
        for msg in visible_roots {
            let date = local_zoned(msg.created_at).map(|z| z.date());
            if date.is_some() && date != last_date {
                if let Some(d) = date {
                    timeline_items.push(render_date_separator(&d.to_string(), &theme));
                }
                last_date = date;
            }
            if pending_scroll_id == Some(msg.id.as_str()) {
                target_scroll_item_index = Some(timeline_items.len());
            }
            let reply_count = reply_counts.get(&msg.id).copied().unwrap_or(0);
            let reactions = reactions_by_msg.get(&msg.id).cloned().unwrap_or_default();
            timeline_items.push(self.render_message_card(
                msg,
                &me_id,
                &me_user_name,
                is_archived,
                reactions,
                Some(reply_count),
                "timeline",
                cx,
            ));
        }

        let ch_id_for_join = channel_id.clone();
        let theme = self.state.read(cx).theme();

        let timeline_view = div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            // チャンネルヘッダー
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(theme.bg_surface)
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .overflow_hidden()
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child(ch_label),
                            )
                            .when_some(ch_desc, |d, desc| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .child(format!("— {desc}")),
                                )
                            })
                            .when(is_archived, |d| {
                                d.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .text_xs()
                                        .text_color(theme.warning)
                                        .child("アーカイブ済み"),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .when(failed_draft_count > 0, |d| {
                                d.child(
                                    div()
                                        .id("btn-restore-draft")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme.bg_secondary)
                                        .border_1()
                                        .border_color(theme.warning)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.warning)
                                        .child(format!("下書きを復元 ({failed_draft_count})"))
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.restore_failed_draft();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            })
                            .when(can_join || can_leave, |d| {
                                d.child(
                                    div()
                                        .id("btn-join-or-leave")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(if is_member {
                                            theme.bg_secondary
                                        } else {
                                            theme.accent_button
                                        })
                                        .border_1()
                                        .border_color(theme.border)
                                        .hover(move |s| {
                                            s.bg(if is_member {
                                                theme.bg_hover
                                            } else {
                                                theme.accent_button_hover
                                            })
                                            .cursor_pointer()
                                        })
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(if is_member {
                                            theme.text_primary
                                        } else {
                                            theme.text_on_accent
                                        })
                                        .child(if is_member {
                                            "退出する"
                                        } else {
                                            "参加する"
                                        })
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                                let ch = ch_id_for_join.clone();
                                                this.state.update(cx, |s, cx| {
                                                    s.join_or_leave(ch, !is_member);
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .id("btn-open-search")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("🔍 検索")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, window, cx| {
                                            this.toggle_message_search(window, cx);
                                        }),
                                    ),
                            ),
                    ),
            )
            // 検索バー
            .when(is_searching, |d| {
                d.child(self.render_message_search_bar(cx))
            })
            // 削除確認バー
            .when(confirm_delete_id.is_some(), |d| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .bg(theme.bg_secondary)
                        .border_b_1()
                        .border_color(theme.danger)
                        .flex()
                        .flex_row()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.warning)
                                .child("選択したメッセージを削除しますか？"),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_2()
                                .child(
                                    div()
                                        .id("btn-confirm-delete-yes")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme.danger)
                                        .hover(|s| s.cursor_pointer())
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_primary)
                                        .child("削除する")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.confirm_delete();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("btn-confirm-delete-no")
                                        .px_3()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme.bg_surface)
                                        .border_1()
                                        .border_color(theme.border)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child("キャンセル")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                this.state.update(cx, |s, cx| {
                                                    s.cancel_delete();
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                ),
                        ),
                )
            })
            // タイムライン本体
            .child(
                div()
                    .id("timeline-scroll-area")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.timeline_scroll)
                    .p_4()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(!synced, |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child("メッセージを読み込み中…"),
                        )
                    })
                    .when(synced && timeline_items.is_empty(), |d| {
                        d.child(
                            div()
                                .text_sm()
                                .text_color(theme.text_muted)
                                .child("まだメッセージがありません"),
                        )
                    })
                    .children(timeline_items),
            )
            // 最新へ戻るバー
            .when(!timeline_at_bottom || hidden_after > 0, |d| {
                d.child(
                    div()
                        .px_4()
                        .py_1p5()
                        .bg(theme.bg_surface)
                        .border_t_1()
                        .border_color(theme.border)
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("btn-jump-to-latest")
                                .px_3()
                                .py_1()
                                .rounded_md()
                                .bg(theme.accent_button)
                                .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_on_accent)
                                .child(if hidden_after > 0 {
                                    format!("↓ 最新メッセージへ移動 (新着ほか {hidden_after} 件)")
                                } else {
                                    "↓ 最新メッセージへ移動".to_string()
                                })
                                .on_mouse_up(
                                    MouseButton::Left,
                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                        this.state.update(cx, |s, cx| {
                                            s.jump_to_latest_messages();
                                            cx.notify();
                                        });
                                    }),
                                ),
                        ),
                )
            })
            // 下部入力欄
            .child(self.render_composer_view(cx));

        (timeline_view, target_scroll_item_index)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_message_card(
        &self,
        message: &Message,
        me_id: &str,
        me_user_name: &str,
        is_archived: bool,
        reactions: Vec<ReactionGroup>,
        reply_count: Option<usize>,
        pane_prefix: &str,
        cx: &mut Context<Self>,
    ) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let msg_id = message.id.clone();
        let author_name = state.store.display_name(&message.user_id);
        let is_bot = state.store.user(&message.user_id).is_some_and(|u| u.is_bot);
        let is_mine = message.user_id == me_id;
        let is_edited = message.updated_at.is_some();
        let time_str = local_time_string(message.created_at);
        let reaction_picker_open = !is_archived
            && state
                .active_reaction_message
                .as_ref()
                .is_some_and(|(pane, id)| pane == pane_prefix && id == &msg_id);

        let msg_id_reply = msg_id.clone();
        let msg_id_react = msg_id.clone();
        let pane_for_react = pane_prefix.to_string();
        let msg_id_edit = msg_id.clone();
        let msg_id_delete = msg_id.clone();
        let msg_id_thread = msg_id.clone();

        let mut reaction_pills = Vec::new();
        for group in reactions {
            let emoji = group.emoji.clone();
            let msg_for_toggle = msg_id.clone();
            let pill_bg = if group.mine {
                theme.bg_reaction_mine
            } else {
                theme.bg_secondary
            };
            let pill_border = if group.mine {
                theme.border_accent
            } else {
                theme.border
            };
            let pill_text = if group.mine {
                theme.accent
            } else {
                theme.text_muted
            };
            let pill = div()
                .id(ElementId::Name(SharedString::from(format!(
                    "{pane_prefix}-react-pill-{}-{emoji}",
                    message.id
                ))))
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(pill_bg)
                .border_1()
                .border_color(pill_border)
                .text_xs()
                .text_color(pill_text)
                .child(format!("{} {}", reaction_label(&group.emoji), group.count))
                .when(!is_archived, |d| {
                    d.hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let m = msg_for_toggle.clone();
                                let e = emoji.clone();
                                this.state.update(cx, |s, cx| {
                                    s.toggle_reaction(&m, &e);
                                    cx.notify();
                                });
                            }),
                        )
                });
            reaction_pills.push(pill);
        }

        let mut quick_emoji_buttons = Vec::new();
        if reaction_picker_open {
            for &emoji in QUICK_EMOJIS {
                let m_id = msg_id.clone();
                let emoji_str = emoji.to_string();
                quick_emoji_buttons.push(
                    div()
                        .id(ElementId::Name(SharedString::from(format!(
                            "{pane_prefix}-quick-emoji-{msg_id}-{emoji}"
                        ))))
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .bg(theme.bg_primary)
                        .border_1()
                        .border_color(theme.border)
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_sm()
                        .child(emoji)
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let m = m_id.clone();
                                let e = emoji_str.clone();
                                this.state.update(cx, |s, cx| {
                                    s.active_reaction_message = None;
                                    s.toggle_reaction(&m, &e);
                                    cx.notify();
                                });
                            }),
                        ),
                );
            }
        }

        let root_for_reply_btn = message
            .parent_id
            .clone()
            .unwrap_or_else(|| msg_id_reply.clone());

        let is_search_matched = state
            .message_search
            .as_ref()
            .is_some_and(|ms| ms.matches.contains(&msg_id));
        let is_current_search_match = state
            .message_search
            .as_ref()
            .and_then(|ms| ms.matches.get(ms.current_match_index))
            == Some(&msg_id);

        let card_border_color = if is_current_search_match {
            theme.accent
        } else if is_search_matched {
            theme.border_accent
        } else {
            theme.border
        };
        let card_bg = if is_current_search_match {
            theme.bg_mention_mine
        } else if is_search_matched {
            theme.bg_selected
        } else {
            theme.bg_surface
        };

        let attachments_with_urls: Vec<(
            zoo2_client_core::electric::rows::Attachment,
            Option<String>,
            bool,
            bool,
        )> = state
            .store
            .attachments_for_message(&message.channel_id, &message.id)
            .into_iter()
            .map(|att| {
                let url = state.attachment_download_url(&att.id);
                let is_fetching = state.fetching_attachment_urls.contains(&att.id)
                    || state.pending_open_attachments.contains(&att.id);
                let needs_refresh = state.attachment_url_needs_refresh(&att.id);
                (att.clone(), url, is_fetching, needs_refresh)
            })
            .collect();
        let attachment_views: Vec<Div> = attachments_with_urls
            .into_iter()
            .map(|(att, url, is_fetching, needs_refresh)| {
                render_attachment_item(&att, url, is_fetching, needs_refresh, &theme, cx)
            })
            .collect();

        let ogp_cards = message
            .card
            .as_deref()
            .map(parse_message_cards)
            .unwrap_or_default();
        let ogp_views: Vec<Div> = ogp_cards
            .iter()
            .map(|card| render_ogp_card(card, &theme, cx))
            .collect();

        div()
            .w_full()
            .p_3()
            .rounded_md()
            .bg(card_bg)
            .border_1()
            .border_color(card_border_color)
            .flex()
            .flex_col()
            .gap_1p5()
            // メッセージヘッダー (投稿者・時刻・操作ボタン)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_primary)
                                    .child(author_name),
                            )
                            .when(is_bot, |d| {
                                d.child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child("BOT"),
                                )
                            })
                            .child(div().text_xs().text_color(theme.text_muted).child(time_str))
                            .when(is_edited, |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_dim)
                                        .child("(編集済み)"),
                                )
                            }),
                    )
                    // 操作ボタン群
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap_1p5()
                            .when(!is_archived, |d| {
                                d.child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-reply-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child("返信")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let r_id = root_for_reply_btn.clone();
                                                    this.state.update(cx, |s, cx| {
                                                        s.open_thread(r_id);
                                                        cx.notify();
                                                    });
                                                    window.focus(
                                                        &this.composer_input.focus_handle(cx),
                                                    );
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-react-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child("+ リアクション")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let m_id = msg_id_react.clone();
                                                    let pane_id = pane_for_react.clone();
                                                    let opened = this.state.update(cx, |s, cx| {
                                                        let target_key = (pane_id, m_id);
                                                        if s.active_reaction_message.as_ref()
                                                            == Some(&target_key)
                                                        {
                                                            s.active_reaction_message = None;
                                                            cx.notify();
                                                            false
                                                        } else {
                                                            s.active_reaction_message =
                                                                Some(target_key);
                                                            cx.notify();
                                                            true
                                                        }
                                                    });
                                                    if opened {
                                                        this.reaction_input
                                                            .update(cx, |input, cx| {
                                                                input.clear(cx)
                                                            });
                                                        window.focus(
                                                            &this.reaction_input.focus_handle(cx),
                                                        );
                                                    }
                                                },
                                            ),
                                        ),
                                )
                            })
                            .when(is_mine && !is_archived, |d| {
                                d.child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-edit-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.text_muted)
                                        .child("編集")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let m_id = msg_id_edit.clone();
                                                    this.state.update(cx, |s, cx| {
                                                        s.start_edit(m_id);
                                                        cx.notify();
                                                    });
                                                    window.focus(
                                                        &this.composer_input.focus_handle(cx),
                                                    );
                                                },
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .id(ElementId::Name(SharedString::from(format!(
                                            "{pane_prefix}-btn-delete-{msg_id}"
                                        ))))
                                        .px_2()
                                        .py_0p5()
                                        .rounded_sm()
                                        .bg(theme.bg_secondary)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_xs()
                                        .text_color(theme.danger)
                                        .child("削除")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                                let m_id = msg_id_delete.clone();
                                                this.state.update(cx, |s, cx| {
                                                    s.request_delete(m_id);
                                                    cx.notify();
                                                });
                                            }),
                                        ),
                                )
                            }),
                    ),
            )
            // MFM 本文
            .child(mfm_view::render_mfm_with_theme(
                &format!("{pane_prefix}-{msg_id}"),
                &message.content,
                Some(me_user_name),
                &theme,
            ))
            // 添付ファイル (画像・ファイル)
            .when(!attachment_views.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1p5()
                        .mt_1()
                        .children(attachment_views),
                )
            })
            // OGP リンクカード
            .when(!ogp_views.is_empty(), |d| {
                d.child(div().flex().flex_col().gap_1p5().mt_1().children(ogp_views))
            })
            // リアクションピル一覧
            .when(!reaction_pills.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .flex_wrap()
                        .gap_1p5()
                        .mt_1()
                        .children(reaction_pills),
                )
            })
            // インラインリアクションピッカー
            .when(reaction_picker_open, |d| {
                d.child(
                    div()
                        .mt_1()
                        .p_2()
                        .rounded_md()
                        .bg(theme.bg_secondary)
                        .border_1()
                        .border_color(theme.border_accent)
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .gap_1p5()
                                .children(quick_emoji_buttons),
                        )
                        .child(self.reaction_input.clone()),
                )
            })
            // スレッド返信数ボタン
            .when_some(reply_count.filter(|&c| c > 0), |d, count| {
                d.child(
                    div()
                        .id(ElementId::Name(SharedString::from(format!(
                            "btn-open-thread-{}",
                            message.id
                        ))))
                        .mt_1()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.accent)
                        .child(format!("💬 返信 {count} 件を表示"))
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let r_id = msg_id_thread.clone();
                                this.state.update(cx, |s, cx| {
                                    s.open_thread(r_id);
                                    cx.notify();
                                });
                            }),
                        ),
                )
            })
    }

    fn render_composer_view(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
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

        let (target_label, show_cancel, typing_scope) = match &state.target {
            Target::Channel => {
                let label = state
                    .current_channel
                    .as_deref()
                    .and_then(|id| state.store.channel(id))
                    .map(|ch| format!("{} へ送信", state.store.channel_label(ch, &me_id)))
                    .unwrap_or_else(|| "メッセージ入力".to_string());
                (label, false, None)
            }
            Target::Reply(root_id) => (
                "💬 スレッドに返信中".to_string(),
                true,
                Some(root_id.clone()),
            ),
            Target::Edit(_) => ("✏️ メッセージを編集中".to_string(), true, None),
        };

        let is_editing = matches!(state.target, Target::Edit(_));
        let is_posting_restricted =
            is_synced && has_channel && !is_archived && !can_post && !is_editing;
        let show_composer = !is_archived && (can_post || is_editing);
        let typing_text = state.typing_text(&typing_scope);
        let notice = state.notice.as_ref().map(|(msg, _)| msg.clone());
        let invalid_rows = state.store.invalid_rows;
        let update_available = state.update_available.clone();

        let pending_attachments_bar = self.render_pending_attachments_bar(cx);
        let mention_autocomplete_view = self.render_mention_autocomplete(cx);

        div()
            .w_full()
            .p_3()
            .bg(theme.bg_secondary)
            .border_t_1()
            .border_color(theme.border)
            .flex()
            .flex_col()
            .gap_2()
            .when_some(mention_autocomplete_view, |d, v| d.child(v))
            .when_some(pending_attachments_bar, |d, v| d.child(v))
            .when(is_archived, |d| {
                d.child(
                    div()
                        .w_full()
                        .p_2p5()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(
                            "このチャンネルはアーカイブされているため、メッセージの投稿や編集はできません",
                        ),
                )
            })
            .when(is_posting_restricted, |d| {
                d.child(
                    div()
                        .w_full()
                        .p_2p5()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .border_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child("このチャンネルは管理者のみ投稿できます"),
                )
            })
            .when(show_composer, |d| {
                d
                    // 送信先バナー + 入力中インジケーター
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(if show_cancel {
                                                theme.accent
                                            } else {
                                                theme.text_muted
                                            })
                                            .child(target_label),
                                    )
                                    .when(show_cancel, |d| {
                                        d.child(
                                            div()
                                                .id("btn-cancel-target")
                                                .px_2()
                                                .py_0p5()
                                                .rounded_sm()
                                                .bg(theme.bg_surface)
                                                .border_1()
                                                .border_color(theme.border)
                                                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                                .text_xs()
                                                .text_color(theme.text_muted)
                                                .child("キャンセル (Esc)")
                                                .on_mouse_up(
                                                    MouseButton::Left,
                                                    cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                                        this.state.update(cx, |s, cx| {
                                                            s.cancel_target();
                                                            cx.notify();
                                                        });
                                                    }),
                                                ),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .items_center()
                                    .gap_3()
                                    .when_some(typing_text, |d, t| {
                                        d.child(div().text_xs().text_color(theme.accent).child(t))
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_dim)
                                            .child("Enter: 送信 / Shift+Enter: 改行"),
                                    ),
                            ),
                    )
                    // 入力欄 + 送信ボタン & アクションボタン
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_end()
                            .gap_2()
                            .when(!is_editing, |d| {
                                d.child(
                                    div()
                                        .id("btn-attach-file")
                                        .px_2p5()
                                        .py_2()
                                        .rounded_md()
                                        .bg(theme.bg_surface)
                                        .border_1()
                                        .border_color(theme.border)
                                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                        .text_sm()
                                        .child("📎")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(|_, _: &MouseUpEvent, _, cx| {
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
                                            }),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .id("btn-open-emoji-picker")
                                    .px_2p5()
                                    .py_2()
                                    .rounded_md()
                                    .bg(theme.bg_surface)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_sm()
                                    .child("😀")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                if s.emoji_picker_open {
                                                    s.close_emoji_picker();
                                                } else {
                                                    s.open_emoji_picker();
                                                }
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(div().flex_1().child(self.composer_input.clone()))
                            .child(
                                div()
                                    .id("btn-send-message")
                                    .px_4()
                                    .py_2()
                                    .rounded_md()
                                    .bg(theme.accent_button)
                                    .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                    .text_sm()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(theme.text_on_accent)
                                    .child(if is_editing { "保存" } else { "送信" })
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.submit_composer(cx);
                                        }),
                                    ),
                            ),
                    )
            })
            // ステータス通知行
            .when(
                notice.is_some() || invalid_rows > 0 || update_available.is_some(),
                |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_between()
                            .items_center()
                            .text_xs()
                            .child(
                                div()
                                    .text_color(theme.warning)
                                    .child(notice.unwrap_or_default()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_3()
                                    .when(invalid_rows > 0, |d| {
                                        d.child(
                                            div()
                                                .text_color(theme.danger)
                                                .child(format!("読めない行: {invalid_rows}")),
                                        )
                                    })
                                    .when_some(update_available, |d, ver| {
                                        d.child(
                                            div()
                                                .text_color(theme.success)
                                                .child(format!("新しい版 v{ver} があります")),
                                        )
                                    }),
                            ),
                    )
                },
            )
    }

    fn render_thread_view(
        &self,
        root_id: &str,
        pending_scroll_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> (Div, Option<usize>) {
        let state = self.state.read(cx);
        let theme = state.theme();
        let Some(channel_id) = state.current_channel.clone() else {
            return (div(), None);
        };
        let me_id = state.me.as_ref().map(|m| m.id.clone()).unwrap_or_default();
        let me_user_name = state
            .me
            .as_ref()
            .map(|m| m.user_name.clone())
            .unwrap_or_default();

        let is_archived = state
            .store
            .channel(&channel_id)
            .is_some_and(|c| c.archive_at.is_some());
        let can_post = state.can_post_in_channel(&channel_id);
        let reactions_by_msg = state.channel_cache.reactions_by_message.clone();
        let thread_typing = state.typing_text(&Some(root_id.to_string()));
        let root_message = state.channel_cache.thread_root_message.clone();
        let replies = state.channel_cache.thread_replies.clone();

        let mut target_scroll_index = None;
        let mut cards = Vec::new();
        if let Some(root) = &root_message {
            let root_reactions = reactions_by_msg.get(&root.id).cloned().unwrap_or_default();
            cards.push(self.render_message_card(
                root,
                &me_id,
                &me_user_name,
                is_archived,
                root_reactions,
                None,
                "thread",
                cx,
            ));

            cards.push(
                div()
                    .py_1()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.text_muted)
                    .child(format!("返信 ({} 件)", replies.len())),
            );
            for reply in replies.iter() {
                if pending_scroll_id == Some(reply.id.as_str()) {
                    target_scroll_index = Some(cards.len());
                }
                let r_reactions = reactions_by_msg.get(&reply.id).cloned().unwrap_or_default();
                cards.push(self.render_message_card(
                    reply,
                    &me_id,
                    &me_user_name,
                    is_archived,
                    r_reactions,
                    None,
                    "thread",
                    cx,
                ));
            }
        } else {
            cards.push(
                div()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child("元のメッセージは削除されました"),
            );
        }

        let root_for_focus = root_id.to_string();

        let thread_view = div()
            .flex()
            .flex_col()
            .w(px(380.0))
            .h_full()
            .bg(theme.bg_secondary)
            .border_l_1()
            .border_color(theme.border)
            // スレッドヘッダー
            .child(
                div()
                    .px_4()
                    .py_3()
                    .bg(theme.bg_surface)
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child("スレッド"),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .when(!is_archived && can_post, |d| {
                                d.child(
                                    div()
                                        .id("btn-thread-reply-focus")
                                        .px_2p5()
                                        .py_1()
                                        .rounded_md()
                                        .bg(theme.accent_button)
                                        .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_on_accent)
                                        .child("返信を書く")
                                        .on_mouse_up(
                                            MouseButton::Left,
                                            cx.listener(
                                                move |this, _: &MouseUpEvent, window, cx| {
                                                    let r_id = root_for_focus.clone();
                                                    this.state.update(cx, |s, cx| {
                                                        s.focus_thread_reply(r_id);
                                                        cx.notify();
                                                    });
                                                    window.focus(
                                                        &this.composer_input.focus_handle(cx),
                                                    );
                                                },
                                            ),
                                        ),
                                )
                            })
                            .child(
                                div()
                                    .id("btn-close-thread")
                                    .px_2p5()
                                    .py_1()
                                    .rounded_md()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_muted)
                                    .child("× 閉じる")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                s.close_thread();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            ),
                    ),
            )
            // スレッド本文一覧
            .child(
                div()
                    .id("thread-scroll-area")
                    .flex_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.thread_scroll)
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(cards),
            )
            // スレッド内の入力中表示
            .when_some(thread_typing, |d, t| {
                d.child(
                    div()
                        .px_4()
                        .py_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .text_xs()
                        .text_color(theme.accent)
                        .child(t),
                )
            });

        (thread_view, target_scroll_index)
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
        let theme = state.theme();
        let pills = attachments.into_iter().map(|att| {
            let local_id = att.local_id.clone();
            let name = att.file_name.clone();
            let size_str = format_file_size(att.file_size);
            let status_text = if att.is_uploading {
                " (アップロード中…)"
            } else if att.error.is_some() {
                " (エラー)"
            } else {
                ""
            };
            let text_color = if att.error.is_some() {
                theme.danger
            } else if att.is_uploading {
                theme.text_muted
            } else {
                theme.success
            };

            div()
                .px_2p5()
                .py_1()
                .rounded_md()
                .bg(theme.bg_surface)
                .border_1()
                .border_color(theme.border)
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(theme.accent).child("📎"))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_primary)
                        .child(format!("{name} ({size_str})")),
                )
                .child(div().text_xs().text_color(text_color).child(status_text))
                .child(
                    div()
                        .px_1()
                        .rounded_sm()
                        .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                        .text_xs()
                        .text_color(theme.danger)
                        .child("✕")
                        .on_mouse_up(
                            MouseButton::Left,
                            cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                                let lid = local_id.clone();
                                this.state.update(cx, |s, cx| {
                                    s.remove_pending_attachment(&lid);
                                    cx.notify();
                                });
                            }),
                        ),
                )
        });

        Some(
            div()
                .w_full()
                .flex()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .children(pills),
        )
    }

    fn render_mention_autocomplete(&self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.state.read(cx);
        let theme = state.theme();
        let autocomplete = state.mention_autocomplete.as_ref()?;
        if autocomplete.candidates.is_empty() {
            return None;
        }

        let items = autocomplete
            .candidates
            .iter()
            .enumerate()
            .map(|(i, candidate)| {
                let is_selected = i == autocomplete.selected_index;
                let display = candidate.display_name.clone();
                let idx = i;
                div()
                    .w_full()
                    .px_3()
                    .py_1p5()
                    .rounded_sm()
                    .bg(if is_selected {
                        theme.bg_selected
                    } else {
                        theme.bg_surface
                    })
                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                    .flex()
                    .flex_row()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(if is_selected {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(if candidate.is_special {
                                theme.accent
                            } else {
                                theme.text_primary
                            })
                            .child(display),
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
            });

        Some(
            div()
                .w_full()
                .p_1p5()
                .rounded_md()
                .bg(theme.bg_surface)
                .border_1()
                .border_color(theme.border_accent)
                .child(
                    div()
                        .id("mention-autocomplete-list")
                        .w_full()
                        .max_h(px(180.0))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .children(items),
                ),
        )
    }

    fn render_emoji_picker(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let query = state.emoji_picker_query.to_lowercase();

        let common_emojis = [
            "😀", "😃", "😄", "😁", "😆", "😅", "😂", "🤣", "😊", "😇", "🙂", "🙃", "😉", "😌",
            "😍", "🥰", "😘", "😗", "😙", "😚", "😋", "😛", "😝", "😜", "🤪", "🤨", "🧐", "🤓",
            "😎", "🥳", "😏", "😒", "😞", "😔", "😟", "😕", "🙁", "😣", "😖", "😫", "😩", "🥺",
            "😢", "😭", "😤", "😠", "😡", "🤬", "🤯", "😳", "🥵", "🥶", "😱", "😨", "😰", "😥",
            "😓", "🤗", "🤔", "🤭", "🤫", "🤥", "😶", "😐", "😑", "😬", "🙄", "😯", "😦", "😧",
            "😮", "😲", "🥱", "😴", "🤤", "👍", "👎", "👏", "🙌", "👐", "🤲", "🤝", "🙏", "✌️",
            "🤞", "🤟", "🤘", "👌", "🤌", "🤏", "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍",
            "🤎", "💔", "❣️", "💕", "💞", "💓", "💗", "💖", "💘", "💝", "✨", "🎉", "🎊", "🔥",
            "💯", "⭐", "🌟", "👀", "💡", "🚀", "☕", "🍺",
        ];

        let filtered: Vec<&str> = if query.is_empty() {
            common_emojis.to_vec()
        } else {
            common_emojis
                .into_iter()
                .filter(|e| {
                    if let Some(emoji_data) = emojis::get(e) {
                        emoji_data.name().to_lowercase().contains(&query)
                            || emoji_data
                                .shortcode()
                                .is_some_and(|s| s.to_lowercase().contains(&query))
                    } else {
                        false
                    }
                })
                .collect()
        };

        let buttons = filtered.into_iter().map(|e| {
            let emoji_str = e.to_string();
            div()
                .w_7()
                .h_7()
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .text_base()
                .child(e)
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
        });

        div()
            .w(px(320.0))
            .max_h(px(280.0))
            .p_3()
            .rounded_md()
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.border_accent)
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child("絵文字を選択"),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("✕")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.close_emoji_picker();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
            .child(
                div()
                    .id("emoji-picker-list")
                    .w_full()
                    .max_h(px(200.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .gap_1()
                    .children(buttons),
            )
    }

    fn render_create_channel_modal(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let modal = state.create_channel_modal.as_ref().unwrap();
        let in_flight = modal.in_flight;
        let error = modal.error.clone();
        let is_private = modal.is_private;

        div()
            .w(px(400.0))
            .p_5()
            .rounded_lg()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border_accent)
            .flex()
            .flex_col()
            .gap_3()
            .on_action(cx.listener(|this, _: &crate::input::Tab, window, cx| {
                if this
                    .create_channel_name_input
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    window.focus(&this.create_channel_desc_input.focus_handle(cx));
                } else if this
                    .create_channel_desc_input
                    .focus_handle(cx)
                    .is_focused(window)
                {
                    window.focus(&this.create_channel_name_input.focus_handle(cx));
                }
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child("新規チャンネル作成"),
                    )
                    .child(
                        div()
                            .px_1p5()
                            .rounded_sm()
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("✕")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.close_create_channel_modal();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
            .when_some(error, |d, err| {
                d.child(
                    div()
                        .p_2()
                        .rounded_md()
                        .bg(theme.bg_surface)
                        .text_xs()
                        .text_color(theme.danger)
                        .child(err),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("チャンネル名"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.border)
                            .child(self.create_channel_name_input.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("説明 (任意)"),
                    )
                    .child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .border_1()
                            .border_color(theme.border)
                            .child(self.create_channel_desc_input.clone()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .hover(|s| s.cursor_pointer())
                    .child(
                        div()
                            .w_4()
                            .h_4()
                            .rounded_sm()
                            .border_1()
                            .border_color(if is_private {
                                theme.border_accent
                            } else {
                                theme.border
                            })
                            .bg(if is_private {
                                theme.accent_button
                            } else {
                                theme.bg_surface
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_xs()
                            .text_color(theme.text_on_accent)
                            .child(if is_private { "✓" } else { "" }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_primary)
                            .child("非公開チャンネルにする"),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.state.update(cx, |s, cx| {
                                if let Some(m) = &mut s.create_channel_modal {
                                    m.is_private = !m.is_private;
                                }
                                cx.notify();
                            });
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap_2()
                    .mt_2()
                    .child(
                        div()
                            .px_3()
                            .py_1p5()
                            .rounded_md()
                            .bg(theme.bg_surface)
                            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child("キャンセル")
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.close_create_channel_modal();
                                        cx.notify();
                                    });
                                }),
                            ),
                    )
                    .child(
                        div()
                            .px_4()
                            .py_1p5()
                            .rounded_md()
                            .bg(theme.accent_button)
                            .hover(|s| s.bg(theme.accent_button_hover).cursor_pointer())
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_on_accent)
                            .child(if in_flight { "作成中…" } else { "作成" })
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                    this.state.update(cx, |s, cx| {
                                        s.submit_create_channel();
                                        cx.notify();
                                    });
                                }),
                            ),
                    ),
            )
    }

    fn render_quick_switcher_modal(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let qs = state.quick_switcher.as_ref().unwrap();
        let candidates = state.quick_switcher_candidates();
        let selected_index = qs.selected_index;
        self.quick_switcher_scroll.scroll_to_item(selected_index);

        let items = candidates.iter().enumerate().map(|(i, (ch, label))| {
            let is_selected = i == selected_index;
            let is_dm = ch.is_dm;
            let has_unread = state.unread.contains(&ch.id);
            let icon = if is_dm { "@" } else { "#" };
            let ch_id = ch.id.clone();

            div()
                .w_full()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(if is_selected {
                    theme.bg_selected
                } else {
                    theme.bg_surface
                })
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.text_muted)
                                .child(icon),
                        )
                        .child(
                            div()
                                .text_sm()
                                .font_weight(if is_selected || has_unread {
                                    FontWeight::BOLD
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(theme.text_primary)
                                .child(label.clone()),
                        ),
                )
                .when(has_unread, |d| {
                    d.child(div().w_2().h_2().rounded_full().bg(theme.accent))
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
        });

        div()
            .w(px(480.0))
            .max_h(px(400.0))
            .p_4()
            .rounded_lg()
            .bg(theme.bg_secondary)
            .border_1()
            .border_color(theme.border_accent)
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(theme.bg_surface)
                    .border_1()
                    .border_color(theme.border)
                    .child(div().text_sm().child("🔍"))
                    .child(div().flex_1().child(self.quick_switcher_input.clone())),
            )
            .child(
                div()
                    .id("quick-switcher-list")
                    .w_full()
                    .max_h(px(280.0))
                    .track_scroll(&self.quick_switcher_scroll)
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(items),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.text_dim)
                    .child("↑↓ で移動、Enter で決定、Esc で閉じる"),
            )
    }

    fn render_message_search_bar(&self, cx: &mut Context<Self>) -> Div {
        let state = self.state.read(cx);
        let theme = state.theme();
        let ms = state.message_search.as_ref().unwrap();
        let count = ms.matches.len();
        let count_text = if ms.query.trim().is_empty() {
            String::new()
        } else if count == 0 {
            "0 件".to_string()
        } else {
            format!("{}/{} 件", ms.current_match_index + 1, count)
        };

        div()
            .w_full()
            .px_4()
            .py_2()
            .bg(theme.bg_surface)
            .border_b_1()
            .border_color(theme.border)
            .flex()
            .flex_row()
            .items_center()
            .gap_3()
            .child(div().text_sm().child("🔍"))
            .child(div().flex_1().child(self.message_search_input.clone()))
            .when(!count_text.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .text_xs()
                                .text_color(if count > 0 {
                                    theme.accent
                                } else {
                                    theme.text_muted
                                })
                                .child(count_text),
                        )
                        .when(count > 0, |d| {
                            d.child(
                                div()
                                    .id("btn-search-prev")
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_primary)
                                    .child("↑")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                s.search_select_prev();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                            .child(
                                div()
                                    .id("btn-search-next")
                                    .px_1p5()
                                    .py_0p5()
                                    .rounded_sm()
                                    .bg(theme.bg_secondary)
                                    .border_1()
                                    .border_color(theme.border)
                                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                                    .text_xs()
                                    .text_color(theme.text_primary)
                                    .child("↓")
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                                            this.state.update(cx, |s, cx| {
                                                s.search_select_next();
                                                cx.notify();
                                            });
                                        }),
                                    ),
                            )
                        }),
                )
            })
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child("閉じる (Esc)")
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.message_search_input
                                .update(cx, |input, cx| input.clear(cx));
                            this.state.update(cx, |s, cx| {
                                s.close_message_search();
                                cx.notify();
                            });
                        }),
                    ),
            )
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let login_phase = self.state.read(cx).login_phase.clone();
        if let Some(phase) = login_phase {
            return div()
                .size_full()
                .child(self.render_login_view(&phase, window, cx));
        }

        // ウィンドウのアクティブ状態とタイムライン・スレッドのスクロール位置を AppState に同期し、
        // ウィンドウがアクティブかつ最下部を見ているときだけ最新メッセージを既読扱いにする。
        let window_active = window.is_window_active();
        let (scroll_to_bottom, thread_scroll_to_bottom) = self.state.update(cx, |s, _| {
            s.ensure_channel_cache();
            (
                std::mem::take(&mut s.pending_scroll_to_bottom),
                std::mem::take(&mut s.pending_thread_scroll_to_bottom),
            )
        });

        let thread_root = self
            .state
            .read(cx)
            .thread
            .as_ref()
            .map(|t| t.root_id.clone());

        let pending_scroll_id = self
            .state
            .update(cx, |s, _| s.pending_scroll_to_message_id.take());

        let (timeline_div, timeline_scroll_target) =
            self.render_timeline_view(pending_scroll_id.as_deref(), cx);
        if let Some(target_idx) = timeline_scroll_target {
            self.timeline_scroll.scroll_to_item(target_idx);
            self.state.update(cx, |s, _| {
                s.update_timeline_view_state(window_active, false);
            });
        } else if scroll_to_bottom {
            self.timeline_scroll.scroll_to_bottom();
            self.state
                .update(cx, |s, _| s.update_timeline_view_state(window_active, true));
        } else {
            let max_y = self.timeline_scroll.max_offset().height;
            let cur_y = (-self.timeline_scroll.offset().y).max(px(0.0));
            let at_bottom = max_y <= px(4.0) || (max_y - cur_y) <= px(24.0);
            self.state.update(cx, |s, _| {
                s.update_timeline_view_state(window_active, at_bottom)
            });
        }

        let (thread_div, thread_scroll_target) = if let Some(root_id) = &thread_root {
            let (div, target) = self.render_thread_view(root_id, pending_scroll_id.as_deref(), cx);
            (Some(div), target)
        } else {
            (None, None)
        };
        if let Some(target_idx) = thread_scroll_target {
            self.thread_scroll.scroll_to_item(target_idx);
            self.state
                .update(cx, |s, _| s.update_thread_scroll_state(false));
        } else if thread_scroll_to_bottom {
            self.thread_scroll.scroll_to_bottom();
            self.state
                .update(cx, |s, _| s.update_thread_scroll_state(true));
        } else {
            let max_y = self.thread_scroll.max_offset().height;
            let cur_y = (-self.thread_scroll.offset().y).max(px(0.0));
            let at_bottom = max_y <= px(4.0) || (max_y - cur_y) <= px(24.0);
            self.state
                .update(cx, |s, _| s.update_thread_scroll_state(at_bottom));
        }

        let state_ref = self.state.read(cx);
        let has_create_modal = state_ref.create_channel_modal.is_some();
        let has_quick_switcher = state_ref.quick_switcher.is_some();
        let has_emoji_picker = state_ref.emoji_picker_open;
        let has_message_search = state_ref.message_search.is_some();
        let theme = state_ref.theme();

        let should_focus_composer = self
            .state
            .update(cx, |s, _| std::mem::take(&mut s.pending_focus_composer));
        if should_focus_composer
            && !has_create_modal
            && !has_quick_switcher
            && !has_emoji_picker
            && !has_message_search
        {
            window.focus(&self.composer_input.focus_handle(cx));
        }

        let main_view = div()
            .flex()
            .flex_row()
            .size_full()
            .bg(theme.bg_primary)
            .text_color(theme.text_primary)
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
                let key = ev.keystroke.key.as_str();
                let cmd_or_ctrl = ev.keystroke.modifiers.platform || ev.keystroke.modifiers.control;

                // グローバルショートカット
                if cmd_or_ctrl && key == "k" {
                    this.open_quick_switcher(window, cx);
                    return;
                }
                if cmd_or_ctrl && key == "n" {
                    this.open_create_channel_modal(window, cx);
                    return;
                }
                if cmd_or_ctrl && key == "f" {
                    this.toggle_message_search(window, cx);
                    return;
                }
                if key == "escape" {
                    this.create_channel_name_input
                        .update(cx, |i, cx| i.clear(cx));
                    this.create_channel_desc_input
                        .update(cx, |i, cx| i.clear(cx));
                    this.quick_switcher_input.update(cx, |i, cx| i.clear(cx));
                    this.message_search_input.update(cx, |i, cx| i.clear(cx));
                    this.state.update(cx, |s, cx| {
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
                    window.focus(&this.composer_input.focus_handle(cx));
                }
            }))
            .child(self.render_sidebar_view(cx))
            .child(timeline_div)
            .when_some(thread_div, |d, v| d.child(v));

        if has_create_modal || has_quick_switcher || has_emoji_picker {
            div()
                .size_full()
                .relative()
                .track_focus(&self.focus_handle(cx))
                .child(main_view)
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .occlude()
                        .bg(rgba(0x00000077))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(has_create_modal, |d| {
                            d.child(self.render_create_channel_modal(cx))
                        })
                        .when(has_quick_switcher, |d| {
                            d.child(self.render_quick_switcher_modal(cx))
                        })
                        .when(has_emoji_picker, |d| d.child(self.render_emoji_picker(cx))),
                )
        } else {
            div().size_full().child(main_view)
        }
    }
}

fn render_date_separator(date_label: &str, theme: &theme::Theme) -> Div {
    div()
        .w_full()
        .py_1()
        .flex()
        .flex_row()
        .items_center()
        .gap_3()
        .child(div().flex_1().h(px(1.0)).bg(theme.border))
        .child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(date_label.to_string()),
        )
        .child(div().flex_1().h(px(1.0)).bg(theme.border))
}

/// UTC の naive 日時をローカルタイムゾーンへ変換する。
pub fn local_zoned(at: DateTime) -> Option<Zoned> {
    at.to_zoned(TimeZone::UTC)
        .ok()
        .map(|zoned| zoned.with_time_zone(TimeZone::system()))
}

pub fn local_time_string(at: DateTime) -> String {
    local_zoned(at)
        .map(|z| z.strftime("%H:%M").to_string())
        .unwrap_or_default()
}

fn is_activate_key(key: &str) -> bool {
    matches!(key, "enter" | "space" | " ")
}

pub fn format_file_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

pub fn render_ogp_card(
    card: &CardData,
    theme: &theme::Theme,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let border_color = if let Some(color_hex) = &card.color {
        let clean = color_hex.trim_start_matches('#');
        if let Ok(c) = u32::from_str_radix(clean, 16) {
            theme::hex_rgb(c)
        } else {
            theme.border_accent
        }
    } else {
        theme.border_accent
    };

    let title = card.title.clone();
    let url = card.url.clone();
    let desc = card.description.clone();
    let meta_parts: Vec<&str> = [card.source.as_str(), card.context.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
    let meta_text = meta_parts.join(" · ");
    let badge_text = card.badge.clone();
    let author = card.author.clone();

    let open_target = crate::login::validate_http_url(&card.url)
        .map(|u| u.to_string())
        .unwrap_or_default();

    let fields_div = if let Some(fields) = &card.fields {
        if !fields.is_empty() {
            let field_items = fields.iter().map(|f| {
                let f_title = f.title.clone();
                let f_val = f.value.clone();
                let is_code = f.code.unwrap_or(false);
                let f_url = f
                    .url
                    .as_deref()
                    .and_then(crate::login::validate_http_url)
                    .map(|u| u.to_string());

                let mut val_view = div()
                    .text_xs()
                    .text_color(if f_url.is_some() {
                        theme.text_link
                    } else {
                        theme.text_primary
                    })
                    .child(f_val);

                if is_code {
                    val_view = val_view
                        .px_1()
                        .py_0p5()
                        .rounded_sm()
                        .bg(theme.bg_secondary)
                        .border_1()
                        .border_color(theme.border);
                }

                if let Some(target) = f_url {
                    val_view = val_view.hover(|s| s.cursor_pointer()).on_mouse_up(
                        MouseButton::Left,
                        cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                            cx.stop_propagation();
                            cx.open_url(&target);
                        }),
                    );
                }

                div()
                    .min_w(px(120.0))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_muted)
                            .child(f_title),
                    )
                    .child(val_view)
            });

            Some(
                div()
                    .mt_1p5()
                    .flex()
                    .flex_wrap()
                    .gap_3()
                    .children(field_items),
            )
        } else {
            None
        }
    } else {
        None
    };

    div()
        .mt_1p5()
        .rounded_md()
        .bg(theme.bg_surface)
        .border_1()
        .border_color(theme.border)
        .flex()
        .flex_row()
        .overflow_hidden()
        .when(!open_target.is_empty(), |d| {
            d.hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                        cx.open_url(&open_target);
                    }),
                )
        })
        .child(div().w(px(4.0)).bg(border_color))
        .child(
            div()
                .flex_1()
                .p_2p5()
                .flex()
                .flex_col()
                .gap_1()
                .when(!meta_text.is_empty() || !badge_text.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .when(!meta_text.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_muted)
                                        .child(meta_text),
                                )
                            })
                            .when(!badge_text.is_empty(), |d| {
                                d.child(
                                    div()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(theme.border)
                                        .text_xs()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(theme.text_muted)
                                        .child(badge_text),
                                )
                            }),
                    )
                })
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .text_color(theme.text_link)
                        .child(title),
                )
                .when(!desc.is_empty(), |d| {
                    d.child(div().text_xs().text_color(theme.text_muted).child(desc))
                })
                .when_some(fields_div, |d, f| d.child(f))
                .when(!author.is_empty(), |d| {
                    d.child(
                        div()
                            .mt_1()
                            .text_xs()
                            .text_color(theme.text_dim)
                            .child(author),
                    )
                })
                .when(!url.is_empty(), |d| {
                    d.child(div().text_xs().text_color(theme.text_dim).child(url))
                }),
        )
}

pub fn render_attachment_item(
    att: &zoo2_client_core::electric::rows::Attachment,
    download_url: Option<String>,
    is_fetching: bool,
    needs_refresh: bool,
    theme: &theme::Theme,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let is_img = is_image_content_type(&att.content_type);
    let size_label = format_file_size(att.file_size);
    let file_name = att.file_name.clone();
    let att_id = att.id.clone();

    // 描画時に未解決または期限切れ間近で取得中でなければ自動で取得を開始する
    if needs_refresh && !is_fetching {
        let id = att_id.clone();
        let view = cx.entity().clone();
        cx.defer(move |cx| {
            view.update(cx, |this, cx| {
                this.state.update(cx, |s, _| {
                    s.ensure_attachment_url(id);
                });
            });
        });
    }

    if is_img {
        if let Some(url) = download_url {
            let open_img = url.clone();
            div()
                .mt_1p5()
                .rounded_md()
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg_surface)
                .cursor_pointer()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |_, _: &MouseUpEvent, _, cx| {
                        cx.open_url(&open_img);
                    }),
                )
                .child(
                    gpui::img(url)
                        .max_w(px(380.0))
                        .max_h(px(240.0))
                        .rounded_md(),
                )
        } else {
            let att_id_retry = att_id.clone();
            div()
                .mt_1p5()
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg_surface)
                .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
                .flex()
                .flex_row()
                .items_center()
                .gap_2()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                        let id = att_id_retry.clone();
                        this.state.update(cx, |s, cx| {
                            s.failed_attachment_urls.remove(&id);
                            s.ensure_attachment_url(id);
                            cx.notify();
                        });
                    }),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child("画像を読み込み中… (クリックで再試行)"),
                )
        }
    } else {
        let att_id_open = att_id;
        let sub_label = if is_fetching {
            format!("{size_label} (URL取得中…)")
        } else {
            size_label
        };
        div()
            .mt_1p5()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(theme.bg_surface)
            .border_1()
            .border_color(theme.border)
            .hover(|s| s.bg(theme.bg_hover).cursor_pointer())
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                    let id = att_id_open.clone();
                    let url_opt = this.state.update(cx, |s, cx| {
                        let res = s.open_or_queue_attachment(&id);
                        cx.notify();
                        res
                    });
                    if let Some(url) = url_opt {
                        cx.open_url(&url);
                    }
                }),
            )
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .child(div().text_base().child("📎"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.text_primary)
                            .child(file_name),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.text_muted)
                            .child(sub_label),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_file_size_correctly() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(1048576), "1.0 MB");
        assert_eq!(format_file_size(5242880), "5.0 MB");
    }

    #[test]
    fn parses_ogp_hex_color() {
        let c = theme::hex_rgb(0xff0000);
        assert!((c.r - 1.0).abs() < 0.01);
        assert!(c.g.abs() < 0.01);
        assert!(c.b.abs() < 0.01);
        assert!((c.a - 1.0).abs() < 0.01);
    }
}
