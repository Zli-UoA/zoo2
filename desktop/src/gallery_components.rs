//! UI 部品の確認画面 (`zoo2-desktop gallery components [ページ]`)。
//! Web 版の `*.stories.tsx` と同じ状態を並べ、見比べられるようにする。
//! 1 は Button など、2 は入力欄、3 はダイアログとメニュー、4 は Avatar などの表示部品、5 は通知と画像。

use std::sync::Arc;

use gpui::{
    AnyElement, Context, Div, ImageSource, IntoElement, RenderImage, Render, SharedString, Window,
    div, prelude::*, px,
};
use image::{Frame, RgbaImage};

use crate::fonts;
use crate::i18n::{I18n, Locale};
use crate::icons::Icon;
use crate::tokens::{ThemeMode, font_size, radius};
use crate::ui::Palette;
use crate::ui::alert_dialog::{AlertDialog, AlertDialogSize};
use crate::ui::avatar::{Avatar, AvatarGroup, AvatarSize};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::button_group::{ButtonGroup, ButtonGroupItem};
use crate::ui::collapsible_section::CollapsibleSection;
use crate::ui::confirm_action_dialog::ConfirmActionDialog;
use crate::ui::control_panel::{PanelTab, SettingRow, SettingsSection};
use crate::ui::dialog::{Dialog, overlay};
use crate::ui::dropdown_menu::{DropdownMenuContent, Indicator, MenuItem};
use crate::ui::external_link::ExternalLink;
use crate::ui::image_preview::ImagePreview;
use crate::ui::input::Input;
use crate::ui::pane_header::{PaneHeader, ScreenHeader};
use crate::ui::scroll_area::{ScrollArea, Thumb};
use crate::ui::select::{Select, SelectPopup};
use crate::ui::separator::Separator;
use crate::ui::skeleton::Skeleton;
use crate::ui::sonner::{Toast, ToastKind, Toaster};
use crate::ui::spinner::Spinner;
use crate::ui::textarea::Textarea;
use crate::ui::tooltip::{Side, Tooltip};

/// ページ数。
pub const PAGES: usize = 5;

pub struct ComponentsGallery {
    page: usize,
    p: Palette,
    i18n: I18n,
}

impl ComponentsGallery {
    pub fn new(page: usize, mode: ThemeMode) -> Self {
        Self {
            page: page.clamp(1, PAGES),
            p: Palette::new(mode),
            // Web 版のストーリーは日本語の文言を使う。
            i18n: I18n::new(Locale::Ja),
        }
    }
}

/// 名前を付けた枠。
fn section(p: &Palette, title: impl Into<SharedString>) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(8.0))
        .p(px(12.0))
        .border_1()
        .border_color(p.t.border)
        .rounded(radius::LG)
        .child(caption(p, title))
}

fn caption(p: &Palette, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(font_size::XS)
        .line_height(px(16.0))
        .font(fonts::sans(600.0))
        .text_color(p.t.muted_foreground)
        .child(text.into())
}

fn row() -> Div {
    div().flex().flex_wrap().items_center().gap(px(8.0))
}

/// 小さな見出し付きの 1 行。
fn labeled(p: &Palette, label: &str, content: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(
            div()
                .text_size(font_size::PX_10)
                .text_color(p.t.muted_foreground)
                .child(label.to_string()),
        )
        .child(content)
}

/// ダイアログなどを出す舞台 (`relative`)。ストーリーの `min-h-96 bg-background p-6` に当たる。
fn stage(p: &Palette, title: &str, width: f32, height: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .w(px(width))
        .child(caption(p, title.to_string()))
        .child(
            div()
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .w(px(width))
                .h(px(height))
                .rounded(radius::LG)
                .border_1()
                .border_color(p.t.border)
                .bg(p.t.background),
        )
}

/// `stage()` の枠の中身を差し替える。
fn staged(p: &Palette, title: &str, width: f32, height: f32, inner: impl IntoElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .w(px(width))
        .child(caption(p, title.to_string()))
        .child(
            div()
                .relative()
                .overflow_hidden()
                .flex()
                .items_center()
                .justify_center()
                .w(px(width))
                .h(px(height))
                .rounded(radius::LG)
                .border_1()
                .border_color(p.t.border)
                .bg(p.t.background)
                .child(inner),
        )
}

/// 撮影用の画像 (斜めのグラデーション)。実際の画像の代わり。
fn sample_image(w: u32, h: u32) -> ImageSource {
    let mut buf = RgbaImage::new(w, h);
    for (x, y, px) in buf.enumerate_pixels_mut() {
        let u = x as f32 / w as f32;
        let v = y as f32 / h as f32;
        // RGBA で作ってから BGRA に並べ替える。
        *px = image::Rgba([
            (60.0 + 120.0 * u) as u8,
            (140.0 - 60.0 * v) as u8,
            (200.0 - 120.0 * (u + v) / 2.0) as u8,
            255,
        ]);
        px.0.swap(0, 2);
        px.0[3] = 255;
    }
    ImageSource::Render(Arc::new(RenderImage::new(vec![Frame::new(buf)])))
}

// ---- ページ 1: 入力と基本部品 ----

fn buttons(p: &Palette) -> AnyElement {
    let make = |prefix: &str, f: &dyn Fn(Button) -> Button| {
        let mut r = row();
        for v in ButtonVariant::ALL {
            let b = Button::new(SharedString::from(format!("{prefix}-{}", v.name())), p)
                .variant(v)
                .label(v.name());
            r = r.child(f(b));
        }
        r
    };
    let mut sizes = row().items_end();
    for s in ButtonSize::ALL {
        let b = Button::new(SharedString::from(format!("size-{}", s.name())), p).size(s);
        sizes = sizes.child(match s {
            ButtonSize::Icon | ButtonSize::IconXs | ButtonSize::IconSm | ButtonSize::IconLg => {
                b.icon(Icon::Plus)
            }
            _ => b.label(s.name()),
        });
    }
    let icons = row()
        .child(
            Button::new("ic-start", p)
                .variant(ButtonVariant::Outline)
                .icon_start(Icon::Download)
                .label("ダウンロード"),
        )
        .child(
            Button::new("ic-end", p)
                .variant(ButtonVariant::Outline)
                .label("チャンネル操作")
                .icon_end(Icon::ChevronDown),
        )
        .child(
            Button::new("ic-xs", p)
                .size(ButtonSize::Xs)
                .icon_start(Icon::Plus)
                .label("xs"),
        )
        .child(
            Button::new("ic-sm", p)
                .size(ButtonSize::Sm)
                .icon_start(Icon::Plus)
                .label("sm"),
        )
        .child(Button::new("expanded", p).variant(ButtonVariant::Outline).expanded(true).label("expanded"))
        .child(Button::new("expanded-g", p).variant(ButtonVariant::Ghost).expanded(true).label("expanded"));
    let focus = make("focus", &|b| b.focused(true));
    let invalid = make("invalid", &|b| b.invalid(true));
    section(p, "Button  (variants / hover / disabled / focus-visible / aria-invalid / sizes)")
        .child(labeled(p, "variants", make("v", &|b| b)))
        .child(labeled(p, "hover", make("h", &|b| b.hovered(true))))
        .child(labeled(p, "disabled", make("d", &|b| b.disabled(true))))
        .child(labeled(p, "focus-visible", focus))
        .child(labeled(p, "aria-invalid", invalid))
        .child(labeled(p, "sizes xs sm default lg icon icon-xs icon-sm icon-lg", sizes))
        .child(labeled(p, "icons / aria-expanded", icons))
        .into_any_element()
}

fn button_group(p: &Palette) -> AnyElement {
    section(p, "ButtonGroup")
        .child(
            ButtonGroup::new(p)
                .child(ButtonGroupItem::new("bg-1", p, Icon::Hash).pressed(true))
                .child(ButtonGroupItem::new("bg-2", p, Icon::Search))
                .child(ButtonGroupItem::new("bg-3", p, Icon::Settings).hovered(true)),
        )
        .into_any_element()
}

fn inputs(p: &Palette) -> AnyElement {
    let w = px(209.0);
    let grid = div().flex().flex_wrap().gap(px(12.0));
    let grid = grid.child(labeled(p, "default", Input::new(p).placeholder("general").w(w)))
        .child(labeled(p, "value", Input::new(p).value("general").w(w)))
        .child(labeled(p, "focus-visible", Input::new(p).value("general").focused(true).w(w)))
        .child(labeled(p, "disabled", Input::new(p).value("general").disabled(true).w(w)))
        .child(labeled(
            p,
            "aria-invalid (見た目は変わらない)",
            Input::new(p).value("使えない名前").w(w),
        ))
        .child(labeled(
            p,
            "long value",
            Input::new(p).value("very-long-channel-name-".repeat(8)).w(w),
        ));
    section(p, "Input  (w-[209px])")
        .child(grid)
        .into_any_element()
}

fn textareas(p: &Palette) -> AnyElement {
    let long = (1..=6)
        .map(|i| {
            format!("{i} 行目: リリース手順の確認項目です。担当者と期限を確認してから次へ進んでください。")
        })
        .collect::<Vec<_>>()
        .join("\n");
    section(p, "Textarea  (max-w-xl)").w(px(576.0))
        .child(labeled(p, "default", Textarea::new(p).placeholder("メッセージ")).w(px(552.0)))
        .child(labeled(p, "value", Textarea::new(p).value("今日のリリースは 18:00 です。")).w(px(552.0)))
        .child(labeled(p, "focus-visible", Textarea::new(p).value("今日のリリースは 18:00 です。").focused(true)).w(px(552.0)))
        .child(labeled(p, "disabled", Textarea::new(p).value("編集できません").disabled(true)).w(px(552.0)))
        .child(labeled(p, "aria-invalid", Textarea::new(p).value("入力内容を確認してください").invalid(true)).w(px(552.0)))
        .child(labeled(p, "long text", Textarea::new(p).value(long)).w(px(552.0)))
        .into_any_element()
}

fn selects(p: &Palette) -> AnyElement {
    let opts = |popup: SelectPopup| {
        popup
            .option("ライト", false)
            .option("ダーク", false)
            .option("システム", true)
    };
    section(p, "Select  (w-40)")
        .child(
            row()
                .items_start()
                .gap(px(16.0))
                .child(labeled(p, "default", Select::new(p, "システム")))
                .child(labeled(p, "disabled", Select::new(p, "システム").disabled(true)))
                .child(labeled(p, "focus-visible", Select::new(p, "システム").focused(true)))
                .child(labeled(
                    p,
                    "long label",
                    Select::new(p, "長い選択肢の名前でも幅を広げません"),
                ))
                .child(labeled(
                    p,
                    "open (popup)",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(Select::new(p, "システム").focused(true))
                        .child(opts(SelectPopup::new(p).highlighted(1))),
                )),
        )
        .into_any_element()
}

fn misc_basic(p: &Palette) -> AnyElement {
    section(p, "Separator / Skeleton / Spinner")
        .child(labeled(
            p,
            "separator (horizontal / vertical)",
            row()
                .items_center()
                .h(px(24.0))
                .gap(px(12.0))
                .child(div().w(px(160.0)).child(Separator::new(p)))
                .child(Separator::new(p).vertical()),
        ))
        .child(labeled(
            p,
            "skeleton text (h-4 w-64) / avatar (size-10 rounded-full)",
            row()
                .gap(px(16.0))
                .child(Skeleton::new("sk-text", p).animate(false).h(px(16.0)).w(px(256.0)))
                .child(
                    Skeleton::new("sk-avatar", p)
                        .animate(false)
                        .size(px(40.0))
                        .rounded_full(),
                ),
        ))
        .child(labeled(
            p,
            "spinner (size-7, 回転)",
            Spinner::new("spinner", p),
        ))
        .into_any_element()
}

fn page_buttons(p: &Palette) -> Div {
    div()
        .flex()
        .gap(px(12.0))
        .child(div().flex().flex_col().gap(px(12.0)).w(px(700.0)).child(buttons(p)).child(button_group(p)))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .gap(px(12.0))
                .child(misc_basic(p))
                .child(links_and_headers(p)),
        )
}

fn page_fields(p: &Palette) -> Div {
    div()
        .flex()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .w(px(700.0))
                .child(inputs(p))
                .child(selects(p)),
        )
        .child(div().flex().flex_col().flex_1().gap(px(12.0)).child(textareas(p)))
}

// ---- ページ 3: ダイアログ、メニュー ----

fn dialog_stage(p: &Palette) -> Div {
    let dialog = Dialog::new(p, "チャンネルを削除しますか？")
        .description("この操作は取り消せません。チャンネルの履歴もすべて削除されます。")
        .footer(Button::new("dlg-cancel", p).variant(ButtonVariant::Outline).label("キャンセル"))
        .footer(Button::new("dlg-ok", p).variant(ButtonVariant::Destructive).label("削除する"));
    staged(
        p,
        "Dialog (open)",
        456.0,
        384.0,
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(Button::new("dlg-trigger", p).variant(ButtonVariant::Outline).label("ダイアログを開く"))
            .child(overlay(dialog)),
    )
}

fn alert_stage(p: &Palette) -> Div {
    let dialog = AlertDialog::new(p, "メッセージを削除しますか？")
        .description("この操作は取り消せません。削除したメッセージはチャンネルのメンバーからも見えなくなります。")
        .footer(Button::new("al-cancel", p).variant(ButtonVariant::Outline).label("キャンセル"))
        .footer(Button::new("al-ok", p).variant(ButtonVariant::Destructive).label("削除する"));
    staged(
        p,
        "AlertDialog (open)",
        456.0,
        384.0,
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(Button::new("al-trigger", p).variant(ButtonVariant::Outline).label("メッセージを削除"))
            .child(overlay(dialog)),
    )
}

fn alert_variants_stage(p: &Palette) -> Div {
    let dialog = AlertDialog::new(p, "ログアウトしますか？")
        .size(AlertDialogSize::Sm)
        .media(Icon::LogOut)
        .description("この端末の保存済みトークンを削除します。")
        .footer(Button::new("sm-cancel", p).variant(ButtonVariant::Outline).label("キャンセル"))
        .footer(Button::new("sm-ok", p).label("ログアウト"));
    staged(p, "AlertDialog size=sm + media", 456.0, 384.0, overlay(dialog))
}

fn confirm_stage(p: &Palette, i18n: &I18n, title: &str, pending: bool, error: Option<&str>) -> Div {
    let mut dialog = ConfirmActionDialog::new(
        p,
        "Appを削除",
        "このAppと関連する設定を削除します。この操作は取り消せません。",
        "削除",
        i18n.t("common.cancel"),
        i18n.t("moderation.executing"),
    )
    .pending(pending);
    if let Some(e) = error {
        dialog = dialog.error(e.to_string());
    }
    staged(p, title, 456.0, 384.0, overlay(dialog))
}

fn dropdown_stage(p: &Palette) -> Div {
    // `flex min-h-72 items-start justify-center p-12`: トリガーは上から 48px、メニューは 4px 下。
    let menu = DropdownMenuContent::new(p)
        .item(MenuItem::new("dd-1", p, "名前を変更").icon(Icon::Pencil).highlighted(true))
        .item(MenuItem::new("dd-2", p, "アーカイブ").icon(Icon::Archive))
        .item(MenuItem::new("dd-3", p, "チャンネル設定").icon(Icon::ShieldAlert))
        .separator()
        .item(MenuItem::new("dd-4", p, "削除").icon(Icon::Trash2).destructive());
    let trigger = div()
        .relative()
        .child(
            Button::new("dd-trigger", p)
                .variant(ButtonVariant::Outline)
                .label("チャンネル操作")
                .expanded(true),
        )
        .child(
            div()
                .absolute()
                .top(px(32.0 + 4.0))
                .left_0()
                .w_full()
                .flex()
                .justify_center()
                .child(menu),
        );
    staged(
        p,
        "DropdownMenu (open, align=center)",
        456.0,
        384.0,
        div().size_full().flex().justify_center().pt(px(48.0)).child(trigger),
    )
}

fn menu_kinds_stage(p: &Palette) -> Div {
    let menu = DropdownMenuContent::new(p)
        .label("表示")
        .item(MenuItem::new("mk-1", p, "未読のみ").indicator(Indicator::Checkbox(true)))
        .item(MenuItem::new("mk-2", p, "ミュート済み").indicator(Indicator::Checkbox(false)))
        .separator()
        .item(MenuItem::new("mk-3", p, "新しい順").indicator(Indicator::Radio(true)))
        .item(MenuItem::new("mk-4", p, "古い順").indicator(Indicator::Radio(false)))
        .separator()
        .item(MenuItem::new("mk-5", p, "コピー").icon(Icon::Copy).shortcut("⌘C"))
        .item(MenuItem::new("mk-6", p, "移動").icon(Icon::ArrowUpRight).indicator(Indicator::Submenu))
        .item(MenuItem::new("mk-7", p, "無効な項目").disabled(true));
    staged(p, "DropdownMenu (label / checkbox / radio / shortcut / sub / disabled)", 456.0, 384.0, menu)
}

fn page_dialogs(p: &Palette, i18n: &I18n) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .child(
            div()
                .flex()
                .gap(px(12.0))
                .child(dialog_stage(p))
                .child(alert_stage(p))
                .child(confirm_stage(p, i18n, "ConfirmActionDialog (Default)", false, None)),
        )
        .child(
            div()
                .flex()
                .gap(px(12.0))
                .child(confirm_stage(p, i18n, "ConfirmActionDialog (Pending)", true, None))
                .child(confirm_stage(
                    p,
                    i18n,
                    "ConfirmActionDialog (Failed)",
                    false,
                    Some("変更を保存できませんでした。もう一度お試しください。"),
                ))
                .child(dropdown_stage(p)),
        )
}

// ---- ページ 3: そのほか ----

fn avatars(p: &Palette) -> AnyElement {
    section(p, "Avatar")
        .child(labeled(
            p,
            "default / sizes (sm 24, default 32, lg 40)",
            row()
                .items_end()
                .gap(px(12.0))
                .child(Avatar::new(p).initials("AL"))
                .child(Avatar::new(p).size(AvatarSize::Sm).initials("S"))
                .child(Avatar::new(p).size(AvatarSize::Default).initials("M"))
                .child(Avatar::new(p).size(AvatarSize::Lg).initials("L")),
        ))
        .child(labeled(
            p,
            "badge / image / image failed (fallback)",
            row()
                .gap(px(12.0))
                .child(Avatar::new(p).initials("BO").badge(true))
                .child(Avatar::new(p).size(AvatarSize::Lg).initials("AL").badge(true))
                .child(Avatar::new(p).initials("AL").image(sample_image(64, 64)))
                .child(Avatar::new(p).size(AvatarSize::Lg).initials("AL").image(sample_image(64, 64)))
                .child(Avatar::new(p).initials("AL").image("missing-avatar.png")),
        ))
        .child(labeled(
            p,
            "group",
            AvatarGroup::new()
                .avatar(Avatar::new(p).size(AvatarSize::Sm).initials("AL"))
                .avatar(Avatar::new(p).size(AvatarSize::Sm).initials("BO"))
                .avatar(Avatar::new(p).size(AvatarSize::Sm).initials("CA"))
                .count(p, AvatarSize::Sm, "+5"),
        ))
        .into_any_element()
}

fn tooltips(p: &Palette) -> Div {
    staged(
        p,
        "Tooltip (open)",
        300.0,
        140.0,
        div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.0))
            .child(Tooltip::new(p, "メッセージを編集できます").side(Side::Top))
            .child(
                Button::new("tt-trigger", p)
                    .variant(ButtonVariant::Outline)
                    .label("メッセージを編集")
                    .hovered(true),
            ),
    )
}

fn sonner_stage(p: &Palette) -> Div {
    let toaster = Toaster::new(p).toast(Toast::new(ToastKind::Success, "変更を保存しました"));
    staged(
        p,
        "Toaster (success)",
        420.0,
        140.0,
        div()
            .size_full()
            .p(px(16.0))
            .child(div().flex().child(Button::new("toast-trigger", p).label("保存")))
            .child(toaster),
    )
}

fn sonner_stack_stage(p: &Palette) -> Div {
    let toaster = Toaster::new(p)
        .toast(Toast::new(ToastKind::Error, "送信に失敗しました").description("ネットワークを確認してください"))
        .toast(Toast::new(ToastKind::Info, "新しいバージョンがあります"))
        .toast(Toast::new(ToastKind::Normal, "コピーしました"));
    staged(p, "Toaster (stack, collapsed)", 420.0, 200.0, div().size_full().child(toaster))
}

fn collapsibles(p: &Palette) -> Div {
    let body = || div().px(px(16.0)).py(px(8.0)).child("general / design");
    let make = |id: &'static str, open: bool, action: bool, hovered: bool| {
        let mut s = CollapsibleSection::new(id, p, "チャンネル", Icon::Hash)
            .count("2")
            .open(open)
            .hovered(hovered)
            .child(body());
        if action {
            s = s.action(
                Button::new(SharedString::from(format!("{id}-add")), p)
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::IconSm)
                    .icon(Icon::Plus),
            );
        }
        div().w(px(288.0)).bg(p.t.sidebar).text_size(font_size::SM).child(s)
    };
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .child(caption(p, "CollapsibleSection (w-72 bg-sidebar)  Default / Closed / WithActions / hover"))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .child(make("cs-1", true, false, false))
                .child(make("cs-2", false, false, false))
                .child(make("cs-3", true, true, false))
                .child(make("cs-4", false, true, true)),
        )
}

fn scroll_area(p: &Palette) -> AnyElement {
    let mut items = div();
    for i in 1..=30 {
        items = items
            .child(div().py(px(12.0)).child(format!("Channel {i}")))
            .child(Separator::new(p));
    }
    // 30 行 × (24 + 24 + 1) = 1470px の内容を、内側 230px (h-64 − 枠 2 − 上下 24) で見せる。
    let thumb = Thumb::from_extent(230.0, 1470.0, 0.0);
    section(p, "ScrollArea (h-64 w-72 rounded-lg border p-3)")
        .child(
            ScrollArea::new("scroll", p)
                .thumb(thumb)
                .h(px(256.0))
                .w(px(288.0))
                .rounded(radius::LG)
                .border_1()
                .border_color(p.t.border)
                .p(px(12.0))
                .child(items),
        )
        .into_any_element()
}

fn image_previews(p: &Palette) -> AnyElement {
    let hash = "LEHV6nWB2yk8pyo0adR*.7kCMdnj";
    let tile = |pv: ImagePreview| pv.w(px(256.0)).h(px(192.0));
    section(p, "ImagePreview (w-64, 4:3)")
        .child(
            row()
                .gap(px(12.0))
                .w(px(780.0))
                .child(labeled(p, "loading (blurhash)", tile(ImagePreview::new(p).blurhash(Some(hash)))))
                .child(labeled(p, "without metadata", tile(ImagePreview::new(p).blurhash(None))))
                .child(labeled(p, "invalid hash", tile(ImagePreview::new(p).blurhash(Some("invalid")))))
                .child(labeled(
                    p,
                    "loaded",
                    tile(ImagePreview::new(p).blurhash(Some(hash)).src(sample_image(128, 96))),
                )),
        )
        .into_any_element()
}

fn control_panel(p: &Palette) -> AnyElement {
    let long = "名前や説明が長くても操作ボタンと重ならずに読めます。".repeat(8);
    div()
        .flex()
        .flex_col()
        .gap(px(4.0))
        .w(px(576.0))
        .child(caption(p, "ControlPanel (max-w-xl)  Default / LongDescription"))
        .child(
            SettingsSection::new(p)
                .title("メンバー")
                .description("メンバーと権限を管理します。")
                .child(
                    SettingRow::new(p, "Alice")
                        .description("管理者")
                        .control(Button::new("cp-1", p).variant(ButtonVariant::Outline).label("権限を変更")),
                ),
        )
        .child(
            SettingsSection::new(p).child(
                SettingRow::new(p, "長い表示名のメンバー")
                    .icon(Icon::Bell)
                    .description(long)
                    .control(Button::new("cp-2", p).variant(ButtonVariant::Outline).label("変更")),
            ),
        )
        .child(
            row()
                .gap(px(4.0))
                .mt(px(4.0))
                .p(px(4.0))
                .rounded(radius::LG)
                .bg(p.t.muted)
                .child(PanelTab::new("pt-1", p, "全般").active(true))
                .child(PanelTab::new("pt-2", p, "通知"))
                .child(PanelTab::new("pt-3", p, "メンバー").hovered(true)),
        )
        .into_any_element()
}

fn links_and_headers(p: &Palette) -> AnyElement {
    section(p, "ExternalLink / PaneHeader / ScreenHeader")
        .child(labeled(
            p,
            "external link / focus-visible",
            row()
                .gap(px(16.0))
                .child(ExternalLink::new("link-1", p, "https://github.com/Zli-UoA/zoo2", "https://github.com/Zli-UoA/zoo2"))
                .child(ExternalLink::new("link-2", p, "利用規約", "https://example.com").focused(true)),
        ))
        .child(labeled(
            p,
            "PaneHeader (h = 61)",
            div().w(px(560.0)).child(
                PaneHeader::new(p)
                    .child(div().flex_1().child("PaneHeader"))
                    .child(Button::new("ph-1", p).variant(ButtonVariant::Ghost).size(ButtonSize::IconSm).icon(Icon::Settings)),
            ),
        ))
        .child(labeled(
            p,
            "ScreenHeader",
            div().w(px(560.0)).child(
                ScreenHeader::new(p, Icon::Bell, "通知")
                    .description("メンションとリアクション")
                    .action(Button::new("sh-1", p).variant(ButtonVariant::Outline).size(ButtonSize::Sm).label("すべて既読")),
            ),
        ))
        .into_any_element()
}

fn page_display(p: &Palette) -> Div {
    div()
        .flex()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.0))
                .w(px(460.0))
                .child(avatars(p))
                .child(scroll_area(p)),
        )
        .child(div().flex().flex_col().flex_1().gap(px(12.0)).child(collapsibles(p)))
}

fn page_feedback(p: &Palette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.0))
        .child(
            div()
                .flex()
                .gap(px(12.0))
                .child(tooltips(p))
                .child(sonner_stage(p))
                .child(sonner_stack_stage(p)),
        )
        .child(
            div()
                .flex()
                .gap(px(12.0))
                .items_start()
                .child(control_panel(p))
                .child(image_previews(p)),
        )
}

impl Render for ComponentsGallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let p = self.p;
        let body = match self.page {
            1 => page_buttons(&p),
            2 => page_fields(&p),
            3 => page_dialogs(&p, &self.i18n),
            4 => page_display(&p),
            _ => page_feedback(&p),
        };
        div()
            .size_full()
            .p(px(16.0))
            .bg(p.t.background)
            .text_color(p.t.foreground)
            .font(fonts::sans(400.0))
            .text_size(font_size::SM)
            .line_height(px(20.0))
            .child(body)
    }
}
