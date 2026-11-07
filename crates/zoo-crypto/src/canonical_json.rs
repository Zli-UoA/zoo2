//! Matrix の [Canonical JSON](https://spec.matrix.org/latest/appendices/#canonical-json)。
//!
//! - オブジェクトのキーは Unicode のコードポイント順（UTF-8 のバイト順と同じ）に並べる。
//! - 空白を入れない。
//! - 文字列は UTF-8 のまま書き、`"`、`\`、U+0000〜U+001F だけをエスケープする。
//! - 数値は -(2^53)+1 から 2^53-1 までの整数だけを許す。小数は許さない。
//!
//! `serde_json::Value` の `Map` の並び順（`preserve_order` feature の有無）に左右されないように、
//! キーを並べ替えながら自前で書き出す。

use serde_json::Value;

use crate::CryptoError;

/// Canonical JSON で表せる整数の最大値（2^53-1）。
const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;

/// `value` を Canonical JSON の文字列にする。
///
/// 小数や範囲外の整数を含む場合は [`CryptoError::CanonicalJson`] を返す。
pub fn to_canonical_json(value: &Value) -> Result<String, CryptoError> {
    let mut out = String::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn write_value(out: &mut String, value: &Value) -> Result<(), CryptoError> {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(number) => {
            let integer = number
                .as_i64()
                .filter(|n| (-MAX_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(n))
                .ok_or_else(|| {
                    CryptoError::CanonicalJson(format!(
                        "数値は ±(2^53-1) の範囲の整数だけを使えます: {number}"
                    ))
                })?;
            out.push_str(&integer.to_string());
        }
        Value::String(string) => write_string(out, string),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, item)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            // String の比較は UTF-8 のバイト順で、コードポイント順と一致する。
            entries.sort_unstable_by_key(|(key, _)| *key);
            out.push('{');
            for (i, (key, item)) in entries.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_string(out, key);
                out.push(':');
                write_value(out, item)?;
            }
            out.push('}');
        }
    }
    Ok(())
}

fn write_string(out: &mut String, string: &str) {
    out.push('"');
    for c in string.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\u{20}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn canonical(input: &str) -> String {
        to_canonical_json(&serde_json::from_str(input).unwrap()).unwrap()
    }

    /// Matrix の付録にある例。
    ///
    /// 付録には `{"a": -0, "b": 1e10}` を `{"a":0,"b":10000000000}` にする例もあるが、ここには入れない。
    /// 仕様の本文が浮動小数点数を許さないので、`-0` と `1e10` は浮動小数点数として拒否する
    /// （ruma も拒否する）。拒否することは `rejects_float_example_from_matrix_appendix` で確かめる。
    #[test]
    fn matches_matrix_examples() {
        assert_eq!(canonical("{}"), "{}");
        assert_eq!(
            canonical(r#"{"one": 1, "two": "Two"}"#),
            r#"{"one":1,"two":"Two"}"#
        );
        assert_eq!(canonical(r#"{"b": "2", "a": "1"}"#), r#"{"a":"1","b":"2"}"#);
        assert_eq!(
            canonical(
                r#"{
                    "auth": {
                        "success": true,
                        "mxid": "@john.doe:example.com",
                        "profile": {
                            "display_name": "John Doe",
                            "three_pids": [
                                {"medium": "email", "address": "john.doe@example.org"},
                                {"medium": "msisdn", "address": "123456789"}
                            ]
                        }
                    }
                }"#
            ),
            r#"{"auth":{"mxid":"@john.doe:example.com","profile":{"display_name":"John Doe","three_pids":[{"address":"john.doe@example.org","medium":"email"},{"address":"123456789","medium":"msisdn"}]},"success":true}}"#
        );
        assert_eq!(canonical(r#"{"a": "日本語"}"#), r#"{"a":"日本語"}"#);
        assert_eq!(canonical(r#"{"本": 2, "日": 1}"#), r#"{"日":1,"本":2}"#);
        assert_eq!(canonical(r#"{"a": "日"}"#), r#"{"a":"日"}"#);
        assert_eq!(canonical(r#"{"a": null}"#), r#"{"a":null}"#);
    }

    #[test]
    fn escapes_only_required_characters() {
        let value = json!({ "s": "\"\\/\u{0}\u{8}\u{c}\n\r\t\u{1f}\u{7f}é" });
        assert_eq!(
            to_canonical_json(&value).unwrap(),
            "{\"s\":\"\\\"\\\\/\\u0000\\b\\f\\n\\r\\t\\u001f\u{7f}é\"}"
        );
    }

    #[test]
    fn accepts_safe_integers() {
        let value = json!({ "max": MAX_SAFE_INTEGER, "min": -MAX_SAFE_INTEGER });
        assert_eq!(
            to_canonical_json(&value).unwrap(),
            r#"{"max":9007199254740991,"min":-9007199254740991}"#
        );
    }

    /// 付録の `{"a": -0, "b": 1e10}` の例は受け付けない（`matches_matrix_examples` の説明を参照）。
    #[test]
    fn rejects_float_example_from_matrix_appendix() {
        for input in [r#"{"a": -0, "b": 1e10}"#, r#"{"a": -0}"#, r#"{"b": 1e10}"#] {
            let value: Value = serde_json::from_str(input).unwrap();
            assert!(
                matches!(
                    to_canonical_json(&value),
                    Err(CryptoError::CanonicalJson(_))
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn rejects_floats_and_unsafe_integers() {
        for value in [
            json!({ "a": 1.5 }),
            json!({ "a": 1.0 }),
            json!([MAX_SAFE_INTEGER + 1]),
            json!([-MAX_SAFE_INTEGER - 1]),
            json!(u64::MAX),
        ] {
            assert!(
                matches!(
                    to_canonical_json(&value),
                    Err(CryptoError::CanonicalJson(_))
                ),
                "{value}"
            );
        }
    }
}
