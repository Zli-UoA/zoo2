//! lucide のアイコン。SVG は `assets/icons/` にあり、`scripts/collect-lucide-icons.mjs` で
//! frontend の import から集める。アイコンを足すときは、スクリプトを流して下の一覧にも足す
//! (足し忘れは `every_svg_has_a_variant` が見つける)。
//! frontend の import と `Icon::ALL` が食い違えば `matches_frontend_lucide_imports` が落ちる。

use gpui::{Hsla, Pixels, Svg, prelude::*, svg};

/// アイコンの一覧。variant と lucide の名前 (kebab-case) をここに 1 度だけ書く。
/// `Icon` 本体、`ALL`、`name()`、`path()` はすべてこの表から作る。
macro_rules! icons {
    ($($variant:ident => $name:literal,)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Icon {
            $($variant,)+
        }

        impl Icon {
            pub const ALL: &'static [Icon] = &[$(Icon::$variant,)+];

            /// lucide の名前 (kebab-case)。
            pub const fn name(self) -> &'static str {
                match self {
                    $(Icon::$variant => $name,)+
                }
            }

            /// アセットソース上のパス。
            pub const fn path(self) -> &'static str {
                match self {
                    $(Icon::$variant => concat!("icons/", $name, ".svg"),)+
                }
            }
        }
    };
}

icons! {
    Activity => "activity",
    Archive => "archive",
    ArchiveRestore => "archive-restore",
    ArrowDown => "arrow-down",
    ArrowLeft => "arrow-left",
    ArrowUpRight => "arrow-up-right",
    AtSign => "at-sign",
    Bell => "bell",
    BellOff => "bell-off",
    Bot => "bot",
    Check => "check",
    CheckCheck => "check-check",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    CloudOff => "cloud-off",
    Copy => "copy",
    Download => "download",
    Eraser => "eraser",
    ExternalLink => "external-link",
    Eye => "eye",
    EyeOff => "eye-off",
    Fingerprint => "fingerprint",
    Globe => "globe",
    Hash => "hash",
    Headphones => "headphones",
    Home => "home",
    ImagePlus => "image-plus",
    KeyRound => "key-round",
    Languages => "languages",
    Link => "link",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    LockKeyhole => "lock-keyhole",
    LogOut => "log-out",
    MessageCircle => "message-circle",
    MessageCircleMore => "message-circle-more",
    MessageSquare => "message-square",
    MessageSquarePlus => "message-square-plus",
    MessageSquareText => "message-square-text",
    MessagesSquare => "messages-square",
    Mic => "mic",
    MicOff => "mic-off",
    Monitor => "monitor",
    Moon => "moon",
    MoreHorizontal => "more-horizontal",
    Network => "network",
    Palette => "palette",
    Paperclip => "paperclip",
    Pencil => "pencil",
    PhoneOff => "phone-off",
    Plus => "plus",
    RefreshCw => "refresh-cw",
    Repeat2 => "repeat-2",
    Reply => "reply",
    Search => "search",
    Send => "send",
    Settings => "settings",
    Settings2 => "settings-2",
    ShieldCheck => "shield-check",
    SmilePlus => "smile-plus",
    SquareArrowOutUpRight => "square-arrow-out-up-right",
    SquarePen => "square-pen",
    Sun => "sun",
    Trash => "trash",
    Trash2 => "trash-2",
    Unlink => "unlink",
    Unplug => "unplug",
    Upload => "upload",
    UserCheck => "user-check",
    UserMinus => "user-minus",
    UserPlus => "user-plus",
    UserRound => "user-round",
    UserSearch => "user-search",
    Users => "users",
    UsersRound => "users-round",
    Volume2 => "volume-2",
    Webhook => "webhook",
    X => "x",
}

/// `svg().path(..).text_color(..)` を組んで返す。`size` は幅と高さに使う。
/// lucide は 24px 角で、線の太さは SVG 側の指定 (2) のまま拡縮する。
pub fn icon(icon: Icon, size: Pixels, color: impl Into<Hsla>) -> Svg {
    svg()
        .path(icon.path())
        .size(size)
        .flex_none()
        .text_color(color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Assets;
    use gpui::AssetSource;

    #[test]
    fn every_variant_has_an_embedded_svg() {
        for icon in Icon::ALL {
            let data = Assets.load(icon.path()).unwrap();
            let data = data.unwrap_or_else(|| panic!("{} がありません", icon.path()));
            assert!(data.starts_with(b"<!--") || data.starts_with(b"<svg"));
        }
    }

    #[test]
    fn every_svg_has_a_variant() {
        let listed: Vec<_> = Assets.list("icons").unwrap();
        assert_eq!(listed.len(), Icon::ALL.len());
        for path in listed {
            assert!(
                Icon::ALL.iter().any(|i| i.path() == path.as_ref()),
                "{path} に対応する Icon がありません"
            );
        }
    }

    #[test]
    fn names_are_unique_kebab_case() {
        let mut seen = std::collections::HashSet::new();
        for icon in Icon::ALL {
            assert!(seen.insert(icon.name()));
            assert!(
                icon.name()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            );
        }
    }

    /// frontend が `lucide-react` から import している名前を kebab-case で集める。
    /// 規則は `scripts/collect-lucide-icons.mjs` の `isScanned` / `importedNames` / `toKebab` と同じ。
    /// どちらかを変えたら、もう一方も合わせること。
    mod frontend_imports {
        use regex::Regex;
        use std::collections::BTreeSet;
        use std::path::{Path, PathBuf};

        /// `isScanned` と同じ。*.ts / *.tsx から、stories・test・vrt.test と `src/test/` を除く。
        fn is_scanned(path: &Path, test_dir: &Path) -> bool {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !(name.ends_with(".ts") || name.ends_with(".tsx")) {
                return false;
            }
            let stem = name.trim_end_matches('x').trim_end_matches(".ts");
            if ["stories", "test", "vrt.test"]
                .iter()
                .any(|s| stem.ends_with(&format!(".{s}")))
            {
                return false;
            }
            !path.starts_with(test_dir)
        }

        fn walk(dir: &Path, test_dir: &Path, out: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    if path.file_name().is_some_and(|n| n == "node_modules") || path == test_dir {
                        continue;
                    }
                    walk(&path, test_dir, out);
                } else if is_scanned(&path, test_dir) {
                    out.push(path);
                }
            }
        }

        /// `importedNames` と同じ。`import type { .. }` と `type X` 指定は無視し、`as` の左側を取る。
        pub fn imported_names(source: &str) -> Vec<String> {
            let re = Regex::new(r#"import\s+\{([^}]*)\}\s*from\s*["']lucide-react["']"#).unwrap();
            let as_re = Regex::new(r"\s+as\s+").unwrap();
            let mut names = Vec::new();
            for m in re.captures_iter(source) {
                for part in m[1].split(',') {
                    let spec = part.trim();
                    if spec.is_empty() || spec.starts_with("type ") {
                        continue;
                    }
                    let left = as_re.split(spec).next().unwrap();
                    names.push(left.trim().to_string());
                }
            }
            names
        }

        /// `toKebab` と同じ。CloudOffIcon / LucideCloudOff / CloudOff -> cloud-off。
        pub fn to_kebab(name: &str) -> String {
            let base = match name.strip_prefix("Lucide") {
                Some(rest) if rest.starts_with(|c: char| c.is_ascii_uppercase()) => rest,
                _ => name,
            };
            let base = base.strip_suffix("Icon").unwrap_or(base);
            let mut s = base.to_string();
            for (pat, rep) in [
                (r"([a-z0-9])([A-Z])", "$1-$2"),
                (r"([A-Za-z])([0-9])", "$1-$2"),
                (r"([0-9])([A-Z])", "$1-$2"),
            ] {
                s = Regex::new(pat).unwrap().replace_all(&s, rep).into_owned();
            }
            s.to_lowercase()
        }

        pub fn collect() -> BTreeSet<String> {
            let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/src");
            let test_dir = src.join("test");
            let mut files = Vec::new();
            walk(&src, &test_dir, &mut files);
            assert!(
                !files.is_empty(),
                "{} に ts/tsx がありません",
                src.display()
            );
            let mut kebabs = BTreeSet::new();
            for file in files {
                let source = std::fs::read_to_string(&file).unwrap();
                kebabs.extend(imported_names(&source).iter().map(|n| to_kebab(n)));
            }
            kebabs
        }
    }

    #[test]
    fn matches_frontend_lucide_imports() {
        let imported = frontend_imports::collect();
        let have: std::collections::BTreeSet<String> =
            Icon::ALL.iter().map(|i| i.name().to_string()).collect();
        let missing: Vec<_> = imported.difference(&have).collect();
        let extra: Vec<_> = have.difference(&imported).collect();
        assert!(
            missing.is_empty() && extra.is_empty(),
            "frontend の lucide-react の import と Icon::ALL が合っていません。\n  \
             足りないアイコン (frontend が import、desktop にない): {missing:?}\n  \
             余分なアイコン (desktop にあるが frontend は import しない): {extra:?}\n\
             `node desktop/scripts/collect-lucide-icons.mjs` を実行して SVG を更新し、\
             icons! の表を合わせてください。"
        );
    }

    #[test]
    fn frontend_import_rules() {
        use frontend_imports::{imported_names, to_kebab};
        assert_eq!(to_kebab("CloudOffIcon"), "cloud-off");
        assert_eq!(to_kebab("LucideCloudOff"), "cloud-off");
        assert_eq!(to_kebab("CloudOff"), "cloud-off");
        assert_eq!(to_kebab("Trash2"), "trash-2");
        assert_eq!(to_kebab("Repeat2"), "repeat-2");
        assert_eq!(to_kebab("Lucide"), "lucide");
        let src = "import { A, B as C,\n  type D, E } from 'lucide-react';\n\
                   import type { F } from \"lucide-react\";\n\
                   import { G } from \"other\";";
        assert_eq!(imported_names(src), ["A", "B", "E"]);
    }
}
