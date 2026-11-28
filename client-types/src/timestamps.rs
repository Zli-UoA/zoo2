//! 投稿・検索結果・埋め込みで共有する、端末の暦での表示。
use jiff::civil::{Date, DateTime};

pub fn message_timestamp(local: DateTime, today: Date) -> String {
    let pattern = if local.date() == today {
        "%H:%M"
    } else if local.year() == today.year() {
        "%m/%d %H:%M"
    } else {
        "%Y/%m/%d %H:%M"
    };
    local.strftime(pattern).to_string()
}
