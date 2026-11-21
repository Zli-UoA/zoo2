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
    let all = search(&app, &alice, &format!("q={}&limit=50", q("会議"))).await;
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
        &format!("q={}&limit=50&cursor={}", q("会議"), q(cursor)),
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
    let bob_ids = ids(&search(&app, &bob, &format!("q={}&limit=50", q("会議"))).await);
    assert!(bob_ids.contains(&m(1)) && !bob_ids.contains(&m(4)));
    // ゲストは入っているチャンネルだけ（公開チャンネルも見えない）。
    assert_eq!(
        ids(&search(&app, &guest, &format!("q={}", q("会議"))).await),
        vec![m(4)]
    );

    // 1 文字、複数の語、英字の大文字・小文字。
    assert_eq!(
        ids(&search(&app, &alice, &format!("q={}", q("秘"))).await),
        vec![m(4)]
    );
    assert_eq!(
        ids(&search(&app, &alice, &format!("q={}", q("会議 資料"))).await),
        vec![m(1)]
    );
    assert_eq!(
        ids(&search(&app, &alice, "q=hello%20world").await),
        vec![m(2)]
    );
    // LIKE の特殊文字は文字どおりに扱う。
    assert!(ids(&search(&app, &alice, &format!("q={}", q("%"))).await).is_empty());

    // 同じ月の中の keyset ページング。
    let page1 = search(&app, &alice, &format!("q={}&limit=2", q("会議"))).await;
    assert_eq!(ids(&page1), vec![m(1), m(2)]);
    let c = page1["next_cursor"].as_str().unwrap();
    let page2 = search(
        &app,
        &alice,
        &format!("q={}&limit=2&cursor={}", q("会議"), q(c)),
    )
    .await;
    assert_eq!(ids(&page2), vec![m(3), m(4)]);

    // 絞り込み: チャンネル、見えないチャンネル、期間。
    assert_eq!(
        ids(&search(
            &app,
            &alice,
            &format!("q={}&channel_id={private}", q("会議"))
        )
        .await),
        vec![m(4)]
    );
    assert!(
        ids(&search(&app, &bob, &format!("q={}&channel_id={private}", q("会議"))).await).is_empty()
    );
    assert!(
        ids(&search(
            &app,
            &alice,
            &format!("q={}&channel_id={encrypted}", q("会議"))
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
        &format!("q={}&limit=50&after={today}", q("会議")),
    )
    .await);
    assert_eq!(recent, vec![m(1), m(2), m(3), m(4), m(9)]);

    // 投稿者で絞る。
    assert_eq!(
        ids(&search(&app, &alice, &format!("q={}&user_id={bob}", q("会議"))).await),
        vec![m(9)]
    );
    assert!(
        !ids(&search(
            &app,
            &alice,
            &format!("q={}&limit=50&user_id={alice}", q("会議"))
        )
        .await)
        .contains(&m(9))
    );
    // before はその日を含まない。月初なら前の月から探す（今月の行は出ない）。
    let month_start = sql
        .query_one(
            "SELECT date_trunc('month', LOCALTIMESTAMP)::date::text",
            &[],
        )
        .await?
        .get::<_, String>(0);
    let before_month = ids(&search(
        &app,
        &alice,
        &format!("q={}&limit=50&before={month_start}", q("会議")),
    )
    .await);
    assert!(
        before_month.contains(&m(7)) && !before_month.contains(&m(1)),
        "{before_month:?}"
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
            &format!("q={}&limit=50&before={tomorrow}", q("会議"))
        )
        .await)
        .contains(&m(1))
    );

    // ファイル名とノート。
    sql.batch_execute(&format!(
        "INSERT INTO attachments (id, message_id, uploader_id, channel_id, file_name, file_size, content_type, storage_key)
           VALUES ('a1', '{m1}', '{alice}', '{public}', '議事録_2026.PDF', 10, 'application/pdf', 'k1'),
                  ('a2', '{m6}', '{alice}', '{public}', '議事録_消した.pdf', 10, 'application/pdf', 'k2');
         INSERT INTO channel_notes (id, channel_id, title, content, created_user_id)
           VALUES ('n1', '{public}', '運用メモ', '障害対応の手順', '{alice}'),
                  ('n2', '{private}', '秘密のメモ', '障害の記録', '{alice}');",
        m1 = m(1), m6 = m(6)
    )).await?;
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
        ids(&search(&app, &alice, &format!("type=files&q=pdf&user_id={bob}")).await).is_empty()
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
    let hit = search(&app, &alice, &format!("q={}", q("末尾の会議"))).await;
    assert!(
        hit["items"][0]["snippet"]
            .as_str()
            .unwrap()
            .contains("末尾の会議"),
        "{hit}"
    );

    // 入力の検証。
    let bad_cursor = format!("q=x&cursor={}", q(r#"{"m":3000000000}"#));
    for bad in [
        bad_cursor.as_str(),
        "q=",
        "q=a%20b%20c%20d%20e%20f",
        "q=x&cursor=nope",
        "q=x&after=2026-13-01",
        "q=x&type=users",
    ] {
        let (status, _) = call(&app, "GET", &format!("/api/search?{bad}"), &alice, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
    }

    fixture.cleanup().await?;
    Ok(())
}
