//! Web 版 (`frontend/src/lib/i18n/`) と同じ翻訳を使う。
//!
//! 翻訳は `frontend/src/lib/i18n/locales/*.json` を埋め込む。
//! キーは `a.b.c` の入れ子、補間は `{{var}}`、複数形は i18next と同じ `_one` / `_other` などの接尾辞。
//! 言語の決め方は `locale.ts` と同じで、見つからなければ ja に戻す。

use std::collections::HashMap;
use std::sync::OnceLock;

use serde_json::Value;

/// 対応する言語。`locale.ts` の `LOCALES` と同じ並び。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Locale {
    Ja,
    En,
    ZhHant,
    ZhHans,
    Yue,
    KoKr,
    KoKp,
}

/// フォールバックの言語 (`DEFAULT_LOCALE`)。
pub const DEFAULT_LOCALE: Locale = Locale::Ja;

impl Locale {
    pub const ALL: [Locale; 7] = [
        Locale::Ja,
        Locale::En,
        Locale::ZhHant,
        Locale::ZhHans,
        Locale::Yue,
        Locale::KoKr,
        Locale::KoKp,
    ];

    /// `locale.ts` の識別子 (`zh-Hant` など)。
    pub const fn code(self) -> &'static str {
        match self {
            Locale::Ja => "ja",
            Locale::En => "en",
            Locale::ZhHant => "zh-Hant",
            Locale::ZhHans => "zh-Hans",
            Locale::Yue => "yue",
            Locale::KoKr => "ko-KR",
            Locale::KoKp => "ko-KP",
        }
    }

    pub const fn native_name(self) -> &'static str {
        match self {
            Locale::Ja => "日本語",
            Locale::En => "English",
            Locale::ZhHant => "繁體中文",
            Locale::ZhHans => "简体中文",
            Locale::Yue => "廣東話",
            Locale::KoKr => "한국어",
            Locale::KoKp => "조선말 (문화어)",
        }
    }

    /// 識別子から言語を引く。`locale.ts` の `isLocale`。
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.code() == code)
    }

    /// OS などの言語タグを言語に寄せる。`locale.ts` の `matchLocale`。
    /// 文字種 (Hant / Hans) が地域より優先で、`zh` だけなら簡体字。
    pub fn match_tag(tag: &str) -> Option<Self> {
        // `ja_JP.UTF-8` や `de_DE@euro` のような POSIX の書き方も読む。
        let tag = tag.split(['.', '@']).next().unwrap_or(tag);
        let tag = tag.to_lowercase().replace('_', "-");
        let mut parts = tag.split('-');
        let language = parts.next()?;
        let subtags: Vec<&str> = parts.collect();
        match language {
            "ja" => Some(Locale::Ja),
            "en" => Some(Locale::En),
            "yue" => Some(Locale::Yue),
            "ko" => Some(if subtags.contains(&"kp") {
                Locale::KoKp
            } else {
                Locale::KoKr
            }),
            "zh" => {
                if subtags.contains(&"yue") {
                    Some(Locale::Yue)
                } else if subtags.contains(&"hant") {
                    Some(Locale::ZhHant)
                } else if subtags.contains(&"hans") {
                    Some(Locale::ZhHans)
                } else if subtags.iter().any(|p| ["tw", "hk", "mo"].contains(p)) {
                    Some(Locale::ZhHant)
                } else {
                    Some(Locale::ZhHans)
                }
            }
            _ => None,
        }
    }

    fn json(self) -> &'static str {
        match self {
            Locale::Ja => include_str!("../../frontend/src/lib/i18n/locales/ja.json"),
            Locale::En => include_str!("../../frontend/src/lib/i18n/locales/en.json"),
            Locale::ZhHant => include_str!("../../frontend/src/lib/i18n/locales/zh-Hant.json"),
            Locale::ZhHans => include_str!("../../frontend/src/lib/i18n/locales/zh-Hans.json"),
            Locale::Yue => include_str!("../../frontend/src/lib/i18n/locales/yue.json"),
            Locale::KoKr => include_str!("../../frontend/src/lib/i18n/locales/ko-KR.json"),
            Locale::KoKp => include_str!("../../frontend/src/lib/i18n/locales/ko-KP.json"),
        }
    }
}

/// 保存済みの指定があればそれ、なければ言語タグの先頭から順に合うものを使う。
/// `locale.ts` の `resolveLocale`。
pub fn resolve_locale<S: AsRef<str>>(stored: Option<&str>, languages: &[S]) -> Locale {
    if let Some(locale) = stored.and_then(Locale::from_code) {
        return locale;
    }
    languages
        .iter()
        .find_map(|tag| Locale::match_tag(tag.as_ref()))
        .unwrap_or(DEFAULT_LOCALE)
}

/// OS の言語から決める。環境変数 `ZOO_LOCALE` (`ja`, `zh-Hant` など) があればそれを優先する。
pub fn detect_locale() -> Locale {
    let stored = std::env::var("ZOO_LOCALE").ok();
    resolve_locale(
        stored.as_deref(),
        &sys_locale::get_locales().collect::<Vec<_>>(),
    )
}

type Flat = HashMap<String, String>;

fn flatten(prefix: &str, value: &Value, out: &mut Flat) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, v, out);
            }
        }
        Value::String(s) => {
            out.insert(prefix.to_string(), s.clone());
        }
        _ => {}
    }
}

fn resources() -> &'static HashMap<Locale, Flat> {
    static RESOURCES: OnceLock<HashMap<Locale, Flat>> = OnceLock::new();
    RESOURCES.get_or_init(|| {
        Locale::ALL
            .into_iter()
            .map(|locale| {
                let value: Value = serde_json::from_str(locale.json())
                    .unwrap_or_else(|e| panic!("{} の翻訳を読めません: {e}", locale.code()));
                let mut flat = Flat::new();
                flatten("", &value, &mut flat);
                (locale, flat)
            })
            .collect()
    })
}

/// 複数形の区分 (`Intl.PluralRules` の結果)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

impl Plural {
    pub const fn suffix(self) -> &'static str {
        match self {
            Plural::Zero => "zero",
            Plural::One => "one",
            Plural::Two => "two",
            Plural::Few => "few",
            Plural::Many => "many",
            Plural::Other => "other",
        }
    }
}

/// 対応する 7 言語の複数形の規則。en は 1 だけ one、ほかは (日本語・中国語・韓国語・広東語) 常に other。
pub fn plural_category(locale: Locale, count: i64) -> Plural {
    match locale {
        Locale::En if count == 1 => Plural::One,
        _ => Plural::Other,
    }
}

/// `{{var}}` を `vars` で置き換える。空白は許す (`{{ var }}`)。
/// 値が無い変数は i18next と同じく `{{var}}` のまま残す。
pub fn interpolate(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = after[..end].trim();
        match vars.iter().find(|(k, _)| *k == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(&rest[start..start + 2 + end + 2]),
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// 1 つの言語の翻訳。`t` で引く。
#[derive(Clone, Copy, Debug)]
pub struct I18n {
    locale: Locale,
}

impl I18n {
    pub fn new(locale: Locale) -> Self {
        Self { locale }
    }

    /// OS の言語で作る。
    pub fn detect() -> Self {
        Self::new(detect_locale())
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    fn lookup(&self, key: &str) -> Option<&'static str> {
        let res = resources();
        [self.locale, DEFAULT_LOCALE]
            .iter()
            .find_map(|l| res.get(l).and_then(|m| m.get(key)))
            .map(String::as_str)
    }

    /// キーを引く。無ければ ja、それも無ければキーをそのまま返す (i18next と同じ)。
    pub fn t(&self, key: &str) -> String {
        self.t_with(key, &[])
    }

    pub fn t_with(&self, key: &str, vars: &[(&str, &str)]) -> String {
        match self.lookup(key) {
            Some(template) => interpolate(template, vars),
            None => key.to_string(),
        }
    }

    /// `count` に合う複数形のキー (`key_one` など) を引き、`{{count}}` を埋める。
    /// 区分のキーが無ければ `key_other`、さらに無ければ `key` を使う。
    pub fn t_count(&self, key: &str, count: i64, vars: &[(&str, &str)]) -> String {
        let count_text = count.to_string();
        let mut all: Vec<(&str, &str)> = vec![("count", count_text.as_str())];
        all.extend_from_slice(vars);

        // 言語ごとに「区分 → other → 接尾辞なし」の順で試し、無ければ ja に戻る。
        let res = resources();
        for locale in [self.locale, DEFAULT_LOCALE] {
            let Some(map) = res.get(&locale) else {
                continue;
            };
            let category = plural_category(locale, count);
            let candidates = [
                format!("{key}_{}", category.suffix()),
                format!("{key}_other"),
                key.to_string(),
            ];
            if let Some(template) = candidates.iter().find_map(|k| map.get(k)) {
                return interpolate(template, &all);
            }
        }
        key.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_locales_parse_and_share_ja_keys() {
        let res = resources();
        let ja = &res[&Locale::Ja];
        assert!(ja.len() > 800);
        for locale in Locale::ALL {
            let keys = &res[&locale];
            assert_eq!(keys.len(), ja.len(), "{}", locale.code());
            for key in ja.keys() {
                assert!(keys.contains_key(key), "{}: {key}", locale.code());
            }
        }
    }

    #[test]
    fn nested_keys() {
        let ja = I18n::new(Locale::Ja);
        assert_eq!(ja.t("nav.main"), "メインナビゲーション");
        assert_eq!(I18n::new(Locale::En).t("channels.join"), "Join");
    }

    #[test]
    fn missing_key_returns_key() {
        assert_eq!(I18n::new(Locale::En).t("no.such.key"), "no.such.key");
    }

    #[test]
    fn interpolation() {
        let en = I18n::new(Locale::En);
        assert_eq!(
            en.t_with("channels.descriptionHelp", &[("max", "250")]),
            "Up to 250 characters."
        );
        assert_eq!(
            interpolate("a {{ x }} b {{y}}", &[("x", "1")]),
            "a 1 b {{y}}"
        );
        assert_eq!(interpolate("{{x", &[("x", "1")]), "{{x");
        assert_eq!(interpolate("{{x}}{{x}}", &[("x", "日本")]), "日本日本");
    }

    #[test]
    fn plurals_in_english() {
        let en = I18n::new(Locale::En);
        assert_eq!(en.t_count("messages.count", 1, &[]), "1 message");
        assert_eq!(en.t_count("messages.count", 0, &[]), "0 messages");
        assert_eq!(en.t_count("messages.count", 5, &[]), "5 messages");
    }

    #[test]
    fn plurals_in_japanese_use_other() {
        let ja = I18n::new(Locale::Ja);
        assert_eq!(ja.t_count("messages.count", 1, &[]), "1 件");
        assert_eq!(ja.t_count("messages.count", 3, &[]), "3 件");
    }

    #[test]
    fn plural_with_extra_vars() {
        let en = I18n::new(Locale::En);
        assert_eq!(
            en.t_count("notifications.reactionGroup", 3, &[("actor", "Ann")]),
            en.t_with(
                "notifications.reactionGroup_other",
                &[("actor", "Ann"), ("count", "3")]
            )
        );
    }

    #[test]
    fn plural_falls_back_to_plain_key() {
        // `_one` / `_other` が無いキーは、そのままのキーを使う。
        assert_eq!(
            I18n::new(Locale::En).t_count("channels.join", 2, &[]),
            "Join"
        );
    }

    #[test]
    fn match_tag_follows_locale_ts() {
        assert_eq!(Locale::match_tag("ja-JP"), Some(Locale::Ja));
        assert_eq!(Locale::match_tag("en_US.UTF-8"), Some(Locale::En));
        assert_eq!(Locale::match_tag("yue-Hant-HK"), Some(Locale::Yue));
        assert_eq!(Locale::match_tag("zh-yue"), Some(Locale::Yue));
        assert_eq!(Locale::match_tag("zh"), Some(Locale::ZhHans));
        assert_eq!(Locale::match_tag("zh-CN"), Some(Locale::ZhHans));
        assert_eq!(Locale::match_tag("zh-TW"), Some(Locale::ZhHant));
        assert_eq!(Locale::match_tag("zh_HK"), Some(Locale::ZhHant));
        // 文字種が地域より優先。
        assert_eq!(Locale::match_tag("zh-Hans-TW"), Some(Locale::ZhHans));
        assert_eq!(Locale::match_tag("zh-Hant-CN"), Some(Locale::ZhHant));
        assert_eq!(Locale::match_tag("ko"), Some(Locale::KoKr));
        assert_eq!(Locale::match_tag("ko-KP"), Some(Locale::KoKp));
        assert_eq!(Locale::match_tag("de-DE"), None);
        assert_eq!(Locale::match_tag("C"), None);
    }

    #[test]
    fn resolve_locale_order() {
        assert_eq!(resolve_locale(Some("en"), &["ja"]), Locale::En);
        // 不正な保存値は無視して言語の一覧を見る。
        assert_eq!(resolve_locale(Some("xx"), &["de", "ko-KR"]), Locale::KoKr);
        assert_eq!(resolve_locale(None, &["de", "fr"]), Locale::Ja);
        assert_eq!(resolve_locale::<&str>(None, &[]), Locale::Ja);
    }

    #[test]
    fn locale_codes_roundtrip() {
        for l in Locale::ALL {
            assert_eq!(Locale::from_code(l.code()), Some(l));
        }
    }
}
