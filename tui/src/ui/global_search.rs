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
    /// ファイル名を探す（false ならメッセージ）。
    pub files: bool,
    /// 今の結果を取ったときの語と種類。違えば Enter で検索し直す。
    pub searched: Option<(String, bool)>,
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<String>,
    pub searched_until: Option<String>,
    pub selected: usize,
    pub loading: bool,
    pub error: Option<String>,
    /// 遅れて届いた古い取得結果を捨てるための番号。
    pub generation: u64,
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
    pub fn kind(&self) -> SearchKind {
        if self.files {
            SearchKind::Files
        } else {
            SearchKind::Messages
        }
    }

    fn current(&self) -> Option<(String, bool)> {
        let query = self.query.split_whitespace().collect::<Vec<_>>().join(" ");
        (!query.is_empty()).then_some((query, self.files))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Esc => Action::Close,
            KeyCode::Enter => match self.current() {
                Some(current) if self.searched.as_ref() != Some(&current) => Action::Search,
                _ if !self.items.is_empty() => Action::Open,
                _ => Action::None,
            },
            KeyCode::Tab | KeyCode::BackTab => {
                self.files = !self.files;
                if self.searched.is_some() && self.current().is_some() {
                    Action::Search
                } else {
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

    /// 取得を始める。返した番号と一緒に結果を `apply` に渡す。
    pub fn start(&mut self, append: bool) -> Option<(u64, String, SearchKind, Option<String>)> {
        let (query, files) = if append {
            self.searched.clone()?
        } else {
            self.current()?
        };
        self.generation += 1;
        self.loading = true;
        self.error = None;
        if !append {
            self.items.clear();
            self.selected = 0;
            self.next_cursor = None;
            self.searched_until = None;
            self.searched = Some((query.clone(), files));
        }
        let cursor = if append {
            self.next_cursor.clone()
        } else {
            None
        };
        Some((self.generation, query, self.kind(), cursor))
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

#[cfg(test)]
mod tests {
    use super::*;

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
            ("会議 資料", SearchKind::Messages, None)
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
        let (_, _, _, cursor) = s.start(true).unwrap();
        assert_eq!(cursor.as_deref(), Some("c"));
        assert_eq!(
            s.on_key(key(KeyCode::Down)),
            Action::None,
            "already loading"
        );
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
        assert_eq!(kind, SearchKind::Files);
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
}
