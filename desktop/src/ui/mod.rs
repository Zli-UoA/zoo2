//! Web 版 `frontend/src/components/ui/` に合わせた UI 部品。
//!
//! Tailwind のクラスを px に直して移してある (1 単位 = 4px、`text-sm` = 14px/20px)。
//! 部品は `RenderOnce` のビルダーで、色は gpui の `Global` に置いた `Palette` (トークンとモード) を
//! 描画のたびに `cx.global::<Palette>()` で読む。起動時に `set_theme` で入れ、切り替えも同じ関数で行う。
//! 入力欄は見た目だけを持ち、IME などの入力処理は既存の `input.rs` に任せる。
//!
//! 使うときの注意:
//! - Web 版の `inline-flex` (Button、ButtonGroup、Tooltip など) は、GPUI では幅が親いっぱいに広がる
//!   ブロックの `flex` になる。これらは `h_flex()` / `v_flex()` などの flex の親の中に置き、
//!   ブロックの親に直接入れない (ボタンが横に伸びる)。
//! - gpui 0.2.2 には `:focus-visible` が無く、`.focus()` だけがある。フォーカスの輪は、
//!   キーボードだけでなくクリックでも出る。見た目の確認用に `.focused(true)` で強制もできる。

#![allow(clippy::new_without_default)]

use gpui::{
    App, BoxShadow, Div, ElementId, Entity, Global, Hsla, Pixels, Rgba, Svg, Window, div, point,
    prelude::*, px, svg,
};

use crate::icons::Icon;
use crate::tokens::{ThemeMode, Tokens};

pub mod alert_dialog;
pub mod avatar;
pub mod button;
pub mod button_group;
pub mod collapsible_section;
pub mod color;
pub mod confirm_action_dialog;
pub mod control_panel;
pub mod dialog;
pub mod dropdown_menu;
pub mod external_link;
pub mod image_preview;
pub mod input;
pub mod pane_header;
pub mod scroll_area;
pub mod select;
pub mod separator;
pub mod skeleton;
pub mod sonner;
pub mod spinner;
pub mod textarea;
pub mod tooltip;

/// クリックの受け口。
pub type ClickHandler = Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static>;

/// 色トークンと配色モード。`dark:` の指定がある部品はモードも見る。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub t: Tokens,
    pub mode: ThemeMode,
}

impl Global for Palette {}

/// 現在の配色を入れ、開いている画面を描き直す。起動時と、テーマの切り替えで使う。
pub fn set_theme(cx: &mut App, mode: ThemeMode) {
    cx.set_global(Palette::new(mode));
    cx.refresh_windows();
}

/// 描画中に現在の配色を読む。`set_theme` を先に呼んでおくこと。
pub fn palette(cx: &App) -> Palette {
    *cx.global::<Palette>()
}

/// 横並びの flex (`flex`)。
pub fn h_flex() -> Div {
    div().flex().items_center()
}

/// 縦並びの flex (`flex flex-col`)。
pub fn v_flex() -> Div {
    div().flex().flex_col()
}

/// 要素の ID で見つかる `Entity` を、描画をまたいで 1 つだけ持つ。
/// 毎フレーム通知する部品 (アニメーション) は、これで自分だけのビューに入れる。
/// そうすると通知が親のビューに届かない。
pub fn keyed_entity<V: 'static>(
    id: ElementId,
    window: &mut Window,
    cx: &mut App,
    init: impl FnOnce() -> V,
) -> Entity<V> {
    window.with_global_id(id, |global_id, window| {
        window.with_element_state(global_id, |state: Option<Entity<V>>, _| {
            let entity = state.unwrap_or_else(|| cx.new(|_| init()));
            (entity.clone(), entity)
        })
    })
}

impl Palette {
    pub fn new(mode: ThemeMode) -> Self {
        Self {
            t: mode.tokens(),
            mode,
        }
    }

    pub fn is_dark(&self) -> bool {
        self.mode == ThemeMode::Dark
    }

    /// `light` と `dark:` で値が変わる色の選び分け。
    pub fn pick<T>(&self, light: T, dark: T) -> T {
        if self.is_dark() { dark } else { light }
    }
}

/// `color/NN` (Tailwind の不透明度指定)。元の不透明度に掛ける。
pub fn alpha(color: Rgba, a: f32) -> Rgba {
    Rgba {
        a: color.a * a,
        ..color
    }
}

pub const TRANSPARENT: Rgba = Rgba {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.0,
};

/// `bg-black/NN` などに使う黒。
pub fn black(a: f32) -> Rgba {
    Rgba {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a,
    }
}

/// `ring-N` (ぼかし 0 の広がりだけの影)。レイアウトには影響しない。
pub fn ring(color: Rgba, width: f32) -> BoxShadow {
    BoxShadow {
        color: Hsla::from(color),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: px(width),
    }
}

fn drop_shadow(y: f32, blur: f32, spread: f32, a: f32) -> BoxShadow {
    BoxShadow {
        color: Hsla::from(black(a)),
        offset: point(px(0.0), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

/// `shadow-sm`: `0 1px 3px 0 / 0 1px 2px -1px` (黒 10%)。
pub fn shadow_sm() -> Vec<BoxShadow> {
    vec![
        drop_shadow(1.0, 3.0, 0.0, 0.1),
        drop_shadow(1.0, 2.0, -1.0, 0.1),
    ]
}

/// `shadow-md`: `0 4px 6px -1px / 0 2px 4px -2px` (黒 10%)。
pub fn shadow_md() -> Vec<BoxShadow> {
    vec![
        drop_shadow(4.0, 6.0, -1.0, 0.1),
        drop_shadow(2.0, 4.0, -2.0, 0.1),
    ]
}

/// `shadow-lg`: `0 10px 15px -3px / 0 4px 6px -4px` (黒 10%)。
pub fn shadow_lg() -> Vec<BoxShadow> {
    vec![
        drop_shadow(10.0, 15.0, -3.0, 0.1),
        drop_shadow(4.0, 6.0, -4.0, 0.1),
    ]
}

/// 色付きのアイコン。CSS の `currentColor` の代わりに、色は呼び出し側が渡す
/// (GPUI の `svg()` は親の文字色を引き継がない)。
pub fn icon_colored(icon: Icon, size: f32, color: Rgba) -> Svg {
    svg()
        .path(icon.path())
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// 文字サイズと行の高さ (px)。Tailwind の `text-*` の対。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextSpec {
    pub size: f32,
    pub line: f32,
}

impl TextSpec {
    pub const XS: Self = Self {
        size: 12.0,
        line: 16.0,
    };
    pub const SM: Self = Self {
        size: 14.0,
        line: 20.0,
    };
    pub const BASE: Self = Self {
        size: 16.0,
        line: 24.0,
    };

    pub fn size(self) -> Pixels {
        px(self.size)
    }

    pub fn line(self) -> Pixels {
        px(self.line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_multiplies() {
        let c = alpha(alpha(Tokens::light().foreground, 0.5), 0.5);
        assert!((c.a - 0.25).abs() < 1e-6);
    }

    #[test]
    fn pick_follows_mode() {
        assert_eq!(Palette::new(ThemeMode::Light).pick(1, 2), 1);
        assert_eq!(Palette::new(ThemeMode::Dark).pick(1, 2), 2);
    }

    #[test]
    fn ring_has_no_blur() {
        let s = ring(Tokens::light().ring, 3.0);
        assert_eq!(s.blur_radius, px(0.0));
        assert_eq!(s.spread_radius, px(3.0));
    }
}
