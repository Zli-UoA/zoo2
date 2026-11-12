//! `zoo2_client_core::mfm` の解析結果を GPUI の `StyledText` 要素へ変換する。

use std::ops::Range;

use gpui::{
    Div, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveText, SharedString,
    StrikethroughStyle, StyledText, UnderlineStyle, div, prelude::*, px,
};
use zoo2_client_core::mfm::{self, Line, Style, Tone};

use crate::{login, theme};

/// MFM の `Style` を GPUI の `HighlightStyle` へ変換する (テーマ指定)。
pub fn style_to_highlight_with_theme(style: Style, theme: &theme::Theme) -> HighlightStyle {
    let mut highlight = HighlightStyle::default();

    let color = match style.tone {
        Tone::Default => None,
        Tone::Code => Some(theme.text_code.into()),
        Tone::Link | Tone::Hashtag => Some(theme.text_link.into()),
        Tone::Mention => Some(theme.accent.into()),
        Tone::Math => Some(theme.text_math.into()),
        Tone::Quote => Some(theme.text_muted.into()),
    };
    highlight.color = color;

    if style.reversed {
        highlight.background_color = Some(theme.bg_mention_mine.into());
        highlight.color = Some(theme.text_primary.into());
    } else if matches!(style.tone, Tone::Code) {
        highlight.background_color = Some(theme.bg_code.into());
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

/// MFM の `Style` を GPUI の `HighlightStyle` へ変換する (デフォルトダークテーマ)。
pub fn style_to_highlight(style: Style) -> HighlightStyle {
    style_to_highlight_with_theme(style, &theme::Theme::dark())
}

pub type HighlightSpan = (Range<usize>, HighlightStyle);
pub type LinkSpan = (Range<usize>, String);

/// 1行分の `mfm::Line` からプレーン文字列、GPUI ハイライト範囲、およびクリック可能なリンク範囲を構築する (テーマ指定)。
pub fn build_line_highlights_and_links_with_theme(
    line: &Line,
    theme: &theme::Theme,
) -> (String, Vec<HighlightSpan>, Vec<LinkSpan>) {
    let mut text = String::new();
    let mut highlights = Vec::new();
    let mut links = Vec::new();

    for (idx, span) in line.spans.iter().enumerate() {
        if span.content.is_empty() {
            continue;
        }
        let start = text.len();
        text.push_str(&span.content);
        let end = text.len();
        let highlight = style_to_highlight_with_theme(span.style, theme);
        if highlight != HighlightStyle::default() {
            highlights.push((start..end, highlight));
        }
        if matches!(span.style.tone, Tone::Link) {
            let target_url = line
                .spans
                .get(idx + 1)
                .filter(|next| next.style.dim)
                .and_then(|next| extract_bracketed_url(&next.content))
                .or_else(|| {
                    login::validate_http_url(&span.content).map(|_| span.content.trim().to_string())
                });
            if let Some(url) = target_url {
                links.push((start..end, url));
            }
        }
    }

    (text, highlights, links)
}

/// 1行分の `mfm::Line` からプレーン文字列、GPUI ハイライト範囲、およびクリック可能なリンク範囲を構築する。
pub fn build_line_highlights_and_links(line: &Line) -> (String, Vec<HighlightSpan>, Vec<LinkSpan>) {
    build_line_highlights_and_links_with_theme(line, &theme::Theme::dark())
}

/// 1行分の `mfm::Line` からプレーン文字列と GPUI ハイライト範囲の列を構築する。
pub fn build_line_highlights(line: &Line) -> (String, Vec<(Range<usize>, HighlightStyle)>) {
    let (text, highlights, _) = build_line_highlights_and_links(line);
    (text, highlights)
}

fn extract_bracketed_url(s: &str) -> Option<String> {
    let inner = s.strip_prefix(" <")?.strip_suffix('>')?;
    login::validate_http_url(inner).map(|_| inner.trim().to_string())
}

/// MFM 本文を解析して、GPUI の行要素 (`Div`) として描画する (テーマ指定)。
pub fn render_mfm_with_theme(
    elem_prefix: &str,
    content: &str,
    me_user_name: Option<&str>,
    theme: &theme::Theme,
) -> Div {
    let parsed = mfm::parse(content, me_user_name);
    let mut container = div().flex().flex_col().gap_0p5().w_full();

    for (line_idx, line) in parsed.iter().enumerate() {
        let (text, highlights, links) = build_line_highlights_and_links_with_theme(line, theme);
        if text.is_empty() {
            container = container.child(div().h(px(14.0)));
        } else {
            let styled = if highlights.is_empty() {
                StyledText::new(text)
            } else {
                StyledText::new(text).with_highlights(highlights)
            };
            let line_div = div()
                .w_full()
                .text_sm()
                .line_height(px(20.0))
                .text_color(theme.text_primary);
            if links.is_empty() {
                container = container.child(line_div.child(styled));
            } else {
                let ranges: Vec<Range<usize>> = links.iter().map(|(r, _)| r.clone()).collect();
                let urls: Vec<String> = links.into_iter().map(|(_, u)| u).collect();
                let interactive = InteractiveText::new(
                    ElementId::Name(SharedString::from(format!(
                        "{elem_prefix}-mfm-line-{line_idx}"
                    ))),
                    styled,
                )
                .on_click(ranges, move |clicked_idx, _window, _cx| {
                    if let Some(url) = urls.get(clicked_idx) {
                        login::open_browser(url);
                    }
                });
                container = container.child(line_div.child(interactive));
            }
        }
    }

    container
}

/// MFM 本文を解析して、GPUI の行要素 (`Div`) として描画する (デフォルトダークテーマ)。
pub fn render_mfm(elem_prefix: &str, content: &str, me_user_name: Option<&str>) -> Div {
    render_mfm_with_theme(elem_prefix, content, me_user_name, &theme::Theme::dark())
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

        // リンク範囲の抽出（直接 URL と [label](url)、およびラベル自体が URL の場合）
        let link_lines = mfm::parse(
            "詳細: [公式サイト](https://example.com/docs) または https://zli.chat と [https://a.example](https://b.example) を参照",
            None,
        );
        let (l_text, _, links) = build_line_highlights_and_links(&link_lines[0]);
        assert_eq!(links.len(), 3);
        assert_eq!(&l_text[links[0].0.clone()], "公式サイト");
        assert_eq!(links[0].1, "https://example.com/docs");
        assert_eq!(&l_text[links[1].0.clone()], "https://zli.chat");
        assert_eq!(links[1].1, "https://zli.chat");
        assert_eq!(&l_text[links[2].0.clone()], "https://a.example");
        assert_eq!(links[2].1, "https://b.example");
    }
}
