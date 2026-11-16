//! 部品の木のモデル。Discord Components V2 の JSON（数値の `type` とフィールド名）をそのまま使う。
//!
//! - 読むときは、`type` に数値と Discord の名前（`"container"` など）の両方を受け付ける。書くときは数値にする。
//! - 知らないキーは捨てる（Discord が項目を増やしても読める）。
//! - 知らない `type` は [`Component::Unsupported`] にして、中の文字を拾える分だけ残す。
//! - Slack にしかない部品は、zoo 独自の番号（1000 番台）を使う。Discord が番号を増やしてもぶつからない。

use serde::de::{self, Deserializer};
use serde::ser::{self, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// メッセージのフラグ。立っていると本文（`content`）を隠し、部品だけを表示する。
pub const IS_COMPONENTS_V2: u32 = 1 << 15;

/// 部品の番号。Discord にあるものは Discord の番号、Slack にしかないものは 1000 番台。
pub mod kind {
    pub const ACTION_ROW: u16 = 1;
    pub const BUTTON: u16 = 2;
    pub const STRING_SELECT: u16 = 3;
    pub const TEXT_INPUT: u16 = 4;
    pub const USER_SELECT: u16 = 5;
    pub const ROLE_SELECT: u16 = 6;
    pub const MENTIONABLE_SELECT: u16 = 7;
    pub const CHANNEL_SELECT: u16 = 8;
    pub const SECTION: u16 = 9;
    pub const TEXT_DISPLAY: u16 = 10;
    pub const THUMBNAIL: u16 = 11;
    pub const MEDIA_GALLERY: u16 = 12;
    pub const FILE: u16 = 13;
    pub const SEPARATOR: u16 = 14;
    pub const CONTAINER: u16 = 17;
    pub const LABEL: u16 = 18;
    pub const FILE_UPLOAD: u16 = 19;
    pub const RADIO_GROUP: u16 = 21;
    pub const CHECKBOX_GROUP: u16 = 22;
    pub const CHECKBOX: u16 = 23;
    // zoo 独自（Slack にしかない部品）
    pub const UNSUPPORTED: u16 = 1000;
    pub const EXTERNAL_SELECT: u16 = 1001;
    pub const OVERFLOW: u16 = 1002;
    pub const DATE_PICKER: u16 = 1003;
    pub const TIME_PICKER: u16 = 1004;
    pub const DATETIME_PICKER: u16 = 1005;
    pub const VIDEO: u16 = 1008;
    pub const TABLE: u16 = 1009;
    pub const ALERT: u16 = 1010;
    pub const CARD: u16 = 1011;
    pub const CAROUSEL: u16 = 1012;
    pub const CHART: u16 = 1013;
    pub const GRID: u16 = 1014;
    pub const PLAN: u16 = 1015;
    pub const TASK_CARD: u16 = 1016;

    /// Discord の名前と zoo の名前。入力の `"type": "container"` を数値に直すのに使う。
    pub const NAMES: &[(&str, u16)] = &[
        ("action_row", ACTION_ROW),
        ("button", BUTTON),
        ("string_select", STRING_SELECT),
        ("text_input", TEXT_INPUT),
        ("user_select", USER_SELECT),
        ("role_select", ROLE_SELECT),
        ("mentionable_select", MENTIONABLE_SELECT),
        ("channel_select", CHANNEL_SELECT),
        ("section", SECTION),
        ("text_display", TEXT_DISPLAY),
        ("thumbnail", THUMBNAIL),
        ("media_gallery", MEDIA_GALLERY),
        ("file", FILE),
        ("separator", SEPARATOR),
        ("container", CONTAINER),
        ("label", LABEL),
        ("file_upload", FILE_UPLOAD),
        ("radio_group", RADIO_GROUP),
        ("checkbox_group", CHECKBOX_GROUP),
        ("checkbox", CHECKBOX),
        ("unsupported", UNSUPPORTED),
        ("external_select", EXTERNAL_SELECT),
        ("overflow", OVERFLOW),
        ("date_picker", DATE_PICKER),
        ("time_picker", TIME_PICKER),
        ("datetime_picker", DATETIME_PICKER),
        ("video", VIDEO),
        ("table", TABLE),
        ("alert", ALERT),
        ("card", CARD),
        ("carousel", CAROUSEL),
        ("chart", CHART),
        ("grid", GRID),
        ("plan", PLAN),
        ("task_card", TASK_CARD),
    ];

    pub fn from_name(name: &str) -> Option<u16> {
        NAMES.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
    }

    pub fn name(kind: u16) -> Option<&'static str> {
        NAMES.iter().find(|(_, k)| *k == kind).map(|(n, _)| *n)
    }
}

/// 保存と送信の形：`{"flags":32768,"components":[...]}`。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub flags: u32,
    #[serde(default)]
    pub components: Vec<Component>,
}

impl Document {
    pub fn new(flags: u32, components: Vec<Component>) -> Self {
        Self { flags, components }
    }

    /// 本文を隠して部品だけを表示するか。
    pub fn hides_content(&self) -> bool {
        self.flags & IS_COMPONENTS_V2 != 0
    }

    /// JSON から読む。壊れていれば None（クライアントは本文を表示する）。
    pub fn parse(json: &str) -> Option<Self> {
        serde_json::from_str(json).ok()
    }

    /// キーの順序が固定された JSON にする（Slack の再チェックはバイト単位で比べる）。
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Document serializes")
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Emoji {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub animated: bool,
}

/// 画像・動画・ファイルの参照（Discord の unfurled media item）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Media {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// 押す前の確認ダイアログ（zoo 独自。Slack の confirm）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Confirm {
    pub title: String,
    pub text: String,
    pub confirm: String,
    pub deny: String,
    /// `primary` か `danger`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<Emoji>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub default: bool,
}

/// user/role/mentionable/channel select の既定値。`type` は `user`・`role`・`channel`。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DefaultValue {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ActionRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default)]
    pub components: Vec<Component>,
}

pub mod button_style {
    pub const PRIMARY: u8 = 1;
    pub const SECONDARY: u8 = 2;
    pub const SUCCESS: u8 = 3;
    pub const DANGER: u8 = 4;
    pub const LINK: u8 = 5;
    pub const PREMIUM: u8 = 6;
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Button {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default = "secondary")]
    pub style: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<Emoji>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Discord の premium ボタン。zoo では使えない（検証で弾く）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sku_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    /// zoo 独自。押したときにアプリへ `values[custom_id]` として渡す値（Slack の button.value）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

fn secondary() -> u8 {
    button_style::SECONDARY
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StringSelect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    #[serde(default)]
    pub options: Vec<SelectOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

/// text_input の書式（zoo 独自。Slack の number/email/url/rich_text input）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFormat {
    #[default]
    Plain,
    Number,
    Email,
    Url,
    Rich,
}

impl TextFormat {
    fn is_plain(&self) -> bool {
        *self == TextFormat::Plain
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    /// 1: 1 行、2: 複数行
    #[serde(default = "one")]
    pub style: u8,
    /// 古い Discord の形（label の外で使う text_input）の見出し。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "TextFormat::is_plain")]
    pub format: TextFormat,
    /// format が number のときの範囲と、小数を許すか。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_value: Option<f64>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub decimal: bool,
}

fn one() -> u8 {
    1
}

/// user(5)・role(6)・mentionable(7)・channel(8) の select。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EntitySelect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_values: Vec<DefaultValue>,
    /// channel select の種類の絞り込み（Discord の channel type の番号）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channel_types: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Section {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default)]
    pub components: Vec<Component>,
    /// Discord では thumbnail か button。zoo では Slack の accessory（select、overflow、ピッカーなど）も置ける。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessory: Option<Box<Component>>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TextDisplay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// zoo の MFM
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Thumbnail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub media: Media,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spoiler: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GalleryItem {
    pub media: Media,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spoiler: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MediaGallery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default)]
    pub items: Vec<GalleryItem>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct File {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// アプリのメッセージでは `attachment://<名前>`。Webhook と Slack の取り込みでは https の外部ファイル（zoo 独自）。
    pub file: Media,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spoiler: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Separator {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default = "yes")]
    pub divider: bool,
    /// 1: 小、2: 大
    #[serde(default = "one")]
    pub spacing: u8,
}

impl Default for Separator {
    fn default() -> Self {
        Self {
            id: None,
            divider: true,
            spacing: 1,
        }
    }
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Container {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// 0xRRGGBB。入力では `"#rrggbb"` と `good`・`warning`・`danger` も受け付ける（zoo 独自）。
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_color"
    )]
    pub accent_color: Option<u32>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub spoiler: bool,
    #[serde(default)]
    pub components: Vec<Component>,
    /// zoo 独自（Slack の container ブロック）：見出し（MFM）と、折りたためるか。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsible: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub collapsed: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Label {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub component: Box<Component>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FileUpload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    /// 受け付けるファイルの種類（`image`・`video`・`audio` か `.pdf` のような拡張子）。Slack の filetypes も受け付ける。
    #[serde(default, skip_serializing_if = "Vec::is_empty", alias = "filetypes")]
    pub file_types: Vec<String>,
}

/// 選択肢の一覧から選ぶ部品（radio group、checkbox group、overflow）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct OptionGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    #[serde(default)]
    pub options: Vec<SelectOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

/// 1 つのチェックボックス（Discord の checkbox）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Checkbox {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub default: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
}

/// 入力に合わせてアプリに選択肢を問い合わせる select（Slack の external_select）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ExternalSelect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_query_length: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_values: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_values: Option<u8>,
    /// 最初から選ばれている選択肢
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_options: Vec<SelectOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

/// 日付（YYYY-MM-DD）・時刻（HH:mm）・日時（UNIX 秒）のピッカー。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Picker {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub custom_id: String,
    /// date: `YYYY-MM-DD`、time: `HH:mm`、datetime: UNIX 秒の文字列
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_value: Option<String>,
    /// time picker の IANA タイムゾーン
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
}

/// 動画（Slack の video ブロック）。埋め込みは許可したプロバイダーだけで、ほかはリンクとして表示する。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Video {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub title: String,
    pub video_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ColumnSetting {
    /// `left`・`center`・`right`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_wrapped: bool,
}

/// 表のセル。`text` は MFM。数値のセルは `number` で並べ替える。ボタンのセルも置ける（Slack の data_table）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TableCell {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<Box<Component>>,
}

/// 表（Slack の table と data_table）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Table {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default)]
    pub rows: Vec<Vec<TableCell>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub column_settings: Vec<ColumnSetting>,
    /// 1 行目を見出しにする（data_table）。見出しの列で並べ替えられる。
    #[serde(default, skip_serializing_if = "is_false")]
    pub header: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// 1 ページの行数（data_table）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u8>,
}

/// 目立たせる短いお知らせ（Slack の alert）。`level` は default・info・warning・error・success。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Alert {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
}

/// カード（Slack の card）。文字は MFM。ボタンは 3 個まで。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Card {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hero_image: Option<Media>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<Media>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtext: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Component>,
}

/// 横に並ぶカード（Slack の carousel）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Carousel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default)]
    pub cards: Vec<Component>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartPoint {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: String,
    #[serde(default)]
    pub data: Vec<ChartPoint>,
}

/// グラフ（Slack の data_visualization）。`chart_type` は pie・bar・area・line。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Chart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub title: String,
    pub chart_type: String,
    /// pie の区切り
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<ChartPoint>,
    /// bar・area・line の系列
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<ChartSeries>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridItem {
    /// 1: narrow、2: standard、3: wide（幅の比）
    #[serde(default = "two")]
    pub width: u8,
    /// start・center・end
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    pub component: Box<Component>,
}

fn two() -> u8 {
    2
}

/// 格子状の配置（Slack の layout）。行ごとに 1〜3 個並べる。狭い画面では縦に積む。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Grid {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// none・rows・outer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grid_lines: Option<String>,
    #[serde(default)]
    pub rows: Vec<Vec<GridItem>>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TaskSource {
    pub url: String,
    pub text: String,
}

/// 作業の進み具合（Slack の task_card）。`status` は pending・in_progress・complete・error。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct TaskCard {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub task_id: String,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<TaskSource>,
}

/// 作業の一覧（Slack の plan）。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    pub title: String,
    #[serde(default)]
    pub tasks: Vec<TaskCard>,
}

/// まだ知らない部品。中の文字だけを表示する。
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Unsupported {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<i32>,
    /// 元の type（`"25"` や `"slack:foo"` など）。メトリクスに使う。
    pub original_type: String,
    /// 拾えた文字（無ければ空）。
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    ActionRow(ActionRow),
    Button(Button),
    StringSelect(StringSelect),
    TextInput(TextInput),
    UserSelect(EntitySelect),
    RoleSelect(EntitySelect),
    MentionableSelect(EntitySelect),
    ChannelSelect(EntitySelect),
    Section(Section),
    TextDisplay(TextDisplay),
    Thumbnail(Thumbnail),
    MediaGallery(MediaGallery),
    File(File),
    Separator(Separator),
    Container(Container),
    Label(Label),
    FileUpload(FileUpload),
    RadioGroup(OptionGroup),
    CheckboxGroup(OptionGroup),
    Checkbox(Checkbox),
    Unsupported(Unsupported),
    ExternalSelect(ExternalSelect),
    Overflow(OptionGroup),
    DatePicker(Picker),
    TimePicker(Picker),
    DateTimePicker(Picker),
    Video(Video),
    Table(Table),
    Alert(Alert),
    Card(Card),
    Carousel(Carousel),
    Chart(Chart),
    Grid(Grid),
    Plan(Plan),
    TaskCard(TaskCard),
}

impl Component {
    pub fn kind(&self) -> u16 {
        use Component as C;
        match self {
            C::ActionRow(_) => kind::ACTION_ROW,
            C::Button(_) => kind::BUTTON,
            C::StringSelect(_) => kind::STRING_SELECT,
            C::TextInput(_) => kind::TEXT_INPUT,
            C::UserSelect(_) => kind::USER_SELECT,
            C::RoleSelect(_) => kind::ROLE_SELECT,
            C::MentionableSelect(_) => kind::MENTIONABLE_SELECT,
            C::ChannelSelect(_) => kind::CHANNEL_SELECT,
            C::Section(_) => kind::SECTION,
            C::TextDisplay(_) => kind::TEXT_DISPLAY,
            C::Thumbnail(_) => kind::THUMBNAIL,
            C::MediaGallery(_) => kind::MEDIA_GALLERY,
            C::File(_) => kind::FILE,
            C::Separator(_) => kind::SEPARATOR,
            C::Container(_) => kind::CONTAINER,
            C::Label(_) => kind::LABEL,
            C::FileUpload(_) => kind::FILE_UPLOAD,
            C::RadioGroup(_) => kind::RADIO_GROUP,
            C::CheckboxGroup(_) => kind::CHECKBOX_GROUP,
            C::Checkbox(_) => kind::CHECKBOX,
            C::Unsupported(_) => kind::UNSUPPORTED,
            C::ExternalSelect(_) => kind::EXTERNAL_SELECT,
            C::Overflow(_) => kind::OVERFLOW,
            C::DatePicker(_) => kind::DATE_PICKER,
            C::TimePicker(_) => kind::TIME_PICKER,
            C::DateTimePicker(_) => kind::DATETIME_PICKER,
            C::Video(_) => kind::VIDEO,
            C::Table(_) => kind::TABLE,
            C::Alert(_) => kind::ALERT,
            C::Card(_) => kind::CARD,
            C::Carousel(_) => kind::CAROUSEL,
            C::Chart(_) => kind::CHART,
            C::Grid(_) => kind::GRID,
            C::Plan(_) => kind::PLAN,
            C::TaskCard(_) => kind::TASK_CARD,
        }
    }

    pub fn text(content: impl Into<String>) -> Self {
        Component::TextDisplay(TextDisplay {
            id: None,
            content: content.into(),
        })
    }

    /// 子の部品（action row、section、container の中身と、section の accessory、label の中身）。
    pub fn children(&self) -> Vec<&Component> {
        match self {
            Component::ActionRow(r) => r.components.iter().collect(),
            Component::Container(c) => c.components.iter().collect(),
            Component::Section(s) => s
                .components
                .iter()
                .chain(s.accessory.as_deref())
                .collect(),
            Component::Label(l) => vec![l.component.as_ref()],
            Component::Card(c) => c.actions.iter().collect(),
            Component::Carousel(c) => c.cards.iter().collect(),
            Component::Grid(g) => g.rows.iter().flatten().map(|i| i.component.as_ref()).collect(),
            Component::Table(t) => t.rows.iter().flatten().filter_map(|c| c.button.as_deref()).collect(),
            _ => Vec::new(),
        }
    }

    pub fn children_mut(&mut self) -> Vec<&mut Component> {
        match self {
            Component::ActionRow(r) => r.components.iter_mut().collect(),
            Component::Container(c) => c.components.iter_mut().collect(),
            Component::Section(s) => s
                .components
                .iter_mut()
                .chain(s.accessory.as_deref_mut())
                .collect(),
            Component::Label(l) => vec![l.component.as_mut()],
            Component::Card(c) => c.actions.iter_mut().collect(),
            Component::Carousel(c) => c.cards.iter_mut().collect(),
            Component::Grid(g) => g.rows.iter_mut().flatten().map(|i| i.component.as_mut()).collect(),
            Component::Table(t) => t.rows.iter_mut().flatten().filter_map(|c| c.button.as_deref_mut()).collect(),
            _ => Vec::new(),
        }
    }

    /// アプリへ値を送る部品なら、その custom_id。
    pub fn custom_id(&self) -> Option<&str> {
        use Component as C;
        match self {
            C::Button(b) => b.custom_id.as_deref(),
            C::StringSelect(s) => Some(&s.custom_id),
            C::TextInput(t) => Some(&t.custom_id),
            C::UserSelect(s) | C::RoleSelect(s) | C::MentionableSelect(s) | C::ChannelSelect(s) => {
                Some(&s.custom_id)
            }
            C::FileUpload(f) => Some(&f.custom_id),
            C::RadioGroup(g) | C::CheckboxGroup(g) | C::Overflow(g) => Some(&g.custom_id),
            C::Checkbox(c) => Some(&c.custom_id),
            C::ExternalSelect(s) => Some(&s.custom_id),
            C::DatePicker(p) | C::TimePicker(p) | C::DateTimePicker(p) => Some(&p.custom_id),
            _ => None,
        }
    }

    /// 押せない状態にする（受け取るアプリがない Webhook と Slack の取り込み）。
    pub fn set_disabled(&mut self) {
        use Component as C;
        match self {
            C::Button(b) if b.custom_id.is_some() => b.disabled = true,
            C::StringSelect(s) => s.disabled = true,
            C::UserSelect(s) | C::RoleSelect(s) | C::MentionableSelect(s) | C::ChannelSelect(s) => {
                s.disabled = true
            }
            C::RadioGroup(g) | C::CheckboxGroup(g) | C::Overflow(g) => g.disabled = true,
            C::Checkbox(c) => c.disabled = true,
            C::ExternalSelect(s) => s.disabled = true,
            C::DatePicker(p) | C::TimePicker(p) | C::DateTimePicker(p) => p.disabled = true,
            _ => {}
        }
    }

    pub fn is_disabled(&self) -> bool {
        use Component as C;
        match self {
            C::Button(b) => b.disabled,
            C::StringSelect(s) => s.disabled,
            C::UserSelect(s) | C::RoleSelect(s) | C::MentionableSelect(s) | C::ChannelSelect(s) => {
                s.disabled
            }
            C::RadioGroup(g) | C::CheckboxGroup(g) | C::Overflow(g) => g.disabled,
            C::Checkbox(c) => c.disabled,
            C::ExternalSelect(s) => s.disabled,
            C::DatePicker(p) | C::TimePicker(p) | C::DateTimePicker(p) => p.disabled,
            _ => false,
        }
    }
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// `0xRRGGBB` の数値、`"#rrggbb"`、`"rrggbb"`、`good`・`warning`・`danger` を受け付ける。
pub fn parse_color(value: &str) -> Option<u32> {
    match value {
        "good" => Some(0x2eb886),
        "warning" => Some(0xdaa038),
        "danger" => Some(0xa30200),
        _ => {
            let hex = value.strip_prefix('#').unwrap_or(value);
            (hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))
                .then(|| u32::from_str_radix(hex, 16).ok())
                .flatten()
        }
    }
}

fn deserialize_color<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    match Option::<Value>::deserialize(d)? {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|n| *n <= 0xFF_FFFF)
            .map(|n| Some(n as u32))
            .ok_or_else(|| de::Error::custom("accent_color must be 0..=0xFFFFFF")),
        Some(Value::String(s)) => parse_color(&s)
            .map(Some)
            .ok_or_else(|| de::Error::custom("invalid accent_color")),
        Some(_) => Err(de::Error::custom("invalid accent_color")),
    }
}

/// `type` の値を番号にする。数値と名前の両方を受け付け、知らなければ None。
fn type_of(value: &Value) -> Result<Option<u16>, String> {
    match value.get("type") {
        Some(Value::Number(n)) => Ok(n.as_u64().and_then(|n| u16::try_from(n).ok()).filter(|k| kind::name(*k).is_some())),
        Some(Value::String(s)) => Ok(kind::from_name(s)),
        _ => Err("component needs a type".into()),
    }
}

fn original_type(value: &Value) -> String {
    match value.get("type") {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// 知らない部品から、表示できる文字を拾う。
pub fn salvage_text(value: &Value) -> String {
    let mut out = Vec::new();
    fn walk(v: &Value, out: &mut Vec<String>, depth: usize) {
        if depth > 6 || out.len() > 20 {
            return;
        }
        match v {
            Value::Object(map) => {
                for key in ["label", "title", "content", "text", "alt_text", "description", "placeholder"] {
                    match map.get(key) {
                        Some(Value::String(s)) if !s.trim().is_empty() => out.push(s.clone()),
                        Some(obj @ Value::Object(_)) => walk(obj, out, depth + 1),
                        _ => {}
                    }
                }
                for key in ["components", "elements", "fields", "items", "component"] {
                    if let Some(child) = map.get(key) {
                        walk(child, out, depth + 1);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out, depth + 1)),
            _ => {}
        }
    }
    walk(value, &mut out, 0);
    out.dedup();
    out.join("\n")
}

impl<'de> Deserialize<'de> for Component {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(d)?;
        Component::from_value(value).map_err(de::Error::custom)
    }
}

impl Component {
    /// JSON の値から読む。知らない type は [`Component::Unsupported`] にする。
    pub fn from_value(value: Value) -> Result<Self, String> {
        if !value.is_object() {
            return Err("component must be an object".into());
        }
        let Some(k) = type_of(&value)? else {
            return Ok(Component::Unsupported(Unsupported {
                id: value.get("id").and_then(Value::as_i64).and_then(|i| i32::try_from(i).ok()),
                original_type: original_type(&value),
                text: salvage_text(&value),
            }));
        };
        fn from<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
            serde_json::from_value(v).map_err(|e| e.to_string())
        }
        use Component as C;
        Ok(match k {
            kind::ACTION_ROW => C::ActionRow(from(value)?),
            kind::BUTTON => C::Button(from(value)?),
            kind::STRING_SELECT => C::StringSelect(from(value)?),
            kind::TEXT_INPUT => C::TextInput(from(value)?),
            kind::USER_SELECT => C::UserSelect(from(value)?),
            kind::ROLE_SELECT => C::RoleSelect(from(value)?),
            kind::MENTIONABLE_SELECT => C::MentionableSelect(from(value)?),
            kind::CHANNEL_SELECT => C::ChannelSelect(from(value)?),
            kind::SECTION => C::Section(from(value)?),
            kind::TEXT_DISPLAY => C::TextDisplay(from(value)?),
            kind::THUMBNAIL => C::Thumbnail(from(value)?),
            kind::MEDIA_GALLERY => C::MediaGallery(from(value)?),
            kind::FILE => C::File(from(value)?),
            kind::SEPARATOR => C::Separator(from(value)?),
            kind::CONTAINER => C::Container(from(value)?),
            kind::LABEL => C::Label(from(value)?),
            kind::FILE_UPLOAD => C::FileUpload(from(value)?),
            kind::RADIO_GROUP => C::RadioGroup(from(value)?),
            kind::CHECKBOX_GROUP => C::CheckboxGroup(from(value)?),
            kind::CHECKBOX => C::Checkbox(from(value)?),
            kind::UNSUPPORTED => C::Unsupported(from(value)?),
            kind::EXTERNAL_SELECT => C::ExternalSelect(from(value)?),
            kind::OVERFLOW => C::Overflow(from(value)?),
            kind::DATE_PICKER => C::DatePicker(from(value)?),
            kind::TIME_PICKER => C::TimePicker(from(value)?),
            kind::DATETIME_PICKER => C::DateTimePicker(from(value)?),
            kind::VIDEO => C::Video(from(value)?),
            kind::TABLE => C::Table(from(value)?),
            kind::ALERT => C::Alert(from(value)?),
            kind::CARD => C::Card(from(value)?),
            kind::CAROUSEL => C::Carousel(from(value)?),
            kind::CHART => C::Chart(from(value)?),
            kind::GRID => C::Grid(from(value)?),
            kind::PLAN => C::Plan(from(value)?),
            kind::TASK_CARD => C::TaskCard(from(value)?),
            _ => unreachable!("type_of only returns known kinds"),
        })
    }
}

impl Serialize for Component {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        fn to<T: Serialize>(v: &T) -> Result<Value, serde_json::Error> {
            serde_json::to_value(v)
        }
        use Component as C;
        let body = match self {
            C::ActionRow(v) => to(v),
            C::Button(v) => to(v),
            C::StringSelect(v) => to(v),
            C::TextInput(v) => to(v),
            C::UserSelect(v) | C::RoleSelect(v) | C::MentionableSelect(v) | C::ChannelSelect(v) => to(v),
            C::Section(v) => to(v),
            C::TextDisplay(v) => to(v),
            C::Thumbnail(v) => to(v),
            C::MediaGallery(v) => to(v),
            C::File(v) => to(v),
            C::Separator(v) => to(v),
            C::Container(v) => to(v),
            C::Label(v) => to(v),
            C::FileUpload(v) => to(v),
            C::RadioGroup(v) | C::CheckboxGroup(v) | C::Overflow(v) => to(v),
            C::Checkbox(v) => to(v),
            C::Unsupported(v) => to(v),
            C::ExternalSelect(v) => to(v),
            C::DatePicker(v) | C::TimePicker(v) | C::DateTimePicker(v) => to(v),
            C::Video(v) => to(v),
            C::Table(v) => to(v),
            C::Alert(v) => to(v),
            C::Card(v) => to(v),
            C::Carousel(v) => to(v),
            C::Chart(v) => to(v),
            C::Grid(v) => to(v),
            C::Plan(v) => to(v),
            C::TaskCard(v) => to(v),
        }
        .map_err(ser::Error::custom)?;
        let Value::Object(fields) = body else {
            return Err(ser::Error::custom("component must serialize to an object"));
        };
        // serde_json の Map はキーの順に並ぶので、出力のバイト列は毎回同じになる。
        let mut map = Map::new();
        map.insert("type".into(), Value::from(self.kind()));
        map.extend(fields);
        map.serialize(s)
    }
}
