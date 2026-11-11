//! lucide のアイコン。SVG は `assets/icons/` にあり、`scripts/collect-lucide-icons.mjs` で
//! frontend の import から集める。アイコンを足すときは、スクリプトを流して下の一覧にも足す
//! (足し忘れは `every_svg_has_a_variant` が見つける)。

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
    Globe => "globe",
    Hash => "hash",
    Headphones => "headphones",
    Home => "home",
    ImagePlus => "image-plus",
    Languages => "languages",
    Link => "link",
    LoaderCircle => "loader-circle",
    Lock => "lock",
    LockKeyhole => "lock-keyhole",
    LogOut => "log-out",
    MessageCircle => "message-circle",
    MessageCircleMore => "message-circle-more",
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
}
