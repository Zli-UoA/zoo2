//! GET /api/search: 見える範囲、暗号化チャンネルの除外、月ごとのページング。
#[path = "support/thread_db.rs"]
mod thread_db;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;

use backend::{db::init_db, routes::create_router, state::AppState};

async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    user: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("X-Dev-User", user);
    if body.is_some() {
        req = req.header("content-type", "application/json");
    }
    let req = req
        .body(body.map_or_else(Body::empty, |b| Body::from(serde_json::to_vec(&b).unwrap())))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn search(app: &Router, user: &str, query: &str) -> Value {
    let (status, body) = call(app, "GET", &format!("/api/search?{query}"), user, None).await;
    assert_eq!(status, StatusCode::OK, "{query}: {body}");
    body
}

fn ids(body: &Value) -> Vec<String> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect()
}

fn q(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[tokio::test]
async fn search_respects_visibility_encryption_and_months() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let config = fixture.config.clone();
    let db = init_db(&config.database_url).await?;
    let (sql, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let app = create_router(AppState::new(db, config));

    let [alice, bob, guest] = [0; 3].map(|_| uuid::Uuid::new_v4().to_string());
    for (id, name) in [(&alice, "alice"), (&bob, "bob"), (&guest, "guest")] {
        let (status, _) = call(&app, "POST", "/api/users", id, Some(json!({
            "id": id, "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()), "display_name": name,
        }))).await;
        assert_eq!(status, StatusCode::OK);
    }
    let (app_ref, alice_ref) = (&app, &alice);
    let channel = |private: bool| async move {
        let (app, alice) = (app_ref, alice_ref);
        let id = uuid::Uuid::new_v4().to_string();
        let (status, body) = call(app, "POST", "/api/channels", alice, Some(json!({
            "id": id, "channel_name": format!("s-{}", uuid::Uuid::new_v4().simple()), "is_private": private,
        }))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        id
    };
    let public = channel(false).await;
    let private = channel(true).await;
    let encrypted = channel(true).await;
    // ゲストは private にだけ入る。
    sql.batch_execute(&format!(
        "UPDATE users SET is_guest = true WHERE id = '{guest}';
         INSERT INTO channel_user (channel_id, user_id, joined_at) VALUES ('{private}', '{guest}', LOCALTIMESTAMP);"
    )).await?;

    // created_at を直接決めるため SQL で入れる。月の境界をまたぐよう、今月と 3 か月前・20 か月前を使う。
    let insert = |id: &str, channel: &str, content: &str, ago: &str| {
        format!(
            "INSERT INTO messages (id, user_id, channel_id, content, created_at) \
             VALUES ('{id}', '{alice}', '{channel}', '{content}', LOCALTIMESTAMP - interval '{ago}');"
        )
    };
    let m = |n: u8| format!("00000000-0000-4000-8000-{n:012}");
    sql.batch_execute(
        &[
            insert(&m(1), &public, "明日の会議の資料です", "1 minute"),
            insert(&m(2), &public, "会議は中止 Hello WORLD", "2 minutes"),
            insert(&m(3), &public, "定例の会議", "3 minutes"),
            insert(&m(4), &private, "秘密の会議", "4 minutes"),
            insert(&m(5), &encrypted, "暗号化前の会議メモ", "5 minutes"),
            insert(&m(6), &public, "消した会議", "6 minutes"),
            insert(&m(7), &public, "三か月前の会議", "95 days"),
            insert(&m(8), &public, "二十か月前の会議", "610 days"),
            insert(&m(9), &public, "ボブの会議", "7 minutes")
                .replace(&format!("'{alice}'"), &format!("'{bob}'")),
        ]
        .concat(),
    )
    .await?;
    sql.batch_execute(&format!(
        "UPDATE messages SET deleted_at = LOCALTIMESTAMP WHERE id = '{}';
         UPDATE channels SET encrypted = true WHERE id = '{encrypted}';",
        m(6)
    ))
    .await?;

    // alice: 公開・private が見える。暗号化チャンネル（切り替え前の平文も）と削除済みは出ない。
    let all = search(
        &app,
        &alice,
        &format!("type=messages&q={}&limit=50", q("会議")),
    )
    .await;
    let got = ids(&all);
    assert_eq!(&got[..4], &[m(1), m(2), m(3), m(4)], "{all}");
    assert!(!got.contains(&m(5)) && !got.contains(&m(6)));
    assert!(
        got.contains(&m(7)),
        "3 months ago is within the first 12 months"
    );
    assert!(!got.contains(&m(8)), "20 months ago needs the next request");
    let cursor = all["next_cursor"].as_str().expect("older months remain");
    let older = search(
        &app,
        &alice,
        &format!(
            "type=messages&q={}&limit=50&cursor={}",
            q("会議"),
            q(cursor)
        ),
    )
    .await;
    assert_eq!(ids(&older), vec![m(8)], "{older}");
    assert!(older["next_cursor"].is_null());
    assert_eq!(all["items"][0]["channel_id"], json!(public));
    assert_eq!(all["items"][0]["author"]["id"], json!(alice));
    assert!(
        all["items"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("会議")
    );

    // bob は private に入っていない。
    let bob_ids = ids(&search(
        &app,
        &bob,
        &format!("type=messages&q={}&limit=50", q("会議")),
    )
    .await);
    assert!(bob_ids.contains(&m(1)) && !bob_ids.contains(&m(4)));
    // ゲストは入っているチャンネルだけ（公開チャンネルも見えない）。
    assert_eq!(
        ids(&search(&app, &guest, &format!("type=messages&q={}", q("会議"))).await),
        vec![m(4)]
    );

    // 取り込みで、チャンネルの作成日時より古い履歴を持つチャンネル。最古のトップレベル（親）が削除されても、
    // 生きている返信は、生きている最古のトップレベルより古くても、続きをたどれば見つかる（next_cursor が途中で
    // null にならない）。
    let imported = channel(false).await;
    sql.batch_execute(&format!(
        "INSERT INTO messages (id, user_id, channel_id, content, created_at, deleted_at) \
           VALUES ('{p}', '{alice}', '{imported}', '消した親', LOCALTIMESTAMP - interval '900 days', LOCALTIMESTAMP);
         INSERT INTO messages (id, user_id, channel_id, parent_id, content, created_at) \
           VALUES ('{r}', '{alice}', '{imported}', '{p}', '取込返信の本文', LOCALTIMESTAMP - interval '899 days');
         INSERT INTO messages (id, user_id, channel_id, content, created_at) \
           VALUES ('{t}', '{alice}', '{imported}', '最近の投稿', LOCALTIMESTAMP - interval '1 minute');",
        p = m(30),
        r = m(31),
        t = m(32),
    ))
    .await?;
    let mut found = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..6 {
        let mut path = format!(
            "type=messages&q={}&channel_id={imported}&limit=50",
            q("取込返信")
        );
        if let Some(c) = &cursor {
            path += &format!("&cursor={}", q(c));
        }
        let page = search(&app, &alice, &path).await;
        found.extend(ids(&page));
        cursor = page["next_cursor"].as_str().map(str::to_owned);
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(
        found,
        vec![m(31)],
        "reply older than every live top-level message"
    );
    assert!(cursor.is_none());

    // 1 文字、複数の語、英字の大文字・小文字。
    assert_eq!(
        ids(&search(&app, &alice, &format!("type=messages&q={}", q("秘"))).await),
        vec![m(4)]
    );
    assert_eq!(
        ids(&search(&app, &alice, &format!("type=messages&q={}", q("会議 資料"))).await),
        vec![m(1)]
    );
    assert_eq!(
        ids(&search(&app, &alice, "type=messages&q=hello%20world").await),
        vec![m(2)]
    );
    // LIKE の特殊文字は文字どおりに扱う。
    assert!(ids(&search(&app, &alice, &format!("type=messages&q={}", q("%"))).await).is_empty());

    // 同じ月の中の keyset ページング。
    let page1 = search(
        &app,
        &alice,
        &format!("type=messages&q={}&limit=2", q("会議")),
    )
    .await;
    assert_eq!(ids(&page1), vec![m(1), m(2)]);
    let c = page1["next_cursor"].as_str().unwrap();
    let page2 = search(
        &app,
        &alice,
        &format!("type=messages&q={}&limit=2&cursor={}", q("会議"), q(c)),
    )
    .await;
    assert_eq!(ids(&page2), vec![m(3), m(4)]);

    // 絞り込み: チャンネル、見えないチャンネル、期間。
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&channel_id={private}", q("会議"))
        )
        .await),
        vec![m(4)]
    );
    assert!(
        ids(&search(
            &app,
            &bob,
            &format!("type=messages&q={}&channel_id={private}", q("会議"))
        )
        .await)
        .is_empty()
    );
    assert!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&channel_id={encrypted}", q("会議"))
        )
        .await)
        .is_empty()
    );
    let today = sql
        .query_one(
            "SELECT (LOCALTIMESTAMP - interval '30 days')::date::text",
            &[],
        )
        .await?
        .get::<_, String>(0);
    let recent = ids(&search(
        &app,
        &alice,
        &format!("type=messages&q={}&limit=50&after={today}", q("会議")),
    )
    .await);
    assert_eq!(recent, vec![m(1), m(2), m(3), m(4), m(9)]);

    // 投稿者で絞る。
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&user_id={bob}", q("会議"))
        )
        .await),
        vec![m(9)]
    );
    assert!(
        !ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&limit=50&user_id={alice}", q("会議"))
        )
        .await)
        .contains(&m(9))
    );
    // before はその日を含まない。月初なら前の月から探す。月初は 1 回だけ取り、境界の行（月初ちょうどと
    // 1 秒前）と before の両方に使う（実行中に月が変わっても結果が変わらない）。
    let month_start = sql
        .query_one(
            "SELECT date_trunc('month', LOCALTIMESTAMP)::date::text",
            &[],
        )
        .await?
        .get::<_, String>(0);
    // 日の境界: 月初の 10 日前の 0 時ちょうどと、その 1 秒前（同じ月の中、時刻を固定）。
    let day = sql
        .query_one("SELECT ($1::text::date - 10)::text", &[&month_start])
        .await?
        .get::<_, String>(0);
    sql.batch_execute(&format!(
        "INSERT INTO messages (id, user_id, channel_id, content, created_at) VALUES
           ('{at_start}', '{alice}', '{public}', '月境界の記録', '{month_start}'::timestamp),
           ('{before_start}', '{alice}', '{public}', '月境界の記録', '{month_start}'::timestamp - interval '1 second'),
           ('{at_day}', '{alice}', '{public}', '日境界の記録', '{day}'::timestamp),
           ('{before_day}', '{alice}', '{public}', '日境界の記録', '{day}'::timestamp - interval '1 second');",
        at_start = m(11),
        before_start = m(12),
        at_day = m(15),
        before_day = m(16),
    ))
    .await?;
    let before_month = ids(&search(
        &app,
        &alice,
        &format!("type=messages&q={}&before={month_start}", q("月境界")),
    )
    .await);
    assert_eq!(before_month, vec![m(12)], "{before_month:?}");
    let all_boundary =
        ids(&search(&app, &alice, &format!("type=messages&q={}", q("月境界"))).await);
    assert_eq!(all_boundary, vec![m(11), m(12)]);
    // 指定した日の 0 時ちょうどの行は出ず、その 1 秒前の行は出る。
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&before={day}", q("日境界"))
        )
        .await),
        vec![m(16)]
    );
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&after={day}", q("日境界"))
        )
        .await),
        vec![m(15)]
    );
    // オフセット付きの境界（端末の日付の 0 時）。JST の {day} 9 時は UTC の {day} 0 時ちょうど。
    let jst = |time: &str| q(&format!("{day}T{time}+09:00"));
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&before={}", q("日境界"), jst("09:00:00"))
        )
        .await),
        vec![m(16)]
    );
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&after={}", q("日境界"), jst("08:59:00"))
        )
        .await),
        vec![m(15), m(16)]
    );
    let tomorrow = sql
        .query_one(
            "SELECT (LOCALTIMESTAMP + interval '1 day')::date::text",
            &[],
        )
        .await?
        .get::<_, String>(0);
    assert!(
        ids(&search(
            &app,
            &alice,
            &format!("type=messages&q={}&limit=50&before={tomorrow}", q("会議"))
        )
        .await)
        .contains(&m(1))
    );

    // ファイル名とノート。
    sql.batch_execute(&format!(
        "INSERT INTO attachments (id, message_id, uploader_id, channel_id, file_name, file_size, content_type, storage_key)
           VALUES ('a1', '{m1}', '{alice}', '{public}', '議事録_2026.PDF', 10, 'application/pdf', 'k1'),
                  ('a2', '{m6}', '{alice}', '{public}', '議事録_消した.pdf', 10, 'application/pdf', 'k2'),
                  ('a3', '{m1}', '{bob}', '{public}', '見積書_bob.pdf', 10, 'application/pdf', 'k3');
         INSERT INTO channel_notes (id, channel_id, title, content, created_user_id)
           VALUES ('n1', '{public}', '運用メモ', '障害対応の手順', '{alice}'),
                  ('n2', '{private}', '秘密のメモ', '障害の記録', '{alice}');",
        m1 = m(1), m6 = m(6)
    )).await?;
    // type=all はメッセージとファイルをまとめて新しい順。種類をまたいで続きを取れる。type なしは今までどおりメッセージだけ。
    sql.batch_execute(&format!(
        "INSERT INTO messages (id, user_id, channel_id, content, created_at) \
           VALUES ('{m40}', '{alice}', '{public}', '議事録を共有しました', LOCALTIMESTAMP - interval '1 hour');",
        m40 = m(40)
    ))
    .await?;
    assert_eq!(
        ids(&search(&app, &alice, &format!("q={}", q("議事録"))).await),
        vec![m(40)],
        "no type keeps the messages-only default"
    );
    let mixed = search(&app, &alice, &format!("type=all&q={}", q("議事録"))).await;
    assert_eq!(ids(&mixed), vec!["a1".to_string(), m(40)], "{mixed}");
    assert_eq!(mixed["items"][0]["kind"], json!("file"));
    assert_eq!(mixed["items"][1]["kind"], json!("message"));
    let first = search(&app, &alice, &format!("type=all&q={}&limit=1", q("議事録"))).await;
    assert_eq!(ids(&first), vec!["a1"]);
    let rest = search(
        &app,
        &alice,
        &format!(
            "type=all&q={}&limit=1&cursor={}",
            q("議事録"),
            q(first["next_cursor"].as_str().unwrap())
        ),
    )
    .await;
    assert_eq!(ids(&rest), vec![m(40)], "{rest}");
    let files = search(&app, &alice, &format!("type=files&q={}", q("議事録 pdf"))).await;
    assert_eq!(ids(&files), vec!["a1"], "{files}");
    assert_eq!(files["items"][0]["message_id"], json!(m(1)));
    assert_eq!(
        ids(&search(&app, &alice, &format!("type=notes&q={}", q("障害"))).await).len(),
        2
    );
    assert_eq!(
        ids(&search(&app, &bob, &format!("type=notes&q={}", q("障害"))).await),
        vec!["n1"]
    );
    // ファイルはアップロードした人、ノートは作った人で絞る。
    assert_eq!(
        ids(&search(&app, &alice, &format!("type=files&q=pdf&user_id={alice}")).await),
        vec!["a1"]
    );
    assert!(
        ids(&search(&app, &alice, &format!("type=files&q=pdf&user_id={bob}")).await) == vec!["a3"]
    );
    assert!(
        ids(&search(
            &app,
            &alice,
            &format!("type=notes&q={}&user_id={bob}", q("障害"))
        )
        .await)
        .is_empty()
    );
    // 作った人で絞ると、その人のノートが返る（alice は n1・n2 の両方を見られる）。
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("type=notes&q={}&user_id={alice}", q("障害"))
        )
        .await)
        .len(),
        2
    );
    // タイトルだけに当たったノートも、スニペットに語を含む。
    let notes = search(&app, &alice, &format!("type=notes&q={}", q("運用"))).await;
    assert!(
        notes["items"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("運用"),
        "{notes}"
    );
    // 長い本文の後ろで当たっても、スニペットに語を含む。
    let long = format!("{}末尾の会議メモ", "あ".repeat(5000));
    sql.batch_execute(&insert(&m(10), &public, &long, "8 minutes"))
        .await?;
    let hit = search(
        &app,
        &alice,
        &format!("type=messages&q={}", q("末尾の会議")),
    )
    .await;
    // 語の順番によらず、最初に当たった位置から切り出す（先頭の「資料」も残る）。
    let spread = format!("資料{}会議の続き", "い".repeat(1500));
    sql.batch_execute(&insert(&m(13), &public, &spread, "9 minutes"))
        .await?;
    let both = search(
        &app,
        &alice,
        &format!("type=messages&q={}", q("会議の続き 資料")),
    )
    .await;
    assert!(
        both["items"][0]["snippet"]
            .as_str()
            .unwrap()
            .starts_with("資料"),
        "{both}"
    );
    // 一致の判定は SQL の lower() だけで行う（ASCII 以外の大文字も、SQL が当たったものはスニペットに入る）。
    let accented = format!("{}ÉTÉ の予定", "う".repeat(600));
    sql.batch_execute(&insert(&m(14), &public, &accented, "10 minutes"))
        .await?;
    let lower_matches: bool = sql
        .query_one("SELECT lower('ÉTÉ') = 'été'", &[])
        .await?
        .get(0);
    let summer = search(
        &app,
        &alice,
        &format!(
            "type=messages&q={}",
            q(if lower_matches { "été" } else { "ÉTÉ" })
        ),
    )
    .await;
    let summer_snippet = summer["items"][0]["snippet"].as_str().unwrap();
    assert!(
        summer_snippet.contains("ÉTÉ") && summer_snippet.starts_with('…'),
        "{summer}"
    );
    // ファイル名も同じ: 160 文字より後ろの ÉTÉ がスニペットに入る。
    let long_name = format!("{}ÉTÉ.pdf", "x".repeat(200));
    sql.batch_execute(&format!(
        "INSERT INTO attachments (id, message_id, uploader_id, channel_id, file_name, file_size, content_type, storage_key)
           VALUES ('a-long', '{m1}', '{alice}', '{public}', '{long_name}', 10, 'application/pdf', 'k-long');",
        m1 = m(1)
    ))
    .await?;
    let summer_file = search(
        &app,
        &alice,
        &format!(
            "type=files&q={}",
            q(if lower_matches { "été" } else { "ÉTÉ" })
        ),
    )
    .await;
    let file_snippet = summer_file["items"][0]["snippet"].as_str().unwrap();
    assert!(
        file_snippet.contains("ÉTÉ") && file_snippet.starts_with('…'),
        "{summer_file}"
    );
    assert!(
        hit["items"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("末尾の会議"),
        "{hit}"
    );

    // 入力の検証。
    let bad_cursor = format!("type=messages&q=x&cursor={}", q(r#"{"m":3000000000}"#));
    for bad in [
        bad_cursor.as_str(),
        "type=messages&q=",
        "type=messages&q=a%20b%20c%20d%20e%20f",
        "type=messages&q=x&cursor=nope",
        "type=messages&q=x&after=2026-13-01",
        "type=messages&q=x&after=2026-11-01T00:00:00",
        "type=messages&q=x&type=users",
    ] {
        let (status, _) = call(&app, "GET", &format!("/api/search?{bad}"), &alice, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    fixture.cleanup().await?;
    Ok(())
}

#[tokio::test]
async fn search_filters_by_many_people_and_channels_bots_and_my_conversations() -> anyhow::Result<()>
{
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let config = fixture.config.clone();
    let db = init_db(&config.database_url).await?;
    let (sql, connection) =
        tokio_postgres::connect(&config.database_url, tokio_postgres::NoTls).await?;
    tokio::spawn(connection);
    let app = create_router(AppState::new(db, config));

    let [alice, bob, carol, bot] = [0; 4].map(|_| uuid::Uuid::new_v4().to_string());
    for (id, name) in [
        (&alice, "alice"),
        (&bob, "bob"),
        (&carol, "carol"),
        (&bot, "bot"),
    ] {
        let (status, _) = call(&app, "POST", "/api/users", id, Some(json!({
            "id": id, "user_name": format!("{name}_{}", uuid::Uuid::new_v4().simple()), "display_name": name,
        }))).await;
        assert_eq!(status, StatusCode::OK);
    }
    sql.batch_execute(&format!(
        "UPDATE users SET is_bot = true WHERE id = '{bot}';"
    ))
    .await?;
    let channel = |owner: String| {
        let app = app.clone();
        async move {
            let id = uuid::Uuid::new_v4().to_string();
            let (status, body) = call(&app, "POST", "/api/channels", &owner, Some(json!({
                "id": id, "channel_name": format!("f-{}", uuid::Uuid::new_v4().simple()), "is_private": false,
            }))).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            id
        }
    };
    // alice は mine に所属し、other には所属しない（公開なので見える）。
    let mine = channel(alice.clone()).await;
    let other = channel(bob.clone()).await;
    let m = |n: u8| format!("00000000-0000-4000-9000-{n:012}");
    let insert = |id: &str, user: &str, channel: &str, parent: Option<&str>, ago: u8| {
        format!(
            "INSERT INTO messages (id, user_id, channel_id, parent_id, content, created_at) \
             VALUES ('{id}', '{user}', '{channel}', {parent}, '絞り込みの記録', LOCALTIMESTAMP - interval '{ago} minutes');",
            parent = parent.map_or("NULL".into(), |p| format!("'{p}'")),
        )
    };
    sql.batch_execute(
        &[
            insert(&m(1), &alice, &mine, None, 1),
            insert(&m(2), &carol, &mine, None, 2),
            insert(&m(3), &bot, &mine, None, 3),
            // other: alice が参加するスレッド（親と返信）、alice へのメンション、関係のない投稿。
            insert(&m(4), &bob, &other, None, 4),
            insert(&m(5), &bob, &other, Some(&m(4)), 5),
            insert(&m(6), &bob, &other, None, 6),
            insert(&m(7), &bob, &other, None, 7),
            // メンションされたスレッドの返信も入る。
            insert(&m(8), &carol, &other, Some(&m(6)), 8),
            // 所属しないチャンネルで、本文で alice にメンションした投稿（通知はメンバーにしか作られない）。
            insert(&m(9), &carol, &other, None, 9),
        ]
        .concat(),
    )
    .await?;
    sql.batch_execute(&format!(
        "UPDATE messages SET content = content || ' @' || (SELECT user_name FROM users WHERE id = '{alice}') WHERE id = '{}';",
        m(9)
    ))
    .await?;
    sql.batch_execute(&format!(
        "INSERT INTO thread_participants (thread_id, user_id) VALUES ('{t}', '{alice}') ON CONFLICT DO NOTHING;
         INSERT INTO notifications (id, user_id, actor_id, channel_id, message_id, notification_type)
           VALUES ('{n}', '{alice}', '{bob}', '{other}', '{mention}', 'mention');",
        t = m(4),
        n = uuid::Uuid::new_v4(),
        mention = m(6),
    ))
    .await?;
    let find = |extra: String| {
        let (app, alice) = (app.clone(), alice.clone());
        async move {
            let mut got = ids(&search(
                &app,
                &alice,
                &format!("type=messages&q={}&limit=50{extra}", q("絞り込み")),
            )
            .await);
            got.sort();
            got
        }
    };
    let sorted = |ns: &[u8]| {
        let mut v: Vec<String> = ns.iter().map(|&n| m(n)).collect();
        v.sort();
        v
    };
    assert_eq!(
        find(String::new()).await,
        sorted(&[1, 2, 3, 4, 5, 6, 7, 8, 9])
    );
    // 送信者とチャンネルは、複数のどれか。
    assert_eq!(
        find(format!("&user_id={alice},{carol}")).await,
        sorted(&[1, 2, 8, 9])
    );
    assert_eq!(
        find(format!("&channel_id={mine},{other}&user_id={bot}")).await,
        sorted(&[3])
    );
    // ボットを除く。
    assert_eq!(
        find("&exclude_bots=true".into()).await,
        sorted(&[1, 2, 4, 5, 6, 7, 8, 9])
    );
    // 自分が参加中の会話: 所属チャンネル、参加しているスレッド、メンションされたスレッド（通知）、
    // 所属しないチャンネルの本文のメンション。
    assert_eq!(
        find("&involving_me=true".into()).await,
        sorted(&[1, 2, 3, 4, 5, 6, 8, 9])
    );
    assert_eq!(
        find(format!(
            "&involving_me=true&exclude_bots=true&channel_id={other}"
        ))
        .await,
        sorted(&[4, 5, 6, 8, 9])
    );
    // 指定できる数には上限がある。
    let many: Vec<String> = (0..21).map(|i| format!("u{i}")).collect();
    let (status, _) = call(
        &app,
        "GET",
        &format!("/api/search?q=x&user_id={}", many.join(",")),
        &alice,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    fixture.cleanup().await?;
    Ok(())
}
