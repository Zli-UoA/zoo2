//! メッセージ一覧の表示用ロジック (frontend の `messageGrouping.ts` / `messageFormat.ts` /
//! `useMessageReadState.ts` の `firstUnreadMessage`)。GPUI に依存しない。

use jiff::civil::{Date, DateTime, Weekday};
use jiff::tz::TimeZone;
use zoo2_client_core::electric::rows::Message;

use crate::i18n::{I18n, Locale};

/// 同一投稿者の連投をまとめる時間のしきい値 (`GROUP_GAP_MS`)。
pub const GROUP_GAP_MS: i64 = 5 * 60 * 1000;

/// 1 行ぶんの表示情報 (`MessageItem`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageItem {
    /// 連投のかたまりの先頭 (アバター・投稿者名を出す)。
    pub is_group_start: bool,
    /// 連投のかたまりの末尾 (下余白を広げる)。
    pub is_group_end: bool,
    /// 直前の行と日付が変わるため、上に日付区切りを描く。
    pub show_date_separator: bool,
}

impl MessageItem {
    /// スレッドのルートのように、単独で出す行 (`isGroupStart` も `isGroupEnd` も true)。
    pub const SOLO: Self = Self {
        is_group_start: true,
        is_group_end: true,
        show_date_separator: false,
    };
}

/// UTC の naive 日時をミリ秒にする。
fn utc_millis(at: DateTime) -> Option<i64> {
    at.to_zoned(TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp().as_millisecond())
}

fn within_group(earlier: &Message, later: &Message, gap_ms: i64) -> bool {
    match (utc_millis(earlier.created_at), utc_millis(later.created_at)) {
        (Some(a), Some(b)) => (0..=gap_ms).contains(&(b - a)),
        _ => false,
    }
}

/// メッセージ列を表示用の行にする (`buildMessageItems`)。
///
/// - 日付が変わった行には区切りを出す。
/// - 同じ投稿者の投稿が `gap_ms` 以内に続けば連投としてまとめる。未読線の行はまとめない。
///
/// `to_local` は UTC の日時を端末のローカル日時にする (日付の境界を決める)。
pub fn build_message_items(
    messages: &[&Message],
    gap_ms: i64,
    first_unread_id: Option<&str>,
    to_local: impl Fn(DateTime) -> DateTime,
) -> Vec<MessageItem> {
    // ローカル日時は 1 通につき 1 回だけ求める。
    let days: Vec<Date> = messages
        .iter()
        .map(|m| to_local(m.created_at).date())
        .collect();
    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let previous = i.checked_sub(1).and_then(|j| messages.get(j));
            let next = messages.get(i + 1);
            let show_date_separator = i.checked_sub(1).is_none_or(|j| days[j] != days[i]);
            let continues_from_previous = first_unread_id != Some(m.id.as_str())
                && !show_date_separator
                && previous.is_some_and(|p| p.user_id == m.user_id && within_group(p, m, gap_ms));
            let continues_to_next = next.is_some_and(|n| {
                first_unread_id != Some(n.id.as_str())
                    && n.user_id == m.user_id
                    && days[i + 1] == days[i]
                    && within_group(m, n, gap_ms)
            });
            MessageItem {
                is_group_start: !continues_from_previous,
                is_group_end: !continues_to_next,
                show_date_separator,
            }
        })
        .collect()
}

/// 未読線を置くメッセージ (`firstUnreadMessage`)。自分以外の投稿で、既読の位置より後の最初のもの。
pub fn first_unread_id<'a>(
    messages: &'a [Message],
    me_id: &str,
    last_read: DateTime,
) -> Option<&'a str> {
    messages
        .iter()
        .find(|m| m.user_id != me_id && m.created_at > last_read)
        .map(|m| m.id.as_str())
}

/// `first_unread_id` を全ルートメッセージで求め、描画窓 (`start_idx` 以降) にある行のときだけ返す。
/// 既読位置が窓より古いと、窓の先頭行に線が付いて「過去を表示」で動くので、その場合は置かない。
pub fn first_unread_id_in_window<'a>(
    all_roots: &'a [Message],
    start_idx: usize,
    me_id: &str,
    last_read: DateTime,
) -> Option<&'a str> {
    let id = first_unread_id(all_roots, me_id, last_read)?;
    let idx = all_roots.iter().position(|m| m.id == id)?;
    (idx >= start_idx).then_some(id)
}

/// `HH:mm` (`formatClock`)。
pub fn format_clock(local: DateTime) -> String {
    format!("{:02}:{:02}", local.hour(), local.minute())
}

fn weekday_ja(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "月",
        Weekday::Tuesday => "火",
        Weekday::Wednesday => "水",
        Weekday::Thursday => "木",
        Weekday::Friday => "金",
        Weekday::Saturday => "土",
        Weekday::Sunday => "日",
    }
}

fn weekday_ko(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "월",
        Weekday::Tuesday => "화",
        Weekday::Wednesday => "수",
        Weekday::Thursday => "목",
        Weekday::Friday => "금",
        Weekday::Saturday => "토",
        Weekday::Sunday => "일",
    }
}

fn weekday_zh(w: Weekday, simplified: bool) -> &'static str {
    let (hans, hant) = match w {
        Weekday::Monday => ("周一", "週一"),
        Weekday::Tuesday => ("周二", "週二"),
        Weekday::Wednesday => ("周三", "週三"),
        Weekday::Thursday => ("周四", "週四"),
        Weekday::Friday => ("周五", "週五"),
        Weekday::Saturday => ("周六", "週六"),
        Weekday::Sunday => ("周日", "週日"),
    };
    if simplified { hans } else { hant }
}

fn weekday_en(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "Mon",
        Weekday::Tuesday => "Tue",
        Weekday::Wednesday => "Wed",
        Weekday::Thursday => "Thu",
        Weekday::Friday => "Fri",
        Weekday::Saturday => "Sat",
        Weekday::Sunday => "Sun",
    }
}

const MONTHS_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// 日付区切りのラベル (`formatRelativeDay`)。今日と昨日は相対表記、それ以前は
/// 「2026年6月10日(水)」(英語は「Wed, June 10, 2026」)。韓国語と中国語・広東語はそれぞれの言語の形にする。
pub fn relative_day_label(date: Date, today: Date, i18n: &I18n) -> String {
    if date == today {
        return i18n.t("dates.today");
    }
    if today.yesterday().is_ok_and(|y| y == date) {
        return i18n.t("dates.yesterday");
    }
    match i18n.locale() {
        Locale::En => format!(
            "{}, {} {}, {}",
            weekday_en(date.weekday()),
            MONTHS_EN[usize::from(date.month() as u8 - 1)],
            date.day(),
            date.year()
        ),
        Locale::KoKr | Locale::KoKp => format!(
            "{}년 {}월 {}일 ({})",
            date.year(),
            date.month(),
            date.day(),
            weekday_ko(date.weekday())
        ),
        Locale::ZhHans => format!(
            "{}年{}月{}日 {}",
            date.year(),
            date.month(),
            date.day(),
            weekday_zh(date.weekday(), true)
        ),
        Locale::ZhHant | Locale::Yue => format!(
            "{}年{}月{}日 {}",
            date.year(),
            date.month(),
            date.day(),
            weekday_zh(date.weekday(), false)
        ),
        Locale::Ja => format!(
            "{}年{}月{}日({})",
            date.year(),
            date.month(),
            date.day(),
            weekday_ja(date.weekday())
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn msg(id: &str, user: &str, at: &str) -> Message {
        Message {
            id: id.into(),
            user_id: user.into(),
            channel_id: "c".into(),
            content: String::new(),
            card: None,
            created_at: at.parse().unwrap(),
            updated_at: None,
            deleted_at: None,
            parent_id: None,
            display_reactions: None,
            display_attachments: None,
            thread_archive_at: None,
            ciphertext: None,
            epoch: None,
            revision: 0,
            message_kind: None,
        }
    }

    #[test]
    fn unread_line_only_inside_render_window() {
        let m = [
            msg("1", "a", "2025-01-02T09:00:00"),
            msg("2", "a", "2025-01-02T10:00:00"),
            msg("3", "a", "2025-01-02T11:00:00"),
        ];
        let read: DateTime = "2025-01-02T09:30:00".parse().unwrap();
        // 最初の未読は 2 番目。窓が 2 番目から始まるなら出す。
        assert_eq!(first_unread_id_in_window(&m, 1, "me", read), Some("2"));
        assert_eq!(first_unread_id_in_window(&m, 0, "me", read), Some("2"));
        // 窓が 3 番目から始まるなら、窓の先頭行に付け替えず出さない。
        assert_eq!(first_unread_id_in_window(&m, 2, "me", read), None);
    }

    #[test]
    fn relative_day_label_uses_locale_format() {
        let today = date(2026, 6, 10);
        let d = date(2026, 6, 3);
        let label = |l| relative_day_label(d, today, &I18n::new(l));
        assert_eq!(label(Locale::KoKr), "2026년 6월 3일 (수)");
        assert_eq!(label(Locale::ZhHans), "2026年6月3日 周三");
        assert_eq!(label(Locale::ZhHant), "2026年6月3日 週三");
        assert_eq!(label(Locale::Yue), "2026年6月3日 週三");
    }

    fn items(messages: &[Message], unread: Option<&str>) -> Vec<MessageItem> {
        let refs: Vec<&Message> = messages.iter().collect();
        build_message_items(&refs, GROUP_GAP_MS, unread, |d| d)
    }

    #[test]
    fn same_author_within_five_minutes_is_grouped() {
        let m = [
            msg("1", "a", "2025-01-02T09:00:00"),
            msg("2", "a", "2025-01-02T09:05:00"),
            msg("3", "a", "2025-01-02T09:10:01"),
        ];
        let got = items(&m, None);
        assert!(got[0].is_group_start && !got[0].is_group_end);
        assert!(!got[1].is_group_start && got[1].is_group_end);
        // 5 分 1 秒を超えたら別のかたまり。
        assert!(got[2].is_group_start && got[2].is_group_end);
    }

    #[test]
    fn gap_boundary_is_inclusive() {
        let m = [
            msg("1", "a", "2025-01-02T09:00:00"),
            msg("2", "a", "2025-01-02T09:05:00"),
        ];
        assert!(!items(&m, None)[1].is_group_start);
    }

    #[test]
    fn different_author_breaks_the_group() {
        let m = [
            msg("1", "a", "2025-01-02T09:00:00"),
            msg("2", "b", "2025-01-02T09:01:00"),
            msg("3", "a", "2025-01-02T09:02:00"),
        ];
        let got = items(&m, None);
        assert!(got.iter().all(|i| i.is_group_start && i.is_group_end));
    }

    #[test]
    fn date_change_shows_separator_and_breaks_the_group() {
        let m = [
            msg("1", "a", "2025-01-02T23:58:00"),
            msg("2", "a", "2025-01-03T00:01:00"),
        ];
        let got = items(&m, None);
        assert!(got[0].show_date_separator);
        assert!(got[1].show_date_separator);
        assert!(got[0].is_group_end);
        assert!(got[1].is_group_start);
    }

    #[test]
    fn local_day_decides_the_separator() {
        // UTC では同じ日でも、+9 時間のローカルでは日付をまたぐ。
        let m = [
            msg("1", "a", "2025-01-02T14:00:00"),
            msg("2", "a", "2025-01-02T15:01:00"),
        ];
        let refs: Vec<&Message> = m.iter().collect();
        let got = build_message_items(&refs, GROUP_GAP_MS, None, |d| {
            d.checked_add(jiff::SignedDuration::from_hours(9)).unwrap()
        });
        assert!(got[1].show_date_separator);
    }

    #[test]
    fn unread_line_row_starts_a_new_group() {
        let m = [
            msg("1", "a", "2025-01-02T09:00:00"),
            msg("2", "a", "2025-01-02T09:01:00"),
        ];
        let got = items(&m, Some("2"));
        assert!(got[0].is_group_end);
        assert!(got[1].is_group_start);
    }

    #[test]
    fn out_of_order_timestamps_are_not_grouped() {
        let m = [
            msg("1", "a", "2025-01-02T09:05:00"),
            msg("2", "a", "2025-01-02T09:00:00"),
        ];
        assert!(items(&m, None)[1].is_group_start);
    }

    #[test]
    fn first_unread_skips_own_and_read_messages() {
        let m = [
            msg("1", "bob", "2025-01-02T09:00:00"),
            msg("2", "me", "2025-01-02T09:10:00"),
            msg("3", "bob", "2025-01-02T09:20:00"),
            msg("4", "bob", "2025-01-02T09:30:00"),
        ];
        let boundary: DateTime = "2025-01-02T09:05:00".parse().unwrap();
        assert_eq!(first_unread_id(&m, "me", boundary), Some("3"));
        let later: DateTime = "2025-01-02T09:30:00".parse().unwrap();
        assert_eq!(first_unread_id(&m, "me", later), None);
    }

    #[test]
    fn clock_is_zero_padded() {
        let d: DateTime = "2025-01-02T09:05:07".parse().unwrap();
        assert_eq!(format_clock(d), "09:05");
    }

    #[test]
    fn day_label_is_relative_for_today_and_yesterday() {
        let ja = I18n::new(Locale::Ja);
        let today = date(2026, 6, 10);
        assert_eq!(relative_day_label(today, today, &ja), "今日");
        assert_eq!(relative_day_label(date(2026, 6, 9), today, &ja), "昨日");
        assert_eq!(
            relative_day_label(date(2026, 6, 3), today, &ja),
            "2026年6月3日(水)"
        );
        // 月をまたぐ昨日。
        assert_eq!(
            relative_day_label(date(2026, 5, 31), date(2026, 6, 1), &ja),
            "昨日"
        );
    }

    #[test]
    fn english_day_label() {
        let en = I18n::new(Locale::En);
        assert_eq!(
            relative_day_label(date(2026, 6, 3), date(2026, 6, 10), &en),
            "Wed, June 3, 2026"
        );
    }
}
