//! `zoo2_client_core::mfm` の解析結果を GPUI の `StyledText` 要素へ変換する。

use std::ops::Range;

use gpui::{
    Div, ElementId, FontStyle, FontWeight, HighlightStyle, InteractiveText, SharedString,
    StrikethroughStyle, StyledText, UnderlineStyle, div, prelude::*, px,
};
use zoo2_client_core::mfm::{self, Line, Style, Tone};

use crate::{login, theme, tokens::Tokens};

fn account_fallback(initial: String) -> Div {
    div()
        .w(px(18.0))
        .h(px(18.0))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .child(initial)
}

/// MFM の描画に使う色と行の高さ。旧テーマ (`theme::Theme`) とトークン (`tokens::Tokens`) のどちらからも作れる。
#[derive(Clone)]
pub struct MfmColors {
    pub text: gpui::Rgba,
    pub code: gpui::Rgba,
    pub link: gpui::Rgba,
    pub mention: gpui::Rgba,
    pub math: gpui::Rgba,
    pub quote: gpui::Rgba,
    pub code_bg: gpui::Rgba,
    /// 自分あてのメンション (反転) の背景と文字。
    pub reversed_bg: gpui::Rgba,
    pub reversed_fg: gpui::Rgba,
    /// メンションの背景 (トークン版だけ。`bg-brand-muted`)。
    pub mention_bg: Option<gpui::Rgba>,
    /// 1 行の高さ (px)。
    pub line_height: f32,
    /// 行と行の間 (px)。
    pub line_gap: f32,
    /// `#名前` を、あるチャンネルの ID に結ぶ表 (`MfmChannelsProvider`)。無ければリンクにしない。
    pub channels: Option<std::rc::Rc<std::collections::HashMap<String, String>>>,
    /// チャンネルのリンクを押したときの動作 (チャンネル ID を渡す)。
    pub on_channel: Option<ChannelOpener>,
}

/// チャンネルのリンクを押したときに呼ぶ関数。
pub type ChannelOpener = std::rc::Rc<dyn Fn(&str, &mut gpui::App)>;

/// チャンネルのリンクの行き先に付ける接頭辞 (ブラウザの URL と取り違えない)。
const CHANNEL_SCHEME: &str = "zoo2-channel:";

impl MfmColors {
    pub fn from_theme(theme: &theme::Theme) -> Self {
        Self {
            text: theme.text_primary,
            code: theme.text_code,
            link: theme.text_link,
            mention: theme.accent,
            math: theme.text_math,
            quote: theme.text_muted,
            code_bg: theme.bg_code,
            reversed_bg: theme.bg_mention_mine,
            reversed_fg: theme.text_primary,
            mention_bg: None,
            line_height: 20.0,
            line_gap: 2.0,
            channels: None,
            on_channel: None,
        }
    }

    /// frontend の `MfmContent.tsx` に合わせた色。メンションは `bg-brand-muted text-brand-muted-foreground`、
    /// インラインコードは `bg-muted`。本文は `text-sm leading-relaxed` (14px / 22.75px)。
    pub fn from_tokens(t: &Tokens) -> Self {
        Self {
            text: t.foreground,
            code: t.foreground,
            link: t.link,
            mention: t.brand_muted_foreground,
            math: t.foreground,
            quote: t.muted_foreground,
            code_bg: t.muted,
            reversed_bg: t.brand_muted,
            reversed_fg: t.brand_muted_foreground,
            mention_bg: Some(t.brand_muted),
            line_height: 22.75,
            line_gap: 0.0,
            channels: None,
            on_channel: None,
        }
    }
}

/// MFM の `Style` を GPUI の `HighlightStyle` へ変換する (色指定)。
pub fn style_to_highlight_with_colors(style: Style, colors: &MfmColors) -> HighlightStyle {
    let mut highlight = HighlightStyle::default();

    let color = match style.tone {
        Tone::Default => None,
        Tone::Code => Some(colors.code.into()),
        Tone::Link | Tone::Hashtag => Some(colors.link.into()),
        Tone::Mention => Some(colors.mention.into()),
        Tone::Math => Some(colors.math.into()),
        Tone::Quote => Some(colors.quote.into()),
    };
    highlight.color = color;

    if style.reversed {
        highlight.background_color = Some(colors.reversed_bg.into());
        highlight.color = Some(colors.reversed_fg.into());
    } else if matches!(style.tone, Tone::Code) {
        highlight.background_color = Some(colors.code_bg.into());
    } else if matches!(style.tone, Tone::Mention)
        && let Some(bg) = colors.mention_bg
    {
        highlight.background_color = Some(bg.into());
        // `font-medium`
        highlight.font_weight = Some(FontWeight::MEDIUM);
    }

    // メンションの太さは `font-medium` (上で決めた)。ほかは本文の太字に従う。
    let medium_mention = colors.mention_bg.is_some() && matches!(style.tone, Tone::Mention);
    if style.bold && !medium_mention {
        highlight.font_weight = Some(FontWeight::BOLD);
    }
    if style.italic {
        highlight.font_style = Some(FontStyle::Italic);
    }
    // 本文のリンクは `underline`。
    let underlined = style.underlined
        || (colors.mention_bg.is_some() && matches!(style.tone, Tone::Link | Tone::Hashtag));
    if underlined {
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

/// MFM の `Style` を GPUI の `HighlightStyle` へ変換する (テーマ指定)。
pub fn style_to_highlight_with_theme(style: Style, theme: &theme::Theme) -> HighlightStyle {
    style_to_highlight_with_colors(style, &MfmColors::from_theme(theme))
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
    build_line_highlights_and_links_with_colors(line, &MfmColors::from_theme(theme))
}

/// ハッシュタグが開けるチャンネルを指すなら、そのチャンネルの ID。
fn known_channel_id<'a>(content: &str, colors: &'a MfmColors) -> Option<&'a String> {
    colors
        .channels
        .as_ref()
        .and_then(|map| map.get(content.trim_start_matches('#')))
}

/// 表が渡されていて、そこに無い `#名前` は、本文と同じ見た目にする (frontend の `ChannelHashtag`)。
fn span_style(span: &mfm::Span, colors: &MfmColors) -> Style {
    let mut style = span.style;
    if matches!(style.tone, Tone::Hashtag)
        && colors.channels.is_some()
        && known_channel_id(&span.content, colors).is_none()
    {
        style.tone = Tone::Default;
    }
    style
}

/// 1行分の `mfm::Line` からプレーン文字列、ハイライト範囲、リンク範囲を構築する (色指定)。
pub fn build_line_highlights_and_links_with_colors(
    line: &Line,
    colors: &MfmColors,
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
        let highlight = style_to_highlight_with_colors(span_style(span, colors), colors);
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
        } else if matches!(span.style.tone, Tone::Hashtag)
            && let Some(id) = known_channel_id(&span.content, colors)
        {
            links.push((start..end, format!("{CHANNEL_SCHEME}{id}")));
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
    render_mfm_accounts_with_theme(
        elem_prefix,
        content,
        me_user_name,
        theme,
        &Default::default(),
    )
}
pub fn render_mfm_accounts_with_theme(
    elem_prefix: &str,
    content: &str,
    me_user_name: Option<&str>,
    theme: &theme::Theme,
    accounts: &zoo2_client_core::store::AccountProfiles,
) -> Div {
    render_mfm_accounts_with_colors(
        elem_prefix,
        content,
        me_user_name,
        &MfmColors::from_theme(theme),
        accounts,
    )
}

/// MFM 本文を解析して、GPUI の行要素 (`Div`) として描画する (色指定)。
pub fn render_mfm_accounts_with_colors(
    elem_prefix: &str,
    content: &str,
    me_user_name: Option<&str>,
    colors: &MfmColors,
    accounts: &zoo2_client_core::store::AccountProfiles,
) -> Div {
    let parsed = mfm::parse_with_accounts(content, me_user_name, accounts);
    let mut container = div().flex().flex_col().gap(px(colors.line_gap)).w_full();

    for (line_idx, line) in parsed.iter().enumerate() {
        if line.spans.iter().any(|span| span.avatar_fallback.is_some()) {
            let mut row = div()
                .flex()
                .flex_wrap()
                .items_center()
                .w_full()
                .text_sm()
                .line_height(px(colors.line_height))
                .text_color(colors.text);
            for (index, span) in line.spans.iter().enumerate() {
                if let Some(url) = span
                    .avatar_url
                    .as_ref()
                    .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
                {
                    let initial = span
                        .avatar_fallback
                        .clone()
                        .unwrap_or_else(|| "?".to_owned());
                    row = row.child(
                        gpui::img(url.clone())
                            .with_fallback(move || {
                                account_fallback(initial.clone()).into_any_element()
                            })
                            .w(px(18.0))
                            .h(px(18.0))
                            .rounded_full(),
                    );
                } else if let Some(initial) = &span.avatar_fallback {
                    row = row.child(account_fallback(initial.clone()));
                } else {
                    let styled = StyledText::new(span.content.clone()).with_highlights(vec![(
                        0..span.content.len(),
                        style_to_highlight_with_colors(span_style(span, colors), colors),
                    )]);
                    let channel_id = matches!(span.style.tone, Tone::Hashtag)
                        .then(|| known_channel_id(&span.content, colors).cloned())
                        .flatten();
                    if let Some(id) = channel_id {
                        let on_channel = colors.on_channel.clone();
                        row = row.child(
                            InteractiveText::new(
                                ElementId::Name(
                                    format!("{elem_prefix}-channel-{line_idx}-{index}").into(),
                                ),
                                styled,
                            )
                            .on_click(
                                std::iter::once(0..span.content.len()).collect(),
                                move |_, _, cx| {
                                    if let Some(open) = &on_channel {
                                        open(&id, cx);
                                    }
                                },
                            ),
                        );
                    } else if let Some(url) = &span.href {
                        let url = url.clone();
                        row = row.child(
                            InteractiveText::new(
                                ElementId::Name(
                                    format!("{elem_prefix}-account-{line_idx}-{index}").into(),
                                ),
                                styled,
                            )
                            .on_click(
                                std::iter::once(0..span.content.len()).collect(),
                                move |_, _, _| {
                                    login::open_browser(&url);
                                },
                            ),
                        );
                    } else {
                        row = row.child(styled);
                    }
                }
            }
            container = container.child(row);
            continue;
        }
        let (text, highlights, links) = build_line_highlights_and_links_with_colors(line, colors);
        if text.is_empty() {
            let blank = if colors.mention_bg.is_some() {
                colors.line_height
            } else {
                14.0
            };
            container = container.child(div().h(px(blank)));
        } else {
            let styled = if highlights.is_empty() {
                StyledText::new(text)
            } else {
                StyledText::new(text).with_highlights(highlights)
            };
            let line_div = div()
                .w_full()
                .text_sm()
                .line_height(px(colors.line_height))
                .text_color(colors.text);
            if links.is_empty() {
                container = container.child(line_div.child(styled));
            } else {
                let ranges: Vec<Range<usize>> = links.iter().map(|(r, _)| r.clone()).collect();
                let urls: Vec<String> = links.into_iter().map(|(_, u)| u).collect();
                let on_channel = colors.on_channel.clone();
                let interactive = InteractiveText::new(
                    ElementId::Name(SharedString::from(format!(
                        "{elem_prefix}-mfm-line-{line_idx}"
                    ))),
                    styled,
                )
                .on_click(ranges, move |clicked_idx, _window, cx| {
                    let Some(url) = urls.get(clicked_idx) else {
                        return;
                    };
                    match url.strip_prefix(CHANNEL_SCHEME) {
                        Some(id) => {
                            if let Some(open) = &on_channel {
                                open(id, cx);
                            }
                        }
                        None => {
                            login::open_browser(url);
                        }
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

    #[test]
    fn hashtags_link_only_to_known_channels() {
        let tokens = Tokens::light();
        let mut colors = MfmColors::from_tokens(&tokens);
        colors.channels = Some(std::rc::Rc::new(
            [("general".to_string(), "c1".to_string())].into(),
        ));
        let lines = mfm::parse("#general と #nope", None);
        let (text, _, links) = build_line_highlights_and_links_with_colors(&lines[0], &colors);
        assert_eq!(links.len(), 1);
        assert_eq!(&text[links[0].0.clone()], "#general");
        assert_eq!(links[0].1, "zoo2-channel:c1");
        // 表にある #general はリンク色、無い #nope は本文と同じ見た目 (ハイライトなし)。
        let range_of = |word: &str| text.find(word).map(|i| i..i + word.len());
        let general = range_of("#general").unwrap();
        let nope = range_of("#nope").unwrap();
        let (_, highlights, _) = build_line_highlights_and_links_with_colors(&lines[0], &colors);
        assert!(highlights.iter().any(|(r, _)| *r == general));
        assert!(!highlights.iter().any(|(r, _)| *r == nope));
        // 表が無ければ、どれもリンクにしない。
        let plain = MfmColors::from_tokens(&tokens);
        let (_, _, links) = build_line_highlights_and_links_with_colors(&lines[0], &plain);
        assert!(links.is_empty());
    }

    #[test]
    fn mentions_use_brand_muted_with_tokens() {
        let tokens = Tokens::light();
        let colors = MfmColors::from_tokens(&tokens);
        let lines = mfm::parse("@bob @alice", Some("alice"));
        let (text, highlights, _) = build_line_highlights_and_links_with_colors(&lines[0], &colors);
        let bob = highlights
            .iter()
            .find(|(range, _)| &text[range.clone()] == "@bob")
            .expect("@bob");
        assert_eq!(bob.1.background_color, Some(tokens.brand_muted.into()));
        assert_eq!(bob.1.color, Some(tokens.brand_muted_foreground.into()));
        assert_eq!(bob.1.font_weight, Some(FontWeight::MEDIUM));
        // 旧テーマは今までどおり (背景なし)。
        let (_, old, _) =
            build_line_highlights_and_links_with_theme(&lines[0], &theme::Theme::dark());
        assert!(
            old.iter()
                .find(|(r, _)| &text[r.clone()] == "@bob")
                .is_some_and(|(_, h)| h.background_color.is_none())
        );
    }
}
