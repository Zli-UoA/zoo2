//! Resolve only pairs in the bundled Emoji Kitchen catalog; no user-supplied URLs.
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::LazyLock};

#[derive(Deserialize)]
struct Catalog {
    emojis: Vec<(String, String, String)>,
    dates: Vec<String>,
    pairs: HashMap<String, usize>,
}
static CATALOG: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../frontend/src/lib/emoji/emoji-kitchen.json"
    ))
    .expect("bundled Emoji Kitchen catalog")
});
static INDICES: LazyLock<HashMap<String, usize>> = LazyLock::new(|| {
    CATALOG
        .emojis
        .iter()
        .enumerate()
        .map(|(i, e)| (canonical(&e.0), i))
        .collect()
});

fn canonical(code: &str) -> String {
    code.split('-')
        .filter(|part| *part != "fe0f")
        .collect::<Vec<_>>()
        .join("-")
}

pub(super) fn emoji(value: &str) -> Option<Value> {
    if value.len() > 256 {
        return None;
    }
    let raw = if value.starts_with("kitchen:") {
        value.to_owned()
    } else {
        format!(
            "kitchen:{}",
            value.strip_prefix(":kitchen_")?.strip_suffix(':')?
        )
    };
    let normalized = crate::validation::normalize_and_validate_emoji(&raw).ok()?;
    let (left, right) = normalized.strip_prefix("kitchen:")?.split_once('_')?;
    let (a, b) = (
        *INDICES.get(&canonical(left))?,
        *INDICES.get(&canonical(right))?,
    );
    let (a, b) = (a.min(b), a.max(b));
    let packed = *CATALOG.pairs.get(&format!("{a}_{b}"))?;
    let (left, right) = if packed % 2 == 0 { (a, b) } else { (b, a) };
    let code = |i: usize| {
        CATALOG.emojis[i]
            .0
            .split('-')
            .map(|p| format!("u{p}"))
            .collect::<Vec<_>>()
            .join("-")
    };
    let (left, right) = (code(left), code(right));
    let date = CATALOG.dates.get(packed / 2)?;
    let url = format!(
        "https://www.gstatic.com/android/keyboard/emojikitchen/{date}/{left}/{left}_{right}.png"
    );
    let image_id = Sha256::digest(url.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut parts = [
        canonical(&CATALOG.emojis[a].0),
        canonical(&CATALOG.emojis[b].0),
    ];
    parts.sort();
    let name = if value.starts_with(':') {
        value.to_owned()
    } else {
        format!(":kitchen_{}_{}:", parts[0], parts[1])
    };
    Some(
        json!({"id":url,"url":url,"image_id":image_id,"name":name,"local":true,"width":512,"height":512}),
    )
}

pub(super) fn content_emojis(content: &str) -> Vec<Value> {
    static SHORTCODE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r":kitchen_[a-f0-9-]+_[a-f0-9-]+:").unwrap());
    let mut seen = std::collections::HashSet::new();
    SHORTCODE
        .find_iter(content)
        .filter_map(|m| {
            if seen.insert(m.as_str()) {
                emoji(m.as_str())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolves_catalog_orientation_and_rejects_unknown_pairs() {
        let first = emoji("kitchen:1f431_1f600").unwrap();
        let reversed = emoji("kitchen:1f600_1f431").unwrap();
        assert_eq!(first, reversed);
        assert_eq!(first["name"], ":kitchen_1f431_1f600:");
        assert!(
            first["url"]
                .as_str()
                .unwrap()
                .starts_with("https://www.gstatic.com/android/keyboard/emojikitchen/")
        );
        assert_eq!(
            first["url"],
            "https://www.gstatic.com/android/keyboard/emojikitchen/20230301/u1f600/u1f600_u1f431.png"
        );
        assert_eq!(emoji("kitchen:😀_🐱").unwrap(), first);
        assert!(emoji(":kitchen_263a_2615:").is_some());
        assert!(emoji("kitchen:hello_world").is_none());
        assert!(emoji(":kitchen_0_1:").is_none());
        assert!(emoji("kitchen:1f600_1f431_extra").is_none());
        let content = ":kitchen_1f431_1f600: :kitchen_1f431_1f600: :kitchen_0_1:";
        assert_eq!(content_emojis(content), vec![first.clone()]);
        let tag = crate::activitypub::emoji::as_tag(&first);
        let received = crate::activitypub::emoji::reaction(&json!({
            "_misskey_reaction": first["name"], "tag": [tag]
        }))
        .unwrap();
        assert_eq!(received.0, first["name"].as_str().unwrap());
        assert_eq!(received.1.unwrap()["image_id"], first["image_id"]);
    }
}
