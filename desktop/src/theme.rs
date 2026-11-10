//! デスクトップ版の配色定義。

use gpui::{Hsla, Rgba, hsla, rgba};

pub const fn hex_rgb(hex: u32) -> Rgba {
    let [_, r, g, b] = hex.to_be_bytes();
    Rgba {
        r: (r as f32) / 255.0,
        g: (g as f32) / 255.0,
        b: (b as f32) / 255.0,
        a: 1.0,
    }
}

/// メイン背景色 (タイムライン・ログイン画面)
pub const BG_PRIMARY: Rgba = hex_rgb(0x0f172a);
/// サイドバー・スレッドペイン背景色
pub const BG_SECONDARY: Rgba = hex_rgb(0x1e293b);
/// カード・入力欄・ヘッダー背景色
pub const BG_SURFACE: Rgba = hex_rgb(0x162032);
/// 選択中・アクティブ行の背景色
pub const BG_SELECTED: Rgba = hex_rgb(0x283548);
/// ホバー時の背景色
pub const BG_HOVER: Rgba = hex_rgb(0x243044);
/// コードブロック・インラインコードの背景色
pub const BG_CODE: Rgba = hex_rgb(0x1e293b);
/// 自分宛てメンションの背景色
pub const BG_MENTION_MINE: Rgba = hex_rgb(0x164e63);
/// 自分が付けたリアクションピルの背景色
pub const BG_REACTION_MINE: Rgba = hex_rgb(0x153e52);

/// 境界線色
pub const BORDER: Rgba = hex_rgb(0x334155);
/// 強調境界線色 (フォーカス中・自分のリアクションなど)
pub const BORDER_ACCENT: Rgba = hex_rgb(0x06b6d4);

/// 主要テキスト色
pub const TEXT_PRIMARY: Rgba = hex_rgb(0xf8fafc);
/// 補助テキスト色 (時刻・説明・プレースホルダー)
pub const TEXT_MUTED: Rgba = hex_rgb(0x94a3b8);
/// 控えめなテキスト色 (未参加チャンネル・アーカイブなど)
pub const TEXT_DIM: Rgba = hex_rgb(0x64748b);

/// アクセント色 (Cyan)
pub const ACCENT: Rgba = hex_rgb(0x22d3ee);
/// ボタン用アクセント背景色
pub const ACCENT_BUTTON: Rgba = hex_rgb(0x0891b2);
/// ボタンホバー用アクセント背景色
pub const ACCENT_BUTTON_HOVER: Rgba = hex_rgb(0x06b6d4);

/// コード文字色 (Amber)
pub const TEXT_CODE: Rgba = hex_rgb(0xfbbf24);
/// リンク・ハッシュタグ文字色 (Sky Blue)
pub const TEXT_LINK: Rgba = hex_rgb(0x38bdf8);
/// 数式文字色 (Emerald)
pub const TEXT_MATH: Rgba = hex_rgb(0x34d399);
/// 警告・通知文字色
pub const WARNING: Rgba = hex_rgb(0xfacc15);
/// 危険・削除ボタン色
pub const DANGER: Rgba = hex_rgb(0xef4444);
/// 新バージョン通知などの成功色
pub const SUCCESS: Rgba = hex_rgb(0x4ade80);

/// テキスト選択範囲のハイライト色
pub fn selection_bg() -> Hsla {
    rgba(0x22d3ee44).into()
}

/// プレースホルダー文字色
pub fn placeholder_color() -> Hsla {
    hsla(215.0 / 360.0, 0.20, 0.55, 0.7)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub fn toggle(self) -> Self {
        match self {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Dark => "ダーク",
            ThemeMode::Light => "ライト",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub mode: ThemeMode,
    pub bg_primary: Rgba,
    pub bg_secondary: Rgba,
    pub bg_surface: Rgba,
    pub bg_selected: Rgba,
    pub bg_hover: Rgba,
    pub bg_code: Rgba,
    pub bg_mention_mine: Rgba,
    pub bg_reaction_mine: Rgba,
    pub border: Rgba,
    pub border_accent: Rgba,
    pub text_primary: Rgba,
    pub text_muted: Rgba,
    pub text_dim: Rgba,
    pub accent: Rgba,
    pub accent_button: Rgba,
    pub accent_button_hover: Rgba,
    pub text_on_accent: Rgba,
    pub text_code: Rgba,
    pub text_link: Rgba,
    pub text_math: Rgba,
    pub warning: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
}

impl Theme {
    pub fn selection_bg(&self) -> Hsla {
        match self.mode {
            ThemeMode::Dark => rgba(0x22d3ee44).into(),
            ThemeMode::Light => rgba(0x0284c744).into(),
        }
    }

    pub fn placeholder_color(&self) -> Hsla {
        match self.mode {
            ThemeMode::Dark => hsla(215.0 / 360.0, 0.20, 0.55, 0.7),
            ThemeMode::Light => hsla(215.0 / 360.0, 0.20, 0.45, 0.7),
        }
    }

    pub fn from_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Dark => Self::dark(),
            ThemeMode::Light => Self::light(),
        }
    }

    pub const fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            bg_primary: BG_PRIMARY,
            bg_secondary: BG_SECONDARY,
            bg_surface: BG_SURFACE,
            bg_selected: BG_SELECTED,
            bg_hover: BG_HOVER,
            bg_code: BG_CODE,
            bg_mention_mine: BG_MENTION_MINE,
            bg_reaction_mine: BG_REACTION_MINE,
            border: BORDER,
            border_accent: BORDER_ACCENT,
            text_primary: TEXT_PRIMARY,
            text_muted: TEXT_MUTED,
            text_dim: TEXT_DIM,
            accent: ACCENT,
            accent_button: ACCENT_BUTTON,
            accent_button_hover: ACCENT_BUTTON_HOVER,
            text_on_accent: hex_rgb(0xffffff),
            text_code: TEXT_CODE,
            text_link: TEXT_LINK,
            text_math: TEXT_MATH,
            warning: WARNING,
            danger: DANGER,
            success: SUCCESS,
        }
    }

    pub const fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            bg_primary: hex_rgb(0xffffff),
            bg_secondary: hex_rgb(0xf1f5f9),
            bg_surface: hex_rgb(0xf8fafc),
            bg_selected: hex_rgb(0xe2e8f0),
            bg_hover: hex_rgb(0xf1f5f9),
            bg_code: hex_rgb(0xe2e8f0),
            bg_mention_mine: hex_rgb(0xe0f2fe),
            bg_reaction_mine: hex_rgb(0xbae6fd),
            border: hex_rgb(0xcbd5e1),
            border_accent: hex_rgb(0x0284c7),
            text_primary: hex_rgb(0x0f172a),
            text_muted: hex_rgb(0x64748b),
            text_dim: hex_rgb(0x94a3b8),
            accent: hex_rgb(0x0284c7),
            accent_button: hex_rgb(0x0284c7),
            accent_button_hover: hex_rgb(0x0369a1),
            text_on_accent: hex_rgb(0xffffff),
            text_code: hex_rgb(0xd97706),
            text_link: hex_rgb(0x0284c7),
            text_math: hex_rgb(0x059669),
            warning: hex_rgb(0xd97706),
            danger: hex_rgb(0xdc2626),
            success: hex_rgb(0x16a34a),
        }
    }
}
