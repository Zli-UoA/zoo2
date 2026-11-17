//! メッセージ部品（Discord Components V2 形式、frontend の `ComponentTree.tsx`）。
//!
//! - 文字は MFM で描画する。container は左に色の帯、section は横に添え物を置く。
//! - 画像・動画は取得せず、説明とリンクで出す（デスクトップには画像のプロキシがなく、置き場所へ直接取りに行かないため）。
//! - ボタンと選択肢（string select・overflow・radio）は押すとアプリへ送る。入力欄・ピッカー・
//!   ユーザーやチャンネルの選択は、デスクトップでは操作できない旨を出す（モーダルとホームは Web で操作する）。

use gpui::{
    AnyElement, Context, Div, FocusHandle, ParentElement, SharedString, Styled, div, prelude::*, px,
};
use zoo2_client_core::components::{Component, Document, TableCell, button_style};

use crate::fonts;
use crate::i18n::I18n;
use crate::mfm_view::{self, MfmColors};
use crate::ui::button::{Button, ButtonSize, ButtonVariant};
use crate::ui::{TextSpec, alpha, h_flex, palette, v_flex};
use crate::view::WorkspaceView;

use super::ogp_card::card_color;

/// 操作の送り先。`app_id` がなければ、操作部品は押せない状態で出す（Webhook・Slack の取り込み）。
pub struct Target<'a> {
    pub key: &'a str,
    pub app_id: Option<&'a str>,
    pub message_id: &'a str,
    pub colors: &'a MfmColors,
    pub accounts: &'a zoo2_client_core::store::AccountProfiles,
    pub me: Option<&'a str>,
    /// 確認を表示中の操作のキー（WorkspaceView::component_confirm）
    pub confirming: Option<&'a str>,
    /// 表示中の文書の版（内容のハッシュ）。文書が更新されたら、開いていた確認を無効にする
    pub version: u64,
    /// ボタン・リンクのフォーカス（描画をまたいで保つ。Tab で移り、Enter・Space で押せる）
    pub focus: &'a std::cell::RefCell<std::collections::HashMap<String, FocusHandle>>,
}

/// 確認（confirm）を表示中の操作のキー。描画の順番によらず、操作そのもの（custom_id と選んだ値）で決める
/// custom_id と値は区切り文字を含みうるので、JSON の配列にして区切る（`menu-vdelete` のボタンと
/// `menu` の選択肢の値 `delete` が同じキーにならない）
fn confirm_key(target: &Target, custom_id: &str, value: Option<&str>) -> String {
    format!(
        "{}-confirm-{}",
        target.key,
        serde_json::json!([target.version, custom_id, value])
    )
}

/// 開いている確認が、このメッセージの今の文書の版のものでなければ捨てる（文書が A → B → A と戻っても、
/// 古い確認を出さない）。確認のキーは `{key}-confirm-[版, custom_id, 値]`
pub fn drop_stale_confirm(confirm: &std::cell::RefCell<Option<String>>, key: &str, version: u64) {
    let prefix = format!("{key}-confirm-[");
    let current = format!("{prefix}{version},");
    let mut slot = confirm.borrow_mut();
    if slot
        .as_deref()
        .is_some_and(|k| k.starts_with(&prefix) && !k.starts_with(&current))
    {
        *slot = None;
    }
}

/// 表示中のチャンネルとスレッド（`view`）が前回の描画から変わったら、開いていた確認を捨てる。
/// 離れている間に文書が A → B → A と戻っても、戻ったときに古い確認を出さない
pub fn drop_confirm_on_view_change(
    confirm: &std::cell::RefCell<Option<String>>,
    last: &std::cell::RefCell<Option<String>>,
    view: String,
) {
    let mut last = last.borrow_mut();
    if last.as_deref() != Some(view.as_str()) {
        *confirm.borrow_mut() = None;
        *last = Some(view);
    }
}

/// 文書の版。内容（JSON）のハッシュで、文書が更新されると変わる
pub fn document_version(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(doc).unwrap_or_default().hash(&mut h);
    h.finish()
}

fn focus(target: &Target, key: &str, cx: &mut Context<WorkspaceView>) -> FocusHandle {
    target
        .focus
        .borrow_mut()
        .entry(key.to_string())
        .or_insert_with(|| cx.focus_handle().tab_stop(true))
        .clone()
}

pub fn render_components(
    doc: &Document,
    target: &Target,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let mut n = 0usize;
    v_flex()
        .gap(px(6.0))
        .min_w_0()
        .children(
            doc.components
                .iter()
                .map(|c| node(c, target, &mut n, i18n, cx))
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

fn text(content: &str, target: &Target, n: &mut usize) -> AnyElement {
    *n += 1;
    mfm_view::render_mfm_accounts_with_colors(
        &format!("{}-c{n}", target.key),
        content,
        target.me,
        target.colors,
        target.accounts,
    )
    .into_any_element()
}

fn small(s: impl Into<SharedString>, cx: &mut Context<WorkspaceView>) -> Div {
    let t = palette(cx).t;
    let xs = TextSpec::XS;
    div()
        .text_size(xs.size())
        .line_height(xs.line())
        .text_color(t.muted_foreground)
        .child(s.into())
}

/// https の URL だけを開くリンク。キーボードでも開ける
fn link(
    id: String,
    label: String,
    url: &str,
    target: &Target,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    if !zoo2_client_core::components::validate::is_https_url(url) {
        return div().child(label).into_any_element();
    }
    let url = url.to_string();
    let handle = focus(target, &id, cx);
    Button::new(SharedString::from(id))
        .variant(ButtonVariant::Link)
        .size(ButtonSize::Sm)
        .label(label)
        .focus_handle(&handle)
        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
        .into_any_element()
}

fn node(
    c: &Component,
    target: &Target,
    n: &mut usize,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = palette(cx).t;
    *n += 1;
    let id = format!("{}-n{}", target.key, *n);
    match c {
        Component::TextDisplay(x) => text(&x.content, target, n),
        Component::Container(x) => {
            let stripe = x
                .accent_color
                .and_then(|v| card_color(&format!("#{v:06x}")))
                .unwrap_or(t.brand);
            let mut body = v_flex().gap(px(6.0)).min_w_0().flex_1();
            if let Some(title) = &x.title {
                body = body.child(div().font(fonts::sans(600.0)).child(text(title, target, n)));
            }
            if x.spoiler {
                body = body.child(small(i18n.t("apps.components.spoiler"), cx));
            } else {
                for child in &x.components {
                    body = body.child(node(child, target, n, i18n, cx));
                }
            }
            div()
                .max_w(px(560.0))
                .rounded_md()
                .border_l(px(4.0))
                .border_color(stripe)
                .bg(t.card)
                .child(div().p_3().min_w_0().child(body))
                .into_any_element()
        }
        Component::Section(x) => {
            let texts = v_flex().flex_1().min_w_0().gap(px(4.0)).children(
                x.components
                    .iter()
                    .map(|c| node(c, target, n, i18n, cx))
                    .collect::<Vec<_>>(),
            );
            let accessory = x.accessory.as_deref().map(|a| node(a, target, n, i18n, cx));
            h_flex()
                .items_start()
                .gap_3()
                .child(texts)
                .children(accessory)
                .into_any_element()
        }
        Component::Thumbnail(x) => link(
            id,
            format!("🖼 {}", x.description.clone().unwrap_or_default()),
            &x.media.url,
            target,
            cx,
        ),
        Component::MediaGallery(x) => v_flex()
            .children(
                x.items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        let label = format!(
                            "🖼 {}",
                            item.description
                                .clone()
                                .unwrap_or_else(|| format!("{}", i + 1))
                        );
                        link(format!("{id}-{i}"), label, &item.media.url, target, cx)
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::File(x) => link(
            id,
            format!(
                "📄 {}",
                x.name.clone().unwrap_or_else(|| x
                    .file
                    .url
                    .rsplit('/')
                    .next()
                    .unwrap_or("file")
                    .to_string())
            ),
            &x.file.url,
            target,
            cx,
        ),
        Component::Separator(x) => {
            if x.divider {
                div()
                    .my(px(if x.spacing == 2 { 8.0 } else { 2.0 }))
                    .h(px(1.0))
                    .bg(t.border)
                    .into_any_element()
            } else {
                div()
                    .h(px(if x.spacing == 2 { 12.0 } else { 4.0 }))
                    .into_any_element()
            }
        }
        Component::ActionRow(x) => h_flex()
            .flex_wrap()
            .gap_2()
            .children(
                x.components
                    .iter()
                    .map(|c| node(c, target, n, i18n, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::Button(b) => {
            let label = [
                b.emoji.as_ref().and_then(|e| e.name.clone()),
                b.label.clone(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" ");
            if b.style == button_style::LINK {
                let url = b.url.clone().unwrap_or_default();
                let handle = focus(target, &id, cx);
                return Button::new(SharedString::from(id))
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .label(label)
                    .disabled(
                        b.disabled || !zoo2_client_core::components::validate::is_https_url(&url),
                    )
                    .focus_handle(&handle)
                    .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                    .into_any_element();
            }
            let variant = match b.style {
                button_style::PRIMARY => ButtonVariant::Default,
                button_style::SUCCESS => ButtonVariant::Secondary,
                button_style::DANGER => ButtonVariant::Destructive,
                _ => ButtonVariant::Outline,
            };
            let action = action(
                target,
                b.custom_id.clone(),
                serde_json::json!(b.value.clone().unwrap_or_default()),
            );
            // 確認のキーは操作（メッセージと custom_id。文書の中で重複しない）で作る。描画の順番で作ると、
            // 確認の表示中に前の部品が増減したとき、別のボタンの確認になる
            let confirm_key = confirm_key(target, b.custom_id.as_deref().unwrap_or_default(), None);
            // 確認の表示中に押せない状態へ変わったときは、確認を出さない（「はい」で送らない）
            if let (Some(confirm), Some(key)) = (&b.confirm, target.confirming)
                && key == confirm_key
                && !b.disabled
                && action.is_some()
            {
                return confirm_bar(&confirm_key, confirm, action, target, cx);
            }
            let has_confirm = b.confirm.is_some();
            let key = confirm_key;
            let handle = focus(target, &id, cx);
            Button::new(SharedString::from(id))
                .variant(variant)
                .size(ButtonSize::Sm)
                .label(label)
                .focus_handle(&handle)
                .disabled(b.disabled || action.is_none())
                .on_click(
                    cx.listener(move |this, _, _, cx| press(this, &key, has_confirm, &action, cx)),
                )
                .into_any_element()
        }
        // 複数を選ぶ select は、選んだ値をまとめて送る必要があるので Web 版で操作する
        Component::StringSelect(s) if s.max_values.unwrap_or(1) > 1 => {
            small(i18n.t("apps.components.webOnly"), cx).into_any_element()
        }
        Component::StringSelect(s) => choices(
            target,
            &s.custom_id,
            &s.options,
            s.disabled,
            s.placeholder.as_deref(),
            true,
            s.confirm.as_ref(),
            cx,
        ),
        Component::Overflow(g) | Component::RadioGroup(g) => choices(
            target,
            &g.custom_id,
            &g.options,
            g.disabled,
            None,
            false,
            g.confirm.as_ref(),
            cx,
        ),
        Component::Label(l) => v_flex()
            .gap_1()
            .child(div().font(fonts::sans(500.0)).child(l.label.clone()))
            .children(l.description.clone().map(|d| small(d, cx)))
            .child(node(&l.component, target, n, i18n, cx))
            .into_any_element(),
        Component::TextInput(_)
        | Component::UserSelect(_)
        | Component::RoleSelect(_)
        | Component::MentionableSelect(_)
        | Component::ChannelSelect(_)
        | Component::ExternalSelect(_)
        | Component::DatePicker(_)
        | Component::TimePicker(_)
        | Component::DateTimePicker(_)
        | Component::CheckboxGroup(_)
        | Component::Checkbox(_)
        | Component::FileUpload(_) => {
            small(i18n.t("apps.components.webOnly"), cx).into_any_element()
        }
        Component::Video(v) => v_flex()
            .child(link(
                id,
                format!("▶ {}", v.title),
                v.title_url.as_deref().unwrap_or(&v.video_url),
                target,
                cx,
            ))
            .children(v.description.clone().map(|d| small(d, cx)))
            .into_any_element(),
        Component::Table(x) => table(x, target, n, i18n, cx),
        Component::Alert(a) => div()
            .rounded_md()
            .border_1()
            .border_color(match a.level.as_deref() {
                Some("error") => t.destructive,
                _ => t.border,
            })
            .bg(alpha(t.muted, 0.5))
            .px_3()
            .py_2()
            .child(text(&a.content, target, n))
            .into_any_element(),
        Component::Card(card) => card_view(card, target, n, i18n, cx),
        Component::Carousel(c) => h_flex()
            .gap_2()
            .flex_wrap()
            .children(
                c.cards
                    .iter()
                    .map(|x| node(x, target, n, i18n, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::Chart(c) => {
            let mut lines = vec![div().font(fonts::sans(600.0)).child(c.title.clone())];
            for s in &c.segments {
                lines.push(small(format!("{}: {}", s.label, s.value), cx));
            }
            for s in &c.series {
                let values = s
                    .data
                    .iter()
                    .map(|p| format!("{} {}", p.label, p.value))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(small(format!("{}: {values}", s.name), cx));
            }
            v_flex().children(lines).into_any_element()
        }
        Component::Grid(g) => v_flex()
            .gap_2()
            .children(
                g.label
                    .clone()
                    .map(|l| div().font(fonts::sans(600.0)).child(l)),
            )
            .children(
                g.rows
                    .iter()
                    .map(|row| {
                        h_flex().items_start().gap_3().children(
                            row.iter()
                                .map(|item| {
                                    div().flex_1().min_w_0().child(node(
                                        &item.component,
                                        target,
                                        n,
                                        i18n,
                                        cx,
                                    ))
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::Plan(p) => v_flex()
            .child(div().font(fonts::sans(600.0)).child(p.title.clone()))
            .children(
                p.tasks
                    .iter()
                    .map(|task| task_line(task, cx))
                    .collect::<Vec<_>>(),
            )
            .into_any_element(),
        Component::TaskCard(task) => task_line(task, cx).into_any_element(),
        Component::Unsupported(u) => {
            if u.text.is_empty() {
                div().into_any_element()
            } else {
                small(u.text.clone(), cx).into_any_element()
            }
        }
    }
}

type Action = Option<(String, String, String, serde_json::Value)>;

/// 押したときの処理。confirm があれば、まず確認を出し（キーを覚える）、確認の「はい」で送る。
fn send(this: &mut WorkspaceView, action: &Action, cx: &mut Context<WorkspaceView>) {
    if let Some((app, message, custom, value)) = action.clone() {
        *this.component_confirm.borrow_mut() = None;
        this.state.update(cx, |s, cx| {
            s.invoke_component(&app, &message, &custom, value);
            cx.notify();
        });
    }
}

/// confirm の確認の行（タイトル・本文・はい・いいえ）。
fn confirm_bar(
    id: &str,
    confirm: &zoo2_client_core::components::Confirm,
    action: Action,
    target: &Target,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let yes = focus(target, &format!("{id}-yes"), cx);
    let no = focus(target, &format!("{id}-no"), cx);
    v_flex()
        .gap_1()
        .child(div().font(fonts::sans(600.0)).child(confirm.title.clone()))
        .child(small(confirm.text.clone(), cx))
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new(SharedString::from(format!("{id}-yes")))
                        .variant(if confirm.style.as_deref() == Some("danger") {
                            ButtonVariant::Destructive
                        } else {
                            ButtonVariant::Default
                        })
                        .size(ButtonSize::Sm)
                        .label(confirm.confirm.clone())
                        .focus_handle(&yes)
                        // 押した時点で、この確認がまだ開いているときだけ送る（描画の後に捨てられた確認から送らない）
                        .on_click({
                            let key = id.to_string();
                            cx.listener(move |this, _, _, cx| {
                                if this.component_confirm.borrow().as_deref() == Some(key.as_str())
                                {
                                    send(this, &action, cx)
                                }
                            })
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("{id}-no")))
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .label(confirm.deny.clone())
                        .focus_handle(&no)
                        .on_click(cx.listener(|this, _, _, cx| {
                            *this.component_confirm.borrow_mut() = None;
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

/// 操作を押したとき。confirm があれば確認を出し、なければすぐ送る。
fn press(
    this: &mut WorkspaceView,
    key: &str,
    confirm: bool,
    action: &Action,
    cx: &mut Context<WorkspaceView>,
) {
    if confirm {
        *this.component_confirm.borrow_mut() = Some(key.to_string());
        cx.notify();
    } else {
        send(this, action, cx);
    }
}

fn action(target: &Target, custom_id: Option<String>, value: serde_json::Value) -> Action {
    Some((
        target.app_id?.to_string(),
        target.message_id.to_string(),
        custom_id?,
        value,
    ))
}

/// 選択肢を小さなボタンで並べる。押すとその値をアプリへ送る（select は配列、overflow と radio は文字列）。
#[allow(clippy::too_many_arguments)]
fn choices(
    target: &Target,
    custom_id: &str,
    options: &[zoo2_client_core::components::SelectOption],
    disabled: bool,
    placeholder: Option<&str>,
    array: bool,
    confirm: Option<&zoo2_client_core::components::Confirm>,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    // 確認のキーは選択肢の値で作る（確認の表示中に選択肢の順序が変わっても、確認した値を送る）
    let key_of = |o: &zoo2_client_core::components::SelectOption| {
        confirm_key(target, custom_id, Some(&o.value))
    };
    // 確認を表示中の選択肢があれば、その確認だけを出す（押せない状態に変わっていれば出さない）
    for o in options {
        let key = key_of(o);
        if let (Some(c), Some(k)) = (confirm, target.confirming)
            && k == key
            && !disabled
        {
            let value = if array {
                serde_json::json!([o.value])
            } else {
                serde_json::json!(o.value)
            };
            let act = action(target, Some(custom_id.to_string()), value);
            if act.is_some() {
                return confirm_bar(&key, c, act, target, cx);
            }
        }
    }
    let has_confirm = confirm.is_some();
    let buttons = options
        .iter()
        .map(|o| {
            let value = if array {
                serde_json::json!([o.value])
            } else {
                serde_json::json!(o.value)
            };
            let act = action(target, Some(custom_id.to_string()), value);
            let key = key_of(o);
            let handle = focus(target, &key, cx);
            Button::new(SharedString::from(key.clone()))
                .variant(if o.default {
                    ButtonVariant::Secondary
                } else {
                    ButtonVariant::Ghost
                })
                .size(ButtonSize::Xs)
                .label(o.label.clone())
                .focus_handle(&handle)
                .disabled(disabled || act.is_none())
                .on_click(
                    cx.listener(move |this, _, _, cx| press(this, &key, has_confirm, &act, cx)),
                )
        })
        .collect::<Vec<_>>();
    v_flex()
        .gap_1()
        .children(placeholder.map(|p| small(p.to_string(), cx)))
        .child(h_flex().flex_wrap().gap_1().children(buttons))
        .into_any_element()
}

fn table(
    x: &zoo2_client_core::components::Table,
    target: &Target,
    n: &mut usize,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = palette(cx).t;
    let cell = |c: &TableCell, n: &mut usize, cx: &mut Context<WorkspaceView>| -> AnyElement {
        match &c.button {
            Some(b) => node(b, target, n, i18n, cx),
            None => text(&c.text, target, n),
        }
    };
    let mut rows = Vec::new();
    // 行数は検証で上限がある（見出しなし 100 行・見出しあり 201 行）ので、すべて描画する
    for (r, row) in x.rows.iter().enumerate() {
        let cells = row
            .iter()
            .map(|c| div().w(px(120.0)).px_2().child(cell(c, n, cx)))
            .collect::<Vec<_>>();
        let mut line = h_flex()
            .py_1()
            .border_b_1()
            .border_color(t.border)
            .children(cells);
        if x.header && r == 0 {
            line = line.font(fonts::sans(600.0));
        }
        rows.push(line);
    }
    v_flex()
        .children(x.caption.clone().map(|c| small(c, cx)))
        .children(rows)
        .into_any_element()
}

fn card_view(
    card: &zoo2_client_core::components::Card,
    target: &Target,
    n: &mut usize,
    i18n: &I18n,
    cx: &mut Context<WorkspaceView>,
) -> AnyElement {
    let t = palette(cx).t;
    let mut body = v_flex()
        .gap_1()
        .p_2()
        .w(px(240.0))
        .rounded_md()
        .border_1()
        .border_color(t.border)
        .bg(t.card);
    // 画像は取得せず、リンクで出す（ほかの画像の部品と同じ）
    for (i, media) in [&card.hero_image, &card.icon]
        .into_iter()
        .flatten()
        .enumerate()
    {
        *n += 1;
        body = body.child(link(
            format!("{}-card{}-{i}", target.key, *n),
            "🖼".to_string(),
            &media.url,
            target,
            cx,
        ));
    }
    if let Some(title) = &card.title {
        body = body.child(div().font(fonts::sans(600.0)).child(text(title, target, n)));
    }
    for s in [&card.subtitle, &card.body, &card.subtext]
        .into_iter()
        .flatten()
    {
        body = body.child(text(s, target, n));
    }
    let actions = card
        .actions
        .iter()
        .map(|a| node(a, target, n, i18n, cx))
        .collect::<Vec<_>>();
    body.child(h_flex().gap_1().flex_wrap().children(actions))
        .into_any_element()
}

fn task_line(
    task: &zoo2_client_core::components::TaskCard,
    cx: &mut Context<WorkspaceView>,
) -> Div {
    let mark = match task.status.as_str() {
        "complete" => "✓",
        "error" => "✕",
        "in_progress" => "…",
        _ => "○",
    };
    v_flex()
        .child(format!("{mark} {}", task.title))
        .children(task.details.clone().map(|d| small(d, cx)))
}

#[cfg(test)]
mod confirm_key_tests {
    use super::{drop_confirm_on_view_change, drop_stale_confirm};

    #[test]
    fn leaving_the_channel_drops_the_open_confirmation() {
        let confirm = std::cell::RefCell::new(None);
        let last = std::cell::RefCell::new(None);
        drop_confirm_on_view_change(&confirm, &last, "c1:".into());
        *confirm.borrow_mut() = Some("open".to_string());
        // 同じチャンネルの再描画では残る
        drop_confirm_on_view_change(&confirm, &last, "c1:".into());
        assert!(confirm.borrow().is_some());
        // 別のチャンネルへ移ったら捨て、戻っても出ない
        drop_confirm_on_view_change(&confirm, &last, "c2:".into());
        drop_confirm_on_view_change(&confirm, &last, "c1:".into());
        assert!(confirm.borrow().is_none());
    }

    #[test]
    fn an_update_drops_the_open_confirmation_for_good() {
        let confirm = std::cell::RefCell::new(Some(r#"m-confirm-[1,"go",null]"#.to_string()));
        // 同じ版のあいだは残る。ほかのメッセージの描画では触らない
        drop_stale_confirm(&confirm, "m", 1);
        drop_stale_confirm(&confirm, "other", 2);
        assert!(confirm.borrow().is_some());
        // 文書が更新されたら捨てる。元の版に戻っても、もう出ない（A → B → A）
        drop_stale_confirm(&confirm, "m", 2);
        assert!(confirm.borrow().is_none());
        drop_stale_confirm(&confirm, "m", 1);
        assert!(confirm.borrow().is_none());
    }

    #[test]
    fn buttons_and_choices_do_not_share_a_key() {
        let key = |id: &str, v: Option<&str>| serde_json::json!([id, v]).to_string();
        assert_ne!(key("menu-vdelete", None), key("menu", Some("delete")));
        assert_ne!(key("a", Some("b")), key("a", None));
    }
}
