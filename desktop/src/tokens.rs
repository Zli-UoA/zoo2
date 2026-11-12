//! Web 版 (`frontend/src/styles.css`) のデザイントークン。
//!
//! CSS の変数をそのまま移したもので、値は `styles.css` と一致させる。
//! 食い違いは、このファイルのテストが `styles.css` を読んで見つける。

use std::sync::OnceLock;

use gpui::{Pixels, Rgba, px};

/// 配色のモード。Web 版の `:root` が Light、`.dark` が Dark に当たる。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeMode {
    Light,
    Dark,
}

impl ThemeMode {
    /// `light` / `dark` を読む。大文字小文字は区別しない。
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }

    pub fn tokens(self) -> Tokens {
        match self {
            Self::Light => Tokens::light(),
            Self::Dark => Tokens::dark(),
        }
    }
}

static INITIAL_MODE: OnceLock<ThemeMode> = OnceLock::new();

/// 起動時に選んだモードを覚える。最初の 1 回だけ有効。
/// main.rs が `--theme` / `ZOO_DESKTOP_THEME` から呼ぶ。
pub fn set_initial_mode(mode: ThemeMode) {
    let _ = INITIAL_MODE.set(mode);
}

/// 起動時に選ばれたモード。指定がなければ Light。
pub fn initial_mode() -> ThemeMode {
    INITIAL_MODE.get().copied().unwrap_or(ThemeMode::Light)
}

/// `#rrggbb` を Rgba にする。
pub const fn hex(value: u32) -> Rgba {
    let [_, r, g, b] = value.to_be_bytes();
    Rgba {
        r: (r as f32) / 255.0,
        g: (g as f32) / 255.0,
        b: (b as f32) / 255.0,
        a: 1.0,
    }
}

/// OKLCH (CSS の `oklch(L C H)`) を sRGB にする。
/// `l` は 0〜1、`c` は彩度、`h` は色相の角度 (度)。gamut の外は各チャンネルを 0〜1 に丸める。
pub fn oklch_to_srgb(l: f32, c: f32, h_deg: f32) -> Rgba {
    let (l, c, h) = (l as f64, c as f64, (h_deg as f64).to_radians());
    let (a, b) = (c * h.cos(), c * h.sin());

    // OKLab -> LMS' -> LMS
    let l_ = (l + 0.396_337_777_4 * a + 0.215_803_757_3 * b).powi(3);
    let m_ = (l - 0.105_561_345_8 * a - 0.063_854_172_8 * b).powi(3);
    let s_ = (l - 0.089_484_177_5 * a - 1.291_485_548_0 * b).powi(3);

    // LMS -> 線形 sRGB
    let r = 4.076_741_662_1 * l_ - 3.307_711_591_3 * m_ + 0.230_969_929_2 * s_;
    let g = -1.268_438_004_6 * l_ + 2.609_757_401_1 * m_ - 0.341_319_396_5 * s_;
    let bl = -0.004_196_086_3 * l_ - 0.703_418_614_7 * m_ + 1.707_614_701_0 * s_;

    fn gamma(x: f64) -> f32 {
        let x = x.clamp(0.0, 1.0);
        // x は 0〜1 に収めてあり、変換は単調で 0 と 1 を保つので、結果も 0〜1 に収まる。
        let v = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        v as f32
    }

    Rgba {
        r: gamma(r),
        g: gamma(g),
        b: gamma(bl),
        a: 1.0,
    }
}

/// Rgba を 0〜255 の 3 チャンネルにする。
pub fn to_u8(color: Rgba) -> [u8; 3] {
    let ch = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    [ch(color.r), ch(color.g), ch(color.b)]
}

/// `styles.css` の色変数。フィールド名は CSS の変数名 (`--primary-hover` なら `primary_hover`)。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tokens {
    pub background: Rgba,
    pub foreground: Rgba,
    pub card: Rgba,
    pub card_foreground: Rgba,
    pub popover: Rgba,
    pub popover_foreground: Rgba,
    pub primary: Rgba,
    pub primary_hover: Rgba,
    pub primary_foreground: Rgba,
    pub secondary: Rgba,
    pub secondary_foreground: Rgba,
    pub muted: Rgba,
    pub muted_foreground: Rgba,
    pub accent: Rgba,
    pub accent_foreground: Rgba,
    pub destructive: Rgba,
    pub link: Rgba,
    pub brand: Rgba,
    pub brand_foreground: Rgba,
    pub brand_muted: Rgba,
    pub brand_muted_foreground: Rgba,
    pub border: Rgba,
    pub input: Rgba,
    pub ring: Rgba,
    pub chart_1: Rgba,
    pub chart_2: Rgba,
    pub chart_3: Rgba,
    pub chart_4: Rgba,
    pub chart_5: Rgba,
    pub sidebar: Rgba,
    pub sidebar_foreground: Rgba,
    pub sidebar_primary: Rgba,
    pub sidebar_primary_foreground: Rgba,
    pub sidebar_accent: Rgba,
    pub sidebar_accent_foreground: Rgba,
    pub sidebar_border: Rgba,
    pub sidebar_ring: Rgba,
}

/// CSS にある色以外の変数。ここに挙げたものは `Tokens` に無くてよい。
pub const NON_COLOR_VARS: &[&str] = &["radius"];

impl Tokens {
    /// `:root`
    pub fn light() -> Self {
        let primary = hex(0xffd600);
        let primary_foreground = hex(0x000000);
        let accent = hex(0xfffbe8);
        let accent_foreground = hex(0x424242);
        let border = hex(0xe5e5e5);
        let ring = hex(0x737373);
        Self {
            background: hex(0xffffff),
            foreground: hex(0x000000),
            card: hex(0xffffff),
            card_foreground: hex(0x000000),
            popover: hex(0xffffff),
            popover_foreground: hex(0x000000),
            primary,
            primary_hover: hex(0xffe34d),
            primary_foreground,
            secondary: hex(0xf5f5f5),
            secondary_foreground: hex(0x000000),
            muted: hex(0xf5f5f5),
            muted_foreground: hex(0x666666),
            accent,
            accent_foreground,
            destructive: oklch_to_srgb(0.51, 0.21, 27.325),
            link: hex(0x1d4ed8),
            brand: hex(0x626262),
            brand_foreground: hex(0xffffff),
            brand_muted: hex(0xfff6cc),
            brand_muted_foreground: hex(0x000000),
            border,
            input: hex(0xd4d4d4),
            ring,
            chart_1: hex(0xffd600),
            chart_2: hex(0xd9d9d9),
            chart_3: hex(0xa3a3a3),
            chart_4: hex(0x626262),
            chart_5: hex(0x000000),
            sidebar: hex(0xfafaf8),
            sidebar_foreground: hex(0x000000),
            sidebar_primary: primary,
            sidebar_primary_foreground: primary_foreground,
            sidebar_accent: accent,
            sidebar_accent_foreground: accent_foreground,
            sidebar_border: border,
            sidebar_ring: ring,
        }
    }

    /// `.dark`
    pub fn dark() -> Self {
        let primary = hex(0xffd600);
        let primary_foreground = hex(0x000000);
        let accent = hex(0x2e2e2e);
        let accent_foreground = hex(0xffffff);
        let border = hex(0x454545);
        let ring = hex(0xbdbdbd);
        Self {
            background: hex(0x000000),
            foreground: hex(0xffffff),
            card: hex(0x181818),
            card_foreground: hex(0xffffff),
            popover: hex(0x181818),
            popover_foreground: hex(0xffffff),
            primary,
            primary_hover: hex(0xffe34d),
            primary_foreground,
            secondary: hex(0x262626),
            secondary_foreground: hex(0xffffff),
            muted: hex(0x262626),
            muted_foreground: hex(0xbdbdbd),
            accent,
            accent_foreground,
            destructive: oklch_to_srgb(0.704, 0.191, 22.216),
            link: hex(0x8ab4ff),
            brand: hex(0xd9d9d9),
            brand_foreground: hex(0x000000),
            brand_muted: hex(0x303030),
            brand_muted_foreground: hex(0xe5e5e5),
            border,
            input: hex(0x575757),
            ring,
            chart_1: hex(0xffd600),
            chart_2: hex(0xe5e5e5),
            chart_3: hex(0xd9d9d9),
            chart_4: hex(0xbdbdbd),
            chart_5: hex(0xa3a3a3),
            sidebar: hex(0x141414),
            sidebar_foreground: hex(0xffffff),
            sidebar_primary: primary,
            sidebar_primary_foreground: primary_foreground,
            sidebar_accent: accent,
            sidebar_accent_foreground: accent_foreground,
            sidebar_border: border,
            sidebar_ring: ring,
        }
    }

    /// CSS の変数名 (先頭の `--` なし) と値の組。ギャラリーとドリフトテストが使う。
    pub fn entries(&self) -> Vec<(&'static str, Rgba)> {
        vec![
            ("background", self.background),
            ("foreground", self.foreground),
            ("card", self.card),
            ("card-foreground", self.card_foreground),
            ("popover", self.popover),
            ("popover-foreground", self.popover_foreground),
            ("primary", self.primary),
            ("primary-hover", self.primary_hover),
            ("primary-foreground", self.primary_foreground),
            ("secondary", self.secondary),
            ("secondary-foreground", self.secondary_foreground),
            ("muted", self.muted),
            ("muted-foreground", self.muted_foreground),
            ("accent", self.accent),
            ("accent-foreground", self.accent_foreground),
            ("destructive", self.destructive),
            ("link", self.link),
            ("brand", self.brand),
            ("brand-foreground", self.brand_foreground),
            ("brand-muted", self.brand_muted),
            ("brand-muted-foreground", self.brand_muted_foreground),
            ("border", self.border),
            ("input", self.input),
            ("ring", self.ring),
            ("chart-1", self.chart_1),
            ("chart-2", self.chart_2),
            ("chart-3", self.chart_3),
            ("chart-4", self.chart_4),
            ("chart-5", self.chart_5),
            ("sidebar", self.sidebar),
            ("sidebar-foreground", self.sidebar_foreground),
            ("sidebar-primary", self.sidebar_primary),
            (
                "sidebar-primary-foreground",
                self.sidebar_primary_foreground,
            ),
            ("sidebar-accent", self.sidebar_accent),
            ("sidebar-accent-foreground", self.sidebar_accent_foreground),
            ("sidebar-border", self.sidebar_border),
            ("sidebar-ring", self.sidebar_ring),
        ]
    }
}

/// `--radius` (0.625rem、1rem = 16px)。
pub const RADIUS_BASE: f32 = 10.0;

/// 角丸の段階。`--radius-sm` から `--radius-4xl`。
pub mod radius {
    use super::{Pixels, RADIUS_BASE, px};

    pub const SM: Pixels = px(RADIUS_BASE * 0.6);
    pub const MD: Pixels = px(RADIUS_BASE * 0.8);
    pub const LG: Pixels = px(RADIUS_BASE);
    pub const XL: Pixels = px(RADIUS_BASE * 1.4);
    pub const XXL: Pixels = px(RADIUS_BASE * 1.8);
    pub const XXXL: Pixels = px(RADIUS_BASE * 2.2);
    pub const XXXXL: Pixels = px(RADIUS_BASE * 2.6);

    /// (CSS の名前、倍率)。ドリフトテストが使う。
    pub const SCALE: &[(&str, f32)] = &[
        ("sm", 0.6),
        ("md", 0.8),
        ("lg", 1.0),
        ("xl", 1.4),
        ("2xl", 1.8),
        ("3xl", 2.2),
        ("4xl", 2.6),
    ];
}

/// 文字サイズの段階。Tailwind の `text-*` に、Web 版が使う 11px・10px・9px を足したもの。
pub mod font_size {
    use super::{Pixels, px};

    pub const PX_9: Pixels = px(9.0);
    pub const PX_10: Pixels = px(10.0);
    pub const PX_11: Pixels = px(11.0);
    pub const XS: Pixels = px(12.0);
    pub const SM: Pixels = px(14.0);
    pub const BASE: Pixels = px(16.0);
    pub const LG: Pixels = px(18.0);
    pub const XL: Pixels = px(20.0);
    pub const XXL: Pixels = px(24.0);
    pub const XXXL: Pixels = px(30.0);
    pub const XXXXL: Pixels = px(36.0);
    pub const XXXXXL: Pixels = px(48.0);
    pub const XXXXXXL: Pixels = px(60.0);
    pub const XXXXXXXL: Pixels = px(72.0);
    pub const XXXXXXXXL: Pixels = px(96.0);
    pub const XXXXXXXXXL: Pixels = px(128.0);

    /// (名前、px)。ギャラリーが並べる。
    pub const SCALE: &[(&str, f32)] = &[
        ("9px", 9.0),
        ("10px", 10.0),
        ("11px", 11.0),
        ("xs", 12.0),
        ("sm", 14.0),
        ("base", 16.0),
        ("lg", 18.0),
        ("xl", 20.0),
        ("2xl", 24.0),
        ("3xl", 30.0),
        ("4xl", 36.0),
        ("5xl", 48.0),
        ("6xl", 60.0),
        ("7xl", 72.0),
        ("8xl", 96.0),
        ("9xl", 128.0),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [u8; 3], b: [u8; 3], tolerance: i16) -> bool {
        a.iter()
            .zip(b)
            .all(|(x, y)| (*x as i16 - y as i16).abs() <= tolerance)
    }

    #[test]
    fn oklch_white_and_black() {
        assert_eq!(to_u8(oklch_to_srgb(1.0, 0.0, 0.0)), [255, 255, 255]);
        assert_eq!(to_u8(oklch_to_srgb(0.0, 0.0, 0.0)), [0, 0, 0]);
    }

    #[test]
    fn oklch_matches_tailwind_reds() {
        // Tailwind v4 の red-400 と red-700 (oklch と hex の両方が公開されている)。
        assert!(close(
            to_u8(oklch_to_srgb(0.704, 0.191, 22.216)),
            [0xff, 0x64, 0x67],
            1
        ));
        assert!(close(
            to_u8(oklch_to_srgb(0.505, 0.213, 27.518)),
            [0xc1, 0x00, 0x07],
            1
        ));
    }

    #[test]
    fn oklch_pure_red_and_gray() {
        assert!(close(
            to_u8(oklch_to_srgb(0.627_955, 0.257_683, 29.233_89)),
            [255, 0, 0],
            1
        ));
        // CSS Color 4 の原色。
        assert!(close(
            to_u8(oklch_to_srgb(0.452, 0.313, 264.05)),
            [0, 0, 255],
            1
        ));
        assert!(close(
            to_u8(oklch_to_srgb(0.866, 0.295, 142.5)),
            [0, 255, 0],
            1
        ));
        // 無彩色は R=G=B になる。
        let [r, g, b] = to_u8(oklch_to_srgb(0.5, 0.0, 120.0));
        assert!(r == g && g == b);
    }

    #[test]
    fn oklch_out_of_gamut_is_clamped() {
        let c = oklch_to_srgb(0.9, 0.4, 150.0);
        for v in [c.r, c.g, c.b] {
            assert!((0.0..=1.0).contains(&v));
        }
    }

    #[test]
    fn hex_roundtrip() {
        assert_eq!(to_u8(hex(0xffd600)), [0xff, 0xd6, 0x00]);
    }

    #[test]
    fn theme_mode_parse() {
        assert_eq!(ThemeMode::parse("Dark"), Some(ThemeMode::Dark));
        assert_eq!(ThemeMode::parse(" light "), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::parse("sepia"), None);
    }

    #[test]
    fn radius_scale_matches_pixels() {
        assert_eq!(radius::LG, px(10.0));
        assert_eq!(radius::SM, px(6.0));
        assert_eq!(radius::XL, px(14.0));
    }

    // --- styles.css との突き合わせ ---

    const CSS: &str = include_str!("../../frontend/src/styles.css");

    /// `selector {` から対応する `}` までの `--name: value;` を順に返す。
    fn parse_block(css: &str, selector: &str) -> Vec<(String, String)> {
        // 先頭の桁から始まる行だけを最上位のセレクターとみなす。
        let header = format!("{selector} {{");
        let starts: Vec<usize> = css
            .lines()
            .enumerate()
            .filter(|(_, line)| line.trim_end() == header)
            .map(|(i, _)| i)
            .collect();
        assert!(
            starts.len() <= 1,
            "{selector} のブロックが styles.css の最上位に {} 回出ています。1 つにまとめてください",
            starts.len()
        );
        let start = *starts
            .first()
            .unwrap_or_else(|| panic!("{selector} のブロックが styles.css に見つかりません"));
        let mut vars = Vec::new();
        for line in css.lines().skip(start + 1) {
            let line = line.trim();
            if line == "}" {
                return vars;
            }
            if let Some(rest) = line.strip_prefix("--")
                && let Some((name, value)) = rest.split_once(':')
            {
                vars.push((
                    name.trim().to_string(),
                    value.trim().trim_end_matches(';').trim().to_string(),
                ));
            }
        }
        panic!("{selector} のブロックが閉じていません");
    }

    #[derive(Debug, PartialEq)]
    enum Value {
        Hex([u8; 3]),
        Oklch(f32, f32, f32),
        Other(String),
    }

    fn parse_value(value: &str, block: &[(String, String)]) -> Value {
        if let Some(name) = value
            .strip_prefix("var(--")
            .and_then(|v| v.strip_suffix(')'))
        {
            let target = block
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("var(--{name}) の参照先がありません"));
            return parse_value(&target.1, block);
        }
        if let Some(h) = value.strip_prefix('#')
            && h.len() == 6
        {
            let n = u32::from_str_radix(h, 16).expect("hex の色");
            let [_, r, g, b] = n.to_be_bytes();
            return Value::Hex([r, g, b]);
        }
        if let Some(args) = value
            .strip_prefix("oklch(")
            .and_then(|v| v.strip_suffix(')'))
        {
            let n: Vec<f32> = args
                .split_whitespace()
                .map(|p| p.parse().expect("oklch の数値"))
                .collect();
            assert_eq!(n.len(), 3, "oklch は 3 値のみ対応: {value}");
            return Value::Oklch(n[0], n[1], n[2]);
        }
        Value::Other(value.to_string())
    }

    fn check_block(css: &str, selector: &str, tokens: Tokens) -> Result<(), String> {
        let block = parse_block(css, selector);
        let entries: std::collections::HashMap<_, _> = tokens.entries().into_iter().collect();

        // CSS にあって tokens.rs に無い色変数はエラー。色でない変数は許可リストに載せる。
        for (name, value) in &block {
            if NON_COLOR_VARS.contains(&name.as_str()) {
                continue;
            }
            if !entries.contains_key(name.as_str()) {
                return Err(format!(
                    "{selector}: --{name}: {value} が tokens.rs にありません (色でなければ NON_COLOR_VARS に足す)"
                ));
            }
        }

        // tokens.rs が移したと言っている変数は、CSS の値と一致する。
        for (name, color) in &entries {
            let Some((_, raw)) = block.iter().find(|(n, _)| n == name) else {
                return Err(format!("{selector}: --{name} が styles.css にありません"));
            };
            let actual = to_u8(*color);
            match parse_value(raw, &block) {
                Value::Hex(expected) if actual != expected => {
                    return Err(format!(
                        "{selector}: --{name} は {raw} だが tokens.rs は {actual:?}"
                    ));
                }
                Value::Hex(_) => {}
                Value::Oklch(l, c, h) => {
                    let expected = to_u8(oklch_to_srgb(l, c, h));
                    if !close(actual, expected, 1) {
                        return Err(format!(
                            "{selector}: --{name} は {raw} ({expected:?}) だが tokens.rs は {actual:?}"
                        ));
                    }
                }
                Value::Other(v) => {
                    return Err(format!("{selector}: --{name} の値 {v} を読めません"));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn light_matches_css_root() {
        check_block(CSS, ":root", Tokens::light()).unwrap();
    }

    #[test]
    fn dark_matches_css_dark() {
        check_block(CSS, ".dark", Tokens::dark()).unwrap();
    }

    #[test]
    fn radius_matches_css() {
        let root = parse_block(CSS, ":root");
        let (_, radius) = root.iter().find(|(n, _)| n == "radius").unwrap();
        let rem: f32 = radius
            .strip_suffix("rem")
            .expect("radius は rem")
            .parse()
            .unwrap();
        assert_eq!(rem * 16.0, RADIUS_BASE);

        // @theme の `--radius-sm: calc(var(--radius) * 0.6)` などと倍率を突き合わせる。
        for (name, factor) in radius::SCALE {
            let key = format!("--radius-{name}:");
            let line = CSS
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with(&key))
                .unwrap_or_else(|| panic!("--radius-{name} が styles.css にありません"));
            let value = line[key.len()..].trim().trim_end_matches(';').trim();
            let expected = if value == "var(--radius)" {
                1.0
            } else {
                value
                    .strip_prefix("calc(var(--radius) * ")
                    .and_then(|v| v.strip_suffix(')'))
                    .unwrap_or_else(|| panic!("読めない radius: {value}"))
                    .parse()
                    .unwrap()
            };
            assert_eq!(*factor, expected, "--radius-{name}");
        }
    }

    #[test]
    fn drift_test_fails_on_a_new_css_color() {
        // styles.css に色変数が増えたのに tokens.rs へ足し忘れたら失敗する。
        let css = CSS.replacen(
            "--chart-5: #000000;",
            "--chart-5: #000000;\n  --chart-6: #123456;",
            1,
        );
        let err = check_block(&css, ":root", Tokens::light()).unwrap_err();
        assert!(err.contains("--chart-6"), "{err}");
    }

    #[test]
    fn drift_test_fails_on_a_changed_value() {
        let css = CSS.replacen("--primary: #ffd600;", "--primary: #ffd601;", 1);
        let err = check_block(&css, ":root", Tokens::light()).unwrap_err();
        assert!(err.contains("--primary"), "{err}");
        let css = CSS.replacen("oklch(0.704 0.191 22.216)", "oklch(0.704 0.191 30)", 1);
        assert!(check_block(&css, ".dark", Tokens::dark()).is_err());
    }

    #[test]
    fn non_color_allowlist_is_really_non_color() {
        let root = parse_block(CSS, ":root");
        for name in NON_COLOR_VARS {
            let (_, v) = root.iter().find(|(n, _)| n == name).unwrap();
            assert!(matches!(parse_value(v, &root), Value::Other(_)), "{name}");
        }
    }
}
