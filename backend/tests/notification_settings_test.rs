//! `GET/PATCH /api/channels/{id}/notification-settings` の契約 (#620)。
//! 既定値、権限 (404)、保存と再取得、不正値 (400)、同値 no-op の txid、
//! 他人の行を変更できないことを、スキーマを分けた DB で確認する。
#[path = "support/thread_db.rs"]
mod thread_db;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use backend::{db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tokio_postgres::NoTls;
use tower::ServiceExt;

async fn send(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<(StatusCode, HeaderMap, Value)> {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("X-Dev-User", user)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))?,
        )
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, headers, serde_json::from_slice(&bytes)?))
}

async fn ok(
    app: &Router,
    user: &str,
    method: &str,
    path: &str,
    body: Value,
) -> anyhow::Result<Value> {
    let (status, _, value) = send(app, user, method, path, body).await?;
    anyhow::ensure!(status.is_success(), "{method} {path}: {status} {value}");
    Ok(value)
}

#[tokio::test]
async fn notification_settings_round_trip_and_authorization() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let fixture = thread_db::ThreadDb::new().await?;
    let mut config = fixture.config.clone();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let owner = uuid::Uuid::new_v4().to_string();
    let member = uuid::Uuid::new_v4().to_string();
    let outsider = uuid::Uuid::new_v4().to_string();
    for user in [&owner, &member, &outsider] {
        ok(
            &app,
            user,
            "POST",
            "/api/users",
            json!({"id": user, "user_name": user, "display_name": user}),
        )
        .await?;
    }
    let channel = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/channels",
        json!({"id": channel, "channel_name": channel, "is_private": true}),
    )
    .await?;
    // 非公開チャンネルなので直接 join はできず、作成者が追加する。
    ok(
        &app,
        &owner,
        "POST",
        &format!("/api/channels/{channel}/users"),
        json!({"user_id": member}),
    )
    .await?;

    let path = format!("/api/channels/{channel}/notification-settings");

    // 既定値は mentions。読み取りは no-store。
    let (status, headers, value) = send(&app, &owner, "GET", &path, json!({})).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(value, json!({"level": "mentions"}));

    // 保存して再取得。書き込みは data/txid 形式で、 Shape 変更があるので txid が入る。
    let written = ok(&app, &owner, "PATCH", &path, json!({"level": "all"})).await?;
    assert_eq!(written["data"], json!({"level": "all"}));
    assert!(written["txid"].is_string());
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "all"})
    );
    let mut check = db.transaction().await?;
    let rows = backend::apps::query(
        &mut check,
        "SELECT json_build_object('level', s.notification_level, 'legacy', cu.notification_level)::text \
         FROM user_channel_notification_settings s \
         JOIN channel_user cu ON cu.user_id = s.user_id AND cu.channel_id = s.channel_id \
         WHERE s.user_id=$1 AND s.channel_id=$2",
        &[&owner, &channel],
    )
    .await?;
    check.commit().await?;
    assert_eq!(rows[0]["level"], "all");
    // 旧アプリが読む列には実際の設定を残さない。
    assert_eq!(rows[0]["legacy"], "mentions");

    // 同値の再送は書き込まず txid: null。
    let noop = ok(&app, &owner, "PATCH", &path, json!({"level": "all"})).await?;
    assert_eq!(noop["data"], json!({"level": "all"}));
    assert!(noop["txid"].is_null());

    // 不正値は 400 で、DB は変わらない。
    let (status, _, _) = send(&app, &owner, "PATCH", &path, json!({"level": "loud"})).await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "all"})
    );

    // 別のメンバーの設定は自分のものだけ変更でき、他人の行は変わらない。
    assert_eq!(
        ok(&app, &member, "GET", &path, json!({})).await?,
        json!({"level": "mentions"})
    );
    let member_written = ok(&app, &member, "PATCH", &path, json!({"level": "mute"})).await?;
    assert_eq!(member_written["data"], json!({"level": "mute"}));
    assert!(member_written["txid"].is_string());
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "all"})
    );

    // 非メンバー・存在しないチャンネル・不正 ID はすべて 404。
    for (user, path) in [
        (&outsider, path.clone()),
        (
            &owner,
            format!(
                "/api/channels/{}/notification-settings",
                uuid::Uuid::new_v4()
            ),
        ),
        (
            &owner,
            "/api/channels/not-a-uuid/notification-settings".to_owned(),
        ),
    ] {
        for method in ["GET", "PATCH"] {
            let (status, _, _) = send(&app, user, method, &path, json!({"level": "mute"})).await?;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
        }
    }

    // 削除済みチャンネルも 404。
    toasty::sql::statement("UPDATE channels SET deleted_at = now() WHERE id = $1")
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    let (status, _, _) = send(&app, &owner, "GET", &path, json!({})).await?;
    assert_eq!(status, StatusCode::NOT_FOUND);

    drop(app);
    fixture.cleanup().await
}

/// channel_user の Home refresh トリガーは membership を変える列だけに絞られ、
/// notification_level の UPDATE では発火しない (文レベルの UPDATE OF 条件)。
#[tokio::test]
async fn notification_level_update_does_not_fire_home_refresh_trigger() -> anyhow::Result<()> {
    let fixture = thread_db::ThreadDb::new().await?;
    let (client, connection) = tokio_postgres::connect(&fixture.config.database_url, NoTls).await?;
    tokio::spawn(connection);
    let rows = client
        .query(
            "SELECT pg_get_triggerdef(oid)::text FROM pg_trigger \
             WHERE tgrelid='channel_user'::regclass AND tgname='channel_user_home_live'",
            &[],
        )
        .await?;
    assert_eq!(rows.len(), 1);
    let definition: &str = rows[0].get(0);
    assert!(
        definition.contains("UPDATE OF user_id, channel_id, joined_at"),
        "{definition}"
    );
    assert!(!definition.contains("OR UPDATE ON"), "{definition}");
    fixture.cleanup().await
}

/// ロックを保持している backend の PID を、観測用の別接続から確認する。
/// 固定時間の待機ではなく `pg_blocking_pids` を見るので、競合経路を通っていない実行は失敗する。
async fn wait_until_patch_waits_on(
    observer: &tokio_postgres::Client,
    holder_pid: i32,
) -> anyhow::Result<()> {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let waiting: bool = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                observer.query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                     WHERE wait_event_type='Lock' AND $1=ANY(pg_blocking_pids(pid)))",
                    &[&holder_pid],
                ),
            )
            .await??
            .get(0);
            if waiting {
                return anyhow::Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "PATCH did not wait on the user_channel_notification_settings row lock held by {holder_pid}"
        )
    })?
}

/// 同時に同じ値へ PATCH されたとき、片方は UPDATE が0件になり txid: null の no-op になる。
/// 行ロックで UPDATE を待たせ、その間に別トランザクションで同じ値を確定させる。
#[tokio::test]
async fn concurrent_same_value_patch_is_a_noop() -> anyhow::Result<()> {
    let fixture = thread_db::ThreadDb::new().await?;
    let mut config = fixture.config.clone();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let owner = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/users",
        json!({"id": owner, "user_name": owner, "display_name": owner}),
    )
    .await?;
    let channel = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/channels",
        json!({"id": channel, "channel_name": channel, "is_private": true}),
    )
    .await?;
    let path = format!("/api/channels/{channel}/notification-settings");
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "mentions"})
    );

    let (mut client, connection) =
        tokio_postgres::connect(&fixture.config.database_url, NoTls).await?;
    tokio::spawn(connection);
    let holder_pid: i32 = client
        .query_one("SELECT pg_backend_pid()", &[])
        .await?
        .get(0);
    // 観測はアプリのプールとは別の接続で行う (プールの接続がロック待ちに使われても止まらないように)。
    let (observer, observer_connection) =
        tokio_postgres::connect(&fixture.config.database_url, NoTls).await?;
    tokio::spawn(observer_connection);
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT 1 FROM user_channel_notification_settings \
         WHERE user_id=$1 AND channel_id=$2 FOR UPDATE",
        &[&owner, &channel],
    )
    .await?;

    let patch_app = app.clone();
    let patch_owner = owner.clone();
    let patch_path = path.clone();
    let patch = tokio::spawn(async move {
        ok(
            &patch_app,
            &patch_owner,
            "PATCH",
            &patch_path,
            json!({"level": "all"}),
        )
        .await
    });
    // PATCH が古い値を読み、UPDATE で行ロックを待っていることを期限付きで確認する。
    // 確認できなければ競合経路を通っていないので失敗させ、ロックも解放する。
    if let Err(err) = wait_until_patch_waits_on(&observer, holder_pid).await {
        let _ = tx.rollback().await;
        return Err(err);
    }
    tx.execute(
        "UPDATE user_channel_notification_settings SET notification_level='all' \
         WHERE user_id=$1 AND channel_id=$2",
        &[&owner, &channel],
    )
    .await?;
    tx.commit().await?;

    let result = tokio::time::timeout(std::time::Duration::from_secs(20), patch)
        .await
        .map_err(|_| anyhow::anyhow!("PATCH did not finish within 20s"))???;
    assert_eq!(result["data"], json!({"level": "all"}));
    assert!(result["txid"].is_null());
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "all"})
    );

    drop(app);
    fixture.cleanup().await
}

/// ローリング更新中に旧バックエンドが古い列へ書いても、実際の設定は新しいテーブルにだけ入り、
/// メンバー一覧が配る古い列には残らない。参加・退出で設定行が追随することも確認する。
#[tokio::test]
async fn legacy_notification_level_write_is_mirrored_and_redacted() -> anyhow::Result<()> {
    let fixture = thread_db::ThreadDb::new().await?;
    let mut config = fixture.config.clone();
    config.dev_auth_enabled = true;
    let mut db = init_db(&config.database_url).await?;
    let app = create_router(AppState::new(db.clone(), config));

    let owner = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/users",
        json!({"id": owner, "user_name": owner, "display_name": owner}),
    )
    .await?;
    let channel = uuid::Uuid::new_v4().to_string();
    ok(
        &app,
        &owner,
        "POST",
        "/api/channels",
        json!({"id": channel, "channel_name": channel, "is_private": false}),
    )
    .await?;
    let path = format!("/api/channels/{channel}/notification-settings");
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "mentions"})
    );

    // 旧バックエンドの PATCH と同じ書き込み。トリガーが設定を移し、古い列は既定値へ戻す。
    toasty::sql::statement(
        "UPDATE channel_user SET notification_level='mute' WHERE user_id=$1 AND channel_id=$2",
    )
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "mute"})
    );
    let mut check = db.transaction().await?;
    let rows = backend::apps::query(
        &mut check,
        "SELECT json_build_object('level', s.notification_level, 'legacy', cu.notification_level)::text \
         FROM user_channel_notification_settings s \
         JOIN channel_user cu ON cu.user_id = s.user_id AND cu.channel_id = s.channel_id \
         WHERE s.user_id=$1 AND s.channel_id=$2",
        &[&owner, &channel],
    )
    .await?;
    check.commit().await?;
    assert_eq!(rows[0]["level"], "mute");
    assert_eq!(rows[0]["legacy"], "mentions");

    // 退出で参加行が消えると、設定行も消える。
    toasty::sql::statement("DELETE FROM channel_user WHERE user_id=$1 AND channel_id=$2")
        .bind(owner.as_str())
        .bind(channel.as_str())
        .exec(&mut db)
        .await?;
    let mut check = db.transaction().await?;
    let rows = backend::apps::query(
        &mut check,
        "SELECT json_build_object('n', count(*)::text)::text FROM user_channel_notification_settings \
         WHERE user_id=$1 AND channel_id=$2",
        &[&owner, &channel],
    )
    .await?;
    check.commit().await?;
    assert_eq!(rows[0]["n"], "0");

    // 旧アプリが明示レベル付きで参加行を作っても、設定は新しいテーブルに入り、
    // メンバー一覧が配る列は mentions のまま。
    toasty::sql::statement(
        "INSERT INTO channel_user(user_id,channel_id,joined_at,notification_level) \
         VALUES ($1,$2,now(),'all')",
    )
    .bind(owner.as_str())
    .bind(channel.as_str())
    .exec(&mut db)
    .await?;
    assert_eq!(
        ok(&app, &owner, "GET", &path, json!({})).await?,
        json!({"level": "all"})
    );
    let mut check = db.transaction().await?;
    let rows = backend::apps::query(
        &mut check,
        "SELECT json_build_object('level', s.notification_level, 'legacy', cu.notification_level)::text \
         FROM user_channel_notification_settings s \
         JOIN channel_user cu ON cu.user_id = s.user_id AND cu.channel_id = s.channel_id \
         WHERE s.user_id=$1 AND s.channel_id=$2",
        &[&owner, &channel],
    )
    .await?;
    check.commit().await?;
    assert_eq!(rows[0]["level"], "all");
    assert_eq!(rows[0]["legacy"], "mentions");

    drop(app);
    fixture.cleanup().await
}
