//! 添付ファイルの表示。

use gpui::{
    Context, Div, FocusHandle, FontWeight, KeyDownEvent, MouseButton, MouseUpEvent, ParentElement,
    Styled, div, prelude::*, px,
};

use crate::view::{WorkspaceView, is_activate_key};
use crate::{app::is_image_content_type, theme};

pub fn format_file_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_attachment_item(
    att: &zoo2_client_core::electric::rows::Attachment,
    download_url: Option<String>,
    is_fetching: bool,
    needs_refresh: bool,
    is_failed: bool,
    theme: &theme::Theme,
    attachment_focus_handles: &std::cell::RefCell<std::collections::HashMap<String, FocusHandle>>,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let is_img = is_image_content_type(&att.content_type);
    let size_label = format_file_size(att.file_size);
    let file_name = att.file_name.clone();
    let att_id = att.id.clone();

    // 描画時に未解決または期限切れ間近で取得中でなければ自動で取得を開始する (画像のみ)
    if is_img && needs_refresh && !is_fetching {
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
            let att_id_open = att_id.clone();
            let att_id_key = att_id.clone();
            let img_focus = attachment_focus_handles
                .borrow_mut()
                .entry(format!("{att_id}:img"))
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            div()
                .mt_1p5()
                .track_focus(&img_focus)
                .focus(|s| s.border_color(theme.border_accent))
                .rounded_md()
                .overflow_hidden()
                .border_1()
                .border_color(theme.border)
                .bg(theme.bg_surface)
                .cursor_pointer()
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
                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                    if is_activate_key(&ev.keystroke.key) {
                        cx.stop_propagation();
                        let id = att_id_key.clone();
                        let url_opt = this.state.update(cx, |s, cx| {
                            let res = s.open_or_queue_attachment(&id);
                            cx.notify();
                            res
                        });
                        if let Some(url) = url_opt {
                            cx.open_url(&url);
                        }
                    }
                }))
                .child(
                    gpui::img(url)
                        .max_w(px(380.0))
                        .max_h(px(240.0))
                        .rounded_md(),
                )
        } else {
            let att_id_retry = att_id.clone();
            let att_id_retry_key = att_id.clone();
            let retry_focus = attachment_focus_handles
                .borrow_mut()
                .entry(format!("{att_id}:retry"))
                .or_insert_with(|| cx.focus_handle().tab_stop(true))
                .clone();
            let label = if is_failed {
                "画像の読み込みに失敗しました (クリックで再試行)"
            } else {
                "画像を読み込み中…"
            };
            div()
                .mt_1p5()
                .track_focus(&retry_focus)
                .focus(|s| s.border_color(theme.border_accent).bg(theme.bg_hover))
                .px_3()
                .py_2()
                .rounded_md()
                .border_1()
                .border_color(if is_failed {
                    theme.danger
                } else {
                    theme.border
                })
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
                .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                    if is_activate_key(&ev.keystroke.key) {
                        cx.stop_propagation();
                        let id = att_id_retry_key.clone();
                        this.state.update(cx, |s, cx| {
                            s.failed_attachment_urls.remove(&id);
                            s.ensure_attachment_url(id);
                            cx.notify();
                        });
                    }
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_failed {
                            theme.danger
                        } else {
                            theme.text_muted
                        })
                        .child(label),
                )
        }
    } else {
        let att_id_open = att_id.clone();
        let att_id_key = att_id.clone();
        let att_focus = attachment_focus_handles
            .borrow_mut()
            .entry(att_id)
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        let sub_label = if is_fetching {
            format!("{size_label} (URL取得中…)")
        } else if is_failed {
            format!("{size_label} (取得失敗 - クリックで再試行)")
        } else {
            size_label
        };
        div()
            .mt_1p5()
            .track_focus(&att_focus)
            .focus(|s| s.border_color(theme.border_accent).bg(theme.bg_hover))
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
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                if is_activate_key(&ev.keystroke.key) {
                    cx.stop_propagation();
                    let id = att_id_key.clone();
                    let url_opt = this.state.update(cx, |s, cx| {
                        let res = s.open_or_queue_attachment(&id);
                        cx.notify();
                        res
                    });
                    if let Some(url) = url_opt {
                        cx.open_url(&url);
                    }
                }
            }))
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
}
