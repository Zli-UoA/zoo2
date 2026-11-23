//! `zoo2-client-core::mfm` の解析結果を `ratatui` の `Line` / `Span` に変換する。

pub use core_mfm::set_emoji_catalog;
use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use zoo2_client_core::mfm::{self as core_mfm, Tone};

/// 本文を行ごとのスタイル付きテキストにする。`me` は自分の `user_name` で、自分宛ての
/// メンションを反転表示する。
pub fn render(content: &str, me: Option<&str>) -> Vec<Line<'static>> {
    render_with_accounts(content, me, &Default::default())
}
pub fn render_with_accounts(
    content: &str,
    me: Option<&str>,
    profiles: &zoo2_client_core::store::AccountProfiles,
) -> Vec<Line<'static>> {
    core_mfm::parse_with_accounts(content, me, profiles)
        .into_iter()
        .map(|line| {
            Line::from(
                line.spans
                    .into_iter()
                    .map(|span| Span::styled(span.content, to_ratatui_style(span.style)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn to_ratatui_style(style: core_mfm::Style) -> Style {
    let mut out = match style.tone {
        Tone::Default => Style::new(),
        Tone::Code => Style::new().fg(Color::Yellow),
        Tone::Link => Style::new().fg(Color::Blue),
        Tone::Mention | Tone::Hashtag => Style::new().fg(Color::Cyan),
        Tone::Math => Style::new().fg(Color::Magenta),
        Tone::Quote => Style::new().fg(Color::DarkGray),
    };
    if style.bold {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.italic {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.underlined {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    if style.crossed_out {
        out = out.add_modifier(Modifier::CROSSED_OUT);
    }
    if style.dim {
        out = out.add_modifier(Modifier::DIM);
    }
    if style.reversed {
        out = out.add_modifier(Modifier::REVERSED);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_core_mfm_styles_to_ratatui_lines() {
        let lines = render("hi **@alice** `code`", Some("alice"));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].to_string(), "hi @alice code");
    }
}
