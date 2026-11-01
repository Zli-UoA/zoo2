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
    /// スレッドの親なら、返信の ts の一覧 (エクスポートに含まれる場合)
    #[serde(default)]
    pub replies: Vec<SlackReplyRef>,
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

/// チャンネル参加・トピック変更などのシステムメッセージは取り込まない
const SKIPPED_SUBTYPES: &[&str] = &[
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
];

impl SlackMessage {
    /// 投稿者の Slack ID (ユーザーまたは bot)
    pub fn author(&self) -> Option<&str> {
        self.user
            .as_deref()
            .filter(|u| !u.is_empty())
            .or_else(|| self.bot_id.as_deref().filter(|b| !b.is_empty()))
    }

    pub fn is_importable(&self) -> bool {
        crate::slack::is_valid_ts(&self.ts)
            && self.author().is_some()
            && !self
                .subtype
                .as_deref()
                .is_some_and(|s| SKIPPED_SUBTYPES.contains(&s))
            && (!self.text.trim().is_empty()
                || !self.files.is_empty()
                || !self.attachments_text().is_empty())
    }

    /// リンクの展開 (unfurl) を除いた attachments
    fn own_attachments(&self) -> impl Iterator<Item = &serde_json::Value> {
        self.attachments
            .iter()
            .filter(|a| a.get("from_url").is_none() && a.get("is_app_unfurl").is_none())
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
                let main = if main.is_empty() { text("fallback").to_string() } else { main };
                [text("title").to_string(), main]
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// attachments を zoo のカードにする。リンクの展開 (unfurl) は zoo 側で表示するので除き、
    /// 読み取れないものは 1 件ずつ捨てる。
    pub fn cards(&self) -> Vec<crate::cards::MessageCard> {
        self.attachments
            .iter()
            .filter(|a| a.get("from_url").is_none() && a.get("is_app_unfurl").is_none())
            .map(sanitize_attachment)
            .filter_map(|a| {
                let body = serde_json::json!({ "attachments": [a] }).to_string();
                crate::cards::parse_webhook_cards(body.as_bytes()).ok()
            })
            .flatten()
            .take(100)
            .collect()
    }

    /// スレッドの返信なら親の ts
    pub fn parent_ts(&self) -> Option<&str> {
        self.thread_ts
            .as_deref()
            .filter(|t| *t != self.ts && crate::slack::is_valid_ts(t))
    }
}

fn clip(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

/// Webhook と同じカードの検証 (`MessageCard::encode`) に通る形へ直す。
/// 取り込みでは元のデータを直せないので、拒否せずにリンクを外したり切り詰めたりする。
fn sanitize_attachment(attachment: &serde_json::Value) -> serde_json::Value {
    let mut value = attachment.clone();
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    let text = |object: &serde_json::Map<String, serde_json::Value>, key: &str| {
        object.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string()
    };
    if !text(object, "title_link").starts_with("https://") {
        object.remove("title_link");
    }
    if !text(object, "color").is_empty()
        && crate::cards::normalize_color(&text(object, "color")).is_none()
    {
        object.remove("color");
    }
    for (key, limit) in [("title", 500), ("footer", 2000), ("author_name", 2000), ("fallback", 3000)] {
        let clipped = clip(&text(object, key), limit);
        if object.contains_key(key) {
            object.insert(key.into(), clipped.into());
        }
    }
    // pretext・text・fields を連結した本文が 4000 文字に収まるよう、fields → pretext → text の順に枠を取る
    let fields: Vec<serde_json::Value> = object
        .get("fields")
        .and_then(|v| v.as_array())
        .map(|fields| {
            fields
                .iter()
                .take(10)
                .map(|f| {
                    let get = |k: &str| f.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    serde_json::json!({ "title": clip(get("title"), 50), "value": clip(get("value"), 200) })
                })
                .collect()
        })
        .unwrap_or_default();
    let used: usize = fields
        .iter()
        .map(|f| {
            f["title"].as_str().unwrap_or("").chars().count()
                + f["value"].as_str().unwrap_or("").chars().count()
                + 3
        })
        .sum();
    let pretext = clip(&text(object, "pretext"), 500);
    let budget = 3900usize.saturating_sub(used + pretext.chars().count());
    let body = clip(&text(object, "text"), budget);
    object.insert("fields".into(), fields.into());
    object.insert("pretext".into(), pretext.into());
    object.insert("text".into(), body.into());
    value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSummary {
    pub slack_channel_id: String,
    pub name: String,
    pub is_private: bool,
    pub is_archived: bool,
    pub members: usize,
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
        let names: Vec<String> = zip.file_names().map(str::to_string).collect();
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
                    .filter(|m| m.is_importable())
                    .count() as u64;
            }
            summaries.push(ChannelSummary {
                slack_channel_id: channel.id.clone(),
                name: channel.name.clone(),
                is_private: channel.is_private,
                is_archived: channel.is_archived,
                members: channel.members.len(),
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
}
