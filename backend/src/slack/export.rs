//! Slack 公式エクスポート (ZIP) の読み取り。
//!
//! 構成: `users.json`, `channels.json`, `groups.json` (非公開チャンネル、プランによる) と、
//! チャンネル名のディレクトリに日付ごとの `YYYY-MM-DD.json`。添付ファイルの実体は含まれない。

use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Seek},
    path::Path,
};

use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::error::AppError;

/// JSON 1 ファイルあたりの上限。壊れた / 悪意ある ZIP でメモリを使い切らないようにする。
/// 読み込んだバイト列と解析後の構造を同時に持つので、backend の memory limit (512Mi) より十分小さくする。
const MAX_JSON_ENTRY_SIZE: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct SlackUser {
    pub id: String,
    #[serde(default)]
    pub team_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub profile: SlackProfile,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlackProfile {
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub image_72: String,
}

impl SlackUser {
    pub fn display_name(&self) -> &str {
        [
            self.profile.display_name.as_str(),
            self.profile.real_name.as_str(),
            self.real_name.as_str(),
            self.name.as_str(),
        ]
        .into_iter()
        .find(|s| !s.trim().is_empty())
        .unwrap_or(self.id.as_str())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SlackText {
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackChannel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub members: Vec<String>,
    /// チャンネルを作った Slack ユーザー (各ユーザーが自分のチャンネルを取り込むときに使う)
    #[serde(default)]
    pub creator: String,
    #[serde(default)]
    pub purpose: SlackText,
    #[serde(default)]
    pub is_archived: bool,
    /// `groups.json` 由来なら true
    #[serde(skip)]
    pub is_private: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackMessage {
    #[serde(default)]
    pub ts: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    /// この投稿をしたアプリの ID。自分のアプリ (`SLACK_APP_ID`) の投稿は、取り込まない (ループ防止)
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub edited: Option<SlackEdited>,
    #[serde(default)]
    pub reactions: Vec<SlackReaction>,
    #[serde(default)]
    pub files: Vec<SlackFile>,
    #[serde(default)]
    pub bot_profile: Option<SlackBotProfile>,
    /// bot 投稿のカード (旧形式の attachments)。本文がここにしかない投稿もある
    #[serde(default)]
    pub attachments: Vec<serde_json::Value>,
    /// Block Kit のブロック。人の投稿は本文と同じ rich_text だけなので、それ以外を含むときだけ部品にする
    #[serde(default)]
    pub blocks: Vec<serde_json::Value>,
    /// スレッドの親なら、返信の ts の一覧 (エクスポートに含まれる場合)
    #[serde(default)]
    pub replies: Vec<SlackReplyRef>,
    /// `conversations.history` が返す、スレッドの返信の数と最後の返信の `ts` (差分同期 #292 で返信の取得に使う)
    #[serde(default)]
    pub reply_count: Option<i64>,
    #[serde(default)]
    pub latest_reply: Option<String>,
    /// 編集・削除の履歴を含むエクスポートで、`message_changed` / `message_deleted` の記録が持つ、元の投稿の全体
    #[serde(default)]
    pub original: Option<serde_json::Value>,
    /// 公開されているエクスポートの形式では、削除された元の投稿の `ts` がトップレベルの `original_ts` にある
    #[serde(default)]
    pub original_ts: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackReplyRef {
    #[serde(default)]
    pub ts: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackEdited {
    #[serde(default)]
    pub ts: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackBotProfile {
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackReaction {
    pub name: String,
    #[serde(default)]
    pub users: Vec<String>,
    /// リアクションした人数。`users` は人数が多いと一部しか入らないので、`count` のほうが大きいときは、
    /// 全員がそろっていない (リアクションの削除を反映するときに、API で確かめ直す)
    #[serde(default)]
    pub count: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlackFile {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub mimetype: Option<String>,
    #[serde(default)]
    pub size: Option<i64>,
    #[serde(default)]
    pub url_private_download: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
}

impl SlackFile {
    /// 実体を取得して添付にできるファイルか。外部サービスの共有 (`external`)、削除済み (`tombstone`)、
    /// 無料プランで見えないもの (`hidden_by_limit`)、ダウンロード URL のないものは取り込めない
    pub fn is_importable(&self) -> bool {
        self.url_private_download
            .as_deref()
            .is_some_and(|u| !u.is_empty())
            && !matches!(
                self.mode.as_deref(),
                Some("tombstone" | "hidden_by_limit" | "external")
            )
    }

    /// 表示に使う名前 (`name`、なければ `title`)
    pub fn display_name(&self) -> Option<&str> {
        self.name
            .as_deref()
            .or(self.title.as_deref())
            .map(str::trim)
            .filter(|n| !n.is_empty())
    }
}

/// チャンネル参加・トピック変更などのシステムメッセージは取り込まない
pub(super) const SKIPPED_SUBTYPES: &[&str] = &[
    "channel_join",
    "channel_leave",
    "channel_topic",
    "channel_purpose",
    "channel_name",
    "channel_archive",
    "channel_unarchive",
    "channel_convert_to_private",
    "channel_convert_to_public",
    "group_join",
    "group_leave",
    "group_topic",
    "group_purpose",
    "group_name",
    "group_archive",
    "group_unarchive",
    "pinned_item",
    "unpinned_item",
    "bot_add",
    "bot_remove",
    "bot_enable",
    "bot_disable",
    "tombstone",
    "joiner_notification",
    "reminder_add",
    // 編集・削除の履歴を含むエクスポートの記録。`ts` は編集・削除した日時で、投稿そのものではない
    // (`message_changed` は編集後の本文と投稿者を持つので、取り込むと別の投稿として重複して入る。
    // 編集してから削除した投稿は、元の記録がなくなっても、この記録が残る)
    "message_changed",
    "message_deleted",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackForwardRef {
    pub slack_channel_id: String,
    pub ts: String,
}

/// Slack メッセージのパーマリンク（`.../archives/<CH>/p<digits>`）から `(slack_channel_id, ts)` を取り出す。
pub fn parse_slack_archive_url(url: &str) -> Option<SlackForwardRef> {
    let without_fragment = url.split('#').next().unwrap_or(url);
    let path = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);
    let (_, rest) = path.split_once("/archives/")?;
    let mut segments = rest.split('/');
    let channel_id = segments.next()?.trim();
    if channel_id.is_empty() || !channel_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let p_segment = segments.next()?.trim();
    let digits = p_segment.strip_prefix('p')?;
    if digits.len() < 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let split = digits.len() - 6;
    let ts = format!("{}.{}", &digits[..split], &digits[split..]);
    if !crate::slack::is_valid_ts(&ts) {
        return None;
    }
    Some(SlackForwardRef {
        slack_channel_id: channel_id.to_string(),
        ts,
    })
}

fn is_valid_slack_channel_id(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Slack の `ts` 表記をパーマリンク（`/archives/.../p<digits>`）と同じ小数部 6 桁へ揃える。
/// すでに 6 桁以上の場合はそのまま保つ。
pub fn canonical_slack_ts(ts: &str) -> Option<String> {
    let trimmed = ts.trim();
    if !crate::slack::is_valid_ts(trimmed) {
        return None;
    }
    let (secs, frac) = trimmed.split_once('.').unwrap_or((trimmed, ""));
    if frac.len() >= 6 {
        Some(trimmed.to_string())
    } else {
        Some(format!("{secs}.{frac:0<6}"))
    }
}

impl SlackMessage {
    /// 編集・削除の履歴を含むエクスポートの、編集 (`message_changed`)・削除 (`message_deleted`) の記録か
    pub fn is_history_event(&self) -> bool {
        matches!(
            self.subtype.as_deref(),
            Some("message_changed" | "message_deleted")
        )
    }

    /// 編集・削除の記録が指す、元の投稿の `ts`。`original.ts` (確認したエクスポートの形式) と、
    /// トップレベルの `original_ts` (公開されている形式) のどちらも読む
    pub fn original_message_ts(&self) -> Option<&str> {
        if !self.is_history_event() {
            return None;
        }
        self.original
            .as_ref()
            .and_then(|o| o.get("ts"))
            .and_then(|ts| ts.as_str())
            .or(self.original_ts.as_deref())
            .filter(|ts| crate::slack::is_valid_ts(ts))
    }

    /// 削除の記録 (`message_deleted`) が指す、削除された元の投稿の `ts`。
    /// 削除された投稿の記録は、エクスポートには残らないので、以前の取り込みで入れた分を消すのに使う
    pub fn deleted_original_ts(&self) -> Option<&str> {
        if self.subtype.as_deref() != Some("message_deleted") {
            return None;
        }
        self.original_message_ts()
    }

    /// 投稿者の Slack ID (ユーザーまたは bot)
    pub fn author(&self) -> Option<&str> {
        self.user
            .as_deref()
            .filter(|u| !u.is_empty())
            .or_else(|| self.bot_id.as_deref().filter(|b| !b.is_empty()))
    }

    pub fn is_importable(&self) -> bool {
        self.is_importable_for_team(None)
    }

    /// スレッドの親として `conversations.replies` を辿る候補にできるか（親投稿自身の取り込み可否とは独立）。
    pub fn is_thread_root_candidate(&self) -> bool {
        crate::slack::is_valid_ts(&self.ts)
            && self.author().is_some()
            && !self
                .subtype
                .as_deref()
                .is_some_and(|s| SKIPPED_SUBTYPES.contains(&s))
            && self
                .subtype
                .as_deref()
                .is_none_or(|t| !(t.starts_with("channel_") || t.starts_with("group_")))
    }

    pub fn is_importable_for_team(&self, team: Option<&str>) -> bool {
        crate::slack::is_valid_ts(&self.ts)
            && self.author().is_some()
            && !self
                .subtype
                .as_deref()
                .is_some_and(|s| SKIPPED_SUBTYPES.contains(&s))
            && (!self.text.trim().is_empty()
                || self.files.iter().any(SlackFile::is_importable)
                || !self.files_text(|_| true).is_empty()
                || !self.attachments_text().is_empty()
                || self.has_forward_refs_for_team(team)
                // 本文が blocks にしかない bot 投稿（rich_text 以外のブロック）も取り込む。部品にする条件と同じ
                || self
                    .blocks
                    .iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) != Some("rich_text")))
    }

    /// Slack のメッセージ転送・引用（`is_share` / `is_msg_unfurl`）添付か。
    fn is_forwarded_attachment(a: &serde_json::Value) -> bool {
        a.get("is_share").and_then(|v| v.as_bool()) == Some(true)
            || a.get("is_msg_unfurl").and_then(|v| v.as_bool()) == Some(true)
    }

    fn extract_forwarded_ref(a: &serde_json::Value, team: Option<&str>) -> Option<SlackForwardRef> {
        if !Self::is_forwarded_attachment(a) {
            return None;
        }
        let att_team = ["channel_team", "team_id", "team"]
            .into_iter()
            .find_map(|key| {
                a.get(key)
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
            });
        if let (Some(current_team), Some(att_team)) = (team, att_team)
            && att_team != current_team
        {
            return None;
        }
        let direct = a
            .get("channel_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|ch| is_valid_slack_channel_id(ch))
            .zip(
                a.get("ts")
                    .and_then(|v| v.as_str())
                    .and_then(canonical_slack_ts),
            )
            .map(|(slack_channel_id, ts)| SlackForwardRef {
                slack_channel_id: slack_channel_id.to_string(),
                ts,
            });
        let from_url = a
            .get("from_url")
            .and_then(|v| v.as_str())
            .and_then(parse_slack_archive_url);
        direct.or(from_url)
    }

    /// `channel_id` + `ts` または `/archives/...` の参照を抽出できる転送・引用添付か。
    /// 参照を取り出せるものは `forwarded_message_ids` 側（または非同一ワークスペース除外）として扱い、
    /// 参照を取り出せない古い・不完全な添付だけ従来の本文／カード転記へフォールバックする。
    fn is_resolvable_forwarded_attachment(a: &serde_json::Value) -> bool {
        Self::extract_forwarded_ref(a, None).is_some()
    }

    /// Slack 添付配列からメッセージ転送・引用（`is_share` / `is_msg_unfurl`）の対象を取り出す。
    /// 他ワークスペースの参照は `team` 指定時に除外し、重複排除して最大件数までに制限する。
    pub fn forwarded_refs(&self, team: Option<&str>) -> Vec<SlackForwardRef> {
        let mut refs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for a in &self.attachments {
            let Some(found) = Self::extract_forwarded_ref(a, team) else {
                continue;
            };
            if seen.insert((found.slack_channel_id.clone(), found.ts.clone())) {
                refs.push(found);
                if refs.len() >= crate::routes::forwards::MAX_FORWARDED_MESSAGES_PER_MESSAGE {
                    break;
                }
            }
        }
        refs
    }

    pub fn has_forward_refs(&self) -> bool {
        self.has_forward_refs_for_team(None)
    }

    pub fn has_forward_refs_for_team(&self, team: Option<&str>) -> bool {
        !self.forwarded_refs(team).is_empty()
    }

    /// `include` が真の添付ファイルの名前 (1 行に 1 つ)。本文のない投稿が空にならないよう、代わりの本文に使う
    pub fn files_text(&self, include: impl Fn(&SlackFile) -> bool) -> String {
        self.files
            .iter()
            .filter(|file| include(file))
            .filter_map(SlackFile::display_name)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// リンクの展開 (unfurl) と参照抽出できるメッセージ転送・引用を除く attachments
    fn own_attachments(&self) -> impl Iterator<Item = &serde_json::Value> {
        self.attachments.iter().filter(|a| {
            a.get("from_url").is_none()
                && a.get("is_app_unfurl").is_none()
                && !Self::is_resolvable_forwarded_attachment(a)
        })
    }

    /// カードにできなかったときに本文へ残す文字列 (fallback・タイトル・本文)
    pub fn attachments_text(&self) -> String {
        self.own_attachments()
            .flat_map(|a| {
                let text = |key: &str| a.get(key).and_then(|v| v.as_str()).unwrap_or("").trim();
                let main = [text("pretext"), text("text")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n");
                let main = if main.is_empty() {
                    text("fallback").to_string()
                } else {
                    main
                };
                [text("title").to_string(), main]
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// blocks と attachments を部品の木にする。リンクの展開 (unfurl) は zoo 側で表示するので除く。
    /// blocks が本文と同じ rich_text だけなら、blocks は使わない (本文をそのまま表示する)。
    /// 部品にするものがなければ None。操作部品は押せない状態になる。
    pub fn components(
        &self,
        resolver: &impl super::convert::Resolver,
    ) -> Option<crate::components::Document> {
        let blocks: Vec<&serde_json::Value> = if self
            .blocks
            .iter()
            .all(|b| b.get("type").and_then(|t| t.as_str()) == Some("rich_text"))
        {
            Vec::new()
        } else {
            self.blocks.iter().take(100).collect()
        };
        let attachments: Vec<&serde_json::Value> = self
            .attachments
            .iter()
            .filter(|a| !Self::is_resolvable_forwarded_attachment(a))
            .take(100)
            .collect();
        if blocks.is_empty() && attachments.is_empty() {
            return None;
        }
        let body =
            serde_json::json!({ "text": self.text, "blocks": blocks, "attachments": attachments });
        crate::components::slack_message(&body, resolver, "slack_import")
            .ok()
            .and_then(|i| i.doc)
    }

    /// スレッドの返信なら親の ts
    pub fn parent_ts(&self) -> Option<&str> {
        self.thread_ts
            .as_deref()
            .filter(|t| *t != self.ts && crate::slack::is_valid_ts(t))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub slack_channel_id: String,
    pub name: String,
    pub is_private: bool,
    pub is_archived: bool,
    pub members: usize,
    pub creator: String,
    pub message_count: u64,
}

pub struct ExportArchive<R> {
    zip: ZipArchive<R>,
    /// エクスポートが 1 階層下のディレクトリに入っている場合の接頭辞
    prefix: String,
    /// チャンネルのディレクトリ名 → 日付ファイルのエントリ名 (昇順)
    day_files: HashMap<String, Vec<String>>,
}

impl ExportArchive<File> {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        let file = File::open(path).map_err(|e| AppError::Internal(e.into()))?;
        Self::new(file)
    }
}

impl<R: Read + Seek> ExportArchive<R> {
    pub fn new(reader: R) -> Result<Self, AppError> {
        let zip = ZipArchive::new(reader)
            .map_err(|e| AppError::BadRequest(format!("Not a valid ZIP archive: {e}")))?;
        let names: Vec<String> = zip
            .file_names()
            .map(|name| name.map(|name| name.into_owned()))
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::BadRequest(format!("Invalid ZIP entry name: {e}")))?;
        let channels_entry = names
            .iter()
            .filter(|n| {
                let base = n.rsplit('/').next().unwrap_or(n);
                base == "channels.json" && n.matches('/').count() <= 1
            })
            .min_by_key(|n| n.len())
            .ok_or_else(|| {
                AppError::BadRequest("channels.json was not found in the export".into())
            })?;
        let prefix = channels_entry
            .strip_suffix("channels.json")
            .unwrap_or("")
            .to_string();
        let mut day_files: HashMap<String, Vec<String>> = HashMap::new();
        for name in &names {
            let Some(rel) = name.strip_prefix(&prefix) else {
                continue;
            };
            let mut parts = rel.split('/');
            let (Some(dir), Some(file), None) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if file.len() == "YYYY-MM-DD.json".len()
                && file.ends_with(".json")
                && file.as_bytes()[4] == b'-'
            {
                day_files
                    .entry(dir.to_string())
                    .or_default()
                    .push(name.clone());
            }
        }
        for files in day_files.values_mut() {
            files.sort();
        }
        Ok(Self {
            zip,
            prefix,
            day_files,
        })
    }

    fn read_json<T: for<'de> Deserialize<'de>>(
        &mut self,
        name: &str,
    ) -> Result<Option<T>, AppError> {
        let mut entry = match self.zip.by_name(name) {
            Ok(entry) => entry,
            Err(zip::result::ZipError::FileNotFound) => return Ok(None),
            Err(e) => {
                return Err(AppError::BadRequest(format!(
                    "Broken ZIP entry {name}: {e}"
                )));
            }
        };
        if entry.size() > MAX_JSON_ENTRY_SIZE {
            return Err(AppError::BadRequest(format!("{name} is too large")));
        }
        let mut buf = Vec::with_capacity(entry.size() as usize);
        (&mut entry)
            .take(MAX_JSON_ENTRY_SIZE + 1)
            .read_to_end(&mut buf)
            .map_err(|e| AppError::BadRequest(format!("Broken ZIP entry {name}: {e}")))?;
        serde_json::from_slice(&buf)
            .map(Some)
            .map_err(|e| AppError::BadRequest(format!("Invalid JSON in {name}: {e}")))
    }

    pub fn users(&mut self) -> Result<Vec<SlackUser>, AppError> {
        let name = format!("{}users.json", self.prefix);
        Ok(self.read_json(&name)?.unwrap_or_default())
    }

    /// 公開チャンネル (`channels.json`) と非公開チャンネル (`groups.json`)。
    /// DM / グループ DM はユーザー連携後の差分同期 (#292) で扱う。
    pub fn channels(&mut self) -> Result<Vec<SlackChannel>, AppError> {
        let public: Vec<SlackChannel> = self
            .read_json(&format!("{}channels.json", self.prefix))?
            .unwrap_or_default();
        let mut private: Vec<SlackChannel> = self
            .read_json(&format!("{}groups.json", self.prefix))?
            .unwrap_or_default();
        for c in &mut private {
            c.is_private = true;
        }
        Ok(public.into_iter().chain(private).collect())
    }

    pub fn day_files(&self, channel: &SlackChannel) -> Vec<String> {
        self.day_files
            .get(&channel.name)
            .or_else(|| self.day_files.get(&channel.id))
            .cloned()
            .unwrap_or_default()
    }

    pub fn messages(&mut self, entry: &str) -> Result<Vec<SlackMessage>, AppError> {
        let mut messages: Vec<SlackMessage> = self.read_json(entry)?.unwrap_or_default();
        // スレッドの親を返信より先に入れるため ts 順にそろえる
        messages.sort_by(|a, b| compare_ts(&a.ts, &b.ts));
        Ok(messages)
    }

    /// チャンネル一覧と取り込み対象のメッセージ数を数える
    pub fn summarize(&mut self) -> Result<(String, Vec<ChannelSummary>, usize), AppError> {
        let users = self.users()?;
        // 共有チャンネルなどで別のワークスペースのユーザーが混ざることがあるので、最も多い team_id を使う。
        // 取れない ZIP は連携 (#291) で付け替えられないので受け付けない。
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for user in &users {
            if !user.team_id.is_empty() {
                *counts.entry(user.team_id.as_str()).or_default() += 1;
            }
        }
        let team_id = counts
            .into_iter()
            .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
            .map(|(team, _)| team.to_string())
            .ok_or_else(|| {
                AppError::BadRequest(
                    "users.json does not contain a team_id; export the workspace from Slack again"
                        .into(),
                )
            })?;
        let channels = self.channels()?;
        let mut summaries = Vec::with_capacity(channels.len());
        for channel in channels {
            let mut count = 0u64;
            for entry in self.day_files(&channel) {
                count += self
                    .messages(&entry)?
                    .iter()
                    .filter(|m| m.is_importable_for_team(Some(&team_id)))
                    .count() as u64;
            }
            summaries.push(ChannelSummary {
                slack_channel_id: channel.id.clone(),
                name: channel.name.clone(),
                is_private: channel.is_private,
                is_archived: channel.is_archived,
                members: channel.members.len(),
                creator: channel.creator.clone(),
                message_count: count,
            });
        }
        Ok((team_id, summaries, users.len()))
    }
}

/// ts を数値として比較する (`"9.1"` < `"10.0"`)
pub fn compare_ts(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |ts: &str| {
        let (secs, frac) = ts.split_once('.').unwrap_or((ts, "0"));
        (
            secs.parse::<u64>().unwrap_or(0),
            format!("{frac:0<9}").parse::<u64>().unwrap_or(0),
        )
    };
    key(a).cmp(&key(b))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    /// テスト用のエクスポート ZIP を作る
    pub fn build_zip(prefix: &str, files: &[(&str, serde_json::Value)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default();
            for (name, value) in files {
                zip.start_file(format!("{prefix}{name}"), options).unwrap();
                zip.write_all(value.to_string().as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn sample(prefix: &str) -> Vec<u8> {
        build_zip(
            prefix,
            &[
                (
                    "users.json",
                    serde_json::json!([{"id":"U1","team_id":"T1","name":"alice"}]),
                ),
                (
                    "channels.json",
                    serde_json::json!([{"id":"C1","name":"general","members":["U1"]}]),
                ),
                (
                    "groups.json",
                    serde_json::json!([{"id":"G1","name":"secret","members":["U1"]}]),
                ),
                (
                    "general/2024-01-02.json",
                    serde_json::json!([
                        {"ts":"1704160000.000200","user":"U1","text":"second"},
                        {"ts":"1704160000.000100","user":"U1","text":"first"},
                        {"ts":"1704160001.000000","user":"U1","subtype":"channel_join","text":"joined"}
                    ]),
                ),
                (
                    "general/2024-01-01.json",
                    serde_json::json!([{"ts":"1704070000.000100","user":"U1","text":"hello"}]),
                ),
                (
                    "secret/2024-01-01.json",
                    serde_json::json!([{"ts":"1704070000.000100","user":"U1","text":"psst"}]),
                ),
            ],
        )
    }

    #[test]
    fn summarizes_public_and_private_channels() {
        for prefix in ["", "export/"] {
            let mut archive = ExportArchive::new(Cursor::new(sample(prefix))).unwrap();
            let (team, channels, users) = archive.summarize().unwrap();
            assert_eq!(team, "T1");
            assert_eq!(users, 1);
            assert_eq!(channels.len(), 2);
            assert_eq!(channels[0].message_count, 3);
            assert!(!channels[0].is_private);
            assert!(channels[1].is_private);
            assert_eq!(channels[1].message_count, 1);
        }
    }

    #[test]
    fn orders_day_files_and_messages() {
        let mut archive = ExportArchive::new(Cursor::new(sample(""))).unwrap();
        let channel = archive.channels().unwrap().remove(0);
        let files = archive.day_files(&channel);
        assert_eq!(
            files,
            vec!["general/2024-01-01.json", "general/2024-01-02.json"]
        );
        let texts: Vec<String> = archive
            .messages(&files[1])
            .unwrap()
            .into_iter()
            .map(|m| m.text)
            .collect();
        assert_eq!(texts, vec!["first", "second", "joined"]);
    }

    #[test]
    fn rejects_archives_without_channels() {
        let zip = build_zip("", &[("users.json", serde_json::json!([]))]);
        assert!(ExportArchive::new(Cursor::new(zip)).is_err());
    }

    #[test]
    fn compares_ts_numerically() {
        assert!(compare_ts("9.1", "10.0").is_lt());
        assert!(compare_ts("10.000002", "10.00001").is_lt());
    }

    #[test]
    fn edit_and_delete_history_records_are_not_importable() {
        let message =
            |value: serde_json::Value| -> SlackMessage { serde_json::from_value(value).unwrap() };
        // 編集の記録: 編集後の本文と投稿者を持つが、編集した日時の別の投稿として取り込まない
        let changed = message(serde_json::json!({
            "type": "message", "subtype": "message_changed", "ts": "1794015604.000000",
            "user": "U1", "text": "edited", "edited_by": "U1",
            "original": {"user": "U1", "type": "message", "ts": "1794015588.372869", "text": "before"}
        }));
        assert!(changed.is_history_event());
        assert!(!changed.is_importable());
        assert_eq!(changed.deleted_original_ts(), None);
        // 削除の記録: 削除された元の投稿の `ts` を持つ
        let deleted = message(serde_json::json!({
            "type": "message", "subtype": "message_deleted", "ts": "1756918202.000000",
            "user": "U1", "text": "", "deleted_by": "U1",
            "original": {"user": "U1", "type": "message", "ts": "1756918197.952539", "text": "gone"}
        }));
        assert!(deleted.is_history_event());
        assert!(!deleted.is_importable());
        assert_eq!(deleted.deleted_original_ts(), Some("1756918197.952539"));
        // `original` がない、`ts` が不正な記録は、何も指さない
        for original in [
            serde_json::json!(null),
            serde_json::json!({"ts": "17e9"}),
            serde_json::json!({}),
        ] {
            let record = message(serde_json::json!({
                "subtype": "message_deleted", "ts": "1756918202.000000", "user": "U1", "original": original
            }));
            assert_eq!(record.deleted_original_ts(), None);
        }
        // 公開されている形式: 元の投稿の `ts` はトップレベルの `original_ts` にあり、`original` はない
        let public = message(serde_json::json!({
            "type": "message", "subtype": "message_deleted", "ts": "1756918202.000000",
            "deleted_ts": "1756918202.000000", "hidden": true, "channel": "C1",
            "original_ts": "1756918197.952539",
            "previous": {"user": "U1", "type": "message", "ts": "1756918197.952539", "text": "gone"}
        }));
        assert_eq!(public.deleted_original_ts(), Some("1756918197.952539"));
        assert_eq!(public.original_message_ts(), Some("1756918197.952539"));
        let invalid = message(serde_json::json!({
            "subtype": "message_deleted", "ts": "1756918202.000000", "original_ts": "x"
        }));
        assert_eq!(invalid.deleted_original_ts(), None);
        // 不正な `original` は、何も指さない
        let broken = message(serde_json::json!({
            "subtype": "message_deleted", "ts": "1756918202.000000", "user": "U1", "original": "oops"
        }));
        assert_eq!(broken.deleted_original_ts(), None);
        // 普通の投稿と、これまでどおり取り込む subtype は変わらない
        for subtype in [None, Some("thread_broadcast"), Some("bot_message")] {
            let mut value =
                serde_json::json!({"ts": "1704070000.000100", "user": "U1", "text": "hello"});
            if let Some(subtype) = subtype {
                value["subtype"] = serde_json::json!(subtype);
            }
            let message = message(value);
            assert!(!message.is_history_event());
            assert!(message.is_importable(), "{subtype:?}");
        }
        // `tombstone` は、これまでどおり取り込まない
        let tombstone = message(
            serde_json::json!({"subtype": "tombstone", "ts": "1704070000.000100", "user": "USLACKBOT", "text": "This message was deleted."}),
        );
        assert!(!tombstone.is_importable());
    }

    #[test]
    fn extracts_slack_forwarded_and_msg_unfurl_refs_with_team_filter() {
        let message =
            |value: serde_json::Value| -> SlackMessage { serde_json::from_value(value).unwrap() };

        // コメントなし転送（text = ""）でも is_importable になり、channel_id + ts から参照を抽出する
        let direct = message(serde_json::json!({
            "ts": "1704070001.000100",
            "user": "U1",
            "text": "",
            "attachments": [{
                "is_share": true,
                "channel_id": "C111",
                "ts": "1704070000.000200",
                "from_url": "https://example.slack.com/archives/C111/p1704070000000200",
                "channel_team": "TMAIN"
            }]
        }));
        assert!(direct.has_forward_refs());
        assert!(direct.is_importable());
        assert_eq!(
            direct.forwarded_refs(Some("TMAIN")),
            vec![SlackForwardRef {
                slack_channel_id: "C111".into(),
                ts: "1704070000.000200".into(),
            }]
        );
        // 他ワークスペースの転送は team 指定で除外される
        assert!(direct.forwarded_refs(Some("TOTHER")).is_empty());

        // from_url のみ（スレッド返信 URL + query parameter）でも is_msg_unfurl から抽出できる
        let unfurl = message(serde_json::json!({
            "ts": "1704070002.000100",
            "user": "U1",
            "text": "check this reply",
            "attachments": [{
                "is_msg_unfurl": true,
                "from_url": "https://example.slack.com/archives/C222/p1704070000000300?thread_ts=1704070000.000100&cid=C222"
            }]
        }));
        assert_eq!(
            unfurl.forwarded_refs(None),
            vec![SlackForwardRef {
                slack_channel_id: "C222".into(),
                ts: "1704070000.000300".into(),
            }]
        );

        // 通常の URL 展開（from_url / is_app_unfurl）は転送として扱わない
        let web_unfurl = message(serde_json::json!({
            "ts": "1704070003.000100",
            "user": "U1",
            "text": "",
            "attachments": [{
                "is_app_unfurl": true,
                "from_url": "https://example.slack.com/archives/C222/p1704070000000300"
            }]
        }));
        assert!(!web_unfurl.has_forward_refs());
        assert!(!web_unfurl.is_importable());
        assert!(web_unfurl.forwarded_refs(None).is_empty());

        // 重複排除・上限 5 件
        let many = message(serde_json::json!({
            "ts": "1704070004.000100",
            "user": "U1",
            "text": "many",
            "attachments": [
                {"is_share": true, "channel_id": "C1", "ts": "1704070000.000001"},
                {"is_msg_unfurl": true, "channel_id": "C1", "ts": "1704070000.000001"},
                {"is_share": true, "channel_id": "C1", "ts": "1704070000.000002"},
                {"is_share": true, "channel_id": "C1", "ts": "1704070000.000003"},
                {"is_share": true, "channel_id": "C1", "ts": "1704070000.000004"},
                {"is_share": true, "channel_id": "C1", "ts": "1704070000.000005"},
                {"is_share": true, "channel_id": "C1", "ts": "1704070000.000006"}
            ]
        }));
        assert_eq!(many.forwarded_refs(None).len(), 5);

        struct NoopResolver;
        impl crate::slack::convert::Resolver for NoopResolver {
            fn user_name(&self, _: &str) -> Option<String> {
                None
            }
            fn channel_name(&self, _: &str) -> Option<String> {
                None
            }
        }
        // `from_url` の無い転送・引用添付でも本文 (`attachments_text`) やカード (`components`) へ転記しない
        let commentless_no_from_url = message(serde_json::json!({
            "ts": "1704070005.000100",
            "user": "U1",
            "text": "",
            "attachments": [{
                "is_msg_unfurl": true,
                "channel_id": "C111",
                "ts": "1704070000.000200",
                "channel_team": "TMAIN",
                "text": "secret quoted text"
            }]
        }));
        assert!(commentless_no_from_url.is_importable_for_team(Some("TMAIN")));
        // 他ワークスペースの転送のみの投稿は取り込み時に空投稿にしない
        assert!(!commentless_no_from_url.is_importable_for_team(Some("TOTHER")));
        assert_eq!(commentless_no_from_url.attachments_text(), "");
        assert!(commentless_no_from_url.components(&NoopResolver).is_none());
        assert_eq!(
            commentless_no_from_url.forwarded_refs(Some("TMAIN")),
            vec![SlackForwardRef {
                slack_channel_id: "C111".into(),
                ts: "1704070000.000200".into(),
            }]
        );

        // `channel_id` + `ts` も `from_url` も無く参照を抽出できない転送添付は、引用テキストを欠落させないよう従来どおり本文・カードを残す
        let unresolvable_share = message(serde_json::json!({
            "ts": "1704070006.000100",
            "user": "U1",
            "text": "",
            "attachments": [{
                "is_share": true,
                "text": "legacy fallback quote"
            }]
        }));
        assert!(unresolvable_share.is_importable_for_team(Some("TMAIN")));
        assert_eq!(
            unresolvable_share.attachments_text(),
            "legacy fallback quote"
        );
        assert!(unresolvable_share.components(&NoopResolver).is_some());

        // Slack の短い ts（小数部 1 桁）とパーマリンク表記（6 桁）は同一の正規化 ts に揃い重複排除される
        let short_ts_and_permalink = message(serde_json::json!({
            "ts": "1704070007.000100",
            "user": "U1",
            "text": "",
            "attachments": [
                {
                    "is_share": true,
                    "channel_id": "C111",
                    "ts": "1704070000.1",
                    "channel_team": "TMAIN"
                },
                {
                    "is_msg_unfurl": true,
                    "from_url": "https://example.slack.com/archives/C111/p1704070000100000",
                    "channel_team": "TMAIN"
                }
            ]
        }));
        assert_eq!(
            short_ts_and_permalink.forwarded_refs(Some("TMAIN")),
            vec![SlackForwardRef {
                slack_channel_id: "C111".into(),
                ts: "1704070000.100000".into(),
            }]
        );

        // 親が他ワークスペース転送のみで取り込み対象外でも、スレッド探索対象には含める
        assert!(!commentless_no_from_url.is_importable_for_team(Some("TOTHER")));
        assert!(commentless_no_from_url.is_thread_root_candidate());
    }
}
