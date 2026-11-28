//! 全文検索の画面（`S` / Ctrl+F。docs/search.md）。
//!
//! サーバーの `GET /api/search` を呼ぶ。暗号化チャンネルは対象外（サーバーが返さない）。
//! 1 回の取得で見るのは最大 12 か月なので、件数が少なくても `next_cursor` があれば続きがある。
//! 続きは選択が末尾にあるときの ↓ か PageDown で読む。
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::api::{SearchItem, SearchKind, SearchPage};

#[derive(Debug, Default)]
pub struct GlobalSearch {
    pub query: String,
    /// 探す種類（既定はメッセージとファイルの両方）。Tab で切り替える。
    pub kind: SearchKind,
    /// 今の結果を取ったときの語と種類。違えば Enter で検索し直す。
    pub searched: Option<(String, SearchKind)>,
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<String>,
    pub searched_until: Option<String>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// 直近の取得が続きのページだったか。失敗したとき、Enter で同じ取得をやり直すのに使う。
    pub last_append: bool,
    /// 遅れて届いた古い取得結果を捨てるための番号。
    pub generation: u64,
    /// 1 ページ目を取ったときの端末のタイムゾーン。`after:`・`before:` の 0 時はこれで決め、続きの取得と
    /// 再試行でも変えない（途中でタイムゾーンが変わると、同じカーソルで期間がずれて結果が抜けるため）。
    pub time_zone: Option<jiff::tz::TimeZone>,
}

/// 検索画面のキーの結果。取得と画面の切り替えは `App` が行う。
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Close,
    /// 1 ページ目から取り直す。
    Search,
    /// `next_cursor` の続きを読む。
    More,
    /// 選択中の結果を開く。
    Open,
}

/// 結果を開くときの行き先。
#[derive(Debug, PartialEq, Eq)]
pub struct Destination {
    pub channel_id: String,
    /// 開くメッセージ（ファイルなら添付の付いたメッセージ）。
    pub message_id: String,
    /// スレッドの返信なら親。
    pub parent_id: Option<String>,
}

impl GlobalSearch {
    fn current(&self) -> Option<(String, SearchKind)> {
        let query = self.query.split_whitespace().collect::<Vec<_>>().join(" ");
        (!query.is_empty()).then_some((query, self.kind))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Close,
            KeyCode::Enter => match self.current() {
                Some(current) if self.searched.as_ref() != Some(&current) => Action::Search,
                // 失敗した取得を同じ位置からやり直す（最初のページは検索、続きは続き）。
                _ if self.error.is_some() && !self.loading && self.searched.is_some() => {
                    if self.last_append {
                        Action::More
                    } else {
                        Action::Search
                    }
                }
                _ if !self.items.is_empty() => Action::Open,
                _ => Action::None,
            },
            KeyCode::Tab | KeyCode::BackTab => {
                const ORDER: [SearchKind; 3] =
                    [SearchKind::All, SearchKind::Messages, SearchKind::Files];
                let at = ORDER.iter().position(|k| *k == self.kind).unwrap_or(0);
                let step = if key.code == KeyCode::Tab {
                    1
                } else {
                    ORDER.len() - 1
                };
                self.kind = ORDER[(at + step) % ORDER.len()];
                if self.current().is_some() {
                    if self.searched.is_some() {
                        Action::Search
                    } else {
                        Action::None
                    }
                } else {
                    // 語がなければ検索し直せないので、別の種類の結果を残さない。
                    self.clear();
                    Action::None
                }
            }
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Action::None
            }
            KeyCode::Down if self.selected + 1 < self.items.len() => {
                self.selected += 1;
                Action::None
            }
            KeyCode::Down | KeyCode::PageDown => self.more(),
            KeyCode::Backspace => {
                self.query.pop();
                Action::None
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.query.push(ch);
                Action::None
            }
            _ => Action::None,
        }
    }

    fn more(&self) -> Action {
        if self.next_cursor.is_some() && !self.loading && self.error.is_none() {
            Action::More
        } else {
            Action::None
        }
    }

    fn clear(&mut self) {
        self.generation += 1;
        self.searched = None;
        self.items.clear();
        self.selected = 0;
        self.next_cursor = None;
        self.searched_until = None;
        self.loading = false;
        self.error = None;
    }

    /// 取得を始める。返した番号と一緒に結果を `apply` に渡す。続きのページは、今の表示ではなく
    /// 結果を取ったときの語と種類で取る（途中で種類を切り替えても、別の種類のカーソルを使わない）。
    pub fn start(&mut self, append: bool) -> Option<(u64, String, SearchKind, Option<String>)> {
        let (query, kind) = if append {
            self.searched.clone()?
        } else {
            self.current()?
        };
        self.generation += 1;
        self.loading = true;
        self.error = None;
        self.last_append = append;
        if !append {
            self.items.clear();
            self.selected = 0;
            self.next_cursor = None;
            self.searched_until = None;
            self.searched = Some((query.clone(), kind));
            self.time_zone = Some(jiff::tz::TimeZone::system());
        }
        let cursor = if append {
            self.next_cursor.clone()
        } else {
            None
        };
        Some((self.generation, query, kind, cursor))
    }

    pub fn apply(&mut self, generation: u64, append: bool, page: SearchPage) {
        if generation != self.generation {
            return;
        }
        self.loading = false;
        if !append {
            self.items.clear();
            self.selected = 0;
        }
        self.items.extend(page.items);
        self.next_cursor = page.next_cursor;
        self.searched_until = page.searched_until;
    }

    pub fn fail(&mut self, generation: u64, message: String) {
        if generation == self.generation {
            self.loading = false;
            self.error = Some(message);
        }
    }

    pub fn destination(&self) -> Option<Destination> {
        let item = self.items.get(self.selected)?;
        Some(Destination {
            channel_id: item.channel_id.clone(),
            message_id: item.message_id.clone().unwrap_or_else(|| item.id.clone()),
            parent_id: item.parent_id.clone(),
        })
    }
}

/// 検索語に書ける絞り込み（Slack と同じ書き方）。
/// `from:@user_name`・`in:#channel_name`（どちらも複数書くと、どれか）・`after:YYYY-MM-DD`・
/// `before:YYYY-MM-DD`・`with:me`（自分が参加中の会話）・`-is:bot`（ボットを除く）。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// 残りの語（空白区切り）。
    pub terms: String,
    pub from: Vec<String>,
    pub channels: Vec<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub with_me: bool,
    pub no_bots: bool,
}

fn is_day(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// 語と絞り込みを分ける。日付の形が違うものは、ふつうの語として残す。
pub fn parse_modifiers(query: &str) -> Modifiers {
    let mut out = Modifiers::default();
    let mut terms = Vec::new();
    for word in query.split_whitespace() {
        if let Some(name) = word.strip_prefix("from:") {
            let name = name.trim_start_matches('@');
            if !name.is_empty() {
                if !out.from.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                    out.from.push(name.to_string());
                }
                continue;
            }
        } else if let Some(name) = word.strip_prefix("in:") {
            let name = name.trim_start_matches('#');
            if !name.is_empty() {
                if !out.channels.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                    out.channels.push(name.to_string());
                }
                continue;
            }
        } else if word.eq_ignore_ascii_case("with:me") {
            out.with_me = true;
            continue;
        } else if word.eq_ignore_ascii_case("-is:bot") {
            out.no_bots = true;
            continue;
        } else if let Some(day) = word.strip_prefix("after:").filter(|d| is_day(d)) {
            out.after = Some(day.to_string());
            continue;
        } else if let Some(day) = word.strip_prefix("before:").filter(|d| is_day(d)) {
            out.before = Some(day.to_string());
            continue;
        }
        terms.push(word);
    }
    out.terms = terms.join(" ");
    out
}

/// 強調する語。今の結果を取ったときの語から、絞り込み（`from:` など）を除いたもの。
/// サーバーと同じく、重複を除いて最大 5 個。
pub fn highlight_terms(search: &GlobalSearch) -> Vec<String> {
    let Some((query, _)) = &search.searched else {
        return Vec::new();
    };
    let mut terms: Vec<String> = Vec::new();
    for term in parse_modifiers(query).terms.split_whitespace() {
        if !terms.iter().any(|t| t == term) {
            terms.push(term.to_string());
        }
    }
    terms.truncate(5);
    terms
}

/// `text` を、語に当たった部分（`true`）とそれ以外に分ける。大文字と小文字は区別しない
/// （サーバーの `lower()` と同じ）。重なる語は長い方を優先する。
pub fn highlight_segments(text: &str, terms: &[String]) -> Vec<(String, bool)> {
    let mut terms: Vec<Vec<char>> = terms
        .iter()
        .filter(|t| !t.is_empty())
        .map(|t| t.chars().collect())
        .collect();
    terms.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let chars: Vec<char> = text.chars().collect();
    let same = |a: char, b: char| a == b || a.to_lowercase().eq(b.to_lowercase());
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut push = |piece: String, hit: bool| match out.last_mut() {
        Some((last, last_hit)) if *last_hit == hit && !hit => last.push_str(&piece),
        _ => out.push((piece, hit)),
    };
    let mut i = 0;
    while i < chars.len() {
        let hit = terms.iter().find(|term| {
            chars.len() - i >= term.len() && term.iter().zip(&chars[i..]).all(|(t, c)| same(*c, *t))
        });
        match hit {
            Some(term) => {
                push(chars[i..i + term.len()].iter().collect(), true);
                i += term.len();
            }
            None => {
                push(chars[i].to_string(), false);
                i += 1;
            }
        }
    }
    out
}

/// 最初に強調する区間が幅 `width` に収まるよう、その前を `…` で省く（全角は 2）。
/// サーバーの抜粋は一致箇所の 40 文字前から始まるため、日本語では先頭から詰めると一致箇所が欄の外に出る。
/// 一致箇所の前には欄の 3 分の 1 までを残し、末尾の切り詰めは呼び出し側に任せる。
pub fn focus_first_hit(segments: Vec<(String, bool)>, width: usize) -> Vec<(String, bool)> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let Some(hit) = segments.iter().position(|(_, hit)| *hit) else {
        return segments;
    };
    let before: usize = segments[..hit].iter().map(|(text, _)| text.width()).sum();
    let hit_width = segments[hit].0.width();
    if before + hit_width <= width {
        return segments;
    }
    let keep = width.saturating_sub(hit_width + 1).min(width / 3);
    let prefix: String = segments[..hit]
        .iter()
        .map(|(text, _)| text.as_str())
        .collect();
    let mut kept = Vec::new();
    let mut used = 0;
    for ch in prefix.chars().rev() {
        let w = ch.width().unwrap_or(0);
        if used + w > keep {
            break;
        }
        used += w;
        kept.push(ch);
    }
    let mut out = vec![(
        std::iter::once('…').chain(kept.into_iter().rev()).collect(),
        false,
    )];
    out.extend(segments.into_iter().skip(hit));
    out
}

/// `after:`・`before:` の日付（端末の暦の日）の 0 時を、オフセット付きの時刻にして API に送る。
/// UTC の日付で区切ると、JST の 0〜9 時の投稿が前日に入るため（Web と同じ）。夏時間で 0 時がない日は、
/// その日の最初の時刻。日付にならない値はそのまま送り、サーバーの 400 に任せる。
pub fn local_midnight(day: &str, tz: &jiff::tz::TimeZone) -> String {
    day.parse::<jiff::civil::Date>()
        .ok()
        .and_then(|date| date.to_zoned(tz.clone()).ok())
        .map(|at| at.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string())
        .unwrap_or_else(|| day.to_string())
}

/// 結果の日付。`created_at` は UTC の naive 値なので、タイムラインと同じく `tz`（端末のタイムゾーン）に直す。
pub fn result_time(created_at: &str, now: &jiff::Zoned) -> String {
    created_at
        .parse::<jiff::civil::DateTime>()
        .ok()
        .and_then(|at| at.to_zoned(jiff::tz::TimeZone::UTC).ok())
        .map(|at| {
            zoo2_client_core::timestamps::message_timestamp(
                at.with_time_zone(now.time_zone().clone()).datetime(),
                now.date(),
            )
        })
        .unwrap_or_else(|| created_at.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_first_hit_keeps_a_late_hit_in_view() {
        let text = format!("…{}会議の資料", "あ".repeat(40));
        let segments = focus_first_hit(highlight_segments(&text, &["会議".to_string()]), 74);
        let shown: usize = segments
            .iter()
            .take_while(|(_, hit)| !hit)
            .map(|(text, _)| unicode_width::UnicodeWidthStr::width(text.as_str()))
            .sum();
        assert!(shown + 4 <= 74, "{segments:?}");
        assert!(segments[0].0.starts_with('…'), "{segments:?}");
        assert_eq!(segments[1], ("会議".to_string(), true));
        // 収まるなら変えない。
        let short = highlight_segments("明日の会議", &["会議".to_string()]);
        assert_eq!(focus_first_hit(short.clone(), 74), short);
    }

    #[test]
    fn local_midnight_sends_the_terminal_day_with_its_offset() {
        let jst = jiff::tz::TimeZone::fixed(jiff::tz::offset(9));
        assert_eq!(
            local_midnight("2026-11-22", &jst),
            "2026-11-22T00:00:00+09:00"
        );
        assert_eq!(
            local_midnight("2026-11-22", &jiff::tz::TimeZone::UTC),
            "2026-11-22T00:00:00+00:00"
        );
        assert_eq!(local_midnight("2026-13-01", &jst), "2026-13-01");
    }

    #[test]
    fn result_time_uses_the_terminal_time_zone() {
        let now = "2026-11-22T12:00+09:00[Asia/Tokyo]".parse().unwrap();
        assert_eq!(result_time("2026-11-21T23:30:00.000", &now), "08:30");
        assert_eq!(result_time("2026-11-21T08:00:00", &now), "11/21 17:00");
        assert_eq!(result_time("2026-12-31T16:00:00", &now), "2027/01/01 01:00");
        assert_eq!(result_time("壊れた値", &now), "壊れた値");
    }

    #[test]
    fn highlight_segments_marks_hits_case_insensitively_and_prefers_longer_terms() {
        let terms = vec![
            "検索".to_string(),
            "全文検索".to_string(),
            "rust".to_string(),
        ];
        assert_eq!(
            highlight_segments("Rust の全文検索と検索", &terms),
            vec![
                ("Rust".to_string(), true),
                (" の".to_string(), false),
                ("全文検索".to_string(), true),
                ("と".to_string(), false),
                ("検索".to_string(), true),
            ]
        );
        assert_eq!(
            highlight_segments("本文", &[]),
            vec![("本文".to_string(), false)]
        );
    }

    #[test]
    fn highlight_terms_drop_modifiers_and_duplicates() {
        let search = GlobalSearch {
            searched: Some((
                "会議 from:@alice 資料 会議 in:#general after:2026-01-01".into(),
                SearchKind::All,
            )),
            ..Default::default()
        };
        assert_eq!(highlight_terms(&search), vec!["会議", "資料"]);
        assert!(highlight_terms(&GlobalSearch::default()).is_empty());
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn item(id: &str, parent: Option<&str>, message: Option<&str>) -> SearchItem {
        serde_json::from_value(serde_json::json!({
            "id": id, "channel_id": "c1", "channel_name": "general", "parent_id": parent,
            "message_id": message, "snippet": "s", "created_at": "2026-11-21T00:00:00.000"
        }))
        .unwrap()
    }

    #[test]
    fn enter_searches_new_query_then_opens() {
        let mut s = GlobalSearch::default();
        assert_eq!(s.on_key(key(KeyCode::Enter)), Action::None);
        for ch in " 会議  資料 ".chars() {
            s.on_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(s.on_key(key(KeyCode::Enter)), Action::Search);
        let (generation, query, kind, cursor) = s.start(false).unwrap();
        assert_eq!(
            (query.as_str(), kind, cursor),
            ("会議 資料", SearchKind::All, None)
        );
        s.apply(
            generation,
            false,
            SearchPage {
                items: vec![item("m1", None, None), item("m2", Some("root"), None)],
                next_cursor: Some("c".into()),
                searched_until: Some("2025-12".into()),
            },
        );
        assert_eq!(s.on_key(key(KeyCode::Down)), Action::None);
        assert_eq!(s.on_key(key(KeyCode::Enter)), Action::Open);
        assert_eq!(
            s.destination(),
            Some(Destination {
                channel_id: "c1".into(),
                message_id: "m2".into(),
                parent_id: Some("root".into())
            })
        );
        // 末尾で ↓ は続きを読む。
        assert_eq!(s.on_key(key(KeyCode::Down)), Action::More);
        // 1 ページ目のタイムゾーンを、続きの取得でも使う。
        let first_zone = jiff::tz::TimeZone::fixed(jiff::tz::offset(9));
        s.time_zone = Some(first_zone.clone());
        let (_, _, _, cursor) = s.start(true).unwrap();
        assert_eq!(cursor.as_deref(), Some("c"));
        assert_eq!(s.time_zone, Some(first_zone));
        assert_eq!(
            s.on_key(key(KeyCode::Down)),
            Action::None,
            "already loading"
        );
    }

    #[test]
    fn enter_retries_the_failed_request_from_the_same_place() {
        let mut s = GlobalSearch {
            query: "会議".into(),
            ..Default::default()
        };
        let (first, ..) = s.start(false).unwrap();
        s.fail(first, "timeout".into());
        assert_eq!(
            s.on_key(key(KeyCode::Enter)),
            Action::Search,
            "first page retries"
        );
        let (second, ..) = s.start(false).unwrap();
        s.apply(
            second,
            false,
            SearchPage {
                items: vec![item("m1", None, None)],
                next_cursor: Some("c".into()),
                searched_until: None,
            },
        );
        assert_eq!(s.on_key(key(KeyCode::Down)), Action::More);
        let (third, _, _, cursor) = s.start(true).unwrap();
        assert_eq!(cursor.as_deref(), Some("c"));
        s.fail(third, "timeout".into());
        assert_eq!(
            s.on_key(key(KeyCode::Enter)),
            Action::More,
            "older page retries"
        );
        let (_, _, _, cursor) = s.start(true).unwrap();
        assert_eq!(cursor.as_deref(), Some("c"));
    }

    #[test]
    fn older_pages_keep_the_kind_they_were_searched_with() {
        let mut s = GlobalSearch {
            query: "pdf".into(),
            ..Default::default()
        };
        let (generation, ..) = s.start(false).unwrap();
        s.apply(
            generation,
            false,
            SearchPage {
                items: vec![item("m1", None, None)],
                next_cursor: Some("c".into()),
                searched_until: None,
            },
        );
        // 語を消して Tab: 検索し直せないので結果を消す（別の種類の結果を残さない）。
        s.query.clear();
        assert_eq!(s.on_key(key(KeyCode::Tab)), Action::None);
        assert!(s.items.is_empty() && s.next_cursor.is_none() && s.start(true).is_none());
        // 結果を残したまま種類だけ変わることはないが、start(true) は取ったときの種類を使う。
        s.query = "pdf".into();
        s.kind = SearchKind::Messages;
        let (generation, ..) = s.start(false).unwrap();
        s.apply(
            generation,
            false,
            SearchPage {
                items: vec![item("m1", None, None)],
                next_cursor: Some("c".into()),
                searched_until: None,
            },
        );
        s.kind = SearchKind::Files;
        let (_, _, kind, _) = s.start(true).unwrap();
        assert_eq!(kind, SearchKind::Messages);
    }

    #[test]
    fn tab_switches_kind_and_stale_results_are_dropped() {
        let mut s = GlobalSearch {
            query: "pdf".into(),
            ..Default::default()
        };
        let (first, ..) = s.start(false).unwrap();
        assert_eq!(s.on_key(key(KeyCode::Tab)), Action::Search);
        let (second, _, kind, _) = s.start(false).unwrap();
        assert_eq!(kind, SearchKind::Messages, "all → messages → files");
        assert_eq!(s.on_key(key(KeyCode::BackTab)), Action::Search);
        assert_eq!(s.kind, SearchKind::All);
        s.apply(
            first,
            false,
            SearchPage {
                items: vec![item("old", None, None)],
                next_cursor: None,
                searched_until: None,
            },
        );
        assert!(s.items.is_empty() && s.loading);
        s.apply(
            second,
            false,
            SearchPage {
                items: vec![item("a1", None, Some("m1"))],
                next_cursor: None,
                searched_until: None,
            },
        );
        assert_eq!(s.destination().unwrap().message_id, "m1");
        assert_eq!(s.on_key(key(KeyCode::Down)), Action::None, "no more pages");
        s.fail(second, "x".into());
        assert_eq!(s.error.as_deref(), Some("x"));
    }

    #[test]
    fn modifiers_are_split_from_the_terms() {
        let parsed = parse_modifiers(
            "会議 from:@alice in:#general after:2026-11-01 before:11/30 資料 from:bob from:@Alice with:me -is:bot in:#random",
        );
        assert_eq!(
            parsed,
            Modifiers {
                terms: "会議 before:11/30 資料".into(),
                from: vec!["alice".into(), "bob".into()],
                channels: vec!["general".into(), "random".into()],
                after: Some("2026-11-01".into()),
                before: None,
                with_me: true,
                no_bots: true,
            }
        );
        assert_eq!(parse_modifiers("from: in:#").terms, "from: in:#");
    }
}
