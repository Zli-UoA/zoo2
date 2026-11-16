//! 部品の木から、本文の代わり（`messages.content`）にする文字を作る。
//! 通知・プッシュ・検索・TUI・古いクライアントは、この文字だけを見る。

use crate::model::*;

impl Document {
    /// 部品の文字を改行でつなぎ、`max` 文字（Unicode のスカラー値）で切る。
    pub fn fallback_text(&self, max: usize) -> String {
        let mut lines = Vec::new();
        for c in &self.components {
            collect(c, &mut lines);
        }
        lines
            .into_iter()
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
            .chars()
            .take(max)
            .collect()
    }
}

fn collect(c: &Component, out: &mut Vec<String>) {
    use Component as C;
    match c {
        C::TextDisplay(t) => out.push(t.content.clone()),
        C::Label(l) => {
            out.push(l.label.clone());
            collect(&l.component, out);
        }
        C::Button(b) if b.custom_id.is_none() => {
            // リンクボタンは行き先も残す
            if let (Some(label), Some(url)) = (&b.label, &b.url) {
                out.push(format!("{label}: {url}"));
            }
        }
        C::Thumbnail(t) => out.extend(t.description.clone()),
        C::MediaGallery(g) => out.extend(g.items.iter().filter_map(|i| i.description.clone())),
        C::File(f) => out.push(f.name.clone().unwrap_or_else(|| f.file.url.clone())),
        C::Container(c) => out.extend([&c.title, &c.subtitle].into_iter().flatten().cloned()),
        C::Video(v) => out.push(format!(
            "{} {}",
            v.title,
            v.title_url.as_ref().unwrap_or(&v.video_url)
        )),
        C::Table(t) => {
            out.extend(t.caption.clone());
            out.extend(t.rows.iter().map(|r| {
                r.iter()
                    .map(|c| c.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" | ")
            }))
        }
        C::Alert(a) => out.push(a.content.clone()),
        C::Card(c) => out.extend(
            [&c.title, &c.subtitle, &c.body, &c.subtext]
                .into_iter()
                .flatten()
                .cloned(),
        ),
        C::Chart(c) => out.push(c.title.clone()),
        C::Grid(g) => out.extend(g.label.clone()),
        C::Plan(p) => {
            out.push(p.title.clone());
            out.extend(p.tasks.iter().map(|t| format!("- {}", t.title)));
        }
        C::TaskCard(t) => out.push(t.title.clone()),
        C::Unsupported(u) => out.push(u.text.clone()),
        _ => {}
    }
    for child in c.children() {
        if !matches!(c, C::Label(_) | C::Table(_)) {
            collect(child, out);
        }
    }
}
