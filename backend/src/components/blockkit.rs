//! Slack Block Kit（`blocks`）を部品の木にする。
//!
//! - App API（[`Mode::Strict`]）では、知らないブロックと要素を 400 にする。
//! - Webhook と Slack の取り込み（[`Mode::Lenient`]）では、知らないものを「未対応の部品」にして文字だけ残す。
//! - mrkdwn は [`crate::slack::convert::mrkdwn_to_mfm`] で MFM にする。plain_text は記号を無効にする。

use crate::slack::convert::{Resolver, mrkdwn_to_mfm};
use serde_json::Value;
use zoo_components::legacy::{merge_texts, plain};
use zoo_components::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Strict,
    Lenient,
}

/// Slack の ID を zoo の名前にしない変換（Webhook と App API）。`<@U1>` は `@U1` のように残る。
pub struct NoResolver;
impl Resolver for NoResolver {
    fn user_name(&self, _: &str) -> Option<String> {
        None
    }
    fn channel_name(&self, _: &str) -> Option<String> {
        None
    }
}

pub struct Converter<'a, R: Resolver> {
    pub mode: Mode,
    pub resolver: &'a R,
    /// action_id のない要素に付ける custom_id の番号
    auto: usize,
}

type Res<T> = Result<T, String>;

fn s<'v>(v: &'v Value, k: &str) -> &'v str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}
fn opt(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}
fn arr<'v>(v: &'v Value, k: &str) -> &'v [Value] {
    v.get(k)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}
fn u8_of(v: &Value, k: &str) -> Option<u8> {
    v.get(k).and_then(Value::as_u64).map(|n| n.min(255) as u8)
}

impl<'a, R: Resolver> Converter<'a, R> {
    pub fn new(mode: Mode, resolver: &'a R) -> Self {
        Self {
            mode,
            resolver,
            auto: 0,
        }
    }

    fn unknown(&self, what: &str, v: &Value) -> Res<Component> {
        match self.mode {
            Mode::Strict => Err(format!("unsupported Block Kit {what}: {}", s(v, "type"))),
            Mode::Lenient => Ok(Component::Unsupported(Unsupported {
                id: None,
                original_type: format!("slack:{}", s(v, "type")),
                text: salvage_text(v),
            })),
        }
    }

    fn custom_id(&mut self, v: &Value) -> String {
        opt(v, "action_id")
            .map(|a| clip(&a, 100))
            .unwrap_or_else(|| {
                self.auto += 1;
                format!("slack-auto-{}", self.auto)
            })
    }

    /// テキストオブジェクト（plain_text / mrkdwn）を MFM にする。
    pub fn text(&self, v: &Value) -> String {
        match v {
            Value::String(t) => plain(t),
            Value::Object(_) => match s(v, "type") {
                "mrkdwn" => mrkdwn_to_mfm(s(v, "text"), self.resolver),
                _ => plain(s(v, "text")),
            },
            _ => String::new(),
        }
    }

    /// 書式を付けない文字（ボタンの文字、選択肢の文字など）。
    fn raw(&self, v: &Value) -> String {
        match v {
            Value::String(t) => t.clone(),
            Value::Object(_) => s(v, "text").to_string(),
            _ => String::new(),
        }
    }

    fn confirm(&self, v: &Value) -> Option<Confirm> {
        let c = v.get("confirm")?;
        Some(Confirm {
            title: clip(&self.raw(c.get("title")?), 100),
            text: clip(&self.raw(c.get("text")?), 300),
            confirm: clip(&self.raw(c.get("confirm")?), 30),
            deny: clip(&self.raw(c.get("deny")?), 30),
            style: opt(c, "style"),
        })
    }

    fn option(&self, o: &Value, defaults: &[String]) -> SelectOption {
        let value =
            opt(o, "value").unwrap_or_else(|| self.raw(o.get("text").unwrap_or(&Value::Null)));
        SelectOption {
            label: clip(&self.raw(o.get("text").unwrap_or(&Value::Null)), 100),
            default: defaults.contains(&value),
            value: clip(&value, 100),
            description: o.get("description").map(|d| clip(&self.raw(d), 100)),
            emoji: None,
        }
    }

    /// アプリが返した Slack 形式の選択肢 1 つ。
    pub fn option_value(&self, o: &Value) -> Result<SelectOption, crate::error::AppError> {
        Ok(self.option(o, &[]))
    }

    /// options と option_groups を平らにする。initial_option(s) を default にする。
    fn options(&self, v: &Value) -> Vec<SelectOption> {
        let mut defaults: Vec<String> = arr(v, "initial_options")
            .iter()
            .filter_map(|o| opt(o, "value"))
            .collect();
        if let Some(o) = v.get("initial_option").and_then(|o| opt(o, "value")) {
            defaults.push(o);
        }
        let mut out: Vec<SelectOption> = arr(v, "options")
            .iter()
            .map(|o| self.option(o, &defaults))
            .collect();
        for group in arr(v, "option_groups") {
            out.extend(
                arr(group, "options")
                    .iter()
                    .map(|o| self.option(o, &defaults)),
            );
        }
        out.truncate(limits::MAX_OPTIONS);
        out
    }

    fn placeholder(&self, v: &Value) -> Option<String> {
        v.get("placeholder").map(|p| clip(&self.raw(p), 150))
    }

    fn entity_defaults(v: &Value, kind: &str, keys: &[&str]) -> Vec<DefaultValue> {
        let mut out = Vec::new();
        for k in keys {
            match v.get(*k) {
                Some(Value::String(id)) => out.push(DefaultValue {
                    id: id.clone(),
                    kind: kind.into(),
                }),
                Some(Value::Array(ids)) => {
                    out.extend(ids.iter().filter_map(Value::as_str).map(|id| DefaultValue {
                        id: id.into(),
                        kind: kind.into(),
                    }))
                }
                _ => {}
            }
        }
        out
    }

    /// 要素（ボタン、select、入力など）を部品にする。
    pub fn element(&mut self, v: &Value) -> Res<Component> {
        let ty = s(v, "type");
        let multi = ty.starts_with("multi_");
        let max = if multi {
            Some(u8_of(v, "max_selected_items").unwrap_or(25))
        } else {
            None
        };
        let min = if multi { Some(0) } else { None };
        Ok(match ty {
            "button" | "workflow_button" => {
                let label = clip(&self.raw(v.get("text").unwrap_or(&Value::Null)), 80);
                match opt(v, "url").filter(|u| zoo_components::validate::is_https_url(u)) {
                    Some(url) if ty == "button" => Component::Button(Button {
                        style: button_style::LINK,
                        label: Some(label),
                        url: Some(url),
                        ..Default::default()
                    }),
                    _ => Component::Button(Button {
                        style: match s(v, "style") {
                            "primary" => button_style::PRIMARY,
                            "danger" => button_style::DANGER,
                            _ => button_style::SECONDARY,
                        },
                        label: Some(label),
                        custom_id: Some(self.custom_id(v)),
                        value: opt(v, "value").map(|x| clip(&x, 2000)),
                        confirm: self.confirm(v),
                        ..Default::default()
                    }),
                }
            }
            "static_select" | "multi_static_select" => Component::StringSelect(StringSelect {
                custom_id: self.custom_id(v),
                options: self.options(v),
                placeholder: self.placeholder(v),
                min_values: min,
                max_values: max,
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "external_select" | "multi_external_select" => {
                Component::ExternalSelect(ExternalSelect {
                    custom_id: self.custom_id(v),
                    placeholder: self.placeholder(v),
                    min_query_length: u8_of(v, "min_query_length"),
                    min_values: min,
                    max_values: max,
                    initial_options: {
                        let mut o: Vec<SelectOption> = arr(v, "initial_options")
                            .iter()
                            .map(|o| self.option(o, &[]))
                            .collect();
                        o.extend(v.get("initial_option").map(|o| self.option(o, &[])));
                        o
                    },
                    confirm: self.confirm(v),
                    ..Default::default()
                })
            }
            "users_select" | "multi_users_select" => Component::UserSelect(EntitySelect {
                custom_id: self.custom_id(v),
                placeholder: self.placeholder(v),
                min_values: min,
                max_values: max,
                default_values: Self::entity_defaults(
                    v,
                    "user",
                    &["initial_user", "initial_users"],
                ),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "conversations_select"
            | "multi_conversations_select"
            | "channels_select"
            | "multi_channels_select" => Component::ChannelSelect(EntitySelect {
                custom_id: self.custom_id(v),
                placeholder: self.placeholder(v),
                min_values: min,
                max_values: max,
                default_values: Self::entity_defaults(
                    v,
                    "channel",
                    &[
                        "initial_conversation",
                        "initial_conversations",
                        "initial_channel",
                        "initial_channels",
                    ],
                ),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "overflow" => Component::Overflow(OptionGroup {
                custom_id: self.custom_id(v),
                options: {
                    let mut o = self.options(v);
                    o.truncate(limits::MAX_OVERFLOW_OPTIONS);
                    o
                },
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "checkboxes" => {
                let options = self.options(v);
                Component::CheckboxGroup(OptionGroup {
                    custom_id: self.custom_id(v),
                    min_values: Some(0),
                    max_values: Some(options.len().max(1) as u8),
                    options,
                    confirm: self.confirm(v),
                    ..Default::default()
                })
            }
            "radio_buttons" => Component::RadioGroup(OptionGroup {
                custom_id: self.custom_id(v),
                options: self.options(v),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "datepicker" => Component::DatePicker(Picker {
                custom_id: self.custom_id(v),
                initial_value: opt(v, "initial_date"),
                placeholder: self.placeholder(v),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "timepicker" => Component::TimePicker(Picker {
                custom_id: self.custom_id(v),
                initial_value: opt(v, "initial_time"),
                timezone: opt(v, "timezone"),
                placeholder: self.placeholder(v),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "datetimepicker" => Component::DateTimePicker(Picker {
                custom_id: self.custom_id(v),
                initial_value: v
                    .get("initial_date_time")
                    .and_then(Value::as_i64)
                    .map(|t| t.to_string()),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            "plain_text_input" | "number_input" | "email_text_input" | "url_text_input"
            | "rich_text_input" => {
                let format = match ty {
                    "number_input" => TextFormat::Number,
                    "email_text_input" => TextFormat::Email,
                    "url_text_input" => TextFormat::Url,
                    "rich_text_input" => TextFormat::Rich,
                    _ => TextFormat::Plain,
                };
                let num = |k: &str| match v.get(k) {
                    Some(Value::String(x)) => x.parse::<f64>().ok(),
                    Some(Value::Number(n)) => n.as_f64(),
                    _ => None,
                };
                let value = match v.get("initial_value") {
                    Some(Value::String(x)) => Some(x.clone()),
                    Some(rich @ Value::Object(_)) => Some(self.rich_text(rich)),
                    _ => None,
                };
                Component::TextInput(TextInput {
                    custom_id: self.custom_id(v),
                    style: if v.get("multiline").and_then(Value::as_bool).unwrap_or(false)
                        || format == TextFormat::Rich
                    {
                        2
                    } else {
                        1
                    },
                    min_length: v
                        .get("min_length")
                        .and_then(Value::as_u64)
                        .map(|n| n.min(4000) as u16),
                    max_length: v
                        .get("max_length")
                        .and_then(Value::as_u64)
                        .map(|n| n.min(4000) as u16),
                    value: value.map(|x| clip(&x, 4000)),
                    placeholder: self.placeholder(v),
                    format,
                    min_value: num("min_value"),
                    max_value: num("max_value"),
                    decimal: v
                        .get("is_decimal_allowed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    ..Default::default()
                })
            }
            "file_input" => Component::FileUpload(FileUpload {
                custom_id: self.custom_id(v),
                min_values: Some(0),
                max_values: u8_of(v, "max_files").map(|n| n.clamp(1, 10)),
                file_types: arr(v, "filetypes")
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                ..Default::default()
            }),
            "image" => {
                let url =
                    opt(v, "image_url").or_else(|| v.get("slack_file").and_then(|f| opt(f, "url")));
                match url {
                    Some(url) => Component::Thumbnail(Thumbnail {
                        media: Media {
                            url,
                            ..Default::default()
                        },
                        description: opt(v, "alt_text").map(|t| clip(&t, 1024)),
                        ..Default::default()
                    }),
                    None => return self.unknown("image", v),
                }
            }
            "icon_button" => Component::Button(Button {
                style: button_style::SECONDARY,
                label: Some(clip(&self.raw(v.get("text").unwrap_or(&Value::Null)), 80))
                    .filter(|l| !l.is_empty())
                    .or_else(|| Some(s(v, "icon").to_string())),
                custom_id: Some(self.custom_id(v)),
                value: opt(v, "value"),
                confirm: self.confirm(v),
                ..Default::default()
            }),
            _ => return self.unknown("element", v),
        })
    }

    /// feedback_buttons は 2 つのボタンになる。
    fn elements(&mut self, v: &Value) -> Res<Vec<Component>> {
        if s(v, "type") == "feedback_buttons" {
            let id = self.custom_id(v);
            let mut out = Vec::new();
            for (k, style) in [
                ("positive_button", button_style::SUCCESS),
                ("negative_button", button_style::DANGER),
            ] {
                if let Some(b) = v.get(k) {
                    out.push(Component::Button(Button {
                        style,
                        label: Some(clip(&self.raw(b.get("text").unwrap_or(&Value::Null)), 80)),
                        custom_id: Some(format!("{id}:{k}")),
                        value: opt(b, "value"),
                        ..Default::default()
                    }));
                }
            }
            return Ok(out);
        }
        Ok(vec![self.element(v)?])
    }

    /// ボタンと overflow は 5 個ずつの行に、それ以外は 1 つずつの行にする。
    fn rows(&mut self, elements: &[Value]) -> Res<Vec<Component>> {
        let mut rows: Vec<Component> = Vec::new();
        let mut small: Vec<Component> = Vec::new();
        let flush = |small: &mut Vec<Component>, rows: &mut Vec<Component>| {
            for chunk in std::mem::take(small).chunks(limits::MAX_BUTTONS_PER_ROW) {
                rows.push(Component::ActionRow(ActionRow {
                    id: None,
                    components: chunk.to_vec(),
                }));
            }
        };
        for e in elements {
            for c in self.elements(e)? {
                match c {
                    Component::Button(_) | Component::Overflow(_) => small.push(c),
                    Component::Unsupported(_) => {
                        flush(&mut small, &mut rows);
                        rows.push(c);
                    }
                    other => {
                        flush(&mut small, &mut rows);
                        rows.push(Component::ActionRow(ActionRow {
                            id: None,
                            components: vec![other],
                        }));
                    }
                }
            }
        }
        flush(&mut small, &mut rows);
        Ok(rows)
    }

    /// blocks の配列を部品の列にする。続く image ブロックは 1 つのギャラリーにまとめる。
    pub fn blocks(&mut self, blocks: &[Value]) -> Res<Vec<Component>> {
        let mut out: Vec<Component> = Vec::new();
        for b in blocks {
            for c in self.block(b)? {
                if let (Component::MediaGallery(next), Some(Component::MediaGallery(prev))) =
                    (&c, out.last_mut())
                    && prev.items.len() + next.items.len() <= limits::MAX_GALLERY_ITEMS
                {
                    prev.items.extend(next.items.iter().cloned());
                    continue;
                }
                out.push(c);
            }
        }
        Ok(merge_texts(out))
    }

    fn block(&mut self, b: &Value) -> Res<Vec<Component>> {
        Ok(match s(b, "type") {
            "section" => {
                let mut texts = Vec::new();
                if let Some(t) = b.get("text") {
                    texts.push(self.text(t));
                }
                let fields: Vec<String> = arr(b, "fields")
                    .iter()
                    .take(10)
                    .map(|f| self.text(f))
                    .collect();
                if !fields.is_empty() {
                    texts.push(fields.join("\n"));
                }
                let texts: Vec<Component> = texts
                    .into_iter()
                    .filter(|t| !t.trim().is_empty())
                    .map(Component::text)
                    .collect();
                match b.get("accessory") {
                    Some(a) => {
                        let accessory = self.element(a)?;
                        if texts.is_empty() {
                            vec![Component::ActionRow(ActionRow {
                                id: None,
                                components: vec![accessory],
                            })]
                        } else {
                            vec![Component::Section(Section {
                                id: None,
                                components: merge_texts(texts),
                                accessory: Some(Box::new(accessory)),
                            })]
                        }
                    }
                    None => texts,
                }
            }
            "header" => {
                let level = b
                    .get("level")
                    .and_then(Value::as_u64)
                    .unwrap_or(3)
                    .clamp(1, 4) as usize;
                vec![Component::text(format!(
                    "{} {}",
                    "#".repeat(level),
                    plain(&self.raw(b.get("text").unwrap_or(&Value::Null)))
                ))]
            }
            "divider" => vec![Component::Separator(Separator::default())],
            "context" => {
                let parts: Vec<String> = arr(b, "elements")
                    .iter()
                    .filter_map(|e| match s(e, "type") {
                        "image" => opt(e, "alt_text").map(|t| plain(&t)),
                        _ => Some(self.text(e)),
                    })
                    .filter(|t| !t.is_empty())
                    .collect();
                if parts.is_empty() {
                    vec![]
                } else {
                    vec![Component::text(format!(
                        "<small>{}</small>",
                        parts.join(" · ")
                    ))]
                }
            }
            "actions" | "context_actions" => self.rows(arr(b, "elements"))?,
            "image" => {
                let url =
                    opt(b, "image_url").or_else(|| b.get("slack_file").and_then(|f| opt(f, "url")));
                let Some(url) = url else {
                    return Ok(vec![self.unknown("block", b)?]);
                };
                let mut out = Vec::new();
                if let Some(title) = b.get("title") {
                    out.push(Component::text(format!("**{}**", plain(&self.raw(title)))));
                }
                out.push(Component::MediaGallery(MediaGallery {
                    id: None,
                    items: vec![GalleryItem {
                        media: Media {
                            url,
                            ..Default::default()
                        },
                        description: opt(b, "alt_text").map(|t| clip(&t, 1024)),
                        spoiler: false,
                    }],
                }));
                out
            }
            "input" => {
                let element = b.get("element").unwrap_or(&Value::Null);
                let mut component = self.element(element)?;
                let optional = b.get("optional").and_then(Value::as_bool).unwrap_or(false);
                match &mut component {
                    Component::TextInput(t) => t.required = Some(!optional),
                    Component::StringSelect(t) => t.required = Some(!optional),
                    Component::UserSelect(t) | Component::ChannelSelect(t) => {
                        t.required = Some(!optional)
                    }
                    Component::ExternalSelect(t) => t.required = Some(!optional),
                    Component::FileUpload(t) => t.required = Some(!optional),
                    Component::RadioGroup(t) | Component::CheckboxGroup(t) => {
                        t.required = Some(!optional)
                    }
                    Component::DatePicker(t)
                    | Component::TimePicker(t)
                    | Component::DateTimePicker(t) => t.required = Some(!optional),
                    _ => {}
                }
                vec![Component::Label(Label {
                    id: None,
                    label: clip(&self.raw(b.get("label").unwrap_or(&Value::Null)), 45),
                    description: b.get("hint").map(|h| clip(&self.raw(h), 100)),
                    component: Box::new(component),
                })]
            }
            "rich_text" => {
                let t = self.rich_text(b);
                if t.trim().is_empty() {
                    vec![]
                } else {
                    vec![Component::text(t)]
                }
            }
            "markdown" => vec![Component::text(markdown_to_mfm(s(b, "text")))],
            "video" => vec![Component::Video(Video {
                id: None,
                title: clip(&self.raw(b.get("title").unwrap_or(&Value::Null)), 200),
                video_url: s(b, "video_url").to_string(),
                thumbnail_url: opt(b, "thumbnail_url"),
                alt_text: opt(b, "alt_text"),
                title_url: opt(b, "title_url"),
                description: b.get("description").map(|d| clip(&self.raw(d), 200)),
                author_name: opt(b, "author_name"),
                provider_name: opt(b, "provider_name"),
            })],
            "table" | "data_table" => {
                let data = s(b, "type") == "data_table";
                let max_rows = if data {
                    limits::MAX_TABLE_DATA_ROWS
                } else {
                    100
                };
                let mut rows: Vec<Vec<TableCell>> = Vec::new();
                for r in arr(b, "rows").iter().take(max_rows) {
                    let mut row = Vec::new();
                    for c in r
                        .as_array()
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                        .iter()
                        .take(limits::MAX_TABLE_COLUMNS)
                    {
                        row.push(self.cell(c)?);
                    }
                    rows.push(row);
                }
                vec![Component::Table(Table {
                    id: None,
                    rows,
                    column_settings: arr(b, "column_settings")
                        .iter()
                        .map(|c| ColumnSetting {
                            align: opt(c, "align"),
                            is_wrapped: c
                                .get("is_wrapped")
                                .and_then(Value::as_bool)
                                .unwrap_or(false),
                        })
                        .collect(),
                    header: data,
                    caption: opt(b, "caption").map(|c| clip(&c, 200)),
                    page_size: if data {
                        Some(u8_of(b, "page_size").unwrap_or(5).clamp(1, 100))
                    } else {
                        None
                    },
                })]
            }
            "alert" => vec![Component::Alert(Alert {
                id: None,
                content: self.text(b.get("text").unwrap_or(&Value::Null)),
                level: opt(b, "level"),
            })],
            "card" => vec![self.card(b)?],
            "carousel" => {
                let mut cards = Vec::new();
                for c in arr(b, "elements").iter().take(limits::MAX_CAROUSEL_CARDS) {
                    cards.push(self.card(c)?);
                }
                vec![Component::Carousel(Carousel { id: None, cards })]
            }
            "container" => {
                let title = match b.get("rich_text_title") {
                    Some(rt) => Some(self.rich_text(rt)),
                    None => b.get("title").map(|t| self.text(t)),
                };
                let mut inner = self.blocks(arr(b, "child_blocks"))?;
                // container の中に container は置けないので、中身を開く
                inner = inner
                    .into_iter()
                    .flat_map(|c| match c {
                        Component::Container(c) => c.components,
                        other => vec![other],
                    })
                    .collect();
                if inner.is_empty() {
                    inner.push(Component::Separator(Separator {
                        divider: false,
                        ..Default::default()
                    }));
                }
                let collapsible = b
                    .get("is_collapsible")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                vec![Component::Container(Container {
                    title,
                    subtitle: b.get("subtitle").map(|t| self.text(t)),
                    collapsible,
                    collapsed: collapsible
                        && b.get("default_collapsed")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    components: inner,
                    ..Default::default()
                })]
            }
            "data_visualization" => {
                let chart = b.get("chart").unwrap_or(&Value::Null);
                let point = |p: &Value| ChartPoint {
                    label: clip(s(p, "label"), 20),
                    value: p.get("value").and_then(Value::as_f64).unwrap_or(0.0),
                };
                let axis = chart.get("axis_config").unwrap_or(&Value::Null);
                vec![Component::Chart(Chart {
                    id: None,
                    title: clip(s(b, "title"), 50),
                    chart_type: s(chart, "type").to_string(),
                    segments: arr(chart, "segments").iter().take(12).map(point).collect(),
                    series: arr(chart, "series")
                        .iter()
                        .take(12)
                        .map(|x| ChartSeries {
                            name: clip(s(x, "name"), 20),
                            data: arr(x, "data").iter().take(20).map(point).collect(),
                        })
                        .collect(),
                    categories: arr(axis, "categories")
                        .iter()
                        .filter_map(Value::as_str)
                        .map(|c| clip(c, 20))
                        .collect(),
                    x_label: opt(axis, "x_label"),
                    y_label: opt(axis, "y_label"),
                })]
            }
            "layout" => {
                let mut rows = Vec::new();
                for r in arr(b, "rows").iter().take(limits::MAX_GRID_ROWS) {
                    let mut items = Vec::new();
                    for item in arr(r, "items").iter().take(limits::MAX_GRID_ITEMS) {
                        let mut inner = self.block(item.get("block").unwrap_or(&Value::Null))?;
                        let component = if inner.len() == 1 {
                            inner.remove(0)
                        } else {
                            Component::Container(Container {
                                components: inner,
                                ..Default::default()
                            })
                        };
                        items.push(GridItem {
                            width: match s(item, "width") {
                                "narrow" => 1,
                                "wide" => 3,
                                _ => 2,
                            },
                            align: opt(item, "horizontal_alignment"),
                            component: Box::new(component),
                        });
                    }
                    rows.push(items);
                }
                vec![Component::Grid(Grid {
                    id: None,
                    label: if b
                        .get("is_label_visible")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        opt(b, "label").map(|l| plain(&l))
                    } else {
                        None
                    },
                    grid_lines: opt(b, "grid_lines"),
                    rows,
                })]
            }
            "plan" => vec![Component::Plan(Plan {
                id: None,
                title: s(b, "title").to_string(),
                tasks: arr(b, "tasks")
                    .iter()
                    .take(limits::MAX_PLAN_TASKS)
                    .map(|t| self.task(t))
                    .collect(),
            })],
            "task_card" => vec![Component::TaskCard(self.task(b))],
            "file" => vec![Component::Unsupported(Unsupported {
                id: None,
                original_type: "slack:file".into(),
                text: opt(b, "external_id").unwrap_or_else(|| "File".into()),
            })],
            _ => vec![self.unknown("block", b)?],
        })
    }

    fn card(&mut self, b: &Value) -> Res<Component> {
        let image = |k: &str| {
            b.get(k)
                .and_then(|i| {
                    opt(i, "image_url").or_else(|| i.get("slack_file").and_then(|f| opt(f, "url")))
                })
                .map(|url| Media {
                    url,
                    ..Default::default()
                })
        };
        let actions = match b.get("actions") {
            Some(a) => {
                let mut out = Vec::new();
                for e in arr(a, "elements").iter().take(limits::MAX_CARD_ACTIONS) {
                    out.extend(self.elements(e)?);
                }
                out
            }
            None => Vec::new(),
        };
        Ok(Component::Card(Card {
            id: None,
            hero_image: image("hero_image"),
            icon: image("icon"),
            title: b.get("title").map(|t| self.text(t)),
            subtitle: b.get("subtitle").map(|t| self.text(t)),
            body: b.get("body").map(|t| self.text(t)),
            subtext: b.get("subtext").map(|t| self.text(t)),
            actions,
        }))
    }

    fn task(&self, t: &Value) -> TaskCard {
        TaskCard {
            id: None,
            task_id: s(t, "task_id").to_string(),
            title: s(t, "title").to_string(),
            status: match s(t, "status") {
                "in_progress" | "complete" | "error" => s(t, "status").to_string(),
                _ => "pending".into(),
            },
            details: t.get("details").map(|d| self.rich_text(d)),
            output: t.get("output").map(|d| self.rich_text(d)),
            sources: arr(t, "sources")
                .iter()
                .filter(|x| zoo_components::validate::is_https_url(s(x, "url")))
                .map(|x| TaskSource {
                    url: s(x, "url").into(),
                    text: s(x, "text").into(),
                })
                .collect(),
        }
    }

    fn cell(&mut self, c: &Value) -> Res<TableCell> {
        Ok(match s(c, "type") {
            "rich_text" => TableCell {
                text: self.rich_text(c),
                ..Default::default()
            },
            "raw_number" => {
                TableCell {
                    text: plain(&opt(c, "text").unwrap_or_else(|| {
                        c.get("value").map(|v| v.to_string()).unwrap_or_default()
                    })),
                    number: c.get("value").and_then(Value::as_f64),
                    button: None,
                }
            }
            "action_cell" => {
                let fallback = c
                    .get("fallback")
                    .map(|f| self.cell(f))
                    .transpose()?
                    .unwrap_or_default();
                TableCell {
                    button: Some(Box::new(
                        self.element(c.get("element").unwrap_or(&Value::Null))?,
                    )),
                    ..fallback
                }
            }
            _ => TableCell {
                text: plain(s(c, "text")),
                ..Default::default()
            },
        })
    }

    /// rich_text ブロック（rich_text_input の値も）を MFM にする。
    pub fn rich_text(&self, v: &Value) -> String {
        let mut out = Vec::new();
        for e in arr(v, "elements") {
            match s(e, "type") {
                "rich_text_section" => out.push(self.inline(arr(e, "elements"))),
                "rich_text_preformatted" => {
                    let body: String = arr(e, "elements")
                        .iter()
                        .map(|x| s(x, "text").to_string() + s(x, "url"))
                        .collect();
                    out.push(format!(
                        "```{}\n{}\n```",
                        s(e, "language"),
                        body.replace("```", "`\u{200b}``")
                    ));
                }
                "rich_text_quote" => out.push(
                    self.inline(arr(e, "elements"))
                        .lines()
                        .map(|l| format!("> {l}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                "rich_text_list" => {
                    let indent = "  ".repeat(
                        e.get("indent").and_then(Value::as_u64).unwrap_or(0).min(8) as usize,
                    );
                    let ordered = s(e, "style") == "ordered";
                    let offset = e.get("offset").and_then(Value::as_u64).unwrap_or(0);
                    for (i, item) in arr(e, "elements").iter().enumerate() {
                        let mark = if ordered {
                            format!("{}.", offset as usize + i + 1)
                        } else {
                            "・".into()
                        };
                        out.push(format!(
                            "{indent}{mark} {}",
                            self.inline(arr(item, "elements"))
                        ));
                    }
                }
                _ => out.push(salvage_text(e)),
            }
        }
        out.join("\n").trim_end().to_string()
    }

    fn inline(&self, elements: &[Value]) -> String {
        let mut out = String::new();
        for e in elements {
            let style = e.get("style").cloned().unwrap_or(Value::Null);
            let on = |k: &str| style.get(k).and_then(Value::as_bool).unwrap_or(false);
            let mut piece = match s(e, "type") {
                "text" => {
                    if on("code") {
                        format!("`{}`", s(e, "text").replace('`', "'"))
                    } else {
                        // 改行は残し、各行の記号だけを無効にする
                        s(e, "text")
                            .split('\n')
                            .map(plain)
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                }
                "link" => {
                    let url = s(e, "url");
                    let label = opt(e, "text").unwrap_or_else(|| url.to_string());
                    if zoo_components::validate::is_https_url(url) || url.starts_with("http://") {
                        format!("[{}]({url})", plain(&label))
                    } else {
                        plain(&label)
                    }
                }
                "emoji" => match opt(e, "unicode") {
                    Some(code) => code
                        .split('-')
                        .filter_map(|h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32))
                        .collect(),
                    None => format!(":{}:", s(e, "name")),
                },
                "user" => format!(
                    "@{}",
                    self.resolver
                        .user_name(s(e, "user_id"))
                        .unwrap_or_else(|| s(e, "user_id").to_string())
                ),
                "usergroup" => format!("@{}", s(e, "usergroup_id")),
                "channel" => format!(
                    "#{}",
                    self.resolver
                        .channel_name(s(e, "channel_id"))
                        .unwrap_or_else(|| s(e, "channel_id").to_string())
                ),
                "broadcast" => format!("@{}", s(e, "range")),
                "date" => match e.get("timestamp").and_then(Value::as_i64) {
                    Some(ts) => format!("$[unixtime {ts}]"),
                    None => plain(s(e, "fallback")),
                },
                "color" => format!("`{}`", s(e, "value")),
                "team" => format!("@{}", s(e, "team_id")),
                "tag" => format!("`{}`", s(e, "text").replace('`', "'")),
                "citation" => {
                    match opt(e, "url").filter(|u| zoo_components::validate::is_https_url(u)) {
                        Some(url) => format!(
                            "[[{}]]({url})",
                            e.get("index")
                                .map(|i| i.to_string())
                                .unwrap_or_else(|| s(e, "text").into())
                        ),
                        None => plain(s(e, "text")),
                    }
                }
                // ファイル・canvas・メッセージ・ワークフローなどへの参照は、文字（あればリンク）にする
                _ => {
                    let text = opt(e, "text")
                        .or_else(|| opt(e, "label"))
                        .unwrap_or_else(|| salvage_text(e));
                    match opt(e, "url").filter(|u| zoo_components::validate::is_https_url(u)) {
                        Some(url) => format!(
                            "[{}]({url})",
                            plain(if text.is_empty() { &url } else { &text })
                        ),
                        None => plain(&text),
                    }
                }
            };
            if s(e, "type") != "text" || !on("code") {
                if on("bold") {
                    piece = format!("**{piece}**");
                }
                if on("italic") {
                    piece = format!("<i>{piece}</i>");
                }
                if on("strike") {
                    piece = format!("~~{piece}~~");
                }
            }
            out.push_str(&piece);
        }
        out
    }
}

/// Slack の旧 attachments を container にする（Webhook と Slack の取り込み）。
/// `pretext` は container の前に置く。リンクの展開（from_url、is_app_unfurl）は除く。
pub fn attachments<R: Resolver>(
    conv: &mut Converter<'_, R>,
    attachments: &[Value],
) -> Vec<Component> {
    let mut out = Vec::new();
    for a in attachments.iter().take(100) {
        if a.get("from_url").is_some() || a.get("is_app_unfurl").is_some() {
            continue;
        }
        let mrkdwn_in: Vec<&str> = arr(a, "mrkdwn_in")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let field_text = |k: &str| -> String {
            let raw = s(a, k);
            if raw.is_empty() {
                String::new()
            } else if mrkdwn_in.is_empty() || mrkdwn_in.contains(&k) {
                // Slack の既定では text は mrkdwn として扱う
                mrkdwn_to_mfm(raw, conv.resolver)
            } else {
                plain(raw)
            }
        };
        if !s(a, "pretext").is_empty() {
            out.push(Component::text(field_text("pretext")));
        }
        let mut parts: Vec<Component> = Vec::new();
        if let Some(author) = opt(a, "author_name") {
            let author =
                match opt(a, "author_link").filter(|u| zoo_components::validate::is_https_url(u)) {
                    Some(link) => format!("[{}]({link})", plain(&author)),
                    None => plain(&author),
                };
            parts.push(Component::text(format!("<small>{author}</small>")));
        }
        if let Some(title) = opt(a, "title") {
            parts.push(Component::text(
                match opt(a, "title_link").filter(|u| zoo_components::validate::is_https_url(u)) {
                    Some(link) => format!("**[{}]({link})**", plain(&title)),
                    None => format!("**{}**", plain(&title)),
                },
            ));
        }
        let text = field_text("text");
        if !text.is_empty() {
            parts.push(Component::text(text));
        }
        let fields: Vec<String> = arr(a, "fields")
            .iter()
            .take(10)
            .map(|f| {
                let value = if mrkdwn_in.contains(&"fields") || mrkdwn_in.is_empty() {
                    mrkdwn_to_mfm(s(f, "value"), conv.resolver)
                } else {
                    plain(s(f, "value"))
                };
                if s(f, "title").is_empty() {
                    value
                } else {
                    format!("**{}**: {value}", plain(s(f, "title")))
                }
            })
            .collect();
        if !fields.is_empty() {
            parts.push(Component::text(fields.join("\n")));
        }
        let mut parts = merge_texts(parts);
        // thumb_url があれば、文字の横にサムネイルを置く
        if let Some(thumb) =
            opt(a, "thumb_url").filter(|u| zoo_components::validate::is_https_url(u))
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
        if let Some(image) =
            opt(a, "image_url").filter(|u| zoo_components::validate::is_https_url(u))
        {
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
        if let Ok(nested) = conv.blocks(arr(a, "blocks")) {
            parts.extend(
                nested
                    .into_iter()
                    .filter(|c| !matches!(c, Component::Container(_))),
            );
        }
        let footer: Vec<String> = [
            opt(a, "footer").map(|f| field_text_plain(&f)),
            a.get("ts").and_then(ts_text),
        ]
        .into_iter()
        .flatten()
        .collect();
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
            if let Some(fallback) = opt(a, "fallback") {
                parts.push(Component::text(plain(&fallback)));
            } else {
                continue;
            }
        }
        out.push(Component::Container(Container {
            accent_color: parse_color(s(a, "color")),
            components: parts,
            ..Default::default()
        }));
    }
    out
}

/// Slack の markdown ブロック（標準の markdown）を MFM に寄せる。MFM にない書き方だけ直す。
pub fn markdown_to_mfm(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let indent = &line[..line.len() - trimmed.len()];
            if let Some(rest) = trimmed
                .strip_prefix("- [ ] ")
                .or_else(|| trimmed.strip_prefix("* [ ] "))
            {
                format!("{indent}☐ {rest}")
            } else if let Some(rest) = trimmed
                .strip_prefix("- [x] ")
                .or_else(|| trimmed.strip_prefix("- [X] "))
                .or_else(|| trimmed.strip_prefix("* [x] "))
            {
                format!("{indent}☑ {rest}")
            } else if trimmed == "---" || trimmed == "***" {
                "──────────".to_string()
            } else if let Some(rest) = trimmed
                .strip_prefix("- ")
                .or_else(|| trimmed.strip_prefix("* "))
            {
                format!("{indent}・{rest}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn field_text_plain(t: &str) -> String {
    plain(t)
}

fn ts_text(v: &Value) -> Option<String> {
    let ts = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.parse::<f64>().ok()?,
        _ => return None,
    };
    Some(format!("$[unixtime {}]", ts as i64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn convert(blocks: Value) -> Vec<Component> {
        Converter::new(Mode::Lenient, &NoResolver)
            .blocks(blocks.as_array().unwrap())
            .unwrap()
    }

    #[test]
    fn section_with_fields_and_image_accessory_becomes_section() {
        let c = convert(
            json!([{"type":"section","text":{"type":"mrkdwn","text":"*Deploy* done"},
            "fields":[{"type":"mrkdwn","text":"*Env*\nprod"}],
            "accessory":{"type":"image","image_url":"https://example.com/a.png","alt_text":"logo"}}]),
        );
        let Component::Section(s) = &c[0] else {
            panic!("{c:?}")
        };
        assert_eq!(s.components.len(), 1);
        assert!(matches!(
            s.accessory.as_deref(),
            Some(Component::Thumbnail(_))
        ));
        let Component::TextDisplay(t) = &s.components[0] else {
            panic!()
        };
        assert!(t.content.starts_with("**Deploy** done\n"), "{}", t.content);
    }

    #[test]
    fn actions_split_buttons_and_selects_into_rows() {
        let buttons: Vec<Value> = (0..6).map(|i| json!({"type":"button","text":{"type":"plain_text","text":format!("B{i}")},"action_id":format!("b{i}")})).collect();
        let mut elements = buttons;
        elements.push(json!({"type":"static_select","action_id":"s","options":[{"text":{"type":"plain_text","text":"A"},"value":"a"}]}));
        elements.push(json!({"type":"button","text":{"type":"plain_text","text":"Open"},"url":"https://example.com"}));
        let c = convert(json!([{"type":"actions","elements":elements}]));
        assert_eq!(c.len(), 4, "{c:?}");
        let doc = Document::new(0, c);
        assert!(
            doc.validate(Context::AppMessage).is_ok(),
            "{:?}",
            doc.validate(Context::AppMessage)
        );
    }

    #[test]
    fn unknown_blocks_are_kept_as_text_in_lenient_mode_and_rejected_in_strict_mode() {
        let blocks = json!([{"type":"future_block","text":{"type":"plain_text","text":"hello"}}]);
        let c = convert(blocks.clone());
        assert!(
            matches!(&c[0], Component::Unsupported(u) if u.text == "hello" && u.original_type == "slack:future_block")
        );
        assert!(
            Converter::new(Mode::Strict, &NoResolver)
                .blocks(blocks.as_array().unwrap())
                .is_err()
        );
    }

    #[test]
    fn rich_text_lists_quotes_and_styles() {
        let c = convert(json!([{"type":"rich_text","elements":[
            {"type":"rich_text_section","elements":[{"type":"text","text":"Hi "},{"type":"text","text":"bold","style":{"bold":true}},{"type":"date","timestamp":1700000000,"format":"{date}"}]},
            {"type":"rich_text_list","style":"ordered","elements":[{"type":"rich_text_section","elements":[{"type":"text","text":"one"}]}]},
            {"type":"rich_text_quote","elements":[{"type":"text","text":"quoted"}]},
            {"type":"rich_text_preformatted","elements":[{"type":"text","text":"code"}]}]}]));
        let Component::TextDisplay(t) = &c[0] else {
            panic!()
        };
        assert_eq!(
            t.content,
            "Hi **bold**$[unixtime 1700000000]\n1. one\n> quoted\n```\ncode\n```"
        );
    }

    #[test]
    fn inputs_become_labels() {
        let c = convert(
            json!([{"type":"input","label":{"type":"plain_text","text":"When"},"optional":true,
            "element":{"type":"datepicker","action_id":"d","initial_date":"2026-11-16"}},
            {"type":"input","label":{"type":"plain_text","text":"Count"},"element":{"type":"number_input","action_id":"n","is_decimal_allowed":false,"min_value":"1"}}]),
        );
        let doc = Document::new(0, c);
        assert!(
            doc.validate(Context::Modal).is_ok(),
            "{:?}",
            doc.validate(Context::Modal)
        );
        assert!(
            matches!(doc.find("n"), Some(Component::TextInput(t)) if t.format == TextFormat::Number && t.required == Some(true))
        );
    }

    #[test]
    fn attachments_become_containers() {
        let mut conv = Converter::new(Mode::Lenient, &NoResolver);
        let c = attachments(
            &mut conv,
            &[
                json!({"color":"danger","pretext":"Alert","title":"CPU","title_link":"https://grafana.example/d",
            "text":"*high*","fields":[{"title":"Host","value":"web1","short":true}],"footer":"Grafana","ts":1700000000,
            "thumb_url":"https://example.com/t.png"}),
            ],
        );
        assert_eq!(c.len(), 2);
        let Component::Container(box_) = &c[1] else {
            panic!()
        };
        assert_eq!(box_.accent_color, Some(0xa30200));
        let doc = Document::new(IS_COMPONENTS_V2, c);
        assert!(
            doc.validate(Context::Webhook).is_ok(),
            "{:?}",
            doc.validate(Context::Webhook)
        );
        assert!(
            doc.fallback_text(4000)
                .contains("**[CPU](https://grafana.example/d)**")
        );
    }
}
