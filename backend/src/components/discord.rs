//! Discord の入力（Components V2 と、旧来の embeds）を部品の木にする。
//! text_display の Discord markdown のうち、MFM にない書き方を MFM に直す。

use serde_json::Value;
use zoo_components::legacy::plain;
use zoo_components::*;

/// Discord markdown を MFM に寄せる。
/// - `-# 小さい字` → `<small>`
/// - `||ネタバレ||` → `$[blur …]`
/// - `<t:1700000000:R>` → `$[unixtime 1700000000]`
/// - `>>> 以降すべて引用` → 各行に `>`
/// - `<@123>` などの ID の参照は、MFM のメンションにならないよう `<plain>` で包む
pub fn markdown_to_mfm(text: &str) -> String {
    let mut out = Vec::new();
    let mut quote_rest = false;
    for line in text.split('\n') {
        let line = if quote_rest {
            format!("> {line}")
        } else if let Some(rest) = line.strip_prefix(">>> ") {
            quote_rest = true;
            format!("> {rest}")
        } else if let Some(rest) = line.strip_prefix("-# ") {
            format!("<small>{rest}</small>")
        } else {
            line.to_string()
        };
        out.push(inline(&line));
    }
    out.join("\n")
}

fn inline(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    loop {
        let spoiler = rest.find("||");
        let angle = rest.find('<');
        match (spoiler, angle) {
            (Some(i), a) if a.is_none_or(|a| i < a) => {
                if let Some(end) = rest[i + 2..].find("||") {
                    out.push_str(&rest[..i]);
                    out.push_str(&format!("$[blur {}]", &rest[i + 2..i + 2 + end]));
                    rest = &rest[i + 2 + end + 2..];
                } else {
                    out.push_str(rest);
                    break;
                }
            }
            (_, Some(a)) => {
                out.push_str(&rest[..a]);
                let tail = &rest[a..];
                let Some(end) = tail.find('>') else {
                    out.push_str(tail);
                    break;
                };
                let token = &tail[1..end];
                out.push_str(&convert_token(token).unwrap_or_else(|| tail[..=end].to_string()));
                rest = &tail[end + 1..];
            }
            _ => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

fn convert_token(token: &str) -> Option<String> {
    if let Some(t) = token.strip_prefix("t:") {
        let secs = t.split(':').next()?;
        return secs.parse::<i64>().ok().map(|s| format!("$[unixtime {s}]"));
    }
    for (prefix, mark) in [("@&", "@"), ("@!", "@"), ("@", "@"), ("#", "#")] {
        if let Some(id) = token.strip_prefix(prefix)
            && !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_digit())
        {
            return Some(format!("<plain>{mark}{id}</plain>"));
        }
    }
    // カスタム絵文字 <:name:id> と <a:name:id> は :name: にする
    let parts: Vec<&str> = token.split(':').collect();
    if parts.len() == 3
        && (parts[0].is_empty() || parts[0] == "a")
        && parts[2].bytes().all(|b| b.is_ascii_digit())
    {
        return Some(format!(":{}:", parts[1]));
    }
    None
}

/// 文書の text_display などに含まれる Discord markdown を MFM にする。
pub fn normalize(doc: &mut Document) {
    fn go(c: &mut Component) {
        match c {
            Component::TextDisplay(t) => t.content = markdown_to_mfm(&t.content),
            Component::Alert(a) => a.content = markdown_to_mfm(&a.content),
            _ => {}
        }
        for child in c.children_mut() {
            go(child);
        }
    }
    doc.components.iter_mut().for_each(go);
}

fn s<'v>(v: &'v Value, k: &str) -> &'v str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

fn https(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|u| zoo_components::validate::is_https_url(u))
        .map(str::to_string)
}

/// Discord の embeds（旧来のリッチな投稿）を container にする。
pub fn embeds(list: &[Value]) -> Vec<Component> {
    let mut out = Vec::new();
    for e in list.iter().take(10) {
        let mut parts = Vec::new();
        if let Some(author) = e.get("author") {
            let name = plain(s(author, "name"));
            if !name.is_empty() {
                parts.push(Component::text(match https(author, "url") {
                    Some(u) => format!("<small>[{name}]({u})</small>"),
                    None => format!("<small>{name}</small>"),
                }));
            }
        }
        if !s(e, "title").is_empty() {
            let title = markdown_to_mfm(s(e, "title"));
            parts.push(Component::text(match https(e, "url") {
                Some(u) => format!("**[{title}]({u})**"),
                None => format!("**{title}**"),
            }));
        }
        if !s(e, "description").is_empty() {
            parts.push(Component::text(markdown_to_mfm(s(e, "description"))));
        }
        let fields: Vec<String> = e
            .get("fields")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .take(25)
            .map(|f| {
                format!(
                    "**{}**: {}",
                    markdown_to_mfm(s(f, "name")),
                    markdown_to_mfm(s(f, "value"))
                )
            })
            .collect();
        if !fields.is_empty() {
            parts.push(Component::text(fields.join("\n")));
        }
        let mut parts = legacy::merge_texts(parts);
        if let Some(thumb) = e.get("thumbnail").and_then(|t| https(t, "url"))
            && let Some(Component::TextDisplay(_)) = parts.last()
        {
            let last = parts.pop().unwrap();
            parts.push(Component::Section(Section {
                id: None,
                components: vec![last],
                accessory: Some(Box::new(Component::Thumbnail(Thumbnail {
                    media: Media {
                        url: thumb,
                        ..Default::default()
                    },
                    ..Default::default()
                }))),
            }));
        }
        if let Some(image) = e.get("image").and_then(|t| https(t, "url")) {
            parts.push(Component::MediaGallery(MediaGallery {
                id: None,
                items: vec![GalleryItem {
                    media: Media {
                        url: image,
                        ..Default::default()
                    },
                    description: None,
                    spoiler: false,
                }],
            }));
        }
        let mut footer = Vec::new();
        if let Some(f) = e.get("footer") {
            let text = plain(s(f, "text"));
            if !text.is_empty() {
                footer.push(text);
            }
        }
        if let Some(ts) = e
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|t| t.parse::<jiff::Timestamp>().ok())
        {
            footer.push(format!("$[unixtime {}]", ts.as_second()));
        }
        if !footer.is_empty() {
            parts.push(Component::Separator(Separator {
                divider: false,
                ..Default::default()
            }));
            parts.push(Component::text(format!(
                "<small>{}</small>",
                footer.join(" · ")
            )));
        }
        if parts.is_empty() {
            continue;
        }
        out.push(Component::Container(Container {
            accent_color: e
                .get("color")
                .and_then(Value::as_u64)
                .filter(|c| *c <= 0xFF_FFFF)
                .map(|c| c as u32),
            components: parts,
            ..Default::default()
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_discord_only_markdown() {
        assert_eq!(markdown_to_mfm("-# small"), "<small>small</small>");
        assert_eq!(markdown_to_mfm("a ||secret|| b"), "a $[blur secret] b");
        assert_eq!(
            markdown_to_mfm("at <t:1700000000:R>"),
            "at $[unixtime 1700000000]"
        );
        assert_eq!(
            markdown_to_mfm("hi <@123> in <#45> <:wave:99>"),
            "hi <plain>@123</plain> in <plain>#45</plain> :wave:"
        );
        assert_eq!(markdown_to_mfm(">>> a\nb"), "> a\n> b");
        assert_eq!(
            markdown_to_mfm("**bold** [x](https://e.x) <b>keep</b>"),
            "**bold** [x](https://e.x) <b>keep</b>"
        );
    }

    #[test]
    fn embeds_become_containers() {
        let c = embeds(&[
            json!({"title":"Build","url":"https://ci.example/1","description":"ok","color":3066993,
            "fields":[{"name":"Branch","value":"main","inline":true}],"footer":{"text":"CI"},"timestamp":"2026-11-16T00:00:00Z",
            "thumbnail":{"url":"https://e.x/t.png"}}),
        ]);
        let doc = Document::new(IS_COMPONENTS_V2, c);
        assert!(
            doc.validate(Context::Webhook).is_ok(),
            "{:?}",
            doc.validate(Context::Webhook)
        );
        assert!(
            doc.fallback_text(4000)
                .contains("**[Build](https://ci.example/1)**")
        );
    }
}
