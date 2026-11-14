//! GPUI による画面描画 (`WorkspaceView`、`LoginView`、`SidebarView`、`TimelineView`、`ComposerView`、`ThreadView`)。

use gpui::{
    Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement, Render, ScrollHandle,
    Styled, Window, div, prelude::*, px,
};
use jiff::{Zoned, civil::DateTime, tz::TimeZone};

use crate::{
    app::AppState,
    i18n::I18n,
    input::{InputEvent, TextInput},
    ui::Palette,
};

mod auth;
mod chat;
mod composer;
mod dialogs;
mod shell;

pub use chat::{
    CARD_DESC_MAX_CHARS, CARD_DESC_MAX_LINES, format_file_size, is_ogp_description_long,
    render_message_attachments, render_ogp_card, truncate_ogp_description,
};

/// 3 ペインの枠 (`ShellSkeleton`) だけを出すか。自分の情報が届くまで。
/// 同期のエラーが出ていれば通常のシェルを出し、サイドバーの再試行から進められるようにする。
/// ネットワーク障害が静かに再試行されている間も、ログアウトやテーマの切り替えを塞がない。
pub fn shows_shell_skeleton(me_loaded: bool, has_sync_error: bool) -> bool {
    !me_loaded && !has_sync_error
}

pub struct WorkspaceView {
    pub state: Entity<AppState>,
    pub composer_input: Entity<TextInput>,
    pub reaction_input: Entity<TextInput>,
    pub create_channel_name_input: Entity<TextInput>,
    pub create_channel_desc_input: Entity<TextInput>,
    pub quick_switcher_input: Entity<TextInput>,
    pub message_search_input: Entity<TextInput>,
    pub timeline_scroll: ScrollHandle,
    pub sidebar_scroll: ScrollHandle,
    /// サイドバーの「チャンネル」「ダイレクトメッセージ」の開閉 (`CollapsibleSection` の `defaultOpen`)。
    pub sidebar_channels_open: bool,
    pub sidebar_dms_open: bool,
    pub i18n: I18n,
    pub thread_scroll: ScrollHandle,
    pub quick_switcher_scroll: ScrollHandle,
    pub emoji_picker_scroll: ScrollHandle,
    pub create_channel_private_focus: FocusHandle,
    pub theme_toggle_focus: FocusHandle,
    pub expanded_cards: std::collections::HashSet<String>,
    pub card_focus_handles: std::cell::RefCell<std::collections::HashMap<String, FocusHandle>>,
    pub attachment_focus_handles:
        std::cell::RefCell<std::collections::HashMap<String, FocusHandle>>,
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
                        if state.emoji_picker_open {
                            state.emoji_picker_prev_row();
                            cx.notify();
                            true
                        } else if state.mention_autocomplete.is_some() {
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
                        if state.emoji_picker_open {
                            state.emoji_picker_next_row();
                            cx.notify();
                            true
                        } else if state.mention_autocomplete.is_some() {
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
                InputEvent::Left => {
                    let handled = this.state.update(cx, |state, cx| {
                        if state.emoji_picker_open {
                            state.emoji_picker_prev();
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
                InputEvent::Right => {
                    let handled = this.state.update(cx, |state, cx| {
                        if state.emoji_picker_open {
                            state.emoji_picker_next();
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
                    if this.state.read(cx).emoji_picker_open {
                        this.state.update(cx, |state, cx| {
                            state.emoji_picker_next();
                            cx.notify();
                        });
                        return;
                    }
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
                    if this.state.read(cx).emoji_picker_open {
                        this.state.update(cx, |state, cx| {
                            state.insert_selected_emoji(sel);
                            cx.notify();
                        });
                        return;
                    }
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
                        } else if state.emoji_picker_open {
                            state.close_emoji_picker();
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
                | InputEvent::Left
                | InputEvent::Right
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
                        if let Some(m) = &mut state.create_channel_modal
                            && m.channel_name != *text
                        {
                            m.channel_name = text.clone();
                            m.channel_id = Some(uuid::Uuid::new_v4().to_string());
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
                | InputEvent::Left
                | InputEvent::Right
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
                        if let Some(m) = &mut state.create_channel_modal
                            && m.description != *text
                        {
                            m.description = text.clone();
                            m.channel_id = Some(uuid::Uuid::new_v4().to_string());
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
                | InputEvent::Left
                | InputEvent::Right
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
                InputEvent::Tab
                | InputEvent::Left
                | InputEvent::Right
                | InputEvent::CursorMoved(_) => {}
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
                InputEvent::Tab
                | InputEvent::Left
                | InputEvent::Right
                | InputEvent::CursorMoved(_) => {}
            },
        )
        .detach();

        cx.observe(&state, |this, state, cx| {
            let (
                pending_composer,
                pending_open_url,
                is_mention_active,
                is_emoji_picker_open,
                theme,
            ) = state.update(cx, |s, _| {
                (
                    s.pending_composer_text.take(),
                    s.pending_open_url.take(),
                    s.mention_autocomplete.is_some(),
                    s.emoji_picker_open,
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
            if let Some(url) = pending_open_url {
                cx.open_url(&url);
            }
            this.composer_input.update(cx, |input, _| {
                input.intercept_vertical_arrows = is_mention_active || is_emoji_picker_open;
                input.intercept_horizontal_arrows = is_emoji_picker_open;
                input.intercept_tab = is_mention_active || is_emoji_picker_open;
            });
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
            composer_input,
            reaction_input,
            create_channel_name_input,
            create_channel_desc_input,
            quick_switcher_input,
            message_search_input,
            timeline_scroll: ScrollHandle::new(),
            sidebar_scroll: ScrollHandle::new(),
            sidebar_channels_open: true,
            sidebar_dms_open: true,
            i18n: I18n::detect(),
            thread_scroll: ScrollHandle::new(),
            quick_switcher_scroll: ScrollHandle::new(),
            emoji_picker_scroll: ScrollHandle::new(),
            create_channel_private_focus: cx.focus_handle().tab_stop(true),
            theme_toggle_focus: cx.focus_handle().tab_stop(true),
            expanded_cards: std::collections::HashSet::new(),
            card_focus_handles: std::cell::RefCell::new(std::collections::HashMap::new()),
            attachment_focus_handles: std::cell::RefCell::new(std::collections::HashMap::new()),
            focus_handle: cx.focus_handle(),
            login_primary_focus: cx.focus_handle(),
            login_secondary_focus: cx.focus_handle(),
            login_cancel_focus: cx.focus_handle(),
        }
    }
}

impl Render for WorkspaceView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 部品 (`ui/`) は `Palette` を読む。旧テーマのモードに合わせて入れ直す。
        let mode = shell::left_rail::token_mode(self.state.read(cx).theme_mode);
        if cx.try_global::<Palette>().map(|p| p.mode) != Some(mode) {
            cx.set_global(Palette::new(mode));
        }

        let login_phase = self.state.read(cx).login_phase.clone();
        if let Some(phase) = login_phase {
            return div()
                .size_full()
                .child(self.render_login_view(&phase, window, cx));
        }

        // 自分の情報がまだ無いあいだは、3 ペインの枠だけを出す (`ShellSkeleton`)。
        // チャンネル一覧の読み込みは、通常のシェルのサイドバーが行の骨組みで受ける。
        let loading_shell = {
            let s = self.state.read(cx);
            shows_shell_skeleton(s.me.is_some(), s.sync_error().is_some())
        };
        if loading_shell {
            return div()
                .size_full()
                .child(shell::shell_skeleton::render_shell_skeleton(true, cx));
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

        let channels = self.channel_links(cx);
        let (timeline_div, timeline_scroll_target, timeline_placeholder) =
            self.render_timeline_view(pending_scroll_id.as_deref(), channels.clone(), cx);
        if let Some(target_idx) = timeline_scroll_target {
            self.timeline_scroll.scroll_to_item(target_idx);
            self.state.update(cx, |s, _| {
                s.update_timeline_view_state(window_active, false);
            });
        } else if scroll_to_bottom {
            self.timeline_scroll.scroll_to_bottom();
            self.state
                .update(cx, |s, _| s.update_timeline_view_state(window_active, true));
        } else if pending_scroll_id.is_some() {
            // 返信へ移る途中。タイムラインの古いスクロール位置から最下部と判定して既読にしない。
            self.state.update(cx, |s, _| {
                s.update_timeline_view_state(window_active, false);
            });
        } else {
            // placeholder 表示中は ScrollArea がなく、スクロールの値は前のチャンネルのまま。最下部として扱う。
            let at_bottom = timeline_placeholder || {
                let max_y = self.timeline_scroll.max_offset().height;
                let cur_y = (-self.timeline_scroll.offset().y).max(px(0.0));
                max_y <= px(4.0) || (max_y - cur_y) <= px(24.0)
            };
            self.state.update(cx, |s, _| {
                s.update_timeline_view_state(window_active, at_bottom)
            });
        }

        let (thread_div, thread_scroll_target) = if let Some(root_id) = &thread_root {
            let (div, target) = self.render_thread_view(
                root_id,
                pending_scroll_id.as_deref(),
                channels,
                window,
                cx,
            );
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

        let main_view = self.render_app_shell(timeline_div, thread_div, cx);

        if has_create_modal || has_quick_switcher || has_emoji_picker {
            div()
                .size_full()
                .relative()
                .track_focus(&self.focus_handle(cx))
                .child(main_view)
                .child(self.render_modal_overlay(
                    has_create_modal,
                    has_quick_switcher,
                    has_emoji_picker,
                    window,
                    cx,
                ))
        } else {
            div().size_full().child(main_view)
        }
    }
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

#[cfg(test)]
mod shell_skeleton_tests {
    use super::shows_shell_skeleton;

    #[test]
    fn skeleton_only_until_me_is_loaded() {
        assert!(shows_shell_skeleton(false, false));
        assert!(!shows_shell_skeleton(true, false));
        assert!(!shows_shell_skeleton(true, true));
    }

    #[test]
    fn sync_error_opens_the_shell_for_retry() {
        assert!(!shows_shell_skeleton(false, true));
    }
}
