//! デスクトップ版の配色定義。

use gpui::{Hsla, Rgba, hsla, rgba};

const fn hex_rgb(hex: u32) -> Rgba {
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
