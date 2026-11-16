//! ユーザーごとのチャンネル・DM のピン留めと並び替えの結合テスト。
//!
//! ピンは本人にだけ見え、並びはユーザーごとに独立している。退出・キック・
//! チャンネル削除では同じトランザクションでピンを片付ける。アーカイブ中の
//! ピンは残し、アンアーカイブで元の位置に戻る。

use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode},
};
use backend::{config::Config, db::init_db, routes::create_router, state::AppState};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Env {
    app: Router,
    db: toasty::Db,
}

async fn env() -> Env {
    dotenvy::dotenv().ok();
    dotenvy::from_filename("../.env").ok();
    let mut config = Config::from_env();
    config.dev_auth_enabled = true;
    let db = init_db(&config.database_url).await.unwrap();
    let app = create_router(AppState::new(db.clone(), config));
    Env { app, db }
}

impl Env {
    async fn call(
        &self,
        method: &str,
        path: &str,
        actor: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let (status, _, body) = self.call_with_headers(method, path, actor, body).await;
        (status, body)
    }

    async fn call_with_headers(
        &self,
        method: &str,
        path: &str,
        actor: Option<&str>,
        body: Value,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(actor) = actor {
            request = request.header("X-Dev-User", actor);
        }
        let response = self
            .app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, headers, body)
    }

    async fn user(&self) -> String {
        let id = Uuid::new_v4().to_string();
        let payload = json!({
            "id": id,
            "user_name": format!("pin_{}", Uuid::new_v4().simple()),
            "display_name": "pin",
        });
        let (status, _) = self.call("POST", "/api/users", None, payload).await;
        assert_eq!(status, StatusCode::OK);
        id
    }

    async fn set_guest(&self, user: &str) {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.unwrap();
        toasty::sql::statement("UPDATE users SET guest_kind = 'multi' WHERE id = $1")
            .bind(user)
            .exec(&mut tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
    }

    async fn channel(&self, creator: &str, private: bool, members: &[&str]) -> String {
        let id = Uuid::new_v4().to_string();
        let (status, _) = self
            .call(
                "POST",
                "/api/channels",
                Some(creator),
                json!({
                    "id": id,
                    "channel_name": format!("pins-{}", Uuid::new_v4().simple()),
                    "is_private": private,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        for member in members {
            let (status, _) = self
                .call(
                    "POST",
                    &format!("/api/channels/{id}/users"),
                    Some(creator),
                    json!({ "user_id": member }),
                )
                .await;
            assert_eq!(status, StatusCode::OK);
        }
        id
    }

    async fn dm(&self, actor: &str, target: &str) -> String {
        let (status, body) = self
            .call(
                "POST",
                "/api/dms",
                Some(actor),
                json!({ "target_user_ids": [target] }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["data"]["id"].as_str().unwrap().to_owned()
    }

    async fn pin(&self, actor: &str, channel: &str, expected: Option<i64>) -> (StatusCode, Value) {
        let mut body = json!({ "channel_id": channel });
        if let Some(expected) = expected {
            body["expected_version"] = expected.into();
        }
        self.call("POST", "/api/channel-pins", Some(actor), body)
            .await
    }

    async fn unpin(&self, actor: &str, channel: &str) -> (StatusCode, Value) {
        self.call(
            "DELETE",
            &format!("/api/channel-pins/{channel}"),
            Some(actor),
            Value::Null,
        )
        .await
    }

    async fn pins(&self, actor: &str) -> Value {
        let (status, body) = self
            .call("GET", "/api/channel-pins", Some(actor), Value::Null)
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn reorder(
        &self,
        actor: &str,
        channel_ids: &[&str],
        expected: i64,
    ) -> (StatusCode, Value) {
        self.call(
            "PATCH",
            "/api/channel-pins/order",
            Some(actor),
            json!({ "channel_ids": channel_ids, "expected_version": expected }),
        )
        .await
    }
}

fn pinned_ids(body: &Value) -> Vec<String> {
    body["pins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pin| pin["channel_id"].as_str().unwrap().to_owned())
        .collect()
}

/// ロックを持つトランザクションの接続の PID。待っている問い合わせを、この PID に結び付ける。
async fn backend_pid(tx: &mut toasty::Transaction<'_>) -> String {
    let rows = toasty::sql::query("SELECT pg_backend_pid()::text")
        .exec(tx)
        .await
        .unwrap();
    for row in rows {
        if let toasty::stmt::Value::Record(record) = row
            && let Some(toasty::stmt::Value::String(pid)) = record.fields.into_iter().next()
        {
            return pid;
        }
    }
    panic!("pg_backend_pid returned no row");
}

/// ピンの閲覧確認が `holder_pid` のチャンネル行ロックで待つまで待つ。
async fn wait_until_pin_request_blocked(db: &mut toasty::Db, holder_pid: &str) {
    for _ in 0..100 {
        let rows = toasty::sql::query(
            "SELECT count(*)::text FROM pg_stat_activity \
             WHERE datname = current_database() AND pid <> pg_backend_pid() \
               AND $1::int = ANY(pg_blocking_pids(pid)) \
               AND query ILIKE '%FOR SHARE OF c%'",
        )
        .bind(holder_pid.to_string())
        .exec(&mut *db)
        .await
        .unwrap();
        let blocked = rows.into_iter().next().is_some_and(|row| {
            matches!(row, toasty::stmt::Value::Record(record)
                if matches!(record.fields.first(), Some(toasty::stmt::Value::String(count)) if count != "0"))
        });
        if blocked {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("pin request did not block on the channel row lock held by {holder_pid}");
}

#[tokio::test]
async fn pins_list_is_not_cached() {
    let env = env().await;
    let alice = env.user().await;

    let (status, headers, body) = env
        .call_with_headers("GET", "/api/channel-pins", Some(&alice), Value::Null)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // 端末ごとに版と並びが変わるので、プロキシやブラウザーに残させない。
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(body, json!({ "pins": [], "version": 0 }));
}

#[tokio::test]
async fn pin_unpin_and_reorder_are_ordered_and_versioned() {
    let env = env().await;
    let alice = env.user().await;
    let first = env.channel(&alice, false, &[]).await;
    let second = env.channel(&alice, false, &[]).await;

    let (status, body) = env.pin(&alice, &second, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["version"], 1);
    assert_eq!(pinned_ids(&body["data"]), vec![second.clone()]);

    let (status, body) = env.pin(&alice, &first, Some(1)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["version"], 2);
    assert_eq!(
        pinned_ids(&body["data"]),
        vec![second.clone(), first.clone()]
    );
    assert_eq!(body["data"]["pins"][1]["position"], 1);

    // 同じチャンネルの再送は並びを変えない（txid なし）。
    let (status, body) = env.pin(&alice, &first, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["version"], 2);
    assert!(body["txid"].is_null());

    // 古い版での書き込みは 409 で最新の版とピンを返す。
    let (status, body) = env.pin(&alice, &second, Some(1)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "CHANNEL_PINS_CONFLICT");
    assert_eq!(body["version"], 2);
    assert_eq!(pinned_ids(&body), vec![second.clone(), first.clone()]);

    // 並び替え。要求した順に 0 から振り直す。
    let (status, body) = env.reorder(&alice, &[&first, &second], 2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["version"], 3);
    assert_eq!(
        pinned_ids(&body["data"]),
        vec![first.clone(), second.clone()]
    );

    // 古い版での並び替えは 409。
    let (status, body) = env.reorder(&alice, &[&second, &first], 2).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["version"], 3);
    assert_eq!(pinned_ids(&body), vec![first.clone(), second.clone()]);

    // ピンしていないチャンネルを並び替えで足せない。
    let third = env.channel(&alice, false, &[]).await;
    let (status, body) = env.reorder(&alice, &[&first, &second, &third], 3).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["version"], 3);

    // 解除すると位置を詰める。
    let (status, body) = env.unpin(&alice, &first).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["removed"], true);
    assert_eq!(body["data"]["version"], 4);
    assert_eq!(pinned_ids(&body["data"]), vec![second.clone()]);
    assert_eq!(body["data"]["pins"][0]["position"], 0);

    // 再度の解除は何も変えず、txid も返さない。
    let (status, body) = env.unpin(&alice, &first).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["removed"], false);
    assert_eq!(body["data"]["version"], 4);
    assert!(body["txid"].is_null());

    // 一覧は位置順で、版を返す。
    let body = env.pins(&alice).await;
    assert_eq!(body["version"], 4);
    assert_eq!(pinned_ids(&body), vec![second]);
}

#[tokio::test]
async fn pins_are_per_user_and_hide_inaccessible_channels() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let public = env.channel(&alice, false, &[]).await;
    let private = env.channel(&alice, true, &[&bob]).await;
    let dm = env.dm(&alice, &bob).await;

    // Alice は 3 つ、Bob は自分に見える 2 つをピンできる。
    assert_eq!(env.pin(&alice, &public, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&alice, &private, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&alice, &dm, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&bob, &public, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&bob, &dm, None).await.0, StatusCode::OK);

    assert_eq!(pinned_ids(&env.pins(&alice).await).len(), 3);
    assert_eq!(pinned_ids(&env.pins(&bob).await).len(), 2);

    // 参加していない非公開チャンネルは、存在を隠して 404。
    let outsider = env.user().await;
    let (status, body) = env.pin(&outsider, &private, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, body) = env.pin(&outsider, &dm, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // 不正な UUID は 400。
    let (status, body) = env.pin(&alice, "not-a-uuid", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // 重複した並びは 400。
    let (status, body) = env.reorder(&alice, &[&public, &public], 3).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn archived_pins_are_preserved_and_cannot_be_pinned_again() {
    let env = env().await;
    let alice = env.user().await;
    let first = env.channel(&alice, false, &[]).await;
    let second = env.channel(&alice, false, &[]).await;
    assert_eq!(env.pin(&alice, &first, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&alice, &second, None).await.0, StatusCode::OK);

    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{first}/archive"),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // アーカイブ中のピンは一覧に残る（`archived` 付き）が、新しくピンはできない。
    let body = env.pins(&alice).await;
    assert_eq!(pinned_ids(&body), vec![first.clone(), second.clone()]);
    assert_eq!(body["pins"][0]["archived"], true);
    assert_eq!(body["pins"][1]["archived"], false);
    let (status, _) = env.pin(&alice, &first, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // 並び替えは有効なピンだけを対象にし、アーカイブ中のピンは元のスロットに残す。
    let (status, body) = env.reorder(&alice, &[&second], 2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        pinned_ids(&body["data"]),
        vec![first.clone(), second.clone()]
    );
    assert_eq!(body["data"]["pins"][0]["position"], 0);
    assert_eq!(body["data"]["pins"][1]["position"], 1);

    // アンアーカイブすると元の位置に戻る。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{first}/unarchive"),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let body = env.pins(&alice).await;
    assert_eq!(pinned_ids(&body), vec![first.clone(), second.clone()]);
}

#[tokio::test]
async fn reorder_keeps_a_middle_archived_pin_in_place() {
    let env = env().await;
    let alice = env.user().await;
    let first = env.channel(&alice, false, &[]).await;
    let middle = env.channel(&alice, false, &[]).await;
    let last = env.channel(&alice, false, &[]).await;
    for channel in [&first, &middle, &last] {
        assert_eq!(env.pin(&alice, channel, None).await.0, StatusCode::OK);
    }

    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{middle}/archive"),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);

    // 有効なピン（first, last）を入れ替えても、真ん中のアーカイブ中ピンは動かない。
    let (status, body) = env.reorder(&alice, &[&last, &first], 3).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        pinned_ids(&body["data"]),
        vec![last.clone(), middle.clone(), first.clone()]
    );
    assert_eq!(body["data"]["pins"][1]["archived"], true);

    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{middle}/unarchive"),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let body = env.pins(&alice).await;
    assert_eq!(pinned_ids(&body), vec![last, middle, first]);
}

#[tokio::test]
async fn leaving_kicking_and_deleting_remove_pins() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.channel(&alice, false, &[&bob]).await;

    assert_eq!(env.pin(&alice, &channel, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&bob, &channel, None).await.0, StatusCode::OK);

    // 退出すると本人のピンだけが外れる。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/leave"),
            Some(&bob),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(pinned_ids(&env.pins(&bob).await).is_empty());
    assert_eq!(pinned_ids(&env.pins(&alice).await), vec![channel.clone()]);

    // 再参加しても、退出で消したピンは復活しない。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/join"),
            Some(&bob),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(pinned_ids(&env.pins(&bob).await).is_empty());

    // キックでも対象ユーザーのピンが外れる。
    assert_eq!(env.pin(&bob, &channel, None).await.0, StatusCode::OK);
    let (status, _) = env
        .call(
            "DELETE",
            &format!("/api/channels/{channel}/users/{bob}"),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert!(pinned_ids(&env.pins(&bob).await).is_empty());
    assert_eq!(pinned_ids(&env.pins(&alice).await), vec![channel.clone()]);

    // 複数ユーザーがピン留めした状態でチャンネルを削除すると、全員のピンが外れる。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{channel}/join"),
            Some(&bob),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(env.pin(&bob, &channel, None).await.0, StatusCode::OK);
    assert_eq!(pinned_ids(&env.pins(&bob).await), vec![channel.clone()]);
    assert_eq!(pinned_ids(&env.pins(&alice).await), vec![channel.clone()]);

    let (status, body) = env
        .call(
            "DELETE",
            &format!("/api/channels/{channel}"),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(pinned_ids(&env.pins(&alice).await).is_empty());
    assert!(pinned_ids(&env.pins(&bob).await).is_empty());
}

/// 退出とピン留めが同時に走っても、退出後にピンが残らない。
///
/// 退出側がチャンネル行を `FOR UPDATE` でロックしてメンバー行を消し、同じトランザクションで
/// 状態行をロックする。ピン側は古い参加状態を見たままチャンネル行のロックで待つため、
/// ロック取得後に参加状態を読み直さないと、退出後のピンを作れてしまう。
#[tokio::test]
async fn concurrent_leave_and_pin_do_not_resurrect_the_pin() {
    let env = env().await;
    let alice = env.user().await;
    let bob = env.user().await;
    let channel = env.channel(&alice, true, &[&bob]).await;

    let mut db_for_leave = env.db.clone();
    let mut leave = db_for_leave.transaction().await.unwrap();
    let holder_pid = backend_pid(&mut leave).await;
    toasty::sql::query("SELECT 1 FROM channels WHERE id = $1 FOR UPDATE")
        .bind(&channel)
        .exec(&mut leave)
        .await
        .unwrap();
    toasty::sql::statement("DELETE FROM channel_user WHERE channel_id = $1 AND user_id = $2")
        .bind(&channel)
        .bind(&bob)
        .exec(&mut leave)
        .await
        .unwrap();
    toasty::sql::statement(
        "INSERT INTO user_channel_pin_states (user_id, version) VALUES ($1, 0) \
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(&bob)
    .exec(&mut leave)
    .await
    .unwrap();
    toasty::sql::query("SELECT version FROM user_channel_pin_states WHERE user_id = $1 FOR UPDATE")
        .bind(&bob)
        .exec(&mut leave)
        .await
        .unwrap();

    let app = env.app.clone();
    let pin_channel = channel.clone();
    let pin_user = bob.clone();
    let pin = tokio::spawn(async move {
        let request = Request::builder()
            .method("POST")
            .uri("/api/channel-pins")
            .header("content-type", "application/json")
            .header("X-Dev-User", pin_user)
            .body(Body::from(json!({ "channel_id": pin_channel }).to_string()))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice::<Value>(&body).unwrap_or(Value::Null),
        )
    });

    let mut waiter = env.db.clone();
    wait_until_pin_request_blocked(&mut waiter, &holder_pid).await;

    // 退出をコミットすると、待っていたピン留めは参加状態を読み直して 404 になる。
    leave.commit().await.unwrap();
    let (status, body) = pin.await.unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // 退出後にピン行が残っていない（再参加しても復活しない）。
    let mut db = env.db.clone();
    let mut tx = db.transaction().await.unwrap();
    let rows = toasty::sql::query(
        "SELECT 1 FROM user_pinned_channels WHERE user_id = $1 AND channel_id = $2",
    )
    .bind(&bob)
    .bind(&channel)
    .exec(&mut tx)
    .await
    .unwrap();
    assert!(rows.is_empty());
}

#[tokio::test]
async fn pin_limit_is_enforced() {
    let env = env().await;
    let alice = env.user().await;
    let mut created = Vec::with_capacity(51);

    // 50 件まではピンできる。
    for _ in 0..50 {
        let id = Uuid::new_v4().to_string();
        let (status, _) = env
            .call(
                "POST",
                "/api/channels",
                Some(&alice),
                json!({
                    "id": id,
                    "channel_name": format!("limit-{}", Uuid::new_v4().simple()),
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let (status, body) = env.pin(&alice, &id, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        created.push(id);
    }

    // 1 件アーカイブしても、アーカイブ中のピンも数えるため 51 件目は coded 400 で拒否する。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{}/archive", created[0]),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let overflow = Uuid::new_v4().to_string();
    let (status, _) = env
        .call(
            "POST",
            "/api/channels",
            Some(&alice),
            json!({
                "id": overflow,
                "channel_name": format!("limit-{}", Uuid::new_v4().simple()),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = env.pin(&alice, &overflow, None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "CHANNEL_PINS_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], 50);
    assert_eq!(pinned_ids(&env.pins(&alice).await).len(), 50);

    // アンアーカイブしても、保持しているピンは 50 件のままで増えない。
    let (status, _) = env
        .call(
            "POST",
            &format!("/api/channels/{}/unarchive", created[0]),
            Some(&alice),
            Value::Null,
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(pinned_ids(&env.pins(&alice).await).len(), 50);
    assert_eq!(created.len(), 50);
}

#[tokio::test]
async fn reorder_rejects_more_ids_than_the_limit() {
    let env = env().await;
    let alice = env.user().await;
    let ids: Vec<String> = (0..=backend::routes::channel_pins::MAX_PINNED_CHANNELS)
        .map(|_| Uuid::new_v4().to_string())
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let (status, body) = env.reorder(&alice, &refs, 0).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["code"], "CHANNEL_PINS_LIMIT_EXCEEDED");
    assert_eq!(body["limit"], 50);
}

#[tokio::test]
async fn guests_must_join_before_pinning_and_cannot_reorder_hidden_channels() {
    let env = env().await;
    let alice = env.user().await;
    let guest = env.user().await;
    env.set_guest(&guest).await;
    let public = env.channel(&alice, false, &[]).await;
    let joined = env.channel(&alice, false, &[&guest]).await;
    let private = env.channel(&alice, true, &[&guest]).await;
    let dm = env.dm(&alice, &guest).await;

    // 未参加の公開チャンネルは、ゲストには存在を隠して 404。
    let (status, body) = env.pin(&guest, &public, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(pinned_ids(&env.pins(&guest).await).is_empty());

    // 参加済みの公開・非公開・DM はピンできる。
    for channel in [&joined, &private, &dm] {
        let (status, body) = env.pin(&guest, channel, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(pinned_ids(&env.pins(&guest).await).len(), 3);

    // 権限のないチャンネルを並び替えに混ぜると 404。
    let hidden = env.channel(&alice, true, &[]).await;
    let (status, body) = env
        .reorder(&guest, &[&joined, &hidden, &private, &dm], 3)
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // 参加済みのピンだけなら並び替えられる。
    let (status, body) = env.reorder(&guest, &[&dm, &private, &joined], 3).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(pinned_ids(&body["data"]), vec![dm, private, joined]);
}

#[tokio::test]
async fn stale_reorder_conflict_can_be_retried_with_the_latest_version() {
    let env = env().await;
    let alice = env.user().await;
    let first = env.channel(&alice, false, &[]).await;
    let second = env.channel(&alice, false, &[]).await;
    assert_eq!(env.pin(&alice, &first, None).await.0, StatusCode::OK);
    assert_eq!(env.pin(&alice, &second, None).await.0, StatusCode::OK);

    // 別端末が先に並び替えた（版が進んだ）状態で、古い版から再試行する。
    let (status, body) = env.reorder(&alice, &[&second, &first], 2).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let latest_version = body["data"]["version"].as_i64().unwrap();
    let (status, body) = env.reorder(&alice, &[&first, &second], 2).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["code"], "CHANNEL_PINS_CONFLICT");
    assert_eq!(body["version"], latest_version);
    let (status, body) = env
        .reorder(&alice, &[&first, &second], latest_version)
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(pinned_ids(&body["data"]), vec![first, second]);
}

#[tokio::test]
async fn uppercase_and_unhyphenated_uuids_normalize_to_canonical_channel_ids() {
    let env = env().await;
    let alice = env.user().await;
    let lowercase = Uuid::new_v4().to_string();
    let uppercase = lowercase.to_uppercase();
    let unhyphenated = lowercase.replace('-', "").to_uppercase();

    // 大文字 UUID を指定してチャンネル作成しても、小文字ハイフン付きで保存される。
    let (status, body) = env
        .call(
            "POST",
            "/api/channels",
            Some(&alice),
            json!({
                "id": uppercase,
                "channel_name": format!("upper-{}", Uuid::new_v4().simple()),
            }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], lowercase);

    // 大文字 UUID でのピン留め・ハイフンなし UUID での並び替え・大文字 UUID での解除がすべて一致する。
    let (status, body) = env.pin(&alice, &uppercase, None).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["pin"]["channel_id"], lowercase);

    let (status, body) = env.reorder(&alice, &[&unhyphenated], 1).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(pinned_ids(&body["data"]), vec![lowercase.clone()]);

    let (status, body) = env.unpin(&alice, &uppercase).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["removed"], true);
    assert!(pinned_ids(&env.pins(&alice).await).is_empty());
}
