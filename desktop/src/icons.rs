//! lucide のアイコン。SVG は `assets/icons/` にあり、`scripts/collect-lucide-icons.mjs` で
//! frontend の import から集める。アイコンを足すときは、スクリプトを流して下の一覧にも足す
//! (足し忘れは `every_svg_has_a_variant` が見つける)。

use gpui::{Hsla, Pixels, Svg, prelude::*, svg};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Activity,
    Archive,
    ArchiveRestore,
    ArrowDown,
    ArrowLeft,
    ArrowUpRight,
    AtSign,
    Bell,
    BellOff,
    Bot,
    Check,
    CheckCheck,
    ChevronDown,
    ChevronRight,
    ChevronUp,
    CloudOff,
    Copy,
    Download,
    Eraser,
    ExternalLink,
    Eye,
    EyeOff,
    Globe,
    Hash,
    Headphones,
    Home,
    ImagePlus,
    Languages,
    Link,
    LoaderCircle,
    Lock,
    LockKeyhole,
    LogOut,
    MessageCircle,
    MessageCircleMore,
    MessageSquarePlus,
    MessageSquareText,
    MessagesSquare,
    Mic,
    MicOff,
    Monitor,
    Moon,
    MoreHorizontal,
    Network,
    Palette,
    Paperclip,
    Pencil,
    PhoneOff,
    Plus,
    RefreshCw,
    Repeat2,
    Reply,
    Search,
    Send,
    Settings,
    Settings2,
    ShieldAlert,
    SmilePlus,
    SquareArrowOutUpRight,
    SquarePen,
    Sun,
    Trash,
    Trash2,
    Unlink,
    Unplug,
    Upload,
    UserCheck,
    UserMinus,
    UserPlus,
    UserRound,
    UserSearch,
    Users,
    UsersRound,
    Volume2,
    Webhook,
    X,
}

impl Icon {
    pub const ALL: &'static [Icon] = &[
        Icon::Activity,
        Icon::Archive,
        Icon::ArchiveRestore,
        Icon::ArrowDown,
        Icon::ArrowLeft,
        Icon::ArrowUpRight,
        Icon::AtSign,
        Icon::Bell,
        Icon::BellOff,
        Icon::Bot,
        Icon::Check,
        Icon::CheckCheck,
        Icon::ChevronDown,
        Icon::ChevronRight,
        Icon::ChevronUp,
        Icon::CloudOff,
        Icon::Copy,
        Icon::Download,
        Icon::Eraser,
        Icon::ExternalLink,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Globe,
        Icon::Hash,
        Icon::Headphones,
        Icon::Home,
        Icon::ImagePlus,
        Icon::Languages,
        Icon::Link,
        Icon::LoaderCircle,
        Icon::Lock,
        Icon::LockKeyhole,
        Icon::LogOut,
        Icon::MessageCircle,
        Icon::MessageCircleMore,
        Icon::MessageSquarePlus,
        Icon::MessageSquareText,
        Icon::MessagesSquare,
        Icon::Mic,
        Icon::MicOff,
        Icon::Monitor,
        Icon::Moon,
        Icon::MoreHorizontal,
        Icon::Network,
        Icon::Palette,
        Icon::Paperclip,
        Icon::Pencil,
        Icon::PhoneOff,
        Icon::Plus,
        Icon::RefreshCw,
        Icon::Repeat2,
        Icon::Reply,
        Icon::Search,
        Icon::Send,
        Icon::Settings,
        Icon::Settings2,
        Icon::ShieldAlert,
        Icon::SmilePlus,
        Icon::SquareArrowOutUpRight,
        Icon::SquarePen,
        Icon::Sun,
        Icon::Trash,
        Icon::Trash2,
        Icon::Unlink,
        Icon::Unplug,
        Icon::Upload,
        Icon::UserCheck,
        Icon::UserMinus,
        Icon::UserPlus,
        Icon::UserRound,
        Icon::UserSearch,
        Icon::Users,
        Icon::UsersRound,
        Icon::Volume2,
        Icon::Webhook,
        Icon::X,
    ];

    /// lucide のアイコン名 (kebab-case)。
    pub const fn name(self) -> &'static str {
        match self {
            Icon::Activity => "activity",
            Icon::Archive => "archive",
            Icon::ArchiveRestore => "archive-restore",
            Icon::ArrowDown => "arrow-down",
            Icon::ArrowLeft => "arrow-left",
            Icon::ArrowUpRight => "arrow-up-right",
            Icon::AtSign => "at-sign",
            Icon::Bell => "bell",
            Icon::BellOff => "bell-off",
            Icon::Bot => "bot",
            Icon::Check => "check",
            Icon::CheckCheck => "check-check",
            Icon::ChevronDown => "chevron-down",
            Icon::ChevronRight => "chevron-right",
            Icon::ChevronUp => "chevron-up",
            Icon::CloudOff => "cloud-off",
            Icon::Copy => "copy",
            Icon::Download => "download",
            Icon::Eraser => "eraser",
            Icon::ExternalLink => "external-link",
            Icon::Eye => "eye",
            Icon::EyeOff => "eye-off",
            Icon::Globe => "globe",
            Icon::Hash => "hash",
            Icon::Headphones => "headphones",
            Icon::Home => "home",
            Icon::ImagePlus => "image-plus",
            Icon::Languages => "languages",
            Icon::Link => "link",
            Icon::LoaderCircle => "loader-circle",
            Icon::Lock => "lock",
            Icon::LockKeyhole => "lock-keyhole",
            Icon::LogOut => "log-out",
            Icon::MessageCircle => "message-circle",
            Icon::MessageCircleMore => "message-circle-more",
            Icon::MessageSquarePlus => "message-square-plus",
            Icon::MessageSquareText => "message-square-text",
            Icon::MessagesSquare => "messages-square",
            Icon::Mic => "mic",
            Icon::MicOff => "mic-off",
            Icon::Monitor => "monitor",
            Icon::Moon => "moon",
            Icon::MoreHorizontal => "more-horizontal",
            Icon::Network => "network",
            Icon::Palette => "palette",
            Icon::Paperclip => "paperclip",
            Icon::Pencil => "pencil",
            Icon::PhoneOff => "phone-off",
            Icon::Plus => "plus",
            Icon::RefreshCw => "refresh-cw",
            Icon::Repeat2 => "repeat-2",
            Icon::Reply => "reply",
            Icon::Search => "search",
            Icon::Send => "send",
            Icon::Settings => "settings",
            Icon::Settings2 => "settings-2",
            Icon::ShieldAlert => "shield-alert",
            Icon::SmilePlus => "smile-plus",
            Icon::SquareArrowOutUpRight => "square-arrow-out-up-right",
            Icon::SquarePen => "square-pen",
            Icon::Sun => "sun",
            Icon::Trash => "trash",
            Icon::Trash2 => "trash-2",
            Icon::Unlink => "unlink",
            Icon::Unplug => "unplug",
            Icon::Upload => "upload",
            Icon::UserCheck => "user-check",
            Icon::UserMinus => "user-minus",
            Icon::UserPlus => "user-plus",
            Icon::UserRound => "user-round",
            Icon::UserSearch => "user-search",
            Icon::Users => "users",
            Icon::UsersRound => "users-round",
            Icon::Volume2 => "volume-2",
            Icon::Webhook => "webhook",
            Icon::X => "x",
        }
    }

    /// アセットソース上のパス。
    pub const fn path(self) -> &'static str {
        match self {
            Icon::Activity => "icons/activity.svg",
            Icon::Archive => "icons/archive.svg",
            Icon::ArchiveRestore => "icons/archive-restore.svg",
            Icon::ArrowDown => "icons/arrow-down.svg",
            Icon::ArrowLeft => "icons/arrow-left.svg",
            Icon::ArrowUpRight => "icons/arrow-up-right.svg",
            Icon::AtSign => "icons/at-sign.svg",
            Icon::Bell => "icons/bell.svg",
            Icon::BellOff => "icons/bell-off.svg",
            Icon::Bot => "icons/bot.svg",
            Icon::Check => "icons/check.svg",
            Icon::CheckCheck => "icons/check-check.svg",
            Icon::ChevronDown => "icons/chevron-down.svg",
            Icon::ChevronRight => "icons/chevron-right.svg",
            Icon::ChevronUp => "icons/chevron-up.svg",
            Icon::CloudOff => "icons/cloud-off.svg",
            Icon::Copy => "icons/copy.svg",
            Icon::Download => "icons/download.svg",
            Icon::Eraser => "icons/eraser.svg",
            Icon::ExternalLink => "icons/external-link.svg",
            Icon::Eye => "icons/eye.svg",
            Icon::EyeOff => "icons/eye-off.svg",
            Icon::Globe => "icons/globe.svg",
            Icon::Hash => "icons/hash.svg",
            Icon::Headphones => "icons/headphones.svg",
            Icon::Home => "icons/home.svg",
            Icon::ImagePlus => "icons/image-plus.svg",
            Icon::Languages => "icons/languages.svg",
            Icon::Link => "icons/link.svg",
            Icon::LoaderCircle => "icons/loader-circle.svg",
            Icon::Lock => "icons/lock.svg",
            Icon::LockKeyhole => "icons/lock-keyhole.svg",
            Icon::LogOut => "icons/log-out.svg",
            Icon::MessageCircle => "icons/message-circle.svg",
            Icon::MessageCircleMore => "icons/message-circle-more.svg",
            Icon::MessageSquarePlus => "icons/message-square-plus.svg",
            Icon::MessageSquareText => "icons/message-square-text.svg",
            Icon::MessagesSquare => "icons/messages-square.svg",
            Icon::Mic => "icons/mic.svg",
            Icon::MicOff => "icons/mic-off.svg",
            Icon::Monitor => "icons/monitor.svg",
            Icon::Moon => "icons/moon.svg",
            Icon::MoreHorizontal => "icons/more-horizontal.svg",
            Icon::Network => "icons/network.svg",
            Icon::Palette => "icons/palette.svg",
            Icon::Paperclip => "icons/paperclip.svg",
            Icon::Pencil => "icons/pencil.svg",
            Icon::PhoneOff => "icons/phone-off.svg",
            Icon::Plus => "icons/plus.svg",
            Icon::RefreshCw => "icons/refresh-cw.svg",
            Icon::Repeat2 => "icons/repeat-2.svg",
            Icon::Reply => "icons/reply.svg",
            Icon::Search => "icons/search.svg",
            Icon::Send => "icons/send.svg",
            Icon::Settings => "icons/settings.svg",
            Icon::Settings2 => "icons/settings-2.svg",
            Icon::ShieldAlert => "icons/shield-alert.svg",
            Icon::SmilePlus => "icons/smile-plus.svg",
            Icon::SquareArrowOutUpRight => "icons/square-arrow-out-up-right.svg",
            Icon::SquarePen => "icons/square-pen.svg",
            Icon::Sun => "icons/sun.svg",
            Icon::Trash => "icons/trash.svg",
            Icon::Trash2 => "icons/trash-2.svg",
            Icon::Unlink => "icons/unlink.svg",
            Icon::Unplug => "icons/unplug.svg",
            Icon::Upload => "icons/upload.svg",
            Icon::UserCheck => "icons/user-check.svg",
            Icon::UserMinus => "icons/user-minus.svg",
            Icon::UserPlus => "icons/user-plus.svg",
            Icon::UserRound => "icons/user-round.svg",
            Icon::UserSearch => "icons/user-search.svg",
            Icon::Users => "icons/users.svg",
            Icon::UsersRound => "icons/users-round.svg",
            Icon::Volume2 => "icons/volume-2.svg",
            Icon::Webhook => "icons/webhook.svg",
            Icon::X => "icons/x.svg",
        }
    }
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
