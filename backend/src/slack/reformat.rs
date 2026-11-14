//! 取り込み済みの投稿に、Slack の書式 (`*太字*` など) の変換を当てる 1 回限りの修正。
//!
//! 書式の変換より前に取り込んだ投稿の本文は、記号が変換されないまま保存されている。再取り込みは既存の投稿の
//! 本文を更新しないので、保存済みの本文に [`reformat_stored`] を当てる。
//!
//! - **対象**: Slack から取り込んだ投稿だけ。投稿の ID は Slack のチームとチャンネルと `ts` から決まる
//!   (`slack::message_id`) ので、`created_at` から `ts` の候補 (ミリ秒に丸めた分の 1,000 通り) を作って
//!   SQL で ID を再計算し、一致した行を選ぶ。連携済みのユーザーが Zoo で書いた投稿は、同じ作者でも対象にならない。
//!   `slack_message_map` に Slack の投稿として載っている行も対象にする。
//! - **対象外**: 暗号化チャンネル、削除済み、カード付き、編集済み (`revision > 0`)、すでに書式を変換した本文。
//! - **書き込み**: 読んだ本文と一致する行だけ更新する (読んでから書くまでに変わった行は飛ばす)。
//!   `updated_at` と `revision` は変えず、App へのイベントは止める。
//! - **冪等**: 変換した本文は `reformat_stored` が飛ばすので、もう一度実行しても何も変わらない。

use serde_json::Value;

use super::{convert::reformat_stored, worker::suppress_app_events};
use crate::{apps::query, error::AppError};

/// 1 つのトランザクションで更新する行数
const BATCH: usize = 500;

#[derive(Debug, Clone)]
pub struct Options {
    /// `false` なら何も書かず、件数と例だけを返す
    pub apply: bool,
    /// 報告に入れる、変更前後の例の数
    pub samples: usize,
    /// 指定すると、この Zoo のチャンネルだけを対象にする (少しずつ進めたいとき)
    pub channel: Option<String>,
}

#[derive(Debug, Default)]
pub struct Report {
    pub channels: usize,
    /// 取り込んだ投稿で、記号を含み、対象外の条件に当たらなかった行
    pub candidates: usize,
    /// 変換して本文が変わる行
    pub changed: usize,
    /// 書き込んだ行 (`apply` のときだけ)
    pub applied: usize,
    /// 読んでから書くまでに変わっていて、飛ばした行
    pub raced: usize,
    pub samples: Vec<Change>,
}

#[derive(Debug, Clone)]
pub struct Change {
    pub id: String,
    pub old: String,
    pub new: String,
}

/// Slack の `ts` の候補から ID を再計算して、取り込んだ投稿の (id, 本文) を返す SQL。
/// `$1` Zoo のチャンネル ID、`$2` Slack のチーム ID、`$3` Slack のチャンネル ID。
const CANDIDATES: &str = "\
WITH msgs AS (\
  SELECT m.id, m.content, round(extract(epoch FROM m.created_at)*1000)::bigint AS ms \
  FROM messages m \
  WHERE m.channel_id=$1 AND m.deleted_at IS NULL AND m.card IS NULL AND m.revision=0 \
    AND m.content ~ '[*~_<\\[\\\\$]' AND position(chr(8203) in m.content)=0\
), \
stable AS (\
  SELECT msgs.id FROM msgs, generate_series(0,999) g \
  WHERE encode(substring(sha256(convert_to('slack-message:'||$2||':'||$3||':'||\
    ((msgs.ms*1000-500+g)/1000000)::text||'.'||lpad(((msgs.ms*1000-500+g)%1000000)::text,6,'0'),'UTF8')) \
    from 1 for 16),'hex')::uuid::text=msgs.id\
), \
mapped AS (\
  SELECT msgs.id FROM msgs JOIN slack_message_map s ON s.message_id=msgs.id AND s.origin='slack'\
) \
SELECT json_build_object('id',msgs.id,'content',msgs.content)::text \
FROM msgs WHERE msgs.id IN (SELECT id FROM stable UNION SELECT id FROM mapped) ORDER BY msgs.id";

/// 取り込んだ投稿の本文に書式の変換を当てる。書き換える行ごとに `on_change` を呼ぶ (書き込む前に呼ぶ)。
pub async fn run(
    db: &toasty::Db,
    options: &Options,
    mut on_change: impl FnMut(&Change),
) -> Result<Report, AppError> {
    let mut report = Report::default();
    let channels = {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        let rows = query(
            &mut tx,
            "SELECT json_build_object('team',sc.team_id,'slack',sc.slack_channel_id,'channel',sc.channel_id)::text \
             FROM slack_channels sc JOIN channels c ON c.id=sc.channel_id \
             WHERE NOT c.encrypted AND ($1='' OR sc.channel_id=$1) ORDER BY sc.channel_id",
            &[options.channel.as_deref().unwrap_or("")],
        )
        .await?;
        tx.commit().await?;
        rows
    };
    for channel in channels {
        let (Some(team), Some(slack), Some(zoo)) = (
            channel["team"].as_str(),
            channel["slack"].as_str(),
            channel["channel"].as_str(),
        ) else {
            continue;
        };
        report.channels += 1;
        let rows: Vec<Value> = {
            let mut db = db.clone();
            let mut tx = db.transaction().await?;
            let rows = query(&mut tx, CANDIDATES, &[zoo, team, slack]).await?;
            tx.commit().await?;
            rows
        };
        report.candidates += rows.len();
        let changes: Vec<Change> = rows
            .iter()
            .filter_map(|row| {
                let id = row["id"].as_str()?;
                let old = row["content"].as_str()?;
                let new = reformat_stored(old)?;
                Some(Change {
                    id: id.to_string(),
                    old: old.to_string(),
                    new,
                })
            })
            .collect();
        report.changed += changes.len();
        for change in &changes {
            if report.samples.len() < options.samples {
                report.samples.push(change.clone());
            }
        }
        if !options.apply {
            continue;
        }
        for batch in changes.chunks(BATCH) {
            let mut db = db.clone();
            let mut tx = db.transaction().await?;
            suppress_app_events(&mut tx).await?;
            for change in batch {
                on_change(change);
                let updated = query(
                    &mut tx,
                    "UPDATE messages SET content=$2 \
                     WHERE id=$1 AND content=$3 AND revision=0 AND card IS NULL AND deleted_at IS NULL \
                     RETURNING json_build_object('ok',true)::text",
                    &[&change.id, &change.new, &change.old],
                )
                .await?;
                if updated.is_empty() {
                    report.raced += 1;
                } else {
                    report.applied += 1;
                }
            }
            tx.commit().await?;
        }
    }
    Ok(report)
}

/// `run` が `on_change` に渡した変更を元に戻す。新しい本文と一致する行だけ戻す。戻した行数を返す。
pub async fn restore(db: &toasty::Db, changes: &[Change]) -> Result<usize, AppError> {
    let mut restored = 0;
    for batch in changes.chunks(BATCH) {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        for change in batch {
            restored += query(
                &mut tx,
                "UPDATE messages SET content=$2 WHERE id=$1 AND content=$3 \
                 RETURNING json_build_object('ok',true)::text",
                &[&change.id, &change.old, &change.new],
            )
            .await?
            .len();
        }
        tx.commit().await?;
    }
    Ok(restored)
}
