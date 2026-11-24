//! 転送元解決（`RESOLVE_FORWARDS_SQL`）と送信時権限検証クエリ（`VERIFY_FORWARD_SOURCES_SQL`）が
//! 全履歴を走査せずインデックスのみで O(K log N) に閉じることを `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` で検証する。

use serde_json::{Value, json};
use tokio_postgres::{Client, NoTls, SimpleQueryMessage};

use backend::routes::forwards::{RESOLVE_FORWARDS_SQL, VERIFY_FORWARD_SOURCES_SQL};

fn quote_sql(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn bind_sql(sql: &str, params: &[String]) -> String {
    let mut out = sql.to_string();
    // $10 以上が先に置換されないよう降順で差し替える
    for i in (1..=params.len()).rev() {
        let placeholder = format!("${i}");
        let replacement = quote_sql(&params[i - 1]);
        out = out.replace(&placeholder, &replacement);
    }
    out
}

async fn pg_connect(url: &str) -> anyhow::Result<Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(async move {
        if let Err(err) = connection.await {
            eprintln!("postgres connection error: {err}");
        }
    });
    Ok(client)
}

async fn explain_json(client: &Client, query: &str) -> anyhow::Result<Value> {
    let explain = format!("EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {query}");
    let messages = client.simple_query(&explain).await?;
    let mut body = String::new();
    for msg in messages {
        if let SimpleQueryMessage::Row(row) = msg
            && let Some(line) = row.get(0)
        {
            body.push_str(line);
        }
    }
    let parsed: Vec<Value> = serde_json::from_str(&body)?;
    Ok(parsed
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("empty EXPLAIN JSON result"))?)
}

fn collect_seq_scans(node: &Value, hits: &mut Vec<String>) {
    let node_type = node["Node Type"].as_str().unwrap_or("");
    let relation = node["Relation Name"].as_str().unwrap_or("");
    if node_type == "Seq Scan"
        && matches!(
            relation,
            "messages" | "channel_user" | "channels" | "users" | "attachments"
        )
    {
        hits.push(format!("{node_type} on {relation}"));
    }
    if let Some(plans) = node["Plans"].as_array() {
        for child in plans {
            collect_seq_scans(child, hits);
        }
    }
}

fn collect_indexes(node: &Value, out: &mut Vec<(String, i64)>) {
    if let Some(idx) = node["Index Name"].as_str() {
        let loops = node["Actual Loops"].as_i64().unwrap_or(0);
        out.push((idx.to_string(), loops));
    }
    if let Some(plans) = node["Plans"].as_array() {
        for child in plans {
            collect_indexes(child, out);
        }
    }
}

fn sum_shared_blocks(node: &Value) -> (i64, i64) {
    let hit = node["Shared Hit Blocks"].as_i64().unwrap_or(0);
    let read = node["Shared Read Blocks"].as_i64().unwrap_or(0);
    (hit, read)
}

#[tokio::test]
async fn forward_queries_use_index_scans_without_seq_scan() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let url = std::env::var("DATABASE_URL")?;
    let client = pg_connect(&url).await?;

    // 独立した固定プレフィックスで複数チャンネル・DM・添付・メッセージを一括投入する
    let viewer = "11111111-1111-1111-1111-000000000001";
    client
        .batch_execute(
            r#"
DELETE FROM notification_events WHERE id LIKE '11111111-%';
DELETE FROM attachments WHERE id LIKE '11111111-%';
DELETE FROM channel_user WHERE channel_id LIKE '11111111-%';
DELETE FROM messages WHERE id LIKE '11111111-%';
DELETE FROM channels WHERE id LIKE '11111111-%';
DELETE FROM users WHERE id LIKE '11111111-%';

SET session_replication_role = replica;
INSERT INTO users(id, user_name, display_name)
SELECT '11111111-1111-1111-1111-' || lpad(i::text, 12, '0'),
       'fqb_u_' || i,
       'FQB User ' || i
FROM generate_series(1, 1500) i;

-- 1..500: public, 501..1000: private, 1001..1500: dm, 1501..2000: encrypted
INSERT INTO channels(id, channel_name, created_user_id, is_private, is_dm, encrypted)
SELECT '11111111-1111-2222-1111-' || lpad(i::text, 12, '0'),
       CASE WHEN i BETWEEN 1001 AND 1500
            THEN 'dm:' || '11111111-1111-1111-1111-000000000001:' || '11111111-1111-1111-1111-' || lpad((1 + (i % 1400))::text, 12, '0')
            ELSE 'fqb-ch-' || i END,
       '11111111-1111-1111-1111-000000000001',
       i > 500,
       i BETWEEN 1001 AND 1500,
       i > 1500
FROM generate_series(1, 2000) i;

-- viewer を一部の非公開・DM チャンネルに所属させ、他のメンバーも各チャンネルへ配置する
INSERT INTO channel_user(channel_id, user_id, joined_at)
SELECT '11111111-1111-2222-1111-' || lpad(c::text, 12, '0'),
       '11111111-1111-1111-1111-' || lpad(u::text, 12, '0'),
       LOCALTIMESTAMP
FROM generate_series(1, 1600) c
CROSS JOIN LATERAL (SELECT 1 AS u UNION ALL SELECT 2 + (c % 200)) m;

-- ch_idx = 1 + ((i - 1) % 2000) のため ch(i) は i とそのまま対応する（約 40,000 行で Message_pkey 索引選択を検証）
INSERT INTO messages(id, user_id, channel_id, content, created_at, deleted_at, forwarded_message_ids)
SELECT '11111111-1111-3333-1111-' || lpad(i::text, 12, '0'),
       '11111111-1111-1111-1111-' || lpad((1 + (i % 1500))::text, 12, '0'),
       '11111111-1111-2222-1111-' || lpad((1 + ((i - 1) % 2000))::text, 12, '0'),
       CASE WHEN i % 10 = 0 THEN '' ELSE 'fqb payload ' || i END,
       LOCALTIMESTAMP - (i || ' seconds')::interval,
       CASE WHEN i % 25 = 0 THEN LOCALTIMESTAMP ELSE NULL END,
       CASE WHEN i % 8 = 0 THEN jsonb_build_array('11111111-1111-3333-1111-000000000001') ELSE NULL END
FROM generate_series(1, 40000) i;

INSERT INTO attachments(id, message_id, uploader_id, channel_id, file_name, file_size, content_type, storage_key)
SELECT '11111111-1111-4444-1111-' || lpad(i::text, 12, '0'),
       '11111111-1111-3333-1111-' || lpad(i::text, 12, '0'),
       '11111111-1111-1111-1111-000000000001',
       '11111111-1111-2222-1111-' || lpad((1 + ((i - 1) % 2000))::text, 12, '0'),
       'doc_' || i || '.pdf',
       512,
       'application/pdf',
       'fqb/' || i
FROM generate_series(1, 2000) i;
SET session_replication_role = DEFAULT;
"#,
        )
        .await?;
    client.batch_execute("VACUUM ANALYZE").await?;

    // 未削除の公開(attachments 付)・非公開・DM(dm_participants 用)・削除済み・暗号化・未存在を含む 50 件の ID
    let mut mixed_ids: Vec<String> = Vec::with_capacity(50);
    // アクティブ公開 + attachments フォールバック (i=1..=15 かつ 25 の倍数を除く)
    mixed_ids.extend(
        (1..=16)
            .filter(|i| i % 25 != 0)
            .take(15)
            .map(|i| format!("11111111-1111-3333-1111-{i:012}")),
    );
    // アクティブ非公開メンバー (501..=515)
    mixed_ids.extend(
        (501..=516)
            .filter(|i| i % 25 != 0)
            .take(15)
            .map(|i| format!("11111111-1111-3333-1111-{i:012}")),
    );
    // アクティブ DM メンバー + attachments 付 (1001..=1012, SubPlan 1 idx_channel_user_channel_id が実際に回る)
    mixed_ids.extend(
        (1001..=1015)
            .filter(|i| i % 25 != 0)
            .take(10)
            .map(|i| format!("11111111-1111-3333-1111-{i:012}")),
    );
    // 削除済み (25, 50, 525)・暗号化 (1501, 1502)・未存在 (5件)
    mixed_ids.extend(
        [25usize, 50, 525, 1501, 1502]
            .into_iter()
            .map(|i| format!("11111111-1111-3333-1111-{i:012}")),
    );
    mixed_ids.extend((1..=5).map(|i| format!("11111111-9999-9999-9999-{i:012}")));
    assert_eq!(mixed_ids.len(), 50);

    let resolve_sql = bind_sql(
        RESOLVE_FORWARDS_SQL,
        &[json!(mixed_ids).to_string(), viewer.to_string()],
    );
    let resolve_plan = explain_json(&client, &resolve_sql).await?;

    let mut resolve_seq_scans = Vec::new();
    collect_seq_scans(&resolve_plan["Plan"], &mut resolve_seq_scans);
    let mut resolve_indexes = Vec::new();
    collect_indexes(&resolve_plan["Plan"], &mut resolve_indexes);
    let (res_hit, res_read) = sum_shared_blocks(&resolve_plan["Plan"]);
    let res_time = resolve_plan["Execution Time"].as_f64().unwrap_or(-1.0);
    println!(
        "RESOLVE_FORWARDS_SQL execution_ms={res_time:.3} shared_hit={res_hit} shared_read={res_read} indexes={resolve_indexes:?}"
    );
    assert!(
        resolve_seq_scans.is_empty(),
        "RESOLVE_FORWARDS_SQL fell back to Seq Scan: {resolve_seq_scans:?}\nplan={resolve_plan:#}"
    );
    assert!(
        resolve_indexes
            .iter()
            .any(|(idx, loops)| idx == "idx_channel_user_channel_id" && *loops > 0),
        "expected executed idx_channel_user_channel_id in resolve plan: {resolve_indexes:?}"
    );
    assert!(
        resolve_indexes
            .iter()
            .any(|(idx, loops)| idx == "ChannelUser_pkey" && *loops > 0),
        "expected executed ChannelUser_pkey in resolve plan: {resolve_indexes:?}"
    );

    // VERIFY_FORWARD_SOURCES_SQL（1 件解決: DM メンバーの投稿で channel_user_pkey も踏ませる）
    let single_id = json!(["11111111-1111-3333-1111-000000001001"]).to_string();
    let verify_sql = bind_sql(
        VERIFY_FORWARD_SOURCES_SQL,
        &[
            single_id,
            viewer.to_string(),
            "false".into(),
            "false".into(),
        ],
    );
    let verify_plan = explain_json(&client, &verify_sql).await?;

    let mut verify_seq_scans = Vec::new();
    collect_seq_scans(&verify_plan["Plan"], &mut verify_seq_scans);
    let mut verify_indexes = Vec::new();
    collect_indexes(&verify_plan["Plan"], &mut verify_indexes);
    let (ver_hit, ver_read) = sum_shared_blocks(&verify_plan["Plan"]);
    let ver_time = verify_plan["Execution Time"].as_f64().unwrap_or(-1.0);
    println!(
        "VERIFY_FORWARD_SOURCES_SQL execution_ms={ver_time:.3} shared_hit={ver_hit} shared_read={ver_read} indexes={verify_indexes:?}"
    );
    assert!(
        verify_seq_scans.is_empty(),
        "VERIFY_FORWARD_SOURCES_SQL fell back to Seq Scan: {verify_seq_scans:?}\nplan={verify_plan:#}"
    );
    assert!(
        verify_indexes
            .iter()
            .any(|(idx, loops)| idx == "ChannelUser_pkey" && *loops > 0),
        "expected executed ChannelUser_pkey in verify plan: {verify_indexes:?}"
    );

    // クリーンアップ（共有 DB へテスト行を残さない）
    client
        .batch_execute(
            r#"
SET session_replication_role = replica;
DELETE FROM notification_events WHERE id LIKE '11111111-%';
DELETE FROM attachments WHERE id LIKE '11111111-%';
DELETE FROM channel_user WHERE channel_id LIKE '11111111-%';
DELETE FROM messages WHERE id LIKE '11111111-%';
DELETE FROM channels WHERE id LIKE '11111111-%';
DELETE FROM users WHERE id LIKE '11111111-%';
SET session_replication_role = DEFAULT;
"#,
        )
        .await?;

    Ok(())
}
