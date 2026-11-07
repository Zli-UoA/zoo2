//! `zoo2_client_core::mfm` の解析結果を GPUI の `StyledText` 要素へ変換する。

use std::ops::Range;

use gpui::{
    Div, FontStyle, FontWeight, HighlightStyle, StrikethroughStyle, StyledText, UnderlineStyle,
    div, prelude::*, px,
};
use zoo2_client_core::mfm::{self, Line, Style, Tone};

use crate::theme;

/// MFM の `Style` を GPUI の `HighlightStyle` へ変換する。
pub fn style_to_highlight(style: Style) -> HighlightStyle {
    let mut highlight = HighlightStyle::default();

    let color = match style.tone {
        Tone::Default => None,
        Tone::Code => Some(theme::TEXT_CODE.into()),
        Tone::Link | Tone::Hashtag => Some(theme::TEXT_LINK.into()),
        Tone::Mention => Some(theme::ACCENT.into()),
        Tone::Math => Some(theme::TEXT_MATH.into()),
        Tone::Quote => Some(theme::TEXT_MUTED.into()),
    };
    highlight.color = color;

    if style.reversed {
        highlight.background_color = Some(theme::BG_MENTION_MINE.into());
        highlight.color = Some(theme::TEXT_PRIMARY.into());
    } else if matches!(style.tone, Tone::Code) {
        highlight.background_color = Some(theme::BG_CODE.into());
    }

    if style.bold {
        highlight.font_weight = Some(FontWeight::BOLD);
    }
    if style.italic {
        highlight.font_style = Some(FontStyle::Italic);
    }
    if style.underlined {
        highlight.underline = Some(UnderlineStyle {
            thickness: px(1.0),
            color,
            wavy: false,
        });
    }
    if style.crossed_out {
        highlight.strikethrough = Some(StrikethroughStyle {
            thickness: px(1.0),
            color,
        });
    }
    if style.dim {
        highlight.fade_out = Some(0.45);
    }

    highlight
}

/// 1行分の `mfm::Line` からプレーン文字列と GPUI ハイライト範囲の列を構築する。
pub fn build_line_highlights(line: &Line) -> (String, Vec<(Range<usize>, HighlightStyle)>) {
    let mut text = String::new();
    let mut highlights = Vec::new();

    for span in &line.spans {
        if span.content.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(&span.content);
        let end = text.len();
        let highlight = style_to_highlight(span.style);
        if highlight != HighlightStyle::default() {
            highlights.push((start..end, highlight));
        }
    }

    (text, highlights)
}

/// MFM 本文を解析して、GPUI の行要素 (`Div`) として描画する。
pub fn render_mfm(content: &str, me_user_name: Option<&str>) -> Div {
    let parsed = mfm::parse(content, me_user_name);
    let mut container = div().flex().flex_col().gap_0p5().w_full();

    for line in &parsed {
        let (text, highlights) = build_line_highlights(line);
        if text.is_empty() {
            container = container.child(div().h(px(14.0)));
        } else {
            let styled = if highlights.is_empty() {
                StyledText::new(text)
            } else {
                StyledText::new(text).with_highlights(highlights)
            };
            container = container.child(
                div()
                    .w_full()
                    .text_sm()
                    .line_height(px(20.0))
                    .text_color(theme::TEXT_PRIMARY)
                    .child(styled),
            );
        }
    }

    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_mfm_styles_to_gpui_highlights() {
        let lines = mfm::parse("**bold** @alice `code` ~~strike~~", Some("alice"));
        assert_eq!(lines.len(), 1);
        let (text, highlights) = build_line_highlights(&lines[0]);
        assert_eq!(text, "bold @alice code strike");
        assert_eq!(highlights.len(), 4);

        // "bold"
        assert_eq!(&text[highlights[0].0.clone()], "bold");
        assert_eq!(highlights[0].1.font_weight, Some(FontWeight::BOLD));

        // "@alice" (自分宛てメンションは反転背景 + 太字)
        assert_eq!(&text[highlights[1].0.clone()], "@alice");
        assert_eq!(highlights[1].1.font_weight, Some(FontWeight::BOLD));
        assert!(highlights[1].1.background_color.is_some());

        // "code"
        assert_eq!(&text[highlights[2].0.clone()], "code");
        assert!(highlights[2].1.background_color.is_some());

        // "strike"
        assert_eq!(&text[highlights[3].0.clone()], "strike");
        assert!(highlights[3].1.strikethrough.is_some());
    }
}
