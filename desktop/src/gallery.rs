//! 基礎部品の確認画面 (`zoo2-desktop gallery`)。
//! トークン、フォント、アイコン、ロゴ、翻訳だけを使い、既存の画面には触れない。
//! 画面の撮影は `scripts/desktop-screenshot.sh` で行う。

use gpui::{
    App, Application, Bounds, Context, Div, IntoElement, KeyBinding, Render, SharedString, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};

use crate::assets::{Assets, logo};
use crate::fonts;
use crate::gallery_components::ComponentsGallery;
use crate::i18n::{I18n, Locale};
use crate::icons::{Icon, icon};
use crate::tokens::{ThemeMode, Tokens, font_size, initial_mode, radius};

struct Gallery {
    mode: ThemeMode,
    tokens: Tokens,
    i18n: I18n,
}

fn card(tokens: &Tokens) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(12.0))
        .bg(tokens.card)
        .text_color(tokens.card_foreground)
        .border_1()
        .border_color(tokens.border)
        .rounded(radius::LG)
}

fn heading(tokens: &Tokens, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(font_size::SM)
        .font(fonts::sans(600.0))
        .text_color(tokens.muted_foreground)
        .child(text.into())
}

fn swatches(title: &'static str, tokens: Tokens) -> Div {
    let mut grid = div().flex().flex_wrap().gap(px(6.0));
    for (name, color) in tokens.entries() {
        grid = grid.child(
            div()
                .w(px(88.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(
                    div()
                        .h(px(26.0))
                        .rounded(radius::SM)
                        .border_1()
                        .border_color(tokens.border)
                        .bg(color),
                )
                .child(
                    div()
                        .text_size(font_size::PX_9)
                        .font(fonts::sans(400.0))
                        .text_color(tokens.foreground)
                        .child(name),
                ),
        );
    }
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap(px(8.0))
        .p(px(12.0))
        .bg(tokens.background)
        .text_color(tokens.foreground)
        .border_1()
        .border_color(tokens.border)
        .rounded(radius::LG)
        .child(heading(&tokens, title))
        .child(grid)
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.tokens;
        let i18n = self.i18n;

        let mut fonts_card = card(&t).w(px(470.0)).child(heading(&t, "Geist"));
        for (label, weight) in [
            ("Regular", 400.0),
            ("Medium", 500.0),
            ("SemiBold", 600.0),
            ("Bold", 700.0),
        ] {
            fonts_card = fonts_card.child(
                div()
                    .text_size(font_size::XL)
                    .font(fonts::sans(weight))
                    .child(format!("{label} {weight} あいうアイウ 漢字 Aa")),
            );
        }
        // フォールバックの有無でかなの太さが変わるかを見る。行頭を同じ文字にして比べる。
        for (label, weight, fallbacks) in [
            ("400 fallbacks", 400.0, true),
            ("700 fallbacks", 700.0, true),
            ("700 no fallbacks", 700.0, false),
        ] {
            let mut f = fonts::sans(weight);
            if !fallbacks {
                f.fallbacks = None;
            }
            fonts_card = fonts_card.child(
                div()
                    .text_size(font_size::XL)
                    .font(f)
                    .child(format!("あいうアイウ 漢字 {label}")),
            );
        }

        let mut icons = div().flex().flex_wrap().gap(px(4.0));
        for &ic in Icon::ALL {
            icons = icons.child(
                div()
                    .w(px(32.0))
                    .h(px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon(ic, px(20.0), t.foreground)),
            );
        }
        let icons_card = card(&t)
            .w(px(450.0))
            .child(heading(&t, format!("lucide ({} 個)", Icon::ALL.len())))
            .child(icons);

        let mut strings = card(&t)
            .flex_1()
            .child(heading(&t, format!("i18n ({})", i18n.locale().code())));
        let sample = [
            i18n.t("channels.title"),
            i18n.t("channels.join"),
            i18n.t_count("messages.count", 1, &[]),
            i18n.t_count("messages.count", 3, &[]),
            i18n.t_with("channels.descriptionHelp", &[("max", "250")]),
        ];
        for line in sample {
            strings = strings.child(
                div()
                    .text_size(font_size::SM)
                    .font(fonts::sans(400.0))
                    .child(line),
            );
        }
        // 言語ごとの同じキー。
        for locale in Locale::ALL {
            strings = strings.child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .text_size(font_size::XS)
                    .font(fonts::sans(400.0))
                    .child(
                        div()
                            .w(px(84.0))
                            .text_color(t.muted_foreground)
                            .child(locale.native_name()),
                    )
                    .child(I18n::new(locale).t("channels.title")),
            );
        }

        let logo_card = card(&t)
            .w(px(140.0))
            .items_center()
            .child(heading(&t, "logo"))
            .child(logo(px(96.0)).rounded(radius::XL));

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(16.0))
            .bg(t.background)
            .text_color(t.foreground)
            .font(fonts::sans(400.0))
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(swatches("light (:root)", Tokens::light()))
                    .child(swatches("dark (.dark)", Tokens::dark())),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.0))
                    .child(fonts_card)
                    .child(icons_card)
                    .child(logo_card)
                    .child(strings),
            )
            .child(
                div()
                    .text_size(font_size::PX_11)
                    .text_color(t.muted_foreground)
                    .child(format!("theme: {:?}", self.mode)),
            )
    }
}

/// ギャラリーのウィンドウを開く。モードは `tokens::initial_mode()` (`--theme`) に従う。
/// `args` は `gallery` の後ろの引数。`components [ページ]` なら UI 部品のページを開く。
pub fn run(args: &[String]) {
    let components_page = match args.first().map(String::as_str) {
        Some("components") => Some(
            args.get(1)
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1),
        ),
        _ => None,
    };
    Application::new().with_assets(Assets).run(move |cx: &mut App| {
        if let Err(err) = fonts::register(cx) {
            eprintln!("フォントを登録できませんでした: {err:#}");
        }
        cx.on_action(|_: &crate::Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", crate::Quit, None),
            KeyBinding::new("ctrl-q", crate::Quit, None),
        ]);
        // ウィンドウは 1 つだけなので、閉じたらアプリも終える。
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let mode = initial_mode();
        let bounds = Bounds::centered(None, size(px(1440.0), px(900.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let opened = match components_page {
            Some(page) => cx
                .open_window(options, move |_, cx| {
                    cx.new(|_| ComponentsGallery::new(page, mode))
                })
                .map(|_| ()),
            None => cx
                .open_window(options, move |_, cx| {
                    cx.new(|_| Gallery {
                        mode,
                        tokens: mode.tokens(),
                        i18n: I18n::detect(),
                    })
                })
                .map(|_| ()),
        };
        if let Err(err) = opened {
            eprintln!("ウィンドウを開けませんでした: {err:#}");
            std::process::exit(1);
        }
        cx.activate(true);
    });
}
