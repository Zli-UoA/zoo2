//! Slack 取り込みジョブのワーカー。
//!
//! 本番は複数台で動くため、`apps::delivery` と同じく `FOR UPDATE SKIP LOCKED` とリースで
//! 1 ジョブを 1 台だけが進める。処理中はリースを延長し続け、止まったらリース切れ後に
//! 別の台が同じ段階をやり直す。書き込みは決定的 ID と `ON CONFLICT DO NOTHING` で冪等。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    convert::{Resolver, mrkdwn_to_mfm, reaction_emoji},
    export::{ExportArchive, SlackChannel, SlackMessage, SlackUser},
    import::{Mapping, MappingAction},
};
use crate::{
    apps::{one, query},
    auth::{sanitize_for_display_name, sanitize_for_user_name},
    error::AppError,
    state::AppState,
    storage::delete_storage_object,
    validation::{MAX_CHANNEL_DESCRIPTION_LENGTH, validate_channel_name},
};

/// これを超えて失敗したジョブは `failed` にする
const MAX_ATTEMPTS: i64 = 5;
const LEASE_SQL: &str = "interval '2 minutes'";
const HEARTBEAT: Duration = Duration::from_secs(30);
/// 添付 1 件あたりの取得失敗の上限
const MAX_FILE_ATTEMPTS: i64 = 3;

pub async fn worker(s: AppState) {
    let mut ticks = tokio::time::interval(Duration::from_secs(2));
    loop {
        ticks.tick().await;
        if let Err(e) = run_once(&s).await {
            tracing::warn!(error = %e, "Slack import worker failed");
        }
    }
}

#[derive(Debug, Deserialize)]
struct Job {
    id: String,
    status: String,
    storage_key: String,
    team_id: Option<String>,
    mappings: Option<String>,
    created_by: String,
    attempts: i64,
    token: String,
}

/// 実行できるジョブを 1 件取り、1 段階進める。ジョブがあれば true。
pub async fn run_once(s: &AppState) -> Result<bool, AppError> {
    let Some(job) = claim(s).await? else {
        return Ok(false);
    };
    let lease = Lease::start(s.clone(), job.id.clone(), job.token.clone());
    tracing::info!(import_id = %job.id, status = %job.status, attempt = job.attempts, "Slack import step started");
    let result = match job.status.as_str() {
        "analyzing" => analyze(s, &job, &lease).await,
        "importing" => import(s, &job, &lease).await,
        "fetching_files" => fetch_files(s, &job, &lease).await,
        _ => Ok(()),
    };
    drop(lease);
    if let Err(e) = result {
        tracing::warn!(import_id = %job.id, error = %e, "Slack import step failed");
        fail(s, &job, &e).await?;
    }
    Ok(true)
}

async fn claim(s: &AppState) -> Result<Option<Job>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let token = uuid::Uuid::new_v4().to_string();
    let rows = query(
        &mut tx,
        &format!(
            "WITH c AS (SELECT id FROM slack_import_jobs \
             WHERE status IN ('analyzing','importing','fetching_files') AND available_at<=now() \
             AND (lease_until IS NULL OR lease_until<now()) \
             ORDER BY created_at,id FOR UPDATE SKIP LOCKED LIMIT 1) \
             UPDATE slack_import_jobs j SET attempts=j.attempts+1,lease_until=now()+{LEASE_SQL},\
             lease_token=$1,updated_at=now() FROM c WHERE j.id=c.id \
             RETURNING json_build_object('id',j.id,'status',j.status,'storage_key',j.storage_key,\
             'team_id',j.team_id,'mappings',j.mappings,'created_by',j.created_by,\
             'attempts',j.attempts,'token',j.lease_token)::text"
        ),
        &[&token],
    )
    .await?;
    tx.commit().await?;
    rows.into_iter()
        .next()
        .map(|v| serde_json::from_value(v).map_err(|e| AppError::Internal(e.into())))
        .transpose()
}

/// リースの延長を続け、奪われたら処理を止めるための目印
struct Lease {
    lost: Arc<AtomicBool>,
    handle: tokio::task::JoinHandle<()>,
}

impl Lease {
    fn start(s: AppState, id: String, token: String) -> Self {
        let lost = Arc::new(AtomicBool::new(false));
        let flag = lost.clone();
        let handle = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(HEARTBEAT);
            ticks.tick().await;
            loop {
                ticks.tick().await;
                match extend_lease(&s, &id, &token).await {
                    Ok(true) => {}
                    Ok(false) => {
                        flag.store(true, Ordering::SeqCst);
                        return;
                    }
                    Err(e) => {
                        tracing::warn!(import_id = %id, error = %e, "Failed to extend Slack import lease")
                    }
                }
            }
        });
        Self { lost, handle }
    }

    fn check(&self) -> Result<(), AppError> {
        if self.lost.load(Ordering::SeqCst) {
            return Err(AppError::Conflict("Slack import lease was lost".into()));
        }
        Ok(())
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn extend_lease(s: &AppState, id: &str, token: &str) -> Result<bool, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        &format!(
            "UPDATE slack_import_jobs SET lease_until=now()+{LEASE_SQL} WHERE id=$1 AND lease_token=$2 \
             RETURNING json_build_object('id',id)::text"
        ),
        &[id, token],
    )
    .await?;
    tx.commit().await?;
    Ok(!rows.is_empty())
}

async fn exec(
    tx: &mut toasty::Transaction<'_>,
    sql: &str,
    params: &[&str],
) -> Result<(), AppError> {
    let mut statement = toasty::sql::statement(sql);
    for value in params {
        statement = statement.bind(*value);
    }
    statement.exec(tx).await?;
    Ok(())
}

/// 取り込みで生じる行をアプリへ大量配送しないよう、このトランザクションのイベントを止める
async fn suppress_app_events(tx: &mut toasty::Transaction<'_>) -> Result<(), AppError> {
    one(
        tx,
        "SELECT json_build_object('ok',set_config('zoo.suppress_app_events','on',true))::text",
        &[],
    )
    .await?;
    Ok(())
}

async fn fail(s: &AppState, job: &Job, error: &AppError) -> Result<(), AppError> {
    // ZIP の中身の不備はやり直しても直らない
    let permanent = matches!(error, AppError::BadRequest(_));
    let lost = matches!(error, AppError::Conflict(_));
    if lost {
        return Ok(());
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    exec(
        &mut tx,
        "UPDATE slack_import_jobs SET error=$3,lease_until=NULL,lease_token=NULL,updated_at=now(),\
         status=CASE WHEN $4::boolean OR attempts>=$5::integer THEN 'failed' ELSE status END,\
         finished_at=CASE WHEN $4::boolean OR attempts>=$5::integer THEN now() END,\
         available_at=now()+make_interval(secs=>LEAST(600,10*power(2,attempts))::integer) \
         WHERE id=$1 AND lease_token=$2",
        &[
            &job.id,
            &job.token,
            &error.to_string(),
            if permanent { "true" } else { "false" },
            &MAX_ATTEMPTS.to_string(),
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// ストレージから ZIP を一時ファイルに落とす。破棄時に消す。
struct TempZip {
    path: PathBuf,
}

impl Drop for TempZip {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

async fn download(s: &AppState, key: &str) -> Result<TempZip, AppError> {
    let tmp = TempZip {
        path: std::env::temp_dir().join(format!("zoo-slack-import-{}.zip", uuid::Uuid::new_v4())),
    };
    let object = s
        .s3_client
        .get_object()
        .bucket(&s.config.s3.bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to read the Slack export: {e}")))?;
    let mut reader = object.body.into_async_read();
    let mut file = tokio::fs::File::create(&tmp.path)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    tokio::io::copy(&mut reader, &mut file)
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    file.sync_all()
        .await
        .map_err(|e| AppError::Internal(e.into()))?;
    Ok(tmp)
}

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------

async fn analyze(s: &AppState, job: &Job, lease: &Lease) -> Result<(), AppError> {
    let tmp = download(s, &job.storage_key).await?;
    lease.check()?;
    let path = tmp.path.clone();
    let (team, channels, user_count) =
        tokio::task::spawn_blocking(move || ExportArchive::open(&path)?.summarize())
            .await
            .map_err(|e| AppError::Internal(e.into()))??;
    lease.check()?;

    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let mut entries = Vec::with_capacity(channels.len());
    let mut total = 0u64;
    for channel in channels {
        total += channel.message_count;
        // 以前取り込んだチャンネルは同じ zoo チャンネルへ続けて入れる
        let mapped = query(
            &mut tx,
            "SELECT json_build_object('id',c.id,'name',c.channel_name,'deleted',c.deleted_at IS NOT NULL)::text \
             FROM slack_channels s JOIN channels c ON c.id=s.channel_id \
             WHERE s.team_id=$1 AND s.slack_channel_id=$2",
            &[&team, &channel.slack_channel_id],
        )
        .await?
        .into_iter()
        .next();
        // 以前の取り込み先が削除済みなら取り込み直せない (メッセージ ID が残っているため)
        let deleted = mapped
            .as_ref()
            .filter(|m| m["deleted"] == true)
            .map(|m| m["name"].clone());
        let mapped = mapped.filter(|m| m["deleted"] != true).map(|mut m| {
            m.as_object_mut().map(|o| o.remove("deleted"));
            m
        });
        let name_taken = !query(
            &mut tx,
            "SELECT json_build_object('id',id)::text FROM channels WHERE channel_name=$1",
            &[&channel.name],
        )
        .await?
        .is_empty();
        let suggested_name = if validate_channel_name(&channel.name).is_ok() && !name_taken {
            channel.name.clone()
        } else {
            format!("slack-{}", channel.name)
        };
        let mut entry = serde_json::to_value(&channel).map_err(|e| AppError::Internal(e.into()))?;
        entry["suggested_name"] = json!(suggested_name);
        entry["name_taken"] = json!(name_taken);
        entry["existing_channel"] = mapped.unwrap_or(Value::Null);
        entry["deleted_channel"] = deleted.unwrap_or(Value::Null);
        entries.push(entry);
    }
    let analysis = json!({ "team_id": team, "user_count": user_count, "channels": entries });
    exec(
        &mut tx,
        "UPDATE slack_import_jobs SET status='awaiting_mapping',team_id=$3,analysis=$4,\
         total_messages=$5::bigint,attempts=0,error=NULL,lease_until=NULL,lease_token=NULL,updated_at=now() \
         WHERE id=$1 AND lease_token=$2",
        &[
            &job.id,
            &job.token,
            &team,
            &analysis.to_string(),
            &total.to_string(),
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 取り込み
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ZooUser {
    id: String,
    user_name: String,
}

struct Context {
    team: String,
    users: HashMap<String, ZooUser>,
    profiles: HashMap<String, SlackUser>,
    channel_names: HashMap<String, String>,
}

impl Resolver for Context {
    fn user_name(&self, slack_user_id: &str) -> Option<String> {
        self.users.get(slack_user_id).map(|u| u.user_name.clone())
    }
    fn channel_name(&self, slack_channel_id: &str) -> Option<String> {
        self.channel_names.get(slack_channel_id).cloned()
    }
}

impl Context {
    /// Slack ユーザー (または bot) に対応する zoo ユーザーを返す。なければプレースホルダを作る。
    async fn user(
        &mut self,
        tx: &mut toasty::Transaction<'_>,
        slack_id: &str,
        fallback_name: Option<&str>,
    ) -> Result<ZooUser, AppError> {
        if let Some(user) = self.users.get(slack_id) {
            return Ok(user.clone());
        }
        let profile = self.profiles.get(slack_id).cloned();
        let user = ensure_user(tx, &self.team, slack_id, profile.as_ref(), fallback_name).await?;
        self.users.insert(slack_id.to_string(), user.clone());
        Ok(user)
    }
}

async fn ensure_user(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_id: &str,
    profile: Option<&SlackUser>,
    fallback_name: Option<&str>,
) -> Result<ZooUser, AppError> {
    if let Some(found) = query(
        tx,
        "SELECT json_build_object('id',u.id,'user_name',u.user_name)::text FROM slack_users s \
         JOIN users u ON u.id=s.user_id WHERE s.team_id=$1 AND s.slack_user_id=$2",
        &[team, slack_id],
    )
    .await?
    .into_iter()
    .next()
    {
        return Ok(ZooUser {
            id: found["id"].as_str().unwrap_or_default().to_string(),
            user_name: found["user_name"].as_str().unwrap_or_default().to_string(),
        });
    }

    let id = super::user_id(team, slack_id);
    let handle = profile
        .map(|p| p.name.as_str())
        .filter(|n| !n.is_empty())
        .or(fallback_name)
        .unwrap_or(slack_id);
    let display = profile
        .map(SlackUser::display_name)
        .or(fallback_name)
        .unwrap_or(slack_id);
    let display = sanitize_for_display_name(display, handle);
    let is_bot = profile.map(|p| p.is_bot).unwrap_or(false) || slack_id.starts_with('B');
    let avatar = profile
        .map(|p| p.profile.image_72.as_str())
        .filter(|u| u.starts_with("https://"))
        .unwrap_or("");

    let base = sanitize_for_user_name(handle, &id);
    let suffix = id.replace('-', "");
    let mut user_name = None;
    for candidate in std::iter::once(base.clone())
        .chain((0..4).map(|i| format!("{base}_{}", &suffix[i * 6..i * 6 + 6])))
    {
        let candidate: String = candidate.chars().take(80).collect();
        let taken = !query(
            tx,
            "SELECT json_build_object('id',id)::text FROM users WHERE user_name=$1",
            &[&candidate],
        )
        .await?
        .is_empty();
        if !taken {
            user_name = Some(candidate);
            break;
        }
    }
    let user_name = user_name.unwrap_or_else(|| format!("slack_{suffix}"));

    // ログインできず、メンバー一覧にも出ないよう無効化したユーザーとして作る
    exec(
        tx,
        "INSERT INTO users(id,user_name,display_name,is_bot,avatar_url,guest_kind) \
         VALUES($1,$2,$3,$4::boolean,NULLIF($5,''),'deactivated') ON CONFLICT (id) DO NOTHING",
        &[
            &id,
            &user_name,
            &display,
            if is_bot { "true" } else { "false" },
            avatar,
        ],
    )
    .await?;
    let real_name = profile.map(|p| p.real_name.as_str()).unwrap_or("");
    exec(
        tx,
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name) \
         VALUES($1,$2,$3,false,$4,$5) ON CONFLICT (team_id,slack_user_id) DO NOTHING",
        &[team, slack_id, &id, handle, real_name],
    )
    .await?;
    // 既存行があった場合 (別ジョブとの競合など) はそちらを正とする
    let row = one(
        tx,
        "SELECT json_build_object('id',u.id,'user_name',u.user_name)::text FROM slack_users s \
         JOIN users u ON u.id=s.user_id WHERE s.team_id=$1 AND s.slack_user_id=$2",
        &[team, slack_id],
    )
    .await?;
    Ok(ZooUser {
        id: row["id"].as_str().unwrap_or_default().to_string(),
        user_name: row["user_name"].as_str().unwrap_or_default().to_string(),
    })
}

async fn import(s: &AppState, job: &Job, lease: &Lease) -> Result<(), AppError> {
    let team = job
        .team_id
        .clone()
        .ok_or_else(|| AppError::BadRequest("The export has not been analyzed".into()))?;
    let mappings: Vec<Mapping> = serde_json::from_str(job.mappings.as_deref().unwrap_or("[]"))
        .map_err(|e| AppError::BadRequest(format!("Invalid mappings: {e}")))?;
    let tmp = download(s, &job.storage_key).await?;
    lease.check()?;
    let mut archive = ExportArchive::open(&tmp.path)?;
    let slack_users = archive.users()?;
    let slack_channels: HashMap<String, SlackChannel> = archive
        .channels()?
        .into_iter()
        .map(|c| (c.id.clone(), c))
        .collect();

    let mut ctx = Context {
        team: team.clone(),
        users: HashMap::new(),
        profiles: slack_users
            .iter()
            .map(|u| (u.id.clone(), u.clone()))
            .collect(),
        channel_names: HashMap::new(),
    };
    let mut db = s.db.clone();

    // ユーザーを先にそろえ、本文のメンションを解決できるようにする
    for chunk in slack_users.chunks(200) {
        lease.check()?;
        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        for user in chunk {
            ctx.user(&mut tx, &user.id, None).await?;
        }
        tx.commit().await?;
    }

    // 取り込み先チャンネルを決める
    let mut targets = Vec::new();
    {
        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        for mapping in &mappings {
            let Some(channel) = slack_channels.get(&mapping.slack_channel_id) else {
                continue;
            };
            // 開始後に別のジョブが同じチャンネルを取り込んだ場合も、取り込み先はずらさない
            super::import::ensure_same_target(&mut tx, &team, mapping)
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            let target = match &mapping.action {
                MappingAction::Skip => continue,
                MappingAction::Create { name, is_private } => {
                    create_channel(
                        &mut tx,
                        &team,
                        channel,
                        name,
                        is_private.unwrap_or(channel.is_private),
                        &job.created_by,
                    )
                    .await?
                }
                MappingAction::Existing { channel_id } => {
                    let found = one(
                        &mut tx,
                        "SELECT json_build_object('id',id,'name',channel_name)::text FROM channels \
                         WHERE id=$1 AND NOT is_dm AND deleted_at IS NULL",
                        &[channel_id],
                    )
                    .await
                    .map_err(|_| {
                        AppError::BadRequest(format!("Channel {channel_id} no longer exists"))
                    })?;
                    (
                        channel_id.clone(),
                        found["name"].as_str().unwrap_or_default().to_string(),
                    )
                }
            };
            exec(
                &mut tx,
                "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id) VALUES($1,$2,$3) \
                 ON CONFLICT (team_id,slack_channel_id) DO UPDATE SET channel_id=EXCLUDED.channel_id,updated_at=now()",
                &[&team, &channel.id, &target.0],
            )
            .await?;
            ctx.channel_names
                .insert(channel.id.clone(), target.1.clone());
            let create = matches!(mapping.action, MappingAction::Create { .. });
            targets.push((channel.clone(), target.0, create));
        }
        tx.commit().await?;
    }

    let mut processed = 0u64;
    for (channel, channel_id, created) in &targets {
        lease.check()?;
        {
            let mut tx = db.transaction().await?;
            suppress_app_events(&mut tx).await?;
            for member in &channel.members {
                let user = ctx.user(&mut tx, member, None).await?;
                exec(
                    &mut tx,
                    "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) \
                     ON CONFLICT DO NOTHING",
                    &[&user.id, channel_id],
                )
                .await?;
            }
            tx.commit().await?;
        }

        for entry in archive.day_files(channel) {
            lease.check()?;
            let messages = archive.messages(&entry)?;
            let mut tx = db.transaction().await?;
            suppress_app_events(&mut tx).await?;
            let mut last_ts = None;
            for message in messages.iter().filter(|m| m.is_importable()) {
                insert_message(&mut tx, &mut ctx, &channel.id, channel_id, job, message).await?;
                last_ts = Some(message.ts.as_str());
                processed += 1;
            }
            if let Some(ts) = last_ts {
                exec(
                    &mut tx,
                    "UPDATE slack_channels SET last_ts=$3,updated_at=now() WHERE team_id=$1 AND slack_channel_id=$2 \
                     AND (last_ts IS NULL OR last_ts::numeric<$3::numeric)",
                    &[&team, &channel.id, ts],
                )
                .await?;
            }
            exec(
                &mut tx,
                "UPDATE slack_import_jobs SET processed_messages=$3::bigint,updated_at=now() WHERE id=$1 AND lease_token=$2",
                &[&job.id, &job.token, &processed.to_string()],
            )
            .await?;
            tx.commit().await?;
        }

        // 既存メンバーに過去ログが未読として大量に出ないよう、既読位置を取り込んだ過去ログの最新まで進める。
        // チャンネル全体の最新ではなく同期カーソルを使い、それより新しい通常の投稿の未読は残す。
        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        exec(
            &mut tx,
            "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) \
             SELECT cu.user_id,cu.channel_id,m.latest FROM channel_user cu JOIN users u ON u.id=cu.user_id \
             CROSS JOIN (SELECT (to_timestamp(last_ts::numeric) AT TIME ZONE 'UTC')::timestamp(3) AS latest \
             FROM slack_channels WHERE team_id=$2 AND slack_channel_id=$3 AND last_ts IS NOT NULL) m \
             WHERE cu.channel_id=$1 AND NOT u.is_deactivated \
             ON CONFLICT (user_id,channel_id) DO UPDATE \
             SET last_read_at=GREATEST(channel_read_states.last_read_at,EXCLUDED.last_read_at)",
            &[channel_id, &team, &channel.id],
        )
        .await?;
        if *created && channel.is_archived {
            exec(
                &mut tx,
                "UPDATE channels SET archive_at=CURRENT_TIMESTAMP WHERE id=$1 AND archive_at IS NULL",
                &[channel_id],
            )
            .await?;
        }
        tx.commit().await?;
    }

    // 取り込み中に連携した人の分も含め、連携済みの人の投稿をプレースホルダから本人へ移す
    let mut tx = db.transaction().await?;
    super::link::reconcile_team(&mut tx, &team).await?;
    tx.commit().await?;

    // 添付の取得へ進む。トークンがなければ保留のまま完了にする。
    let mut tx = db.transaction().await?;
    let pending = one(
        &mut tx,
        "SELECT json_build_object('count',count(*))::text FROM slack_files WHERE job_id=$1",
        &[&job.id],
    )
    .await?["count"]
        .as_i64()
        .unwrap_or(0);
    let next = if pending > 0 && s.config.slack.import_bot_token.is_some() {
        "fetching_files"
    } else {
        "completed"
    };
    exec(
        &mut tx,
        "UPDATE slack_import_jobs SET status=$3,processed_messages=$4::bigint,total_files=$5::bigint,\
         attempts=0,error=NULL,lease_until=NULL,lease_token=NULL,available_at=now(),updated_at=now(),\
         finished_at=CASE WHEN $3='completed' THEN now() END WHERE id=$1 AND lease_token=$2",
        &[
            &job.id,
            &job.token,
            next,
            &processed.to_string(),
            &pending.to_string(),
        ],
    )
    .await?;
    tx.commit().await?;

    // 取り込み後は ZIP を残さない
    if let Err(e) = delete_storage_object(&s.s3_client, &s.config.s3.bucket, &job.storage_key).await
    {
        tracing::warn!(import_id = %job.id, error = %e, "Failed to delete imported Slack export");
    }
    Ok(())
}

async fn create_channel(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    channel: &SlackChannel,
    name: &str,
    is_private: bool,
    admin: &str,
) -> Result<(String, String), AppError> {
    validate_channel_name(name)?;
    let id = super::channel_id(team, &channel.id);
    let description: String = channel
        .purpose
        .value
        .chars()
        .take(MAX_CHANNEL_DESCRIPTION_LENGTH)
        .collect();
    let taken = query(
        tx,
        "SELECT json_build_object('id',id)::text FROM channels WHERE channel_name=$1 AND id<>$2",
        &[name, &id],
    )
    .await?;
    if !taken.is_empty() {
        return Err(AppError::BadRequest(format!(
            "Channel name '{name}' is already taken"
        )));
    }
    exec(
        tx,
        "INSERT INTO channels(id,channel_name,description,created_user_id,is_private,created_at) \
         VALUES($1,$2,NULLIF($3,''),$4,$5::boolean,\
         CASE WHEN $6::bigint>0 THEN (to_timestamp($6::bigint) AT TIME ZONE 'UTC')::timestamp(3) ELSE CURRENT_TIMESTAMP END) \
         ON CONFLICT (id) DO NOTHING",
        &[
            &id,
            name,
            &description,
            admin,
            if is_private { "true" } else { "false" },
            &channel.created.max(0).to_string(),
        ],
    )
    .await?;
    // 作成した管理者もメンバーに入れる (zoo でチャンネルを作ったときと同じ)
    exec(
        tx,
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) ON CONFLICT DO NOTHING",
        &[admin, &id],
    )
    .await?;
    let row = one(
        tx,
        "SELECT json_build_object('name',channel_name)::text FROM channels WHERE id=$1",
        &[&id],
    )
    .await?;
    Ok((id, row["name"].as_str().unwrap_or(name).to_string()))
}

async fn insert_message(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    slack_channel: &str,
    channel_id: &str,
    job: &Job,
    message: &SlackMessage,
) -> Result<(), AppError> {
    let author = message.author().unwrap_or_default();
    let fallback = message
        .username
        .as_deref()
        .or(message.bot_profile.as_ref().map(|b| b.name.as_str()))
        .filter(|n| !n.is_empty());
    let user = ctx.user(tx, author, fallback).await?;
    let id = super::message_id(&ctx.team, slack_channel, &message.ts);
    let parent = message
        .parent_ts()
        .map(|ts| super::message_id(&ctx.team, slack_channel, ts))
        .unwrap_or_default();
    let edited = message
        .edited
        .as_ref()
        .map(|e| e.ts.as_str())
        .filter(|ts| super::is_valid_ts(ts))
        .unwrap_or("");
    let content = mrkdwn_to_mfm(&message.text, &*ctx);
    exec(
        tx,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,parent_id) \
         VALUES($1,$2,$3,$4,(to_timestamp($5::numeric) AT TIME ZONE 'UTC')::timestamp(3),\
         CASE WHEN $6='' THEN NULL ELSE (to_timestamp($6::numeric) AT TIME ZONE 'UTC')::timestamp(3) END,\
         (SELECT m.id FROM messages m WHERE m.id=NULLIF($7,'') AND m.channel_id=$3 AND m.parent_id IS NULL)) \
         ON CONFLICT (id) DO NOTHING",
        &[&id, &user.id, channel_id, &content, &message.ts, edited, &parent],
    )
    .await?;

    for reaction in &message.reactions {
        let Some(emoji) = reaction_emoji(&reaction.name) else {
            continue;
        };
        for slack_user in &reaction.users {
            let reactor = ctx.user(tx, slack_user, None).await?;
            exec(
                tx,
                "INSERT INTO message_reactions(id,message_id,user_id,emoji,created_at) \
                 SELECT $1,$2,$3,$4,(to_timestamp($5::numeric) AT TIME ZONE 'UTC')::timestamp(3) \
                 WHERE EXISTS(SELECT 1 FROM messages WHERE id=$2) ON CONFLICT DO NOTHING",
                &[
                    &super::reaction_id(&id, &reactor.id, &emoji),
                    &id,
                    &reactor.id,
                    &emoji,
                    &message.ts,
                ],
            )
            .await?;
        }
    }

    for file in &message.files {
        let Some(url) = file
            .url_private_download
            .as_deref()
            .filter(|u| !u.is_empty())
        else {
            continue;
        };
        if matches!(
            file.mode.as_deref(),
            Some("tombstone" | "hidden_by_limit" | "external")
        ) {
            continue;
        }
        let name: String = file
            .name
            .as_deref()
            .or(file.title.as_deref())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(&file.id)
            .chars()
            .filter(|c| !c.is_control())
            .take(255)
            .collect();
        let mimetype = file
            .mimetype
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or("application/octet-stream");
        exec(
            tx,
            "INSERT INTO slack_files(team_id,slack_file_id,job_id,message_id,channel_id,uploader_id,\
             name,mimetype,size,url_private_download) \
             SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9::bigint,$10 WHERE EXISTS(SELECT 1 FROM messages WHERE id=$4) \
             ON CONFLICT DO NOTHING",
            &[
                &ctx.team,
                &file.id,
                &job.id,
                &id,
                channel_id,
                &user.id,
                &name,
                mimetype,
                &file.size.unwrap_or(0).max(0).to_string(),
                url,
            ],
        )
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 添付ファイルの取得
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct PendingFile {
    team_id: String,
    slack_file_id: String,
    message_id: String,
    channel_id: String,
    uploader_id: String,
    name: String,
    mimetype: String,
    size: i64,
    url: Option<String>,
    attempts: i64,
}

enum FileOutcome {
    Done(String),
    Skipped(String),
}

async fn fetch_files(s: &AppState, job: &Job, lease: &Lease) -> Result<(), AppError> {
    let mut db = s.db.clone();
    if let Some(token) = s.config.slack.import_bot_token.as_deref() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Internal(e.into()))?;
        loop {
            lease.check()?;
            let mut tx = db.transaction().await?;
            let batch = query(
                &mut tx,
                "SELECT json_build_object('team_id',team_id,'slack_file_id',slack_file_id,\
                 'message_id',message_id,'channel_id',channel_id,'uploader_id',uploader_id,'name',name,\
                 'mimetype',mimetype,'size',size,'url',url_private_download,'attempts',attempts)::text \
                 FROM slack_files WHERE job_id=$1 AND status='pending' ORDER BY created_at,slack_file_id,message_id LIMIT 20",
                &[&job.id],
            )
            .await?;
            tx.commit().await?;
            if batch.is_empty() {
                break;
            }
            for value in batch {
                lease.check()?;
                let file: PendingFile =
                    serde_json::from_value(value).map_err(|e| AppError::Internal(e.into()))?;
                let outcome = fetch_one(s, &client, token, &file).await;
                let mut tx = db.transaction().await?;
                let (status, attachment, error) = match outcome {
                    Ok(FileOutcome::Done(attachment)) => ("done", attachment, String::new()),
                    Ok(FileOutcome::Skipped(reason)) => ("skipped", String::new(), reason),
                    Err(e) if file.attempts + 1 >= MAX_FILE_ATTEMPTS => {
                        ("failed", String::new(), e.to_string())
                    }
                    Err(e) => ("pending", String::new(), e.to_string()),
                };
                exec(
                    &mut tx,
                    "UPDATE slack_files SET status=$3,attachment_id=NULLIF($4,''),error=NULLIF($5,''),\
                     attempts=attempts+CASE WHEN $3 IN ('pending','failed') THEN 1 ELSE 0 END,updated_at=now() \
                     WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$6",
                    &[
                        &file.team_id,
                        &file.slack_file_id,
                        status,
                        &attachment,
                        &error,
                        &file.message_id,
                    ],
                )
                .await?;
                exec(
                    &mut tx,
                    "UPDATE slack_import_jobs SET processed_files=(SELECT count(*) FROM slack_files \
                     WHERE job_id=$1 AND status<>'pending'),updated_at=now() WHERE id=$1 AND lease_token=$2",
                    &[&job.id, &job.token],
                )
                .await?;
                tx.commit().await?;
            }
        }
    }
    // 取得中に連携した人の添付も本人へ移す
    if let Some(team) = job.team_id.as_deref() {
        let mut tx = db.transaction().await?;
        super::link::reconcile_team(&mut tx, team).await?;
        tx.commit().await?;
    }
    let mut tx = db.transaction().await?;
    exec(
        &mut tx,
        "UPDATE slack_import_jobs SET status='completed',finished_at=now(),attempts=0,error=NULL,\
         lease_until=NULL,lease_token=NULL,updated_at=now() WHERE id=$1 AND lease_token=$2",
        &[&job.id, &job.token],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn fetch_one(
    s: &AppState,
    client: &reqwest::Client,
    token: &str,
    file: &PendingFile,
) -> Result<FileOutcome, AppError> {
    let Some(url) = file
        .url
        .as_deref()
        .filter(|u| u.starts_with(&s.config.slack.files_base_url))
    else {
        return Ok(FileOutcome::Skipped("Unsupported file URL".into()));
    };
    let max = s.config.s3.max_file_size;
    if file.size as u64 > max {
        return Ok(FileOutcome::Skipped(format!("File exceeds {max} bytes")));
    }
    if crate::routes::attachments::validate_upload_mime(&file.mimetype).is_err() {
        return Ok(FileOutcome::Skipped("Unsupported content type".into()));
    }

    let response = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| AppError::BadGateway(format!("Slack file download failed: {e}")))?;
    if !response.status().is_success() {
        return Err(AppError::BadGateway(format!(
            "Slack file download returned {}",
            response.status()
        )));
    }
    // トークンの権限が足りないと Slack はログイン画面の HTML を返す
    let is_html = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html") && !file.mimetype.starts_with("text/html"));
    if is_html {
        return Err(AppError::BadGateway(
            "Slack returned a login page; check the token's files:read scope".into(),
        ));
    }
    if response.content_length().is_some_and(|len| len > max) {
        return Ok(FileOutcome::Skipped(format!("File exceeds {max} bytes")));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| AppError::BadGateway(format!("Slack file download failed: {e}")))?;
    if bytes.len() as u64 > max {
        return Ok(FileOutcome::Skipped(format!("File exceeds {max} bytes")));
    }

    // 同じファイルを共有するメッセージごとに添付を作る
    let attachment = super::attachment_id(
        &file.team_id,
        &format!("{}:{}", file.slack_file_id, file.message_id),
    );
    let key = format!("attachments/{}/{attachment}", file.channel_id);
    let size = bytes.len();
    s.s3_client
        .put_object()
        .bucket(&s.config.s3.bucket)
        .key(&key)
        .content_type(&file.mimetype)
        .body(bytes.into())
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to store Slack file: {e}")))?;

    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    exec(
        &mut tx,
        "INSERT INTO attachments(id,message_id,uploader_id,channel_id,file_name,file_size,content_type,storage_key,created_at) \
         SELECT $1,m.id,$3,$4,$5,$6::bigint,$7,$8,m.created_at FROM messages m WHERE m.id=$2 \
         ON CONFLICT (id) DO NOTHING",
        &[
            &attachment,
            &file.message_id,
            &file.uploader_id,
            &file.channel_id,
            &file.name,
            &size.to_string(),
            &file.mimetype,
            &key,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(FileOutcome::Done(attachment))
}
