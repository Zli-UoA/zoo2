//! 部品の木の検証。配置のルールと上限は Discord に合わせ、zoo 独自の部品と使い道（Context）の分を足す。

use crate::model::*;
use std::collections::HashSet;

/// どこに置く文書か。置ける部品と上限が変わる。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    /// アプリが自分のメッセージに付ける部品。操作部品を置ける。
    AppMessage,
    /// リンクの展開。操作部品は置けない。
    Unfurl,
    /// 受信 Webhook と Slack の取り込み。操作部品は disabled のときだけ置ける。
    Webhook,
    Home,
    Modal,
    Ephemeral,
}

impl Context {
    fn max_components(self) -> usize {
        match self {
            Context::Home | Context::Modal => limits::MAX_COMPONENTS_SURFACE,
            Context::Webhook => limits::MAX_COMPONENTS_WEBHOOK,
            _ => limits::MAX_COMPONENTS,
        }
    }
    fn max_text(self) -> usize {
        match self {
            Context::Home => limits::MAX_TEXT_HOME,
            Context::Webhook => limits::MAX_TEXT_WEBHOOK,
            _ => limits::MAX_TEXT,
        }
    }
    fn max_bytes(self) -> usize {
        match self {
            Context::Webhook => limits::MAX_BYTES_WEBHOOK,
            _ => limits::MAX_BYTES,
        }
    }
    fn interactive(self) -> bool {
        matches!(
            self,
            Context::AppMessage | Context::Home | Context::Modal | Context::Ephemeral
        )
    }
}

pub mod limits {
    /// 入れ子を含めた部品の数（Discord のメッセージと同じ）。
    pub const MAX_COMPONENTS: usize = 40;
    /// ホームとモーダル
    pub const MAX_COMPONENTS_SURFACE: usize = 100;
    /// 受信 Webhook と Slack の取り込み（旧形式では attachments を 100 件まで受け付けていたため、広めにする）
    pub const MAX_COMPONENTS_WEBHOOK: usize = 500;
    pub const MAX_TEXT_WEBHOOK: usize = 100_000;
    pub const MAX_BYTES_WEBHOOK: usize = 256 * 1024;
    /// text_display・label・表のセルなどの文字の合計
    pub const MAX_TEXT: usize = 4000;
    pub const MAX_TEXT_HOME: usize = 8000;
    /// 保存する JSON の大きさ
    pub const MAX_BYTES: usize = 64 * 1024;
    /// 表のセルの文字の合計（Slack の data_table と同じ）
    pub const MAX_TABLE_TEXT: usize = 20_000;
    pub const MAX_TABLE_DATA_ROWS: usize = 201;
    pub const MAX_CARD_ACTIONS: usize = 3;
    pub const MAX_CAROUSEL_CARDS: usize = 10;
    pub const MAX_GRID_ROWS: usize = 5;
    pub const MAX_GRID_ITEMS: usize = 3;
    pub const MAX_CHART_SERIES: usize = 12;
    pub const MAX_CHART_POINTS: usize = 20;
    pub const MAX_PLAN_TASKS: usize = 50;
    pub const MAX_CUSTOM_ID: usize = 100;
    pub const MAX_BUTTON_LABEL: usize = 80;
    pub const MAX_BUTTONS_PER_ROW: usize = 5;
    pub const MAX_OPTIONS: usize = 25;
    pub const MAX_OVERFLOW_OPTIONS: usize = 5;
    pub const MAX_OPTION_TEXT: usize = 100;
    pub const MAX_PLACEHOLDER: usize = 150;
    pub const MAX_INPUT_LENGTH: u16 = 4000;
    pub const MAX_GALLERY_ITEMS: usize = 10;
    pub const MAX_URL: usize = 2000;
    pub const MAX_DESCRIPTION: usize = 1024;
    pub const MAX_LABEL: usize = 45;
    pub const MAX_LABEL_DESCRIPTION: usize = 100;
    pub const MAX_BUTTON_VALUE: usize = 2000;

    pub const MAX_TABLE_COLUMNS: usize = 20;
    pub const MAX_SECTION_TEXTS: usize = 3;
    pub const MAX_FILES: u8 = 10;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    /// `components[0].components[2]` のような位置
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// 部品がどこにあるか（配置のルールに使う）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    Top,
    Container,
    ActionRow,
    SectionText,
    Accessory,
    Label,
    CardAction,
    Carousel,
    GridItem,
    TableCell,
}

struct Walker<'a> {
    cx: Context,
    errors: Vec<ValidationError>,
    count: usize,
    text: usize,
    table_text: usize,
    ids: HashSet<&'a str>,
}

/// https で、ホストがあり、長すぎない URL か。
pub fn is_https_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty()
        && !host.contains('@')
        && url.chars().count() <= limits::MAX_URL
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

impl<'a> Walker<'a> {
    fn err(&mut self, path: &str, message: impl Into<String>) {
        self.errors.push(ValidationError {
            path: path.to_string(),
            message: message.into(),
        });
    }

    fn add_text(&mut self, s: &str) {
        self.text += chars(s);
    }

    fn len(&mut self, path: &str, field: &str, value: Option<&str>, max: usize) {
        if let Some(v) = value
            && chars(v) > max
        {
            self.err(path, format!("{field} is longer than {max} characters"));
        }
    }

    fn custom_id(&mut self, path: &str, id: &'a str) {
        if id.is_empty() || chars(id) > limits::MAX_CUSTOM_ID {
            self.err(path, "custom_id must be 1-100 characters");
        }
        if !self.ids.insert(id) {
            self.err(path, format!("custom_id {id:?} is used twice"));
        }
    }

    fn interactive(&mut self, path: &str, c: &'a Component) {
        if let Some(id) = c.custom_id() {
            self.custom_id(path, id);
            if !self.cx.interactive() && !(self.cx == Context::Webhook && c.is_disabled()) {
                self.err(path, "interactive components are not allowed here");
            }
        }
    }

    fn confirm(&mut self, path: &str, confirm: &Option<Confirm>) {
        if let Some(c) = confirm {
            self.add_text(&c.title);
            self.add_text(&c.text);
            for (f, v, max) in [
                ("confirm.title", &c.title, 100),
                ("confirm.text", &c.text, 300),
                ("confirm.confirm", &c.confirm, 30),
                ("confirm.deny", &c.deny, 30),
            ] {
                if v.is_empty() {
                    self.err(path, format!("{f} is required"));
                }
                self.len(path, f, Some(v), max);
            }
        }
    }

    fn range(
        &mut self,
        path: &str,
        min: Option<u8>,
        max: Option<u8>,
        count: usize,
        max_allowed: usize,
    ) {
        let min = min.unwrap_or(1) as usize;
        let max = max.unwrap_or(1) as usize;
        if min > max || max > max_allowed.max(1) || (count > 0 && min > count) {
            self.err(path, "min_values/max_values are out of range");
        }
    }

    fn options(&mut self, path: &str, options: &[SelectOption], max: usize) {
        if options.is_empty() || options.len() > max {
            self.err(path, format!("needs 1-{max} options"));
        }
        let mut values = HashSet::new();
        for o in options {
            self.len(
                path,
                "option.label",
                Some(&o.label),
                limits::MAX_OPTION_TEXT,
            );
            self.len(
                path,
                "option.value",
                Some(&o.value),
                limits::MAX_OPTION_TEXT,
            );
            self.len(
                path,
                "option.description",
                o.description.as_deref(),
                limits::MAX_OPTION_TEXT,
            );
            if o.label.is_empty() || o.value.is_empty() {
                self.err(path, "option label and value are required");
            }
            if !values.insert(o.value.as_str()) {
                self.err(path, "option values must be unique");
            }
        }
    }

    fn media(&mut self, path: &str, media: &Media, allow_attachment: bool) {
        let ok = is_https_url(&media.url)
            || (allow_attachment && media.url.starts_with("attachment://") && media.url.len() > 13);
        if !ok {
            self.err(path, "media url must be https");
        }
    }

    fn children(&mut self, path: &str, items: &'a [Component], place: Place) {
        for (i, c) in items.iter().enumerate() {
            self.component(&format!("{path}.components[{i}]"), c, place);
        }
    }

    fn component(&mut self, path: &str, c: &'a Component, place: Place) {
        if place != Place::TableCell {
            self.count += 1;
        }
        use Component as C;
        // 配置のルール
        let block = matches!(place, Place::Top | Place::Container | Place::GridItem);
        let allowed = match c {
            C::ActionRow(_)
            | C::MediaGallery(_)
            | C::File(_)
            | C::Separator(_)
            | C::Video(_)
            | C::Table(_)
            | C::Section(_)
            | C::Label(_)
            | C::Alert(_)
            | C::Chart(_)
            | C::Plan(_)
            | C::TaskCard(_) => block,
            C::TextDisplay(_) => block || place == Place::SectionText,
            C::Container(_) => matches!(place, Place::Top | Place::GridItem),
            C::Card(_) => block || place == Place::Carousel,
            C::Carousel(_) | C::Grid(_) => {
                matches!(place, Place::Top | Place::GridItem | Place::Container)
            }
            C::Thumbnail(_) => place == Place::Accessory,
            C::Button(_) => matches!(
                place,
                Place::ActionRow | Place::Accessory | Place::CardAction | Place::TableCell
            ),
            C::Overflow(_) => matches!(place, Place::ActionRow | Place::Accessory),
            C::StringSelect(_)
            | C::UserSelect(_)
            | C::RoleSelect(_)
            | C::MentionableSelect(_)
            | C::ChannelSelect(_)
            | C::ExternalSelect(_)
            | C::DatePicker(_)
            | C::TimePicker(_)
            | C::DateTimePicker(_)
            | C::RadioGroup(_)
            | C::CheckboxGroup(_)
            | C::Checkbox(_) => matches!(place, Place::ActionRow | Place::Accessory | Place::Label),
            C::TextInput(_) | C::FileUpload(_) => place == Place::Label,
            C::Unsupported(_) => true,
        };
        if !allowed {
            self.err(
                path,
                format!(
                    "{} cannot be placed here",
                    kind::name(c.kind()).unwrap_or("component")
                ),
            );
        }
        self.interactive(path, c);
        match c {
            C::ActionRow(r) => {
                // ボタン（と overflow）を 1〜5 個並べるか、ほかの部品を 1 つだけ置く。
                let small = r
                    .components
                    .iter()
                    .all(|c| matches!(c, C::Button(_) | C::Overflow(_)));
                let ok = if small {
                    (1..=limits::MAX_BUTTONS_PER_ROW).contains(&r.components.len())
                } else {
                    r.components.len() == 1
                };
                if !ok {
                    self.err(path, "action row needs 1-5 buttons or one select");
                }
                self.children(path, &r.components, Place::ActionRow);
            }
            C::Button(b) => {
                self.len(path, "label", b.label.as_deref(), limits::MAX_BUTTON_LABEL);
                self.len(path, "value", b.value.as_deref(), limits::MAX_BUTTON_VALUE);
                if let Some(l) = &b.label {
                    self.add_text(l);
                }
                if b.label.as_deref().is_none_or(str::is_empty) && b.emoji.is_none() {
                    self.err(path, "button needs a label or emoji");
                }
                match b.style {
                    button_style::LINK => {
                        if b.custom_id.is_some() || !b.url.as_deref().is_some_and(is_https_url) {
                            self.err(path, "link buttons need an https url and no custom_id");
                        }
                    }
                    1..=4 => {
                        if b.custom_id.is_none() || b.url.is_some() {
                            self.err(path, "buttons need a custom_id and no url");
                        }
                    }
                    _ => self.err(path, "unsupported button style"),
                }
                if b.sku_id.is_some() {
                    self.err(path, "premium buttons are not supported");
                }
                self.confirm(path, &b.confirm);
            }
            C::StringSelect(s) => {
                self.options(path, &s.options, limits::MAX_OPTIONS);
                self.range(
                    path,
                    s.min_values.or(Some(1)),
                    s.max_values,
                    s.options.len(),
                    limits::MAX_OPTIONS,
                );
                self.len(
                    path,
                    "placeholder",
                    s.placeholder.as_deref(),
                    limits::MAX_PLACEHOLDER,
                );
                self.confirm(path, &s.confirm);
            }
            C::TextInput(t) => {
                if !matches!(t.style, 1 | 2) {
                    self.err(path, "text_input style must be 1 or 2");
                }
                if t.min_length.unwrap_or(0) > t.max_length.unwrap_or(limits::MAX_INPUT_LENGTH)
                    || t.max_length.unwrap_or(0) > limits::MAX_INPUT_LENGTH
                {
                    self.err(path, "min_length/max_length are out of range");
                }
                self.len(
                    path,
                    "value",
                    t.value.as_deref(),
                    limits::MAX_INPUT_LENGTH as usize,
                );
                self.len(
                    path,
                    "placeholder",
                    t.placeholder.as_deref(),
                    limits::MAX_PLACEHOLDER,
                );
                if let (Some(a), Some(b)) = (t.min_value, t.max_value)
                    && a > b
                {
                    self.err(path, "min_value is greater than max_value");
                }
            }
            C::UserSelect(s) | C::RoleSelect(s) | C::MentionableSelect(s) | C::ChannelSelect(s) => {
                self.range(path, s.min_values, s.max_values, 0, limits::MAX_OPTIONS);
                self.len(
                    path,
                    "placeholder",
                    s.placeholder.as_deref(),
                    limits::MAX_PLACEHOLDER,
                );
                if s.default_values.len() > s.max_values.unwrap_or(1) as usize {
                    self.err(path, "too many default_values");
                }
                for d in &s.default_values {
                    if !matches!(d.kind.as_str(), "user" | "role" | "channel") || d.id.is_empty() {
                        self.err(path, "invalid default value");
                    }
                }
                self.confirm(path, &s.confirm);
            }
            C::Section(s) => {
                if s.components.is_empty() || s.components.len() > limits::MAX_SECTION_TEXTS {
                    self.err(path, "section needs 1-3 text displays");
                }
                self.children(path, &s.components, Place::SectionText);
                if let Some(a) = &s.accessory {
                    self.component(&format!("{path}.accessory"), a, Place::Accessory);
                }
            }
            C::TextDisplay(t) => {
                self.add_text(&t.content);
                if t.content.trim().is_empty() {
                    self.err(path, "text display is empty");
                }
            }
            C::Thumbnail(t) => {
                self.media(path, &t.media, false);
                self.len(
                    path,
                    "description",
                    t.description.as_deref(),
                    limits::MAX_DESCRIPTION,
                );
            }
            C::MediaGallery(g) => {
                if g.items.is_empty() || g.items.len() > limits::MAX_GALLERY_ITEMS {
                    self.err(path, "media gallery needs 1-10 items");
                }
                for (i, item) in g.items.iter().enumerate() {
                    let p = format!("{path}.items[{i}]");
                    self.media(&p, &item.media, false);
                    self.len(
                        &p,
                        "description",
                        item.description.as_deref(),
                        limits::MAX_DESCRIPTION,
                    );
                }
            }
            C::File(f) => {
                let attachment = self.cx == Context::AppMessage;
                self.media(path, &f.file, attachment);
                if f.file.url.starts_with("attachment://") && !attachment {
                    self.err(path, "attachment:// files are only allowed in app messages");
                }
                self.len(path, "name", f.name.as_deref(), 255);
            }
            C::Separator(s) => {
                if !matches!(s.spacing, 1 | 2) {
                    self.err(path, "separator spacing must be 1 or 2");
                }
            }
            C::Container(c) => {
                if c.components.is_empty() {
                    self.err(path, "container is empty");
                }
                if c.accent_color.is_some_and(|v| v > 0xFF_FFFF) {
                    self.err(path, "accent_color must be 0..=0xFFFFFF");
                }
                self.children(path, &c.components, Place::Container);
            }
            C::Label(l) => {
                self.add_text(&l.label);
                if l.label.trim().is_empty() {
                    self.err(path, "label is empty");
                }
                self.len(path, "label", Some(&l.label), limits::MAX_LABEL);
                self.len(
                    path,
                    "description",
                    l.description.as_deref(),
                    limits::MAX_LABEL_DESCRIPTION,
                );
                self.component(&format!("{path}.component"), &l.component, Place::Label);
            }
            C::FileUpload(f) => {
                self.range(
                    path,
                    f.min_values,
                    f.max_values,
                    0,
                    limits::MAX_FILES as usize,
                );
            }
            C::RadioGroup(g) => {
                self.options(path, &g.options, 10);
                self.confirm(path, &g.confirm);
            }
            C::CheckboxGroup(g) => {
                self.options(path, &g.options, 10);
                self.range(
                    path,
                    g.min_values.or(Some(0)),
                    g.max_values.or(Some(g.options.len().min(255) as u8)),
                    g.options.len(),
                    10,
                );
                self.confirm(path, &g.confirm);
            }
            C::Overflow(g) => {
                self.options(path, &g.options, limits::MAX_OVERFLOW_OPTIONS);
                self.confirm(path, &g.confirm);
            }
            C::Checkbox(_) => {}
            C::ExternalSelect(s) => {
                self.range(path, s.min_values, s.max_values, 0, limits::MAX_OPTIONS);
                self.len(
                    path,
                    "placeholder",
                    s.placeholder.as_deref(),
                    limits::MAX_PLACEHOLDER,
                );
                for o in &s.initial_options {
                    self.len(
                        path,
                        "option.label",
                        Some(&o.label),
                        limits::MAX_OPTION_TEXT,
                    );
                }
                self.confirm(path, &s.confirm);
            }
            C::DatePicker(p) | C::TimePicker(p) | C::DateTimePicker(p) => {
                if let Some(v) = &p.initial_value {
                    let ok = match c {
                        C::DatePicker(_) => is_date(v),
                        C::TimePicker(_) => is_time(v),
                        _ => v.parse::<i64>().is_ok(),
                    };
                    if !ok {
                        self.err(path, "invalid initial_value");
                    }
                }
                self.len(
                    path,
                    "placeholder",
                    p.placeholder.as_deref(),
                    limits::MAX_PLACEHOLDER,
                );
                self.confirm(path, &p.confirm);
            }
            C::Video(v) => {
                self.add_text(&v.title);
                if v.title.trim().is_empty() || !is_https_url(&v.video_url) {
                    self.err(path, "video needs a title and an https video_url");
                }
                for u in [&v.thumbnail_url, &v.title_url].into_iter().flatten() {
                    if !is_https_url(u) {
                        self.err(path, "video urls must be https");
                    }
                }
                self.len(path, "title", Some(&v.title), 200);
                self.len(path, "description", v.description.as_deref(), 200);
            }
            C::Table(t) => {
                let max_rows = if t.header {
                    limits::MAX_TABLE_DATA_ROWS
                } else {
                    100
                };
                let width = t.rows.first().map_or(0, Vec::len);
                if t.rows.is_empty()
                    || t.rows.len() > max_rows
                    || t.rows
                        .iter()
                        .any(|r| r.is_empty() || r.len() > limits::MAX_TABLE_COLUMNS)
                    || (t.header && t.rows.iter().any(|r| r.len() != width))
                {
                    self.err(path, "table needs rows of 1-20 cells");
                }
                for (i, cell) in t.rows.iter().flatten().enumerate() {
                    self.table_text += chars(&cell.text);
                    if let Some(b) = &cell.button {
                        self.component(&format!("{path}.cells[{i}].button"), b, Place::TableCell);
                    }
                }
                self.len(path, "caption", t.caption.as_deref(), 200);
            }
            C::Alert(a) => {
                self.add_text(&a.content);
                self.len(path, "content", Some(&a.content), 200);
                if !matches!(
                    a.level.as_deref(),
                    None | Some("default" | "info" | "warning" | "error" | "success")
                ) {
                    self.err(path, "invalid alert level");
                }
            }
            C::Card(card) => {
                for t in [&card.title, &card.subtitle, &card.body, &card.subtext]
                    .into_iter()
                    .flatten()
                {
                    self.add_text(t);
                }
                self.len(path, "title", card.title.as_deref(), 150);
                self.len(path, "body", card.body.as_deref(), 200);
                for m in [&card.hero_image, &card.icon].into_iter().flatten() {
                    self.media(path, m, false);
                }
                if card.actions.len() > limits::MAX_CARD_ACTIONS {
                    self.err(path, "card has at most 3 buttons");
                }
                if card.title.is_none()
                    && card.body.is_none()
                    && card.hero_image.is_none()
                    && card.actions.is_empty()
                {
                    self.err(path, "card is empty");
                }
                for (i, a) in card.actions.iter().enumerate() {
                    self.component(&format!("{path}.actions[{i}]"), a, Place::CardAction);
                }
            }
            C::Carousel(c) => {
                if c.cards.is_empty() || c.cards.len() > limits::MAX_CAROUSEL_CARDS {
                    self.err(path, "carousel needs 1-10 cards");
                }
                for (i, card) in c.cards.iter().enumerate() {
                    self.component(&format!("{path}.cards[{i}]"), card, Place::Carousel);
                }
            }
            C::Chart(ch) => {
                self.len(path, "title", Some(&ch.title), 50);
                let ok = match ch.chart_type.as_str() {
                    "pie" => {
                        (1..=limits::MAX_CHART_SERIES).contains(&ch.segments.len())
                            && ch.segments.iter().all(|s| s.value > 0.0)
                    }
                    "bar" | "area" | "line" => {
                        (1..=limits::MAX_CHART_SERIES).contains(&ch.series.len())
                            && ch
                                .series
                                .iter()
                                .all(|s| (1..=limits::MAX_CHART_POINTS).contains(&s.data.len()))
                    }
                    _ => false,
                };
                let finite = ch
                    .segments
                    .iter()
                    .chain(ch.series.iter().flat_map(|s| s.data.iter()))
                    .all(|p| p.value.is_finite());
                if !ok || !finite {
                    self.err(path, "invalid chart");
                }
            }
            C::Grid(g) => {
                if g.rows.is_empty()
                    || g.rows.len() > limits::MAX_GRID_ROWS
                    || g.rows
                        .iter()
                        .any(|r| r.is_empty() || r.len() > limits::MAX_GRID_ITEMS)
                {
                    self.err(path, "grid needs 1-5 rows of 1-3 items");
                }
                for (r, row) in g.rows.iter().enumerate() {
                    for (i, item) in row.iter().enumerate() {
                        if !(1..=3).contains(&item.width) {
                            self.err(path, "grid item width must be 1-3");
                        }
                        self.component(
                            &format!("{path}.rows[{r}][{i}]"),
                            &item.component,
                            Place::GridItem,
                        );
                    }
                }
            }
            C::Plan(p) => {
                if p.tasks.len() > limits::MAX_PLAN_TASKS {
                    self.err(path, "plan has at most 50 tasks");
                }
                self.add_text(&p.title);
                for t in &p.tasks {
                    self.task(path, t);
                }
            }
            C::TaskCard(t) => self.task(path, t),
            C::Unsupported(u) => self.add_text(&u.text),
        }
    }
}

impl<'a> Walker<'a> {
    fn task(&mut self, path: &str, t: &TaskCard) {
        self.add_text(&t.title);
        for x in [&t.details, &t.output].into_iter().flatten() {
            self.add_text(x);
        }
        if !matches!(
            t.status.as_str(),
            "pending" | "in_progress" | "complete" | "error"
        ) {
            self.err(path, "invalid task status");
        }
        for src in &t.sources {
            if !is_https_url(&src.url) {
                self.err(path, "task source urls must be https");
            }
        }
    }
}

fn is_date(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
        && (1..=12).contains(&v[5..7].parse::<u8>().unwrap_or(0))
        && (1..=31).contains(&v[8..10].parse::<u8>().unwrap_or(0))
}

fn is_time(v: &str) -> bool {
    let b = v.as_bytes();
    b.len() == 5
        && b[2] == b':'
        && v[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && v[3..].parse::<u8>().is_ok_and(|m| m < 60)
}

impl Document {
    /// 文書を検証する。問題があれば、位置付きのエラーをすべて返す。
    pub fn validate(&self, cx: Context) -> Result<(), Vec<ValidationError>> {
        let mut w = Walker {
            cx,
            errors: Vec::new(),
            count: 0,
            text: 0,
            table_text: 0,
            ids: HashSet::new(),
        };
        if self.flags & !IS_COMPONENTS_V2 != 0 {
            w.err("flags", "only IS_COMPONENTS_V2 (32768) is supported");
        }
        for (i, c) in self.components.iter().enumerate() {
            w.component(&format!("components[{i}]"), c, Place::Top);
        }
        if w.count > cx.max_components() {
            w.err(
                "",
                format!("too many components (max {})", cx.max_components()),
            );
        }
        if w.table_text > limits::MAX_TABLE_TEXT {
            w.err(
                "",
                format!(
                    "too much table text (max {} characters)",
                    limits::MAX_TABLE_TEXT
                ),
            );
        }
        if w.text > cx.max_text() {
            w.err(
                "",
                format!("too much text (max {} characters)", cx.max_text()),
            );
        }
        if self.to_json().len() > cx.max_bytes() {
            w.err(
                "",
                format!("document is larger than {} KB", cx.max_bytes() / 1024),
            );
        }
        if w.errors.is_empty() {
            Ok(())
        } else {
            Err(w.errors)
        }
    }

    /// 操作部品を custom_id で探す。
    pub fn find(&self, custom_id: &str) -> Option<&Component> {
        self.walk()
            .into_iter()
            .find(|c| c.custom_id() == Some(custom_id))
    }

    /// 木のすべての部品（深さ優先）。
    pub fn walk(&self) -> Vec<&Component> {
        fn go<'a>(c: &'a Component, out: &mut Vec<&'a Component>) {
            out.push(c);
            for child in c.children() {
                go(child, out);
            }
        }
        let mut out = Vec::new();
        for c in &self.components {
            go(c, &mut out);
        }
        out
    }

    /// 操作部品をすべて押せない状態にする（受け取るアプリがない Webhook と Slack の取り込み）。
    pub fn disable_interactive(&mut self) {
        fn go(c: &mut Component) {
            c.set_disabled();
            for child in c.children_mut() {
                go(child);
            }
        }
        self.components.iter_mut().for_each(go);
    }

    /// 知らない部品の type（メトリクスに使う）。
    pub fn unsupported_types(&self) -> Vec<&str> {
        self.walk()
            .into_iter()
            .filter_map(|c| match c {
                Component::Unsupported(u) => Some(u.original_type.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(v: serde_json::Value) -> Document {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn accepts_string_types_and_writes_numbers() {
        let d = doc(
            json!({"flags":32768,"components":[{"type":"container","accent_color":"#ff0000","components":[
            {"type":"text_display","content":"hi"},{"type":"separator"},
            {"type":"section","components":[{"type":10,"content":"x"}],"accessory":{"type":"thumbnail","media":{"url":"https://e.x/a.png"}}}]}]}),
        );
        assert!(
            d.validate(Context::Webhook).is_ok(),
            "{:?}",
            d.validate(Context::Webhook)
        );
        let out: serde_json::Value = serde_json::from_str(&d.to_json()).unwrap();
        assert_eq!(out["components"][0]["type"], 17);
        assert_eq!(out["components"][0]["accent_color"], 0xff0000);
        assert_eq!(Document::parse(&d.to_json()).unwrap(), d);
    }

    #[test]
    fn unknown_types_keep_their_text() {
        let d = doc(
            json!({"components":[{"type":99,"label":"Hello","components":[{"content":"World"}]}]}),
        );
        assert!(
            matches!(&d.components[0], Component::Unsupported(u) if u.original_type == "99" && u.text == "Hello\nWorld")
        );
        assert_eq!(d.unsupported_types(), vec!["99"]);
    }

    #[test]
    fn enforces_placement_and_limits() {
        let nested = doc(
            json!({"components":[{"type":17,"components":[{"type":17,"components":[{"type":10,"content":"x"}]}]}]}),
        );
        assert!(nested.validate(Context::AppMessage).is_err());
        let row = |n: usize| {
            doc(
                json!({"components":[{"type":1,"components":(0..n).map(|i| json!({"type":2,"label":"b","custom_id":format!("b{i}")})).collect::<Vec<_>>()}]}),
            )
        };
        assert!(row(5).validate(Context::AppMessage).is_ok());
        assert!(row(6).validate(Context::AppMessage).is_err());
        let many =
            doc(json!({"components":(0..41).map(|_| json!({"type":14})).collect::<Vec<_>>()}));
        assert!(many.validate(Context::AppMessage).is_err());
        assert!(many.validate(Context::Home).is_ok());
        let long = doc(json!({"components":[{"type":10,"content":"a".repeat(4001)}]}));
        assert!(long.validate(Context::AppMessage).is_err());
        let dup = doc(
            json!({"components":[{"type":1,"components":[{"type":2,"label":"a","custom_id":"x"},{"type":2,"label":"b","custom_id":"x"}]}]}),
        );
        assert!(dup.validate(Context::AppMessage).is_err());
    }

    #[test]
    fn interactive_components_need_an_app_or_must_be_disabled() {
        let mut d = doc(
            json!({"components":[{"type":1,"components":[{"type":2,"label":"Go","custom_id":"go"},{"type":2,"style":5,"label":"Open","url":"https://e.x"}]}]}),
        );
        assert!(d.validate(Context::Webhook).is_err());
        d.disable_interactive();
        assert!(d.validate(Context::Webhook).is_ok());
        assert!(d.validate(Context::Unfurl).is_err());
    }

    #[test]
    fn rejects_unsafe_urls() {
        for url in [
            "javascript:alert(1)",
            "http://e.x",
            "https://",
            "https://user@e.x/",
            "https://e.x/a b",
        ] {
            let d = doc(json!({"components":[{"type":12,"items":[{"media":{"url":url}}]}]}));
            assert!(d.validate(Context::Webhook).is_err(), "{url}");
        }
    }

    #[test]
    fn modal_inputs_and_new_discord_types() {
        let d = doc(json!({"components":[
            {"type":18,"label":"Pick","component":{"type":21,"custom_id":"r","options":[{"label":"A","value":"a"},{"label":"B","value":"b"}]}},
            {"type":18,"label":"Many","component":{"type":22,"custom_id":"c","options":[{"label":"A","value":"a"}],"min_values":0}},
            {"type":18,"label":"Ok","component":{"type":23,"custom_id":"k"}},
            {"type":18,"label":"Files","component":{"type":19,"custom_id":"f","file_types":["image",".pdf"]}},
            {"type":18,"label":"Name","component":{"type":4,"custom_id":"n","style":1}}]}));
        assert!(
            d.validate(Context::Modal).is_ok(),
            "{:?}",
            d.validate(Context::Modal)
        );
        let bare = doc(json!({"components":[{"type":4,"custom_id":"n"}]}));
        assert!(bare.validate(Context::Modal).is_err());
    }

    #[test]
    fn slack_only_blocks_validate() {
        let d = doc(json!({"components":[
            {"type":"alert","content":"Heads up","level":"warning"},
            {"type":"carousel","cards":[{"type":"card","title":"A","actions":[{"type":2,"label":"Go","custom_id":"go"}]}]},
            {"type":"chart","title":"Sales","chart_type":"bar","series":[{"name":"2026","data":[{"label":"Q1","value":3}]}],"categories":["Q1"]},
            {"type":"grid","rows":[[{"component":{"type":10,"content":"left"}},{"width":1,"component":{"type":10,"content":"right"}}]]},
            {"type":"plan","title":"Steps","tasks":[{"task_id":"1","title":"Build","status":"complete"}]},
            {"type":"table","header":true,"rows":[[{"text":"Name"},{"text":"N"}],[{"text":"a","number":1},{"text":"","button":{"type":2,"label":"x","custom_id":"x"}}]]},
            {"type":"date_picker","custom_id":"bad"}]}));
        let errors = d.validate(Context::AppMessage).unwrap_err();
        // date_picker は action row か label の中にしか置けない
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(errors[0].path.starts_with("components[6]"));
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;

    /// fixtures/valid の文書が、名前の context で検証に通り、往復で変わらないこと。
    #[test]
    fn valid_fixtures_validate_and_round_trip() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/valid");
        let mut n = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let cx = match name.split('-').next().unwrap() {
                "message" => Context::AppMessage,
                "modal" => Context::Modal,
                "home" => Context::Home,
                "webhook" => Context::Webhook,
                other => panic!("unknown context {other} in {name}"),
            };
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let doc: Document = serde_json::from_value(v["document"].clone()).unwrap();
            assert!(doc.validate(cx).is_ok(), "{name}: {:?}", doc.validate(cx));
            assert_eq!(
                Document::parse(&doc.to_json()).as_ref(),
                Some(&doc),
                "{name}"
            );
            if let Some(expected) = v["fallback"].as_str() {
                assert_eq!(doc.fallback_text(40_000), expected, "{name}");
            }
            n += 1;
        }
        assert!(n >= 3);
    }
}
