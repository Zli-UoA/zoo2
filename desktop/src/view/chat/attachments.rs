//! 添付ファイルの表示 (frontend の `MessageAttachments.tsx`)。
//!
//! 写していないもの: センシティブな画像の覆い (`SensitiveMedia`)、blurhash のプレースホルダー
//! (デスクトップの `Attachment` に幅・高さ・blurhash がない)、アプリ内のプレビューダイアログ
//! (今のままブラウザで開く。ダイアログは PR 5)。

use std::{cell::RefCell, collections::HashMap};

use gpui::{
    AnyElement, Context, Div, ElementId, FocusHandle, KeyDownEvent, MouseButton, MouseUpEvent,
    ObjectFit, ParentElement, SharedString, Styled, StyledImage as _, div, prelude::*, px,
};
use zoo2_client_core::electric::rows::Attachment;

use crate::app::is_image_content_type;
use crate::fonts;
use crate::i18n::I18n;
use crate::icons::Icon;
use crate::tokens::radius;
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::{TextSpec, alpha, h_flex, icon_colored, palette, v_flex};
use crate::view::{WorkspaceView, is_activate_key};

/// 添付 1 件と、その URL の取得状態。
pub struct AttachmentItem {
    pub attachment: Attachment,
    pub download_url: Option<String>,
    pub is_fetching: bool,
    pub needs_refresh: bool,
    pub is_failed: bool,
}

/// `formatAttachmentSize` (`Intl.NumberFormat`、バイトは整数、それ以外は小数 1 桁まで)。
pub fn format_file_size(bytes: i64) -> String {
    if bytes < 0 {
        return "—".to_string();
    }
    let units = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < units.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    let formatted = if unit == 0 {
        format!("{}", size as i64)
    } else {
        let one = format!("{size:.1}");
        one.strip_suffix(".0").unwrap_or(&one).to_string()
    };
    format!("{formatted} {}", units[unit])
}

/// 画像の最大幅 (`w-64`)。
const IMAGE_WIDTH: f32 = 256.0;
/// 複数の画像を並べる枠の幅 (`max-w-sm`)。
const GRID_WIDTH: f32 = 384.0;
/// 添付の枠の最大幅 (`max-w-xl`)。
const MAX_WIDTH: f32 = 576.0;

fn focus_handle(
    handles: &RefCell<HashMap<String, FocusHandle>>,
    key: String,
    cx: &mut Context<WorkspaceView>,
) -> FocusHandle {
    handles
        .borrow_mut()
        .entry(key)
        .or_insert_with(|| cx.focus_handle().tab_stop(true))
        .clone()
}

/// 添付を開く (URL があればブラウザへ、無ければ取得して開く)。
fn open_attachment(this: &mut WorkspaceView, id: &str, cx: &mut Context<WorkspaceView>) {
    let url = this.state.update(cx, |s, cx| {
        let res = s.open_or_queue_attachment(id);
        cx.notify();
        res
    });
    if let Some(url) = url {
        cx.open_url(&url);
    }
}

/// 取得に失敗したものをもう一度取りにいく。
fn retry_attachment(this: &mut WorkspaceView, id: &str, cx: &mut Context<WorkspaceView>) {
    this.state.update(cx, |s, cx| {
        s.failed_attachment_urls.remove(id);
        s.ensure_attachment_url(id.to_string());
        cx.notify();
    });
}

/// メッセージの添付一覧 (`MessageAttachments`)。画像を先に、ファイルをその下に並べる。
pub fn render_message_attachments(
    items: Vec<AttachmentItem>,
    handles: &RefCell<HashMap<String, FocusHandle>>,
    i18n: I18n,
    cx: &mut Context<WorkspaceView>,
) -> Option<Div> {
    if items.is_empty() {
        return None;
    }
    let (images, files): (Vec<_>, Vec<_>) = items
        .into_iter()
        .partition(|i| is_image_content_type(&i.attachment.content_type));
    let multiple = images.len() > 1;
    let image_views: Vec<AnyElement> = images
        .into_iter()
        .map(|item| render_image(item, multiple, handles, i18n, cx))
        .collect();
    let file_views: Vec<AnyElement> = files
        .into_iter()
        .map(|item| render_file_card(item, handles, i18n, cx))
        .collect();
    // `mt-2 flex max-w-xl flex-col gap-2`
    Some(
        v_flex()
            .mt(px(8.0))
            .max_w(px(MAX_WIDTH))
            .items_start()
            .gap(px(8.0))
            .when(!image_views.is_empty(), |d| {
                // 1 枚は `w-fit max-w-full`、複数は `grid w-full max-w-sm grid-cols-2 gap-2`
                d.child(
                    h_flex()
                        .items_start()
                        .flex_wrap()
                        .gap(px(8.0))
                        .when(multiple, |d| d.w(px(GRID_WIDTH)).max_w_full())
                        .children(image_views),
                )
            })
            .when(!file_views.is_empty(), |d| {
                // `flex flex-col gap-1.5`
                d.child(v_flex().w_full().gap(px(6.0)).children(file_views))
            }),
    )
}

fn render_image(
    item: AttachmentItem,
    multiple: bool,
    handles: &RefCell<HashMap<String, FocusHandle>>,
    i18n: I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let att = item.attachment.clone();
    let id = att.id.clone();
    let t = palette(cx).t;

    // 描画時に未解決または期限切れ間近で取得中でなければ自動で取得を開始する。
    if item.needs_refresh && !item.is_fetching {
        let id = id.clone();
        let view = cx.entity().clone();
        cx.defer(move |cx| {
            view.update(cx, |this, cx| {
                this.state.update(cx, |s, _| {
                    s.ensure_attachment_url(id);
                });
            });
        });
    }

    let Some(url) = item.download_url else {
        if item.is_failed {
            // 取得に失敗した画像は、ファイルのカードにして再試行を出す。
            return render_file_card(item_with(att, None, false, true), handles, i18n, cx);
        }
        // 読み込み中。`aria-busy` の枠 (blurhash が無いので `bg-muted` だけ)。
        // クリックかキーで、取得をもう一度頼める。
        let side = if multiple { 188.0 } else { IMAGE_WIDTH };
        let focus = focus_handle(handles, format!("{id}:img"), cx);
        let id_click = id.clone();
        let id_key = id.clone();
        return div()
            .id(ElementId::Name(SharedString::from(format!("att-img-{id}"))))
            .track_focus(&focus)
            .focus(move |s| s.border_color(alpha(t.ring, 0.5)))
            .w(px(side))
            .h(px(if multiple { 188.0 } else { side * 3.0 / 4.0 }))
            .overflow_hidden()
            .rounded(radius::LG)
            .border_1()
            .border_color(t.border)
            .bg(t.muted)
            .cursor_pointer()
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(move |this, _: &MouseUpEvent, _, cx| {
                    retry_attachment(this, &id_click, cx)
                }),
            )
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
                if is_activate_key(&ev.keystroke.key) {
                    cx.stop_propagation();
                    retry_attachment(this, &id_key, cx);
                }
            }))
            .into_any_element();
    };

    let focus = focus_handle(handles, format!("{id}:img"), cx);
    let id_click = id.clone();
    let id_key = id.clone();
    // `block size-full cursor-zoom-in overflow-hidden rounded-lg border bg-muted` + ring
    let frame = div()
        .id(ElementId::Name(SharedString::from(format!("att-img-{id}"))))
        .track_focus(&focus)
        .focus(move |s| s.border_color(alpha(t.ring, 0.5)))
        .overflow_hidden()
        .rounded(radius::LG)
        .border_1()
        .border_color(t.border)
        .bg(t.muted)
        .cursor_pointer()
        .hover(|s| s.opacity(0.9))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| open_attachment(this, &id_click, cx)),
        )
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
            if is_activate_key(&ev.keystroke.key) {
                cx.stop_propagation();
                open_attachment(this, &id_key, cx);
            }
        }));
    if multiple {
        // `aspect-square w-full` + `object-cover`
        frame
            .size(px(188.0))
            .child(gpui::img(url).size_full().object_fit(ObjectFit::Cover))
            .into_any_element()
    } else {
        // `w-64 max-w-full`、高さは `min 80 / max 384` (縦横比は画像から)。
        frame
            .w(px(IMAGE_WIDTH))
            .max_w_full()
            .child(
                gpui::img(url)
                    .w_full()
                    .min_h(px(80.0))
                    .max_h(px(384.0))
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element()
    }
}

fn item_with(
    attachment: Attachment,
    download_url: Option<String>,
    is_fetching: bool,
    is_failed: bool,
) -> AttachmentItem {
    AttachmentItem {
        attachment,
        download_url,
        is_fetching,
        needs_refresh: false,
        is_failed,
    }
}

/// ファイルのカード (`AttachmentFileCard`)。
fn render_file_card(
    item: AttachmentItem,
    handles: &RefCell<HashMap<String, FocusHandle>>,
    i18n: I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let p = palette(cx);
    let t = p.t;
    let sm = TextSpec::SM;
    let xs = TextSpec::XS;
    let att = &item.attachment;
    let id = att.id.clone();
    let loading = item.is_fetching;
    let failed = item.is_failed;
    let status = if failed {
        Some(i18n.t("attachments.loadError"))
    } else if loading {
        Some(i18n.t("attachments.loading"))
    } else {
        None
    };
    let size_label = match status {
        Some(label) => format!("{} · {label}", format_file_size(att.file_size)),
        None => format_file_size(att.file_size),
    };

    let focus = focus_handle(handles, id.clone(), cx);
    let id_click = id.clone();
    let id_key = id.clone();
    let id_retry = id.clone();
    let id_download = id.clone();

    // `flex items-center gap-2 rounded-lg border bg-muted/30 px-2.5 py-2`
    h_flex()
        .id(ElementId::Name(SharedString::from(format!(
            "att-file-{id}"
        ))))
        .track_focus(&focus)
        .focus(move |s| s.border_color(alpha(t.ring, 0.5)))
        .w_full()
        .gap(px(8.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(radius::LG)
        .border_1()
        .border_color(t.border)
        .bg(alpha(t.muted, 0.3))
        .cursor_pointer()
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseUpEvent, _, cx| open_attachment(this, &id_click, cx)),
        )
        .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, _, cx| {
            if is_activate_key(&ev.keystroke.key) {
                cx.stop_propagation();
                open_attachment(this, &id_key, cx);
            }
        }))
        .child(icon_colored(Icon::Paperclip, 16.0, t.muted_foreground))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .truncate()
                        .text_size(sm.size())
                        .line_height(sm.line())
                        .font(fonts::sans(500.0))
                        .child(att.file_name.clone()),
                )
                .child(
                    div()
                        .truncate()
                        .text_size(xs.size())
                        .line_height(xs.line())
                        .text_color(t.muted_foreground)
                        .child(size_label),
                ),
        )
        .when(loading && !failed, |d| {
            d.child(icon_colored(Icon::LoaderCircle, 16.0, t.muted_foreground))
        })
        .when(failed, |d| {
            d.child(
                div()
                    .id(ElementId::Name(SharedString::from(format!(
                        "att-retry-{id_retry}"
                    ))))
                    .text_size(xs.size())
                    .line_height(xs.line())
                    .font(fonts::sans(500.0))
                    .text_color(t.link)
                    .hover(|s| s.underline())
                    .cursor_pointer()
                    .child(i18n.t("attachments.retry"))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener({
                            let id = id_retry.clone();
                            move |this, _: &MouseUpEvent, _, cx| {
                                cx.stop_propagation();
                                retry_attachment(this, &id, cx);
                            }
                        }),
                    ),
            )
        })
        .child(
            Button::new(ElementId::Name(SharedString::from(format!(
                "att-download-{id_download}"
            ))))
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::IconSm)
            .icon_start(Icon::Download)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                open_attachment(this, &id_download, cx);
            })),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_file_size_like_intl() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1024), "1 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(1048576), "1 MB");
        assert_eq!(format_file_size(5242880), "5 MB");
        assert_eq!(format_file_size(1024 * 1024 * 1024 * 3), "3 GB");
        // 小数 1 桁に丸める (`maximumFractionDigits: 1`)。
        assert_eq!(format_file_size(1100), "1.1 KB");
        assert_eq!(format_file_size(-1), "—");
    }
}
