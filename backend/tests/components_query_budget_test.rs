//! メッセージ部品の SQL の探索量（docs/query-design.md）。使い捨てのマイグレーション済み DB でだけ動かす:
//! `COMPONENTS_SCALE_ROWS=300000`（または 1000000）と `DATABASE_URL`。結果は `COMPONENTS_SCALE_OUTPUT` に JSON で書く。
//!
//! 各 SQL は、ソースにある文字列と同じもの（空白を除いて比べる）を PREPARE し、`EXPLAIN (ANALYZE, BUFFERS)` を
//! BEGIN〜ROLLBACK の中で単独 30 回・8 接続 30 回ずつ測る。トリガーと外部キーの中の計画は auto_explain で拾う。
//! 関係ごとの走査行（Actual Rows＋除外行）×loops に、入力から決めた上限を掛ける。
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio_postgres::{NoTls, SimpleQueryMessage};

/// 処理量の変数。N（各表の履歴）は COMPONENTS_SCALE_ROWS。ほかは規模によらず固定する
const U: usize = 5; // 1 メッセージの展開の数
const F: usize = 10; // file upload の 1 回の送信で送るファイルの数（max_values の上限）
const E: usize = 100; // 期限切れの付与・画面の数（掃除 1 回で消える数）
const DENSITY: usize = 10; // 旧形式の行の割合（1/DENSITY）。バックフィルの 1 バッチの走査は 500×DENSITY 行
const BATCH: usize = 500;
/// マイグレーションで入れる固定の一覧（履歴では増えない）。計画が索引と全件読みの間で切り替わるので、絶対の上限で見る
const STATIC: &[&str] = &["app_event_types", "app_event_aliases"];

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// 中間行（各ノードの出力行×loops の最大）とソート入力（Sort の子の行×loops の合計）
#[derive(Default)]
struct Shape {
    intermediate: f64,
    sort_input: f64,
}

fn scan(node: &Value, relations: &mut BTreeMap<String, f64>, shape: &mut Shape) {
    let loops = node["Actual Loops"].as_f64().unwrap_or(0.);
    shape.intermediate = shape
        .intermediate
        .max(node["Actual Rows"].as_f64().unwrap_or(0.) * loops);
    if node["Node Type"] == "Sort" {
        for child in node["Plans"].as_array().into_iter().flatten() {
            shape.sort_input += child["Actual Rows"].as_f64().unwrap_or(0.)
                * child["Actual Loops"].as_f64().unwrap_or(0.);
        }
    }
    let work = (node["Actual Rows"].as_f64().unwrap_or(0.)
        + node["Rows Removed by Filter"].as_f64().unwrap_or(0.)
        + node["Rows Removed by Index Recheck"].as_f64().unwrap_or(0.))
        * loops;
    if let Some(name) = node["Relation Name"].as_str() {
        *relations.entry(name.into()).or_default() += work;
    }
    assert!(
        !node["Sort Method"]
            .as_str()
            .unwrap_or("")
            .contains("external"),
        "sort spilled to disk"
    );
    if let Some(children) = node["Plans"].as_array() {
        for child in children {
            scan(child, relations, shape)
        }
    }
}

async fn client(url: &str) -> anyhow::Result<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, NoTls).await?;
    tokio::spawn(async move { connection.await.unwrap() });
    Ok(client)
}

/// トリガー・外部キーの中の計画（親の EXPLAIN には出ない）を auto_explain の通知で拾う
async fn plan_client(
    url: &str,
) -> anyhow::Result<(tokio_postgres::Client, Arc<Mutex<Vec<Value>>>)> {
    let (client, mut connection) = tokio_postgres::connect(url, NoTls).await?;
    let plans = Arc::new(Mutex::new(Vec::new()));
    let output = plans.clone();
    tokio::spawn(async move {
        while let Some(message) = std::future::poll_fn(|cx| connection.poll_message(cx)).await {
            if let Ok(tokio_postgres::AsyncMessage::Notice(notice)) = message
                && let Some(at) = notice.message().find('{')
                && let Ok(plan) = serde_json::from_str::<Value>(&notice.message()[at..])
                // 測っている EXECUTE 自身の計画は EXPLAIN の結果から読むので、入れ子の文だけを残す
                && !plan["Query Text"]
                    .as_str()
                    .is_some_and(|q| q.contains("EXECUTE budget"))
            {
                output.lock().unwrap().push(plan);
            }
        }
    });
    client.batch_execute("LOAD 'auto_explain'; SET auto_explain.log_min_duration=0; SET auto_explain.log_analyze=on; SET auto_explain.log_timing=off; SET auto_explain.log_buffers=on; SET auto_explain.log_nested_statements=on; SET auto_explain.log_format=json; SET auto_explain.log_level=notice;").await?;
    Ok((client, plans))
}

/// 測る SQL。`source` にある文字列と同じこと（空白を除いて）を確かめる
struct Query {
    name: &'static str,
    source: &'static str,
    sql: String,
    args: Vec<String>,
    /// 関係ごとの走査の上限。書いていない関係は 8 行まで
    bounds: Vec<(&'static str, f64)>,
    /// 計る前に同じトランザクションで流す準備（ROLLBACK で戻る）
    setup: String,
    /// 返す・書く行数（決まっているとき）。書き込みが本当に起きたか、空の結果かを確かめる
    rows: Option<u64>,
}

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '\\')
        .collect()
}

const UI: &str = include_str!("../src/apps/ui.rs");
const DELIVERY: &str = include_str!("../src/apps/delivery.rs");
const BACKFILL: &str = include_str!("../src/components/backfill.rs");

#[tokio::test]
#[ignore = "requires an empty migrated disposable database and COMPONENTS_SCALE_ROWS"]
async fn component_queries_have_bounded_work() -> anyhow::Result<()> {
    let n: usize = std::env::var("COMPONENTS_SCALE_ROWS")?.parse()?;
    assert!(n >= 300000);
    let url = std::env::var("DATABASE_URL")?;
    let db = client(&url).await?;
    let mid = format!("m-{:08}", n / 2);
    // 履歴（N 行ずつ）: 雑音のメッセージ・展開・添付・画面・付与。対象: 利用者 550e8400-e29b-41d4-a716-4466554400a1 のメッセージ 1 件と画面
    // 履歴の行は、行ごとのトリガー（表示用の表・アプリのイベントなど）を止めて入れる（測る SQL ではトリガーを動かす）
    db.batch_execute(&format!(r#"
SET session_replication_role = replica;
INSERT INTO users(id,user_name,display_name) VALUES('550e8400-e29b-41d4-a716-4466554400a1','550e8400-e29b-41d4-a716-4466554400a1','A'),('cb-noise','cb-noise','N'),('cb-bot','cb-bot','B'),('cb-bot-2','cb-bot-2','B');
INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES('cb-ch','target','550e8400-e29b-41d4-a716-4466554400a1',false),('cb-noise-ch','noise','cb-noise',false);
INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES('550e8400-e29b-41d4-a716-4466554400a1','cb-ch',now()),('cb-bot','cb-ch',now());
INSERT INTO apps(id,owner_id,bot_user_id,name,status) VALUES('cb-app','550e8400-e29b-41d4-a716-4466554400a1','cb-bot','App','installed'),('cb-app-2','cb-noise','cb-bot-2','Noise','installed');
INSERT INTO app_installations(id,app_id,scopes,revision,manifest) VALUES('cb-inst','cb-app','["ui:write","messages:write"]',1,'{{"scopes":["ui:write","messages:write"],"events":["action.invoked","view.submitted"]}}'),('cb-inst-2','cb-app-2','["ui:write"]',1,'{{}}');
-- 雑音のメッセージ（半分は対象のチャンネルの履歴。チャンネルの索引で読むと履歴に比例する）。1/DENSITY に旧カード、1/(2·DENSITY) に旧ブロック、1/DENSITY に展開の一覧
INSERT INTO messages(id,user_id,channel_id,content,card,app_blocks,app_unfurls)
SELECT 'm-'||lpad(i::text,8,'0'),'cb-noise',CASE WHEN i%2=0 THEN 'cb-ch' ELSE 'cb-noise-ch' END,'noise',
  CASE WHEN i%{DENSITY}=0 THEN '{{"title":"T","description":"D"}}' END,
  CASE WHEN i%{d2}=1 THEN '[{{"type":"text","text":"x"}}]' END,
  CASE WHEN i%{DENSITY}=2 THEN '[{{"app_id":"cb-app-2","name":"Noise","url":"https://e.example/'||i||'","card":{{"title":"T"}}}}]' END
FROM generate_series(1,{n}) i;
-- 対象のメッセージ（アプリが部品を付けたもの）
UPDATE messages SET user_id='cb-bot',channel_id='cb-ch',app_origin_id='cb-app',card=NULL,app_blocks=NULL,app_unfurls=NULL,
  components='{{"flags":32768,"components":[{{"type":1,"components":[{{"type":2,"style":1,"label":"Go","custom_id":"go"}}]}}]}}'
  WHERE id='{mid}';
INSERT INTO app_link_unfurls(message_id,installation_id,url,card)
SELECT 'm-'||lpad(i::text,8,'0'),'cb-inst-2','https://e.example/'||i,'{{"title":"T"}}' FROM generate_series(1,{n}) i WHERE i<>{half};
INSERT INTO app_link_unfurls(message_id,installation_id,url,card)
SELECT '{mid}','cb-inst','https://t.example/'||i,'{{"title":"T"}}' FROM generate_series(1,{U}) i;
-- 雑音の添付（投稿済み）と、利用者が上げてまだ投稿していない添付 F 件・期限切れの付与用 E 件
INSERT INTO attachments(id,uploader_id,channel_id,message_id,file_name,file_size,content_type,storage_key)
SELECT 'a-'||i,'cb-noise',CASE WHEN i%2=0 THEN 'cb-ch' ELSE 'cb-noise-ch' END,'m-'||lpad(i::text,8,'0'),'a.pdf',3,'application/pdf','k-'||i FROM generate_series(1,{n}) i;
INSERT INTO attachments(id,uploader_id,channel_id,file_name,file_size,content_type,storage_key)
SELECT 'up-'||i,'550e8400-e29b-41d4-a716-4466554400a1','cb-ch','a.pdf',3,'application/pdf','up-'||i FROM generate_series(1,{F}+{E}) i;
-- 付与: 雑音（期限内）N 件と、期限切れ E 件
INSERT INTO app_submitted_files(attachment_id,installation_id,expires_at) SELECT 'a-'||i,'cb-inst-2',now()+interval '1 day' FROM generate_series(1,{n}) i;
INSERT INTO app_submitted_files(attachment_id,installation_id,expires_at) SELECT 'up-'||({F}+i),'cb-inst',now()-interval '1 hour' FROM generate_series(1,{E}) i;
-- 画面: 雑音のモーダル N 件（1/DENSITY は旧ブロック）、期限切れ E 件、利用者の一時メッセージ（送信しても消えない）・問い合わせ
INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,document,expires_at)
SELECT 'ui-'||lpad(i::text,8,'0'),'cb-inst-2','cb-noise','cb-noise-ch','modal',
  CASE WHEN i%{DENSITY}=0 THEN '{{"blocks":[{{"type":"text","text":"x"}}]}}' ELSE '{{"components":[]}}' END,
  now()+interval '1 day' FROM generate_series(1,{n}) i;
INSERT INTO app_ui(id,installation_id,user_id,kind,expires_at) SELECT 'old-'||i,'cb-inst-2','cb-noise','trigger',now()-interval '1 hour' FROM generate_series(1,{E}) i;
INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,document,expires_at) VALUES
  ('cb-surface','cb-inst','550e8400-e29b-41d4-a716-4466554400a1','cb-ch','ephemeral','{{"components":[{{"type":18,"label":"Name","component":{{"type":4,"custom_id":"name"}}}}]}}',now()+interval '1 day'),
  ('cb-options','cb-inst','550e8400-e29b-41d4-a716-4466554400a1','cb-ch','options','{{}}',now()+interval '1 minute');
SET session_replication_role = DEFAULT;
ANALYZE;
"#, d2 = DENSITY * 2, half = n / 2)).await?;

    let files = json!((1..=F).map(|i| format!("up-{i}")).collect::<Vec<_>>()).to_string();
    let doc = r#"{"flags":32768,"components":[{"type":10,"content":"x"}]}"#;
    let key = json!([mid, "cb-inst", "https://t.example/1"]).to_string();
    let q = |name, source, sql: &str, args: &[&str], bounds: &[(&'static str, f64)]| Query {
        name,
        source,
        sql: sql.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        bounds: bounds.to_vec(),
        setup: String::new(),
        rows: None,
    };
    let unfurls_of = "(SELECT COALESCE(jsonb_agg(jsonb_strip_nulls(jsonb_build_object('app_id',i.app_id,'name',a.name,'url',u.url,'card',u.card::jsonb,'components',u.components::jsonb))),'[]'::jsonb)::text FROM app_link_unfurls u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE u.message_id=m.id)";
    let batch = BATCH as f64;
    let per_batch = batch * DENSITY as f64;
    let mut queries = vec![
        // invoke: 操作した画面・メッセージを読む（主キー 1 行、FOR UPDATE）
        q(
            "invoke-surface",
            UI,
            "SELECT json_build_object('document',document::jsonb,'kind',kind)::text FROM app_ui WHERE id=$1 AND installation_id=$2 AND user_id=$3 AND (expires_at IS NULL OR expires_at>now()) AND kind IN('home','modal','ephemeral') AND channel_id IS NOT DISTINCT FROM NULLIF($4,'') FOR UPDATE",
            &[
                "cb-surface",
                "cb-inst",
                "550e8400-e29b-41d4-a716-4466554400a1",
                "cb-ch",
            ],
            &[],
        ),
        q(
            "invoke-message",
            UI,
            "SELECT json_build_object('components',components::jsonb,'blocks',app_blocks::jsonb)::text FROM messages WHERE id=$1 AND channel_id=$2 AND app_origin_id=$3 AND deleted_at IS NULL AND (components IS NOT NULL OR app_blocks IS NOT NULL)",
            &[&mid, "cb-ch", "cb-app"],
            &[],
        ),
        // external select: 問い合わせを作る・答えを書く・答えを待つ（主キー 1 行）
        q(
            "options-insert",
            UI,
            "INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,expires_at) VALUES($1,$2,$3,NULLIF($4,''),'options',now()+interval '1 minute')",
            &[
                "cb-new-options",
                "cb-inst",
                "550e8400-e29b-41d4-a716-4466554400a1",
                "cb-ch",
            ],
            &[],
        ),
        q(
            "options-provide",
            UI,
            "UPDATE app_ui SET document=$3,updated_at=now() WHERE id=$1 AND installation_id=$2 AND kind='options' AND expires_at>now() RETURNING '{}'::text",
            &["cb-options", "cb-inst", r#"{"options":[]}"#],
            &[],
        ),
        q(
            "options-poll",
            UI,
            "SELECT json_build_object('document',document::jsonb)::text FROM app_ui WHERE id=$1 AND user_id=$2 AND kind='options' AND expires_at>now()",
            &["cb-options", "550e8400-e29b-41d4-a716-4466554400a1"],
            &[],
        ),
        // file upload: 添付 F 件の確認と付与、アプリの取得、期限切れの掃除（E 件）
        q(
            "upload-check",
            UI,
            "SELECT json_build_object('n',count(*))::text FROM (SELECT DISTINCT x FROM jsonb_array_elements_text($1::jsonb) x) ids CROSS JOIN LATERAL (SELECT a.uploader_id,a.channel_id,a.message_id FROM attachments a WHERE a.id=ids.x OFFSET 0) a WHERE a.uploader_id=$2 AND a.channel_id=$3 AND a.message_id IS NULL",
            &[&files, "550e8400-e29b-41d4-a716-4466554400a1", "cb-ch"],
            &[("attachments", F as f64)],
        ),
        q(
            "upload-grant",
            UI,
            "INSERT INTO app_submitted_files(attachment_id,installation_id,expires_at) SELECT x,$2,now()+interval '1 day' FROM jsonb_array_elements_text($1::jsonb) x ON CONFLICT(attachment_id,installation_id) DO UPDATE SET expires_at=EXCLUDED.expires_at",
            &[&files, "cb-inst"],
            &[
                ("app_submitted_files", F as f64),
                ("attachments", F as f64),
                ("app_installations", F as f64),
            ],
        ),
        q(
            "upload-download",
            UI,
            "SELECT json_build_object('key',a.storage_key,'type',a.content_type,'name',a.file_name,'size',a.file_size)::text FROM app_submitted_files g JOIN attachments a ON a.id=g.attachment_id WHERE g.attachment_id=$1 AND g.installation_id=$2 AND g.expires_at>now()",
            &["a-1", "cb-inst-2"],
            &[],
        ),
        q(
            "upload-expire",
            DELIVERY,
            "DELETE FROM app_submitted_files WHERE expires_at<now()",
            &[],
            &[("app_submitted_files", 2. * E as f64)],
        ),
        // 部品を付ける（components の書き換え）と、展開を付ける
        q(
            "publish-read",
            UI,
            "SELECT json_build_object('content',content,'components',components)::text FROM messages WHERE id=$1 FOR UPDATE",
            &[&mid],
            &[],
        ),
        q(
            "publish-attachments",
            UI,
            "SELECT json_build_object('a',COALESCE(app_attachments,'[]')::jsonb)::text FROM messages WHERE id=$1",
            &[&mid],
            &[],
        ),
        q(
            "publish-update",
            UI,
            "UPDATE messages SET components=$2,app_blocks=NULLIF($6,''),app_origin_id=$3,content=CASE WHEN $5='1' THEN $4 ELSE content END,updated_at=CURRENT_TIMESTAMP WHERE id=$1",
            &[&mid, doc, "cb-app", "x", "1", ""],
            &[("users", 16.)],
        ),
        q(
            "unfurl-upsert",
            UI,
            "INSERT INTO app_link_unfurls(message_id,installation_id,url,components,card) VALUES($1,$2,$3,$4,NULLIF($5,'')) ON CONFLICT(message_id,installation_id,url) DO UPDATE SET components=EXCLUDED.components,card=EXCLUDED.card",
            &[&mid, "cb-inst", "https://t.example/1", doc, ""],
            &[],
        ),
        q(
            "unfurl-rebuild",
            UI,
            "UPDATE messages SET app_unfurls=(SELECT COALESCE(jsonb_agg(jsonb_strip_nulls(jsonb_build_object('app_id',i.app_id,'name',a.name,'url',u.url,'card',u.card::jsonb,'components',u.components::jsonb))),'[]'::jsonb)::text FROM app_link_unfurls u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE u.message_id=$1),updated_at=CURRENT_TIMESTAMP WHERE id=$1",
            &[&mid],
            &[
                ("app_link_unfurls", 3. * U as f64),
                ("app_installations", 3. * U as f64),
                ("apps", 3. * U as f64),
            ],
        ),
        q(
            "surfaces-list",
            UI,
            "SELECT json_build_object('id',u.id,'kind',kind,'channel_id',u.channel_id,'document',document::jsonb,'updated_at',u.updated_at)::text FROM app_ui u JOIN app_installations i ON i.id=u.installation_id JOIN apps a ON a.id=i.app_id WHERE a.id=$1 AND a.status='installed' AND i.active AND i.scopes::jsonb ? 'ui:write' AND u.user_id=$2 AND kind NOT IN('trigger','options') AND (expires_at IS NULL OR expires_at>now()) AND (u.channel_id IS NULL OR EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.channel_id=u.channel_id AND cu.user_id=$2 AND c.deleted_at IS NULL)) ORDER BY u.updated_at",
            &["cb-app", "550e8400-e29b-41d4-a716-4466554400a1"],
            &[],
        ),
        q(
            "ephemeral-insert",
            UI,
            "INSERT INTO app_ui(id,installation_id,user_id,channel_id,kind,document,expires_at) VALUES($1,$2,$3,$4,'ephemeral',$5,now()+interval '15 minutes')",
            &[
                "cb-new-ephemeral",
                "cb-inst",
                "550e8400-e29b-41d4-a716-4466554400a1",
                "cb-ch",
                doc,
            ],
            &[],
        ),
        q(
            "ui-expire",
            DELIVERY,
            "DELETE FROM app_ui WHERE expires_at<now()",
            &[],
            &[("app_ui", 2. * E as f64)],
        ),
        // バックフィル（1 回限り）。1 バッチは 500 件を集めるまで主キーの順に読む: 走査は 500×DENSITY 行で、N によらない
        q(
            "backfill-messages-first",
            BACKFILL,
            "SELECT json_build_object('id',m.id,'old',NULL,'card',m.card,'blocks',m.app_blocks)::text FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.id>$1 AND m.components IS NULL AND (m.card IS NOT NULL OR m.app_blocks IS NOT NULL) AND m.deleted_at IS NULL AND NOT c.encrypted ORDER BY m.id LIMIT 500 FOR UPDATE OF m",
            &[""],
            &[("messages", 1.2 * per_batch), ("channels", 1.2 * batch)],
        ),
        q(
            "backfill-messages-middle",
            BACKFILL,
            "SELECT json_build_object('id',m.id,'old',NULL,'card',m.card,'blocks',m.app_blocks)::text FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.id>$1 AND m.components IS NULL AND (m.card IS NOT NULL OR m.app_blocks IS NOT NULL) AND m.deleted_at IS NULL AND NOT c.encrypted ORDER BY m.id LIMIT 500 FOR UPDATE OF m",
            &[&mid],
            &[("messages", 1.2 * per_batch), ("channels", 1.2 * batch)],
        ),
        q(
            "backfill-messages-update",
            BACKFILL,
            "UPDATE messages SET components=$2 WHERE id=$1 AND components IS NULL RETURNING json_build_object('ok',true)::text",
            &["m-00000010", doc],
            &[],
        ),
        q(
            "backfill-unfurls-first",
            BACKFILL,
            "SELECT json_build_object('id',json_build_array(u.message_id,u.installation_id,u.url)::text,'old',NULL,'card',u.card)::text FROM app_link_unfurls u WHERE (u.message_id,u.installation_id,u.url)>($1::jsonb->>0,$1::jsonb->>1,$1::jsonb->>2) AND u.components IS NULL AND u.card IS NOT NULL ORDER BY u.message_id,u.installation_id,u.url LIMIT 500 FOR UPDATE OF u",
            &[r#"["","",""]"#],
            &[("app_link_unfurls", 1.2 * batch)],
        ),
        q(
            "backfill-unfurls-middle",
            BACKFILL,
            "SELECT json_build_object('id',json_build_array(u.message_id,u.installation_id,u.url)::text,'old',NULL,'card',u.card)::text FROM app_link_unfurls u WHERE (u.message_id,u.installation_id,u.url)>($1::jsonb->>0,$1::jsonb->>1,$1::jsonb->>2) AND u.components IS NULL AND u.card IS NOT NULL ORDER BY u.message_id,u.installation_id,u.url LIMIT 500 FOR UPDATE OF u",
            &[&key],
            &[("app_link_unfurls", 1.2 * batch)],
        ),
        q(
            "backfill-unfurls-update",
            BACKFILL,
            "UPDATE app_link_unfurls SET components=$2 WHERE message_id=$1::jsonb->>0 AND installation_id=$1::jsonb->>1 AND url=$1::jsonb->>2 AND components IS NULL RETURNING json_build_object('ok',true)::text",
            &[&key, doc],
            &[],
        ),
        q(
            "backfill-app-unfurls-first",
            BACKFILL,
            &format!(
                "SELECT json_build_object('id',m.id,'old',m.app_unfurls,'new',{unfurls_of})::text FROM messages m JOIN channels c ON c.id=m.channel_id WHERE m.id>$1 AND m.app_unfurls IS JSON ARRAY AND m.deleted_at IS NULL AND NOT c.encrypted AND EXISTS(SELECT 1 FROM jsonb_array_elements(m.app_unfurls::jsonb) e WHERE NOT e ? 'components') ORDER BY m.id LIMIT 500 FOR UPDATE OF m"
            ),
            &[""],
            &[
                ("messages", 1.2 * per_batch),
                ("channels", 1.2 * batch),
                ("app_link_unfurls", 1.2 * batch),
                ("app_installations", 2.4 * batch),
                ("apps", 2.4 * batch),
            ],
        ),
        q(
            "backfill-app-unfurls-update",
            BACKFILL,
            "UPDATE messages SET app_unfurls=$2 WHERE id=$1 RETURNING json_build_object('ok',true)::text",
            &["m-00000002", "[]"],
            &[],
        ),
        q(
            "backfill-ui-first",
            BACKFILL,
            "SELECT json_build_object('id',id,'old',document::jsonb,'kind',kind)::text FROM app_ui WHERE id>$1 AND kind IN('home','modal','ephemeral') AND document IS JSON OBJECT AND document::jsonb ? 'blocks' AND NOT document::jsonb ? 'components' ORDER BY id LIMIT 500 FOR UPDATE",
            &[""],
            &[("app_ui", 1.2 * per_batch)],
        ),
        q(
            "backfill-ui-update",
            BACKFILL,
            "UPDATE app_ui SET document=$2 WHERE id=$1 AND NOT document::jsonb ? 'components' RETURNING json_build_object('ok',true)::text",
            &["ui-00000010", doc],
            &[],
        ),
        // restore: 記録した値のままの行だけを戻す（主キー 1 行）
        q(
            "restore-messages",
            BACKFILL,
            "UPDATE messages SET components=NULL WHERE id=$1 AND components=$2 AND $3='' RETURNING '{}'::text",
            &[&mid, doc, ""],
            &[],
        ),
        q(
            "restore-unfurls",
            BACKFILL,
            "UPDATE app_link_unfurls SET components=NULL WHERE message_id=$1::jsonb->>0 AND installation_id=$1::jsonb->>1 AND url=$1::jsonb->>2 AND components=$2 AND $3='' RETURNING '{}'::text",
            &[&key, doc, ""],
            &[],
        ),
        q(
            "restore-app-unfurls",
            BACKFILL,
            "UPDATE messages SET app_unfurls=$3 WHERE id=$1 AND app_unfurls=$2 RETURNING '{}'::text",
            &["m-00000002", "[]", "[]"],
            &[],
        ),
        q(
            "restore-ui",
            BACKFILL,
            "UPDATE app_ui SET document=$3 WHERE id=$1 AND document=$2 RETURNING '{}'::text",
            &["ui-00000010", doc, "{}"],
            &[],
        ),
    ];
    // 追加の条件: 同じ SQL を、ほかの引数・準備で測る
    let variant = |queries: &Vec<Query>,
                   base: &str,
                   name: &'static str,
                   args: &[&str],
                   setup: &str,
                   rows: u64| {
        let b = queries.iter().find(|q| q.name == base).unwrap();
        Query {
            name,
            source: b.source,
            sql: b.sql.clone(),
            args: args.iter().map(|s| s.to_string()).collect(),
            bounds: b.bounds.clone(),
            setup: setup.to_string(),
            rows: Some(rows),
        }
    };
    let set_doc = quote(doc);
    let extra = vec![
        // restore: 記録した値のままの行を実際に戻す（1 行を書く。トリガーも動く）
        variant(
            &queries,
            "restore-messages",
            "restore-messages-hit",
            &[&mid, doc, ""],
            &format!(
                "UPDATE messages SET components={set_doc} WHERE id={};",
                quote(&mid)
            ),
            1,
        ),
        variant(
            &queries,
            "restore-unfurls",
            "restore-unfurls-hit",
            &[&key, doc, ""],
            &format!(
                "UPDATE app_link_unfurls SET components={set_doc} WHERE message_id={} AND installation_id='cb-inst' AND url='https://t.example/1';",
                quote(&mid)
            ),
            1,
        ),
        variant(
            &queries,
            "restore-app-unfurls",
            "restore-app-unfurls-hit",
            &["m-00000002", "[]", "[{\"url\":\"https://e.example/2\"}]"],
            "UPDATE messages SET app_unfurls='[]' WHERE id='m-00000002';",
            1,
        ),
        variant(
            &queries,
            "restore-ui",
            "restore-ui-hit",
            &["ui-00000010", doc, "{}"],
            &format!("UPDATE app_ui SET document={set_doc} WHERE id='ui-00000010';"),
            1,
        ),
        // 閲覧できない・見つからない: 別の利用者の画面と問い合わせ、別のアプリのメッセージ、他人の添付、付与のない取得
        variant(
            &queries,
            "invoke-surface",
            "invoke-surface-other-user",
            &["cb-surface", "cb-inst", "cb-noise", "cb-ch"],
            "",
            0,
        ),
        variant(
            &queries,
            "invoke-message",
            "invoke-message-other-app",
            &[&mid, "cb-ch", "cb-app-2"],
            "",
            0,
        ),
        variant(
            &queries,
            "options-poll",
            "options-poll-other-user",
            &["cb-options", "cb-noise"],
            "",
            0,
        ),
        variant(
            &queries,
            "upload-check",
            "upload-check-others",
            &[
                &json!((1..=F).map(|i| format!("a-{i}")).collect::<Vec<_>>()).to_string(),
                "550e8400-e29b-41d4-a716-4466554400a1",
                "cb-ch",
            ],
            "",
            1,
        ),
        variant(
            &queries,
            "upload-download",
            "upload-download-not-granted",
            &["a-1", "cb-inst"],
            "",
            0,
        ),
        // 掃除する行がない
        variant(
            &queries,
            "upload-expire",
            "upload-expire-none",
            &[],
            "DELETE FROM app_submitted_files WHERE expires_at<now();",
            0,
        ),
        variant(
            &queries,
            "ui-expire",
            "ui-expire-none",
            &[],
            "DELETE FROM app_ui WHERE expires_at<now();",
            0,
        ),
        // バックフィルの最後（続きがない）
        variant(
            &queries,
            "backfill-messages-first",
            "backfill-messages-end",
            &["m-99999999"],
            "",
            0,
        ),
        variant(
            &queries,
            "backfill-unfurls-first",
            "backfill-unfurls-end",
            &[r#"["m-99999999","",""]"#],
            "",
            0,
        ),
        variant(
            &queries,
            "backfill-ui-first",
            "backfill-ui-end",
            &["ui-99999999"],
            "",
            0,
        ),
    ];
    // 一致しない restore（前の値から変わった行は戻さない）
    for name in [
        "restore-messages",
        "restore-unfurls",
        "restore-app-unfurls",
        "restore-ui",
    ] {
        queries.iter_mut().find(|q| q.name == name).unwrap().rows = Some(0);
    }
    queries.extend(extra);
    for query in &queries {
        // バックフィルの app_unfurls の SELECT は、ソースでは UNFURLS_OF を format! で埋める
        let source_sql = query.sql.replace(unfurls_of, "{UNFURLS_OF}");
        assert!(
            squash(query.source).contains(&squash(&source_sql)),
            "{}: the measured SQL is not in the source",
            query.name
        );
    }

    let samples = Arc::new(Mutex::new(Vec::<Value>::new()));
    let violations = Arc::new(Mutex::new(std::collections::BTreeSet::<String>::new()));
    for query in &queries {
        let max = query.args.len();
        let prepared = format!(
            "PREPARE budget({}) AS {}; EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) EXECUTE budget{};",
            vec!["text"; max].join(","),
            query.sql,
            if max == 0 {
                String::new()
            } else {
                format!(
                    "({})",
                    query
                        .args
                        .iter()
                        .map(|a| quote(a))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        )
        .replace("PREPARE budget() AS", "PREPARE budget AS");
        let bounds: BTreeMap<&str, f64> = query.bounds.iter().cloned().collect();
        for concurrency in [1, 8] {
            db.batch_execute("VACUUM (ANALYZE) messages, app_ui, app_link_unfurls, attachments, app_submitted_files;").await?;
            let mut tasks = Vec::new();
            for _ in 0..concurrency {
                let (url, name, sql, samples, bounds, violations, setup, expected) = (
                    url.clone(),
                    query.name,
                    prepared.clone(),
                    samples.clone(),
                    bounds.clone(),
                    violations.clone(),
                    query.setup.clone(),
                    query.rows,
                );
                tasks.push(tokio::spawn(async move {
                    let (db, nested) = plan_client(&url).await.unwrap();
                    // 初回も捨てずに 30 回
                    for sample in 0..30 {
                        // 準備は同じトランザクションで先に流し、その計画は数えない
                        db.batch_execute(&format!("BEGIN; {setup}")).await.unwrap();
                        nested.lock().unwrap().clear();
                        let start = std::time::Instant::now();
                        let rows = db
                            .simple_query(&format!("{sql} ROLLBACK; DEALLOCATE ALL;"))
                            .await
                            .unwrap();
                        let roundtrip_ms = start.elapsed().as_secs_f64() * 1000.;
                        let inner = std::mem::take(&mut *nested.lock().unwrap());
                        let raw = rows
                            .iter()
                            .find_map(|row| match row {
                                SimpleQueryMessage::Row(row) => {
                                    row.get(0).filter(|s| s.starts_with('['))
                                }
                                _ => None,
                            })
                            .unwrap();
                        let value: Value = serde_json::from_str(raw).unwrap();
                        let plan = &value[0];
                        let mut relations = BTreeMap::new();
                        let mut shape = Shape::default();
                        scan(&plan["Plan"], &mut relations, &mut shape);
                        for child in &inner {
                            scan(&child["Plan"], &mut relations, &mut shape);
                        }
                        if let Some(rows) = expected
                            && plan["Plan"]["Actual Rows"].as_u64() != Some(rows)
                        {
                            violations.lock().unwrap().insert(format!(
                                "{name}: {} rows, expected {rows}",
                                plan["Plan"]["Actual Rows"]
                            ));
                        }
                        for (relation, work) in &relations {
                            let bound = if STATIC.contains(&relation.as_str()) {
                                256.
                            } else {
                                bounds.get(relation.as_str()).copied().unwrap_or(8.)
                            };
                            if *work > bound {
                                violations.lock().unwrap().insert(format!("{name} {relation}: {work} > {bound}"));
                            }
                        }
                        let blocks = |p: &Value| {
                            p["Plan"]["Shared Hit Blocks"].as_u64().unwrap_or(0)
                                + p["Plan"]["Shared Read Blocks"].as_u64().unwrap_or(0)
                        };
                        samples.lock().unwrap().push(json!({
                            "name": name, "concurrency": concurrency, "relations": relations,
                            "blocks": blocks(plan) + inner.iter().map(blocks).sum::<u64>(),
                            "intermediate": shape.intermediate, "sort_input": shape.sort_input,
                            "planning_ms": plan["Planning Time"], "execution_ms": plan["Execution Time"],
                            "roundtrip_ms": roundtrip_ms,
                            "plan": if sample == 0 { plan.clone() } else { Value::Null },
                            "nested_plans": if sample == 0 { json!(inner) } else { Value::Null },
                        }));
                    }
                }));
            }
            for task in tasks {
                task.await?;
            }
        }
    }

    let violations = violations.lock().unwrap().clone();
    assert!(
        violations.is_empty(),
        "work over the bounds:\n{}",
        violations.into_iter().collect::<Vec<_>>().join("\n")
    );

    // API: 認証・プール待ち・JSON を含む invoke の往復（ボタンの操作と、選択肢の問い合わせ）
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let mut config = backend::config::Config::from_env();
    config.database_url = url.clone();
    config.dev_auth_enabled = true;
    let state = backend::state::AppState::new(backend::db::init_db(&url).await?, config);
    let app = backend::routes::create_router(state.clone());
    let api = Arc::new(Mutex::new(Vec::<Value>::new()));
    for concurrency in [1, 8] {
        let mut tasks = Vec::new();
        for _ in 0..concurrency {
            let (app, api, mid) = (app.clone(), api.clone(), mid.clone());
            tasks.push(tokio::spawn(async move {
                for _ in 0..30 {
                    for (name, body) in [
                        ("invoke-action", json!({"kind":"action","channel_id":"cb-ch","message_id":mid,"action_id":"go","values":{}})),
                        ("invoke-submit", json!({"kind":"submit","channel_id":"cb-ch","surface_id":"cb-surface","values":{"name":"x"}})),
                    ] {
                        let request = Request::builder()
                            .method("POST")
                            .uri("/api/apps/cb-app/invoke")
                            .header("X-Dev-User", "550e8400-e29b-41d4-a716-4466554400a1")
                            .header("Content-Type", "application/json")
                            .body(Body::from(body.to_string()))
                            .unwrap();
                        let start = std::time::Instant::now();
                        let response = app.clone().oneshot(request).await.unwrap();
                        let status = response.status();
                        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                        assert!(status.is_success(), "{name} {status}: {}", String::from_utf8_lossy(&bytes));
                        api.lock().unwrap().push(json!({"name":name,"concurrency":concurrency,"roundtrip_ms":start.elapsed().as_secs_f64()*1000.}));
                    }
                }
            }));
        }
        for task in tasks {
            task.await?;
        }
    }

    // バックフィル全体（dry-run）の所要時間と読んだ行数。1 回限りで、表の行数に比例する
    let start = std::time::Instant::now();
    let report = backend::components::backfill::run(&state.db, false, |_| Ok(())).await?;
    let backfill = json!({"ms": start.elapsed().as_secs_f64() * 1000., "scanned": report.scanned, "changed": report.changed});

    let output = std::env::var("COMPONENTS_SCALE_OUTPUT")
        .unwrap_or(format!("/tmp/components-budget-{n}.json"));
    std::fs::write(
        &output,
        serde_json::to_vec_pretty(&json!({
            "rows": n, "queries": queries.len(),
            "query_definitions": queries.iter().map(|q| json!({"name":q.name,"sql":q.sql,"args":q.args,"bounds":q.bounds})).collect::<Vec<_>>(),
            "samples": *samples.lock().unwrap(), "api_samples": *api.lock().unwrap(), "backfill": backfill,
        }))?,
    )?;
    println!(
        "components query budget: {n} rows, {} queries, output {output}",
        queries.len()
    );
    Ok(())
}
