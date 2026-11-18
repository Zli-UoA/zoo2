//! 連携した Slack アカウントで、自分が作った非公開チャンネルを取り込む。
//!
//! エクスポートの ZIP は使わず、本人のトークンで Slack の API (`users.conversations`・`conversations.info`・
//! `conversations.members`) を読む。履歴そのものは、差分同期 (`sync.rs`) のエンジンが全履歴から取る。
//! 取り込みを始めるときに、非公開の zoo チャンネルと `slack_channels` の対応を作り、同期カーソル (`last_ts`) を
//! `0` にして同期のキューに入れる (カーソルのないチャンネルは、同期が勝手に全履歴を取らないよう、対象外にしている)。
//!
//! - 取り込めるのは、本人が作った非公開チャンネルだけ (DM・グループ DM は含まない)。Slack の一覧には、本人が
//!   参加しているチャンネルしか出ない。
//! - 新しい zoo チャンネルは常に非公開で作る。作成者とメンバーの扱いは、エクスポートの取り込みと同じ。
//! - 添付ファイルは、差分同期と同じく、ファイル名を本文に残すだけで、取得はしない。

use std::collections::{HashMap, HashSet};

use axum::{
    Json, Router,
    extract::State,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    export::{SlackChannel, SlackUser},
    sync::{self, Slack, SyncError},
    worker::{
        Context, FilePolicy, create_channel, exec, lock_zip_target_advisory, suppress_app_events,
    },
};
use crate::{apps::query, auth::AuthUser, error::AppError, state::AppState};

/// 一覧・メンバーの取得で辿るページ数の上限 (1 ページ 200 件)。超えたら、不完全な結果を確定せず、エラーにする
const MAX_PAGES: usize = 50;
/// メンバーの名前を `users.info` で調べる人数の上限
const MAX_PROFILE_LOOKUPS: usize = 50;
/// 名前の取得にかけてよい時間
const PROFILE_BUDGET: std::time::Duration = std::time::Duration::from_secs(60);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/slack/private-channels", get(list))
        .route("/api/slack/private-channels/import", post(start))
        .layer(axum::middleware::map_response(crate::apps::no_store))
}

/// API imports and ZIP imports must serialize placeholder creation in the same stable ID order.
///
/// Locking contract: when this returns, the calling transaction already holds the link lock,
/// a per-placeholder advisory lock for every resolved Slack member, and a `users` row lock
/// (`FOR NO KEY UPDATE`) for every resolved or created member Zoo ID **plus the creator**
/// (`additional_user_ids`), all taken once in sorted ID order. Callers must not re-lock those
/// rows with `lock_membership_users`. Callers must re-check the creator's guest state only
/// after this returns, because that read is stable only once the users rows are locked.
async fn resolve_members(
    ctx: &mut Context,
    tx: &mut toasty::Transaction<'_>,
    members: &[String],
    creator_id: &str,
) -> Result<Vec<super::worker::ZooUser>, AppError> {
    super::worker::channel_users_with_additional_users(ctx, tx, members, &[creator_id]).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, time::Duration};

    #[derive(Clone, Copy)]
    enum Path {
        Api,
        Zip,
    }

    async fn resolve(
        path: Path,
        ctx: &mut Context,
        tx: &mut toasty::Transaction<'_>,
        members: &[String],
    ) -> Result<Vec<super::super::worker::ZooUser>, AppError> {
        match path {
            Path::Api => super::resolve_members(ctx, tx, members, "").await,
            Path::Zip => super::super::worker::channel_users(ctx, tx, members).await,
        }
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn api_api_and_zip_api_imports_resolve_users_in_a_shared_stable_order()
    -> anyhow::Result<()> {
        dotenvy::dotenv().ok();
        dotenvy::from_filename("../.env").ok();
        let database_url = std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
        let (guard, driver) = tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(driver);
        let database: String = guard
            .query_one("SELECT current_database()", &[])
            .await?
            .get(0);
        anyhow::ensure!(database.contains("test"), "refusing non-test database");
        let db = crate::db::init_db(&database_url).await?;
        let mut fixture_teams = Vec::new();
        let test_result: anyhow::Result<()> = async {
            for (case, paths) in [
                ("api-api", [Path::Api, Path::Api]),
                ("zip-api", [Path::Zip, Path::Api]),
            ] {
                let team = format!("TAPI{}", uuid::Uuid::new_v4().simple());
                fixture_teams.push(team.clone());
                let suffix = uuid::Uuid::new_v4().simple().to_string();
                let mut members = [format!("U-A-{suffix}"), format!("U-B-{suffix}")];
                members.sort_unstable_by_key(|id| super::super::user_id(&team, id));
                let reverse = vec![members[1].clone(), members[0].clone()];
                let forward = vec![members[0].clone(), members[1].clone()];
                let gate = std::sync::Arc::new(tokio::sync::Barrier::new(2));
                let mut tasks = tokio::task::JoinSet::new();
                for (index, path) in paths.into_iter().enumerate() {
                    let mut task_db = db.clone();
                    let task_team = team.clone();
                    let task_gate = gate.clone();
                    let task_members = if index == 0 {
                        reverse.clone()
                    } else {
                        forward.clone()
                    };
                    tasks.spawn(async move {
                        let mut tx = task_db.transaction().await?;
                        toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'")
                            .exec(&mut tx)
                            .await?;
                        let mut ctx = Context {
                            team: task_team,
                            users: HashMap::new(),
                            profiles: HashMap::new(),
                            fallback_names: HashMap::new(),
                            channel_names: HashMap::new(),
                            files: FilePolicy {
                                token: false,
                                max_size: 0,
                                base_url: String::new(),
                            },
                            app_id: None,
                        };
                        task_gate.wait().await;
                        let resolved = resolve(path, &mut ctx, &mut tx, &task_members)
                            .await
                            .map_err(|error| anyhow::anyhow!("{case}: {error:?}"))?;
                        let mut ids = resolved.into_iter().map(|user| user.id).collect::<Vec<_>>();
                        ids.sort_unstable();
                        tx.commit().await?;
                        Ok::<_, anyhow::Error>(ids)
                    });
                }
                let mut results = Vec::new();
                for _ in 0..2 {
                    let completed = tokio::time::timeout(Duration::from_secs(5), tasks.join_next())
                        .await?
                        .ok_or_else(|| {
                            anyhow::anyhow!("concurrent import task exited without a result")
                        })?;
                    results.push(completed??);
                }
                anyhow::ensure!(
                    results[0] == results[1],
                    "{case}: import user mappings diverged"
                );
                anyhow::ensure!(
                    results[0].len() == 2,
                    "{case}: both placeholders must be resolved"
                );
            }
            Ok(())
        }
        .await;
        // Teams are unique to this test. Delete imported Slack mappings and let the
        // users FK cascade clean every placeholder, including after task errors/timeouts.
        let mut cleanup_db = db.clone();
        let mut cleanup = cleanup_db.transaction().await?;
        for team in fixture_teams {
            toasty::sql::statement(
                "DELETE FROM users WHERE id IN (SELECT user_id FROM slack_users WHERE team_id=$1)",
            )
            .bind(team.as_str())
            .exec(&mut cleanup)
            .await?;
        }
        cleanup.commit().await?;
        test_result
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn api_creator_lock_does_not_invert_member_order_against_e2ee() -> anyhow::Result<()> {
        dotenvy::dotenv().ok();
        dotenvy::from_filename("../.env").ok();
        let database_url = std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
        let (guard, guard_driver) =
            tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(guard_driver);
        let database: String = guard
            .query_one("SELECT current_database()", &[])
            .await?
            .get(0);
        anyhow::ensure!(
            database.contains("test"),
            "refusing non-test database `{database}`"
        );
        let db = crate::db::init_db(&database_url).await?;
        let team = format!("TORDER{}", uuid::Uuid::new_v4().simple());
        let slack_id = format!(
            "U{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .to_ascii_uppercase()
        );
        let member_id = super::super::user_id(&team, &slack_id);
        let creator_id = format!(
            "00000000-0000-0000-0000-{}",
            &uuid::Uuid::new_v4().simple().to_string()[20..]
        );
        anyhow::ensure!(
            creator_id < member_id,
            "fixture must put the creator before the member in lock order"
        );
        let setup: anyhow::Result<()> = async {
            let mut setup_db = db.clone();
            let mut tx = setup_db.transaction().await?;
            toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Creator')")
                .bind(creator_id.as_str())
                .bind(format!("api_creator_{}", uuid::Uuid::new_v4().simple()))
                .exec(&mut tx)
                .await?;
            toasty::sql::statement("INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'Member','deactivated')")
                .bind(member_id.as_str())
                .bind(format!("api_member_{}", uuid::Uuid::new_v4().simple()))
                .exec(&mut tx)
                .await?;
            toasty::sql::statement("INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name,profile_source) VALUES($1,$2,$3,false,$2,'','fallback')")
                .bind(team.as_str())
                .bind(slack_id.as_str())
                .bind(member_id.as_str())
                .exec(&mut tx)
                .await?;
            tx.commit().await?;
            Ok(())
        }.await;
        if let Err(error) = setup {
            cleanup_fixture(&db, &team, &creator_id).await?;
            return Err(error);
        }

        let test_result: anyhow::Result<()> = async {
            // Reproduce the previous API order: members first, creator afterward. E2EE
            // already owns the lower-ID creator and then asks for the higher-ID member.
            let (e2ee_connection, e2ee_driver) = tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
            tokio::spawn(e2ee_driver);
            let mut e2ee_connection = e2ee_connection;
            let e2ee_tx = e2ee_connection.transaction().await?;
            e2ee_tx.batch_execute("SET LOCAL deadlock_timeout='100ms'").await?;
            e2ee_tx.query_one("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE", &[&creator_id]).await?;

            let mut baseline_tasks = tokio::task::JoinSet::new();
            let baseline_db = db.clone();
            let baseline_team = team.clone();
            let baseline_slack = slack_id.clone();
            let baseline_creator = creator_id.clone();
            let (member_locked_tx, member_locked_rx) = tokio::sync::oneshot::channel();
            baseline_tasks.spawn(async move {
                let mut db = baseline_db;
                let mut tx = db.transaction().await?;
                toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'").exec(&mut tx).await?;
                let mut ctx = Context {
                    team: baseline_team, users: HashMap::new(), profiles: HashMap::new(),
                    fallback_names: HashMap::new(), channel_names: HashMap::new(),
                    files: FilePolicy { token: false, max_size: 0, base_url: String::new() }, app_id: None,
                };
                let users = super::super::worker::channel_users(&mut ctx, &mut tx, std::slice::from_ref(&baseline_slack)).await?;
                let _ = member_locked_tx.send(());
                let ids = [users[0].id.as_str(), baseline_creator.as_str()];
                crate::routes::e2ee_channels::lock_membership_users(&mut tx, &ids).await?;
                tx.commit().await?;
                Ok::<_, anyhow::Error>(())
            });
            tokio::time::timeout(Duration::from_secs(5), member_locked_rx).await??;
            let e2ee_result = e2ee_tx.query_one("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE", &[&member_id]).await;
            let reproduced = e2ee_result.as_ref().err().and_then(tokio_postgres::Error::code)
                .is_some_and(|code| code == &tokio_postgres::error::SqlState::T_R_DEADLOCK_DETECTED);
            let api_result = tokio::time::timeout(Duration::from_secs(5), baseline_tasks.join_next()).await?
                .ok_or_else(|| anyhow::anyhow!("previous-order API task exited without a result"))?
                .map_err(anyhow::Error::from)?;
            let api_deadlocked = api_result.as_ref().is_err_and(|error| format!("{error:?}").contains("deadlock detected"));
            anyhow::ensure!(reproduced || api_deadlocked, "previous API/E2EE lock order did not reproduce a PostgreSQL deadlock; e2ee={e2ee_result:?}, api={api_result:?}");
            let _ = e2ee_tx.rollback().await;

            // The fixed path includes the creator in the same sorted set. While E2EE
            // owns the creator row, import must wait before locking the member row.
            let (fixed_e2ee_connection, fixed_e2ee_driver) = tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
            tokio::spawn(fixed_e2ee_driver);
            let mut fixed_e2ee_connection = fixed_e2ee_connection;
            let fixed_e2ee_tx = fixed_e2ee_connection.transaction().await?;
            fixed_e2ee_tx.batch_execute("SET LOCAL deadlock_timeout='100ms'").await?;
            fixed_e2ee_tx.query_one("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE", &[&creator_id]).await?;
            let fixed_e2ee_pid: i32 = fixed_e2ee_tx.query_one("SELECT pg_backend_pid()", &[]).await?.get(0);
            let mut fixed_tasks = tokio::task::JoinSet::new();
            let fixed_db = db.clone();
            let fixed_team = team.clone();
            let fixed_slack = slack_id.clone();
            let fixed_creator = creator_id.clone();
            let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
            fixed_tasks.spawn(async move {
                let mut db = fixed_db;
                let mut tx = db.transaction().await?;
                toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'").exec(&mut tx).await?;
                let pid: i32 = query(&mut tx, "SELECT json_build_object('pid',pg_backend_pid())::text", &[]).await?[0]["pid"].as_i64().unwrap() as i32;
                let _ = pid_tx.send(pid);
                let mut ctx = Context {
                    team: fixed_team, users: HashMap::new(), profiles: HashMap::new(),
                    fallback_names: HashMap::new(), channel_names: HashMap::new(),
                    files: FilePolicy { token: false, max_size: 0, base_url: String::new() }, app_id: None,
                };
                let resolved = super::resolve_members(&mut ctx, &mut tx, &[fixed_slack], &fixed_creator).await?;
                tx.commit().await?;
                Ok::<Vec<String>, anyhow::Error>(resolved.into_iter().map(|user| user.id).collect())
            });
            let api_pid = pid_rx.await?;
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let waiting: bool = guard.query_one(
                        "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND $2=ANY(pg_blocking_pids(pid)))",
                        &[&api_pid, &fixed_e2ee_pid],
                    ).await?.get(0);
                    if waiting { break; }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok::<_, anyhow::Error>(())
            }).await??;
            fixed_e2ee_tx.query_one("SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE", &[&member_id]).await?;
            fixed_e2ee_tx.commit().await?;
            let joined = tokio::time::timeout(Duration::from_secs(5), fixed_tasks.join_next()).await?
                .ok_or_else(|| anyhow::anyhow!("fixed API task exited without a result"))?
                .map_err(anyhow::Error::from)?;
            let resolved = joined?;
            anyhow::ensure!(resolved == vec![member_id], "fixed API import resolved the wrong member: {resolved:?}");
            Ok(())
        }.await;
        cleanup_fixture(&db, &team, &creator_id).await?;
        test_result
    }

    async fn cleanup_fixture(db: &toasty::Db, team: &str, creator_id: &str) -> anyhow::Result<()> {
        let mut db = db.clone();
        let mut tx = db.transaction().await?;
        toasty::sql::statement("DELETE FROM users WHERE id=$1 OR id IN (SELECT user_id FROM slack_users WHERE team_id=$2)")
            .bind(creator_id)
            .bind(team)
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

struct Link {
    team: String,
    slack_user: String,
    token: String,
    scopes: String,
}

impl Link {
    /// 非公開チャンネルの一覧と履歴を読めるスコープがあるか (以前の版で連携した人は、連携し直すまで持たない)
    fn can_read_private(&self) -> bool {
        ["groups:read", "groups:history"]
            .iter()
            .all(|scope| self.scopes.split(',').any(|s| s.trim() == *scope))
    }
}

fn key(s: &AppState) -> Result<[u8; 32], AppError> {
    if !s.config.slack.oauth_enabled() {
        return Err(AppError::NotFound("Slack linking is not configured".into()));
    }
    s.config
        .slack
        .encryption_key()
        .ok_or_else(|| AppError::NotFound("Slack linking is not configured".into()))
}

async fn load_link(
    tx: &mut toasty::Transaction<'_>,
    user_id: &str,
) -> Result<Option<Link>, AppError> {
    Ok(query(
        tx,
        "SELECT json_build_object('team',team_id,'slack_user',slack_user_id,'token',access_token_enc,\
         'scopes',scopes)::text FROM slack_user_links WHERE user_id=$1",
        &[user_id],
    )
    .await?
    .into_iter()
    .next()
    .map(|row| Link {
        team: row["team"].as_str().unwrap_or_default().to_string(),
        slack_user: row["slack_user"].as_str().unwrap_or_default().to_string(),
        token: row["token"].as_str().unwrap_or_default().to_string(),
        scopes: row["scopes"].as_str().unwrap_or_default().to_string(),
    }))
}

fn map_sync(error: SyncError) -> AppError {
    match error {
        SyncError::RateLimited(_) => AppError::RateLimited,
        // トークンが失効している・スコープが足りないなど。本人が連携し直せば直る
        SyncError::NoAccess(reason) => AppError::Forbidden(reason),
        SyncError::InvalidCursor
        | SyncError::LeaseLost
        | SyncError::TargetChanged
        | SyncError::Failed(_) => AppError::BadGateway("Slack request failed".into()),
    }
}

/// 一覧を返せない理由 (画面に出す)。`channels` は空
fn unavailable(linked: bool, reason: &str) -> Json<Value> {
    Json(json!({"linked": linked, "reason": reason, "channels": []}))
}

fn valid_slack_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 20
        && id
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

async fn list(State(s): State<AppState>, user: AuthUser) -> Result<Json<Value>, AppError> {
    let key = key(&s)?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let link = load_link(&mut tx, &user.id).await?;
    tx.commit().await?;
    let Some(link) = link else {
        return Ok(unavailable(false, "not_linked"));
    };
    if !s.config.slack.team_allowed(&link.team) {
        return Ok(unavailable(true, "team_mismatch"));
    }
    if !link.can_read_private() {
        return Ok(unavailable(true, "missing_scope"));
    }

    let mut slack = Slack::single(&s, key, &user.id, &link.token)?;
    let mut own: Vec<Value> = Vec::new();
    let mut cursor = String::new();
    for _ in 0..MAX_PAGES {
        let mut params = vec![
            ("types", "private_channel"),
            ("exclude_archived", "false"),
            ("limit", "200"),
        ];
        if !cursor.is_empty() {
            params.push(("cursor", cursor.as_str()));
        }
        let page = match slack.get("users.conversations", &params).await {
            Ok(page) => page,
            Err(SyncError::NoAccess(reason)) => return Ok(unavailable(true, &reason)),
            Err(e) => return Err(map_sync(e)),
        };
        for channel in page["channels"].as_array().into_iter().flatten() {
            if channel["creator"] == link.slack_user.as_str()
                && channel["is_im"] != true
                && channel["is_mpim"] != true
            {
                own.push(channel.clone());
            }
        }
        cursor = page["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if cursor.is_empty() {
            break;
        }
    }
    // 上限に達しても続きがあるなら、一覧を不完全なまま返さない
    if !cursor.is_empty() {
        return Err(AppError::BadGateway("Too many Slack channels".into()));
    }

    let mut tx = db.transaction().await?;
    let mut channels = Vec::with_capacity(own.len());
    for channel in own {
        let id = channel["id"].as_str().unwrap_or_default().to_string();
        let name = channel["name"].as_str().unwrap_or_default().to_string();
        let found = query(
            &mut tx,
            "SELECT json_build_object('channel_id',sc.channel_id,'name',c.channel_name,\
             'member',EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=sc.channel_id AND cu.user_id=$3),\
             'synced',t.last_synced_at IS NOT NULL,'running',COALESCE(t.running,false),\
             'failed',t.last_error IS NOT NULL,'deleted',c.deleted_at IS NOT NULL)::text \
             FROM slack_channels sc JOIN channels c ON c.id=sc.channel_id \
             LEFT JOIN slack_channel_syncs t ON t.team_id=sc.team_id AND t.slack_channel_id=sc.slack_channel_id \
             WHERE sc.team_id=$1 AND sc.slack_channel_id=$2",
            &[&link.team, &id, &user.id],
        )
        .await?
        .into_iter()
        .next();
        // 取り込み先を削除したチャンネルは、過去ログが削除済みのチャンネルに残っていて、取り込み直せないので、一覧に出さない
        // (`start` も、対応が残っている限り 409 を返す)
        if found.as_ref().is_some_and(|row| row["deleted"] == true) {
            continue;
        }
        let mut entry = json!({
            "slack_channel_id": id,
            "name": name,
            "is_archived": channel["is_archived"] == true,
            "members": channel["num_members"].as_i64().unwrap_or(0),
        });
        match found {
            None => {
                // まだ取り込んでいない。名前が使われていれば、`slack-` を付けた名前を提案する
                let taken = !query(
                    &mut tx,
                    "SELECT json_build_object('id',id)::text FROM channels WHERE channel_name=$1",
                    &[&name],
                )
                .await?
                .is_empty();
                let suggested: String = if taken {
                    format!("slack-{name}").chars().take(80).collect()
                } else {
                    name.clone()
                };
                entry["status"] = "new".into();
                entry["name_taken"] = taken.into();
                entry["suggested_name"] = suggested.into();
            }
            Some(row) if row["member"] != true => {
                // 参加していない非公開チャンネルの名前と ID は見せない (チャンネル一覧と同じ)
                entry["status"] = "hidden".into();
            }
            Some(row) => {
                let status = if row["synced"] == true {
                    "done"
                } else if row["failed"] == true {
                    "error"
                } else if row["running"] == true {
                    "running"
                } else {
                    "queued"
                };
                entry["status"] = status.into();
                entry["channel_id"] = row["channel_id"].clone();
                entry["channel_name"] = row["name"].clone();
            }
        }
        channels.push(entry);
    }
    tx.commit().await?;
    Ok(Json(json!({"linked": true, "channels": channels})))
}

/// 1 回の要求で取り込むのは 1 チャンネル (複数を 1 つの要求にすると、途中で失敗したときに一部だけが確定する)
#[derive(Deserialize)]
struct StartRequest {
    slack_channel_id: String,
    name: String,
}

/// `conversations.members` の全ページ
async fn members(slack: &mut Slack, channel: &str) -> Result<Vec<String>, AppError> {
    let mut all = Vec::new();
    let mut cursor = String::new();
    for _ in 0..MAX_PAGES {
        let mut params = vec![("channel", channel), ("limit", "200")];
        if !cursor.is_empty() {
            params.push(("cursor", cursor.as_str()));
        }
        let page = slack
            .get("conversations.members", &params)
            .await
            .map_err(map_sync)?;
        all.extend(
            page["members"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| m.as_str().map(str::to_string)),
        );
        cursor = page["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if cursor.is_empty() {
            break;
        }
    }
    // 上限に達しても続きがあるなら、不完全なメンバーで取り込みを確定しない
    if !cursor.is_empty() {
        return Err(AppError::BadGateway(
            "Too many Slack channel members".into(),
        ));
    }
    Ok(all)
}

/// まだ zoo にいないメンバーの名前を `users.info` で調べ、`ctx.profiles` に入れる (調べられなくても止めない)
async fn load_profiles(
    db: &mut toasty::Db,
    slack: &mut Slack,
    ctx: &mut Context,
    members: &[String],
) -> Result<(), AppError> {
    if members.is_empty() {
        return Ok(());
    }
    let mut tx = db.transaction().await?;
    let known: HashSet<String> = query(
        &mut tx,
        "SELECT json_build_object('id',slack_user_id)::text FROM slack_users \
         WHERE team_id=$1 AND slack_user_id IN (SELECT jsonb_array_elements_text($2::jsonb))",
        &[&ctx.team, &json!(members).to_string()],
    )
    .await?
    .into_iter()
    .filter_map(|r| r["id"].as_str().map(str::to_string))
    .collect();
    tx.commit().await?;
    let started = std::time::Instant::now();
    for id in members
        .iter()
        .filter(|id| (id.starts_with('U') || id.starts_with('W')) && !known.contains(*id))
        .take(MAX_PROFILE_LOOKUPS)
    {
        if started.elapsed() >= PROFILE_BUDGET {
            break;
        }
        match slack.get("users.info", &[("user", id.as_str())]).await {
            Ok(value) => {
                if let Ok(user) = serde_json::from_value::<SlackUser>(value["user"].clone()) {
                    ctx.profiles.insert(id.clone(), user);
                }
            }
            Err(SyncError::RateLimited(_)) => break,
            Err(_) => {}
        }
    }
    Ok(())
}

/// 自分が作った非公開チャンネルを取り込む。履歴は、同期のワーカーが全履歴から取る
async fn start(
    State(s): State<AppState>,
    user: AuthUser,
    Json(body): Json<StartRequest>,
) -> Result<Json<Value>, AppError> {
    let key = key(&s)?;
    if !valid_slack_id(&body.slack_channel_id) {
        return Err(AppError::BadRequest("Invalid Slack channel".into()));
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let Some(link) = load_link(&mut tx, &user.id).await? else {
        return Err(AppError::BadRequest(
            "Link your Slack account before importing".into(),
        ));
    };
    // SLACK_TEAM_ID を変えたあとに残っている、許可していないワークスペースの連携では取り込まない
    if !s.config.slack.team_allowed(&link.team) {
        return Err(AppError::Forbidden("team_mismatch".into()));
    }
    if !link.can_read_private() {
        return Err(AppError::Forbidden("missing_scope".into()));
    }
    // 通常のチャンネル作成と同じく、ゲストは作れない
    if crate::apps::events::guest(&mut tx, &user.id).await? {
        return Err(AppError::Forbidden(
            "Guests cannot create channels by importing from Slack".into(),
        ));
    }
    tx.commit().await?;

    let mut slack = Slack::single(&s, key, &user.id, &link.token)?;
    let pick = body;
    {
        let info = slack
            .get(
                "conversations.info",
                &[("channel", pick.slack_channel_id.as_str())],
            )
            .await
            .map_err(map_sync)?;
        let channel = &info["channel"];
        // DM・グループ DM と公開チャンネルは取り込まない (Slack の判定を使う)
        if channel["is_private"] != true || channel["is_im"] == true || channel["is_mpim"] == true {
            return Err(AppError::Forbidden(
                "Only private Slack channels can be imported".into(),
            ));
        }
        if channel["creator"] != link.slack_user.as_str() {
            return Err(AppError::Forbidden(
                "You can only import private Slack channels you created".into(),
            ));
        }
        let slack_channel: SlackChannel = serde_json::from_value(json!({
            "id": pick.slack_channel_id,
            "name": channel["name"].as_str().unwrap_or_default(),
            "created": channel["created"].as_i64().unwrap_or(0),
            "creator": link.slack_user,
            "purpose": {"value": channel["purpose"]["value"].as_str().unwrap_or_default()},
            "is_archived": channel["is_archived"] == true,
        }))
        .map_err(|e| AppError::Internal(e.into()))?;

        let member_ids = members(&mut slack, &pick.slack_channel_id).await?;
        let mut ctx = Context {
            team: link.team.clone(),
            users: HashMap::new(),
            profiles: HashMap::new(),
            fallback_names: HashMap::new(),
            channel_names: HashMap::new(),
            files: FilePolicy::new(&s),
            app_id: s.config.slack.app_id.clone(),
        };
        load_profiles(&mut db, &mut slack, &mut ctx, &member_ids).await?;

        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        // API 取り込みと ZIP 取り込み (最初の対応付けトランザクションも、日付ごとの保存
        // トランザクションも) を、同じ Slack チャンネル単位の advisory lock で直列化する。
        // この lock を `slack_channels` の確認とプレースホルダ作成 (`resolve_members`) の
        // 前に取るので、並行する ZIP 取り込みが対応を確定させたあとに、この要求が別の
        // 取り込み先を作って対応を上書きできない (ZIP 側の `ON CONFLICT DO UPDATE` も、
        // この lock を待ってから対応を確認するので、API 側が確定させた対応を上書きしない)。
        lock_zip_target_advisory(&mut tx, &link.team, &pick.slack_channel_id).await?;
        let already = !query(
            &mut tx,
            "SELECT json_build_object('ok',true)::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2",
            &[&link.team, &pick.slack_channel_id],
        )
        .await?
        .is_empty();
        if already {
            return Err(AppError::Conflict(format!(
                "Slack channel {} has already been imported",
                pick.slack_channel_id
            )));
        }
        // resolve_members が全メンバーと作成者の users 行を (ID 順に) すでにロックして
        // いるので、ここで lock_membership_users を呼び直さない (メンバーごとの無駄な
        // ロック往復になる)。
        let imported_users = resolve_members(&mut ctx, &mut tx, &member_ids, &user.id).await?;
        // Slack の応答を待つ間に種別が変わりうる。全参加者 (作成者を含む) の users 行は
        // resolve_members がロック済みなので、ここで読み直した作成者の状態は固定される。
        // 作成者がゲストになった場合はチャンネルを書き込まない。
        if crate::apps::events::guest(&mut tx, &user.id).await? {
            return Err(AppError::Forbidden(
                "Guests cannot create channels by importing from Slack".into(),
            ));
        }
        // 新しい zoo チャンネルは常に非公開で、作成者は本人
        let (target, _) = create_channel(
            &mut tx,
            &link.team,
            &slack_channel,
            pick.name.trim(),
            true,
            &user.id,
        )
        .await?;
        // カーソルを 0 にして同期のキューに入れると、同期が全履歴から取る
        let inserted = query(
            &mut tx,
            "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,$2,$3,'0') \
             ON CONFLICT DO NOTHING RETURNING json_build_object('ok',true)::text",
            &[&link.team, &pick.slack_channel_id, &target.0],
        )
        .await?;
        if inserted.is_empty() {
            return Err(AppError::Conflict(format!(
                "Slack channel {} has already been imported",
                pick.slack_channel_id
            )));
        }
        exec(
            &mut tx,
            "UPDATE channels SET imported_from_slack=true,slack_import_kind=COALESCE(slack_import_kind,'created') WHERE id=$1",
            &[&target.0],
        )
        .await?;
        for zoo_user in &imported_users {
            // 連携したゲストは、管理者が決めたチャンネルにだけ参加する (取り込みでは足さない)。
            // 未連携のプレースホルダは、連携時に本人へ移せるよう参加情報を残す (エクスポートの取り込みと同じ)
            exec(
                &mut tx,
                "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $1,$2,CURRENT_TIMESTAMP \
                 WHERE NOT EXISTS(SELECT 1 FROM users WHERE id=$1 AND guest_kind IN ('single','multi')) \
                 ON CONFLICT DO NOTHING",
                &[&zoo_user.id, &target.0],
            )
            .await?;
        }
        sync::request_channel(&mut tx, &link.team, &pick.slack_channel_id).await?;
        tx.commit().await?;
        Ok(Json(json!({
            "slack_channel_id": pick.slack_channel_id,
            "channel_id": target.0,
            "channel_name": target.1,
        })))
    }
}
