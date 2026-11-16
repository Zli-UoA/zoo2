//! 旧形式（`messages.card`・`messages.app_blocks`・`app_link_unfurls.card`・`app_ui.document` の blocks）を
//! 部品の木に移す 1 回限りの処理。`zoo_components_backfill` から使う。
//!
//! - 既定は dry-run（何も書かない）。`apply` のときだけ書き、変更前後を `on_change` に渡す（バックアップ用）。
//! - 500 件ずつ、主キーの順に進める。各バッチは 1 つのトランザクションで、App のイベントは出さない。
//! - 暗号化チャンネルの行は変換しない（E2EE のトリガーが拒否する。凍結した平文も戻さない）。
//! - 旧列はそのまま残す（マイグレーション B まで、古いクライアントが読む）。
//!
//! 探索の上限：各バッチは `id > 前回の最後` から主キーの索引で進み、LIMIT 500 で止まる。全体では対象の表を
//! 1 回だけ走査する（1 回限りの処理なので、表の行数に比例してよい）。

use crate::apps::query;
use crate::error::AppError;
use serde_json::{Value, json};
use zoo_components::{Document, legacy};

pub const BATCH: usize = 500;

#[derive(Debug, Clone)]
pub struct Change {
    pub table: String,
    pub id: String,
    /// 変更前の値（`messages`・`app_link_unfurls` は null、`app_ui` は元の document）
    pub old: Value,
    pub new: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub scanned: usize,
    pub changed: usize,
    pub applied: usize,
}

/// `messages.card` と `messages.app_blocks` から文書を作る。両方あれば card が先。
pub fn message_document(card: Option<&Value>, blocks: Option<&Value>) -> Option<Document> {
    let mut doc = Document::default();
    if let Some(card) = card.filter(|c| c.is_object()) {
        let d = legacy::cards(card);
        doc.flags |= d.flags;
        doc.components.extend(d.components);
    }
    if let Some(blocks) = blocks.filter(|b| b.is_array()) {
        let d = legacy::blocks(blocks, false);
        doc.flags |= d.flags;
        doc.components.extend(d.components);
    }
    (!doc.components.is_empty()).then_some(doc)
}

async fn suppress(tx: &mut toasty::Transaction<'_>) -> Result<(), AppError> {
    toasty::sql::statement("SELECT set_config('zoo.suppress_app_events','on',true)")
        .exec(tx)
        .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
/// 1 つの表を進める。`select` は `$1`（前回の最後の id）を受け取り、`id` と `old` と変換元を返す。
async fn each_batch(
    db: &toasty::Db,
    select: &str,
    update: &str,
    table: &str,
    apply: bool,
    report: &mut Report,
    convert: impl Fn(&Value) -> Option<String>,
    on_change: &mut impl FnMut(&Change) -> std::io::Result<()>,
) -> Result<(), AppError> {
    let mut last = String::new();
    loop {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        suppress(&mut tx).await?;
        let rows = query(&mut tx, select, &[last.as_str()]).await?;
        if rows.is_empty() {
            break;
        }
        report.scanned += rows.len();
        for row in &rows {
            let id = row["id"].as_str().unwrap_or_default().to_string();
            last = id.clone();
            let Some(new) = convert(row) else { continue };
            report.changed += 1;
            let change = Change {
                table: table.into(),
                id: id.clone(),
                old: row["old"].clone(),
                new: new.clone(),
            };
            if !apply {
                continue;
            }
            // バックアップを書けないまま更新すると戻せなくなる。失敗したら、このバッチをコミットせずに止める
            on_change(&change).map_err(|e| {
                AppError::Internal(anyhow::anyhow!("could not write the backup: {e}"))
            })?;
            report.applied += query(&mut tx, update, &[id.as_str(), new.as_str()])
                .await?
                .len();
        }
        if apply {
            tx.commit().await?;
        }
        if rows.len() < BATCH {
            break;
        }
    }
    Ok(())
}

/// 文字列の列を JSON として読む。読めなければ null（変換しない）。
fn parse(v: &Value) -> Value {
    v.as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}

pub async fn run(
    db: &toasty::Db,
    apply: bool,
    mut on_change: impl FnMut(&Change) -> std::io::Result<()>,
) -> Result<Report, AppError> {
    let mut report = Report::default();
    each_batch(
        db,
        "SELECT json_build_object('id',m.id,'old',NULL,'card',m.card,'blocks',m.app_blocks)::text FROM messages m \
         JOIN channels c ON c.id=m.channel_id \
         WHERE m.id>$1 AND m.components IS NULL AND (m.card IS NOT NULL OR m.app_blocks IS NOT NULL) \
         AND m.deleted_at IS NULL AND NOT c.encrypted ORDER BY m.id LIMIT 500",
        "UPDATE messages SET components=$2 WHERE id=$1 AND components IS NULL RETURNING json_build_object('ok',true)::text",
        "messages",
        apply,
        &mut report,
        |row| message_document(Some(&parse(&row["card"])), Some(&parse(&row["blocks"]))).map(|d| d.to_json()),
        &mut on_change,
    )
    .await?;
    each_batch(
        db,
        "SELECT json_build_object('id',u.message_id||' '||u.installation_id||' '||u.url,'old',NULL,'card',u.card)::text FROM app_link_unfurls u \
         WHERE u.message_id||' '||u.installation_id||' '||u.url>$1 AND u.components IS NULL AND u.card IS NOT NULL \
         ORDER BY u.message_id||' '||u.installation_id||' '||u.url LIMIT 500",
        "UPDATE app_link_unfurls SET components=$2 WHERE message_id||' '||installation_id||' '||url=$1 AND components IS NULL RETURNING json_build_object('ok',true)::text",
        "app_link_unfurls",
        apply,
        &mut report,
        |row| {
            let card = parse(&row["card"]);
            if !card.is_object() {
                return None;
            }
            let mut d = legacy::cards(&card);
            d.flags = 0;
            (!d.components.is_empty()).then(|| d.to_json())
        },
        &mut on_change,
    )
    .await?;
    // 展開の一覧（messages.app_unfurls）を、変換した app_link_unfurls から作り直す
    each_batch(
        db,
        "SELECT json_build_object('id',m.id,'old',m.app_unfurls::jsonb)::text FROM messages m JOIN channels c ON c.id=m.channel_id \
         WHERE m.id>$1 AND m.app_unfurls IS JSON ARRAY AND m.deleted_at IS NULL AND NOT c.encrypted \
         AND EXISTS(SELECT 1 FROM jsonb_array_elements(m.app_unfurls::jsonb) e WHERE NOT e ? 'components') ORDER BY m.id LIMIT 500",
        "UPDATE messages SET app_unfurls=(SELECT COALESCE(jsonb_agg(jsonb_strip_nulls(jsonb_build_object('app_id',i.app_id,'name',a.name,'url',u.url,'card',u.card::jsonb,'components',u.components::jsonb))),'[]'::jsonb)::text FROM app_link_unfurls u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE u.message_id=$1) WHERE id=$1 AND $2<>'' RETURNING json_build_object('ok',true)::text",
        "messages.app_unfurls",
        apply,
        &mut report,
        |_| Some("rebuild".into()),
        &mut on_change,
    )
    .await?;
    each_batch(
        db,
        "SELECT json_build_object('id',id,'old',document::jsonb,'kind',kind)::text FROM app_ui \
         WHERE id>$1 AND kind IN('home','modal','ephemeral') AND document IS JSON OBJECT AND document::jsonb ? 'blocks' ORDER BY id LIMIT 500",
        "UPDATE app_ui SET document=$2 WHERE id=$1 AND document::jsonb ? 'blocks' RETURNING json_build_object('ok',true)::text",
        "app_ui",
        apply,
        &mut report,
        |row| Some(legacy::blocks(&row["old"]["blocks"], row["kind"] == "modal").to_json()),
        &mut on_change,
    )
    .await?;
    Ok(report)
}

/// `run` が書いた変更を元に戻す。戻した件数を返す。
pub async fn restore(db: &toasty::Db, changes: &[Change]) -> Result<usize, AppError> {
    let mut restored = 0;
    for batch in changes.chunks(BATCH) {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        suppress(&mut tx).await?;
        for c in batch {
            let (sql, old) = match c.table.as_str() {
                "messages" => (
                    "UPDATE messages SET components=NULL WHERE id=$1 AND components=$2 AND $3='' RETURNING '{}'::text",
                    String::new(),
                ),
                "app_link_unfurls" => (
                    "UPDATE app_link_unfurls SET components=NULL WHERE message_id||' '||installation_id||' '||url=$1 AND components=$2 AND $3='' RETURNING '{}'::text",
                    String::new(),
                ),
                "messages.app_unfurls" => (
                    "UPDATE messages SET app_unfurls=$3 WHERE id=$1 AND $2<>'' RETURNING '{}'::text",
                    c.old.to_string(),
                ),
                "app_ui" => (
                    "UPDATE app_ui SET document=$3 WHERE id=$1 AND document=$2 RETURNING '{}'::text",
                    c.old.to_string(),
                ),
                _ => continue,
            };
            restored += query(&mut tx, sql, &[c.id.as_str(), c.new.as_str(), old.as_str()])
                .await?
                .len();
        }
        tx.commit().await?;
    }
    Ok(restored)
}

pub fn to_line(c: &Change) -> String {
    json!({"table": c.table, "id": c.id, "old": c.old, "new": c.new}).to_string()
}

pub fn from_line(line: &str) -> anyhow::Result<Change> {
    let v: Value = serde_json::from_str(line)?;
    let s = |k: &str| {
        v[k].as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("missing {k}"))
    };
    Ok(Change {
        table: s("table")?,
        id: s("id")?,
        old: v["old"].clone(),
        new: s("new")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zoo_components::IS_COMPONENTS_V2;

    #[test]
    fn card_and_blocks_merge_in_order() {
        let d = message_document(
            Some(&json!({"title":"Deploy","color":"good"})),
            Some(&json!([{"type":"button","action_id":"a","label":"Retry"}])),
        )
        .unwrap();
        assert_eq!(d.flags, IS_COMPONENTS_V2);
        assert_eq!(d.components.len(), 2);
        assert!(message_document(Some(&Value::Null), Some(&Value::Null)).is_none());
    }
}
