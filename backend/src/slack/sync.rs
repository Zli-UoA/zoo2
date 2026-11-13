//! エクスポートより後の差分同期 (#292)。
//!
//! 対応済みの Slack チャンネル (`slack_channels`) ごとに、連携済みメンバーの誰か 1 人のトークンで
//! `conversations.history` / `conversations.replies` を呼び、同期カーソル (`slack_channels.last_ts`) より後の
//! 投稿を取り込む。カーソルはチャンネル単位で共有するので、同じチャンネルを人数分取得することはない。
//!
//! - **キュー**: チャンネルごとに `slack_channel_syncs` の 1 行がキューの項目で、本番の複数台が
//!   `FOR UPDATE SKIP LOCKED` とリースで 1 チャンネルを 1 台だけが進める (`worker.rs` と同じ流儀)。
//! - **再開**: 1 ページ (または 1 スレッド) ごとに、書き込みと同じトランザクションで途中経過 (ページのカーソル・
//!   残りのスレッド・今回の最大 `ts`) を保存する。止まっても、その続きから再開する。`conversations.history` は新しい順に
//!   返すので、`last_ts` は同期が最後まで終わったときにだけ進める (途中で進めると、間の投稿を取りこぼす)。
//! - **429**: `Retry-After` の秒数だけ次の実行を遅らせる。途中経過は保存済みなので、試行回数には数えない。
//! - **冪等**: 書き込みは `worker::insert_message` (決定的 ID と `ON CONFLICT DO NOTHING`) で、ZIP の取り込みや
//!   ブリッジ (#293) と重なっても重複しない。
//! - **穴埋め**: ブリッジが取りこぼしたときは `request_channel` で、そのチャンネルをすぐに同期する。
//!
//! 取り込み先は、すでに対応付けた zoo のチャンネルだけで、新しいチャンネルは作らない。非公開チャンネルの内容は
//! その取り込み先 (参加者だけに見える) にしか入らない。Slack が非公開と答えたチャンネルが公開の zoo
//! チャンネルに対応付いていたら、取り込まない。
//!
//! 同期するのは新しい投稿と、最近のスレッドの新しい返信だけである。取り込み済みの投稿の編集・削除は反映しない
//! (`conversations.history` は削除を返さないので、イベント (#293) で扱う)。

use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use serde::Deserialize;
use serde_json::Value;

use super::{
    crypto,
    export::{SlackMessage, SlackUser},
    worker::{Context, FilePolicy, insert_message, suppress_app_events},
};
use crate::{apps::query, error::AppError, state::AppState};

/// 1 回の同期が終わってから、同じチャンネルをもう一度同期するまでの間隔
const SYNC_INTERVAL_SECS: u64 = 300;
const LEASE_SQL: &str = "interval '5 minutes'";
/// 1 回の `conversations.history` / `replies` で求める件数
const PAGE_LIMIT: &str = "200";
/// 1 回のキューの処理で進めるページ数。これを超えたら、ほかのチャンネルに順番を譲る
const MAX_PAGES_PER_STEP: usize = 10;
/// 取り込み済みのスレッドに新しい返信がないか確かめる範囲 (スレッドの親の `ts` が、この秒数より新しいもの)
const THREAD_WINDOW_SECS: u64 = 3 * 24 * 60 * 60;
/// 1 回の同期で返信を取り直すスレッドの上限 (新しい順)
const MAX_THREADS: usize = 20;
/// 履歴の取得中に、返信を取り直す候補として覚える投稿の上限 (最後に新しい順の `MAX_THREADS` 件に絞る)
const MAX_CANDIDATES: usize = 200;
/// 1 回の同期で、投稿者の名前を `users.info` で調べる人数の上限
const MAX_PROFILE_LOOKUPS: usize = 20;
/// 名前の取得にかけてよい時間。超えたら、残りの人は取得せず、既定の名前にする
const PROFILE_LOOKUP_BUDGET: Duration = Duration::from_secs(90);
/// トークンの持ち主が、そのチャンネルを読めない (またはトークンが使えない) ときの Slack のエラー。
/// ほかの連携済みメンバーのトークンで取り直す
const ACCESS_ERRORS: &[&str] = &[
    "invalid_auth",
    "token_revoked",
    "token_expired",
    "account_inactive",
    "not_authed",
    "missing_scope",
    "channel_not_found",
    "not_in_channel",
    "access_denied",
    "no_permission",
    "user_is_restricted",
    "team_access_not_granted",
];

pub async fn worker(s: AppState) {
    let mut ticks = tokio::time::interval(Duration::from_secs(2));
    let mut last_enqueue: Option<std::time::Instant> = None;
    loop {
        ticks.tick().await;
        // 新しく対応付いたチャンネルを、30 秒ごとにキューへ入れる
        if last_enqueue.is_none_or(|t| t.elapsed() >= Duration::from_secs(30))
            && s.config.slack.oauth_enabled()
        {
            last_enqueue = Some(std::time::Instant::now());
            let result = async {
                let mut db = s.db.clone();
                let mut tx = db.transaction().await?;
                enqueue(&mut tx, "", false).await?;
                tx.commit().await?;
                Ok::<_, AppError>(())
            }
            .await;
            if let Err(e) = result {
                tracing::warn!(error = %e, "Failed to enqueue Slack syncs");
            }
        }
        // 順番待ちのチャンネルがなくなるまで続ける。ただし、一巡に時間がかかっても新しいチャンネルの登録に
        // 戻れるよう、一定の時間で外側へ戻る
        let started = std::time::Instant::now();
        loop {
            if started.elapsed() >= Duration::from_secs(30) {
                break;
            }
            match run_once(&s).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    tracing::warn!(error = %e, "Slack sync worker failed");
                    break;
                }
            }
        }
    }
}

/// 対応済みのチャンネルをキューに入れる。`team` が空なら全ワークスペース。連携済みのメンバーがいるワークスペースの
/// チャンネルだけを入れる。`now` なら、すでにあるものも待たずに次の実行を今にする
pub async fn enqueue(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    now: bool,
) -> Result<(), AppError> {
    let mut statement = toasty::sql::statement(
        "INSERT INTO slack_channel_syncs(team_id,slack_channel_id) \
         SELECT sc.team_id,sc.slack_channel_id FROM slack_channels sc \
         WHERE ($1='' OR sc.team_id=$1) \
         AND EXISTS(SELECT 1 FROM slack_user_links l WHERE l.team_id=sc.team_id) \
         ON CONFLICT DO NOTHING",
    );
    statement = statement.bind(team);
    statement.exec(tx).await?;
    if now {
        let mut statement = toasty::sql::statement(
            "UPDATE slack_channel_syncs SET available_at=now(),updated_at=now() \
             WHERE ($1='' OR team_id=$1) AND lease_token IS NULL AND available_at>now()",
        );
        statement = statement.bind(team);
        statement.exec(tx).await?;
    }
    Ok(())
}

/// 連携したときに、そのワークスペースのチャンネルをすぐに同期する
pub async fn enqueue_team(tx: &mut toasty::Transaction<'_>, team: &str) -> Result<(), AppError> {
    enqueue(tx, team, true).await
}

/// ブリッジ (#293) が取りこぼした (ダウンタイムなど) チャンネルを、すぐに同期する穴埋め。
/// 同期カーソルより後を取り直すので、取り込み済みの投稿とは重複しない
pub async fn request_channel(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_channel: &str,
) -> Result<(), AppError> {
    let mut statement = toasty::sql::statement(
        "INSERT INTO slack_channel_syncs(team_id,slack_channel_id) \
         SELECT team_id,slack_channel_id FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2 \
         ON CONFLICT DO NOTHING",
    );
    statement = statement.bind(team).bind(slack_channel);
    statement.exec(tx).await?;
    let mut statement = toasty::sql::statement(
        "UPDATE slack_channel_syncs SET available_at=now(),updated_at=now() \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token IS NULL AND available_at>now()",
    );
    statement = statement.bind(team).bind(slack_channel);
    statement.exec(tx).await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
struct Task {
    team_id: String,
    slack_channel_id: String,
    channel_id: String,
    private: bool,
    deleted: bool,
    last_ts: Option<String>,
    /// 一度でも同期が終わったことがあるか
    synced: bool,
    token: String,
    attempts: i64,
    running: bool,
    phase: String,
    run_oldest: Option<String>,
    run_max_ts: Option<String>,
    page_cursor: Option<String>,
    pending_threads: String,
    known_threads: String,
    token_user_id: Option<String>,
}

/// 1 回の処理の結果
enum Step {
    /// 最後まで同期した (キューの項目は、次の実行の日時まで保存済み)
    Done,
    /// ページ数の上限に達した。続きはすぐに別の処理で行う
    More,
    /// このチャンネルは、しばらく同期しない
    Park(u64, String),
}

pub(super) enum SyncError {
    RateLimited(u64),
    /// どのトークンでも読めなかった
    NoAccess(String),
    InvalidCursor,
    LeaseLost,
    Failed(String),
}

impl From<AppError> for SyncError {
    fn from(e: AppError) -> Self {
        SyncError::Failed(e.to_string())
    }
}

/// キューから 1 チャンネルを取り、進められるところまで進める。取ったら true
pub async fn run_once(s: &AppState) -> Result<bool, AppError> {
    let Some(key) = s
        .config
        .slack
        .oauth_enabled()
        .then(|| s.config.slack.encryption_key())
        .flatten()
    else {
        return Ok(false);
    };
    let Some(task) = claim(s).await? else {
        return Ok(false);
    };
    let result = sync_channel(s, &key, &task).await;
    match result {
        Ok(Step::Done) => {}
        Ok(Step::More) => release(s, &task, 0, None, false).await?,
        Ok(Step::Park(secs, reason)) => release(s, &task, secs, Some(&reason), false).await?,
        Err(SyncError::LeaseLost) => {}
        Err(SyncError::RateLimited(secs)) => {
            tracing::info!(team = %task.team_id, channel = %task.slack_channel_id, secs, "Slack sync rate limited");
            release(s, &task, secs.clamp(1, 3600), None, false).await?;
        }
        Err(SyncError::NoAccess(error)) => {
            tracing::info!(team = %task.team_id, channel = %task.slack_channel_id, %error, "No linked Slack user can read the channel");
            release(s, &task, 3600, Some(&error), false).await?;
        }
        Err(SyncError::InvalidCursor) => {
            // ページのカーソルが期限切れ。保存済みの位置を捨てて、カーソルの続きから取り直す (取り込みは冪等)
            reset_run(s, &task).await?;
        }
        Err(SyncError::Failed(error)) => {
            tracing::warn!(team = %task.team_id, channel = %task.slack_channel_id, %error, "Slack sync failed");
            let secs = (30u64 << task.attempts.clamp(0, 7) as u32).min(3600);
            release(s, &task, secs, Some(&error), true).await?;
        }
    }
    Ok(true)
}

async fn claim(s: &AppState) -> Result<Option<Task>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let token = uuid::Uuid::new_v4().to_string();
    // 取り込みのジョブが動いている間は、同じワークスペースの同期を待つ (カーソルを取り合わない)。
    // 取り込みが一度も終わっていないチャンネル (カーソルがない) は、全履歴を取らないよう同期しない
    let rows = query(
        &mut tx,
        &format!(
            "WITH c AS (SELECT t.team_id,t.slack_channel_id FROM slack_channel_syncs t \
             JOIN slack_channels sc ON sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id \
             JOIN channels ch ON ch.id=sc.channel_id \
             WHERE t.available_at<=now() AND (t.lease_until IS NULL OR t.lease_until<now()) \
             AND (sc.last_ts IS NOT NULL OR ch.slack_imported_at IS NOT NULL) \
             AND NOT EXISTS(SELECT 1 FROM slack_import_jobs j WHERE j.team_id=t.team_id \
               AND j.status IN ('analyzing','importing')) \
             ORDER BY t.available_at,t.team_id,t.slack_channel_id FOR UPDATE OF t SKIP LOCKED LIMIT 1) \
             UPDATE slack_channel_syncs t SET lease_until=now()+{LEASE_SQL},lease_token=$1,updated_at=now() \
             FROM c, slack_channels sc, channels ch \
             WHERE t.team_id=c.team_id AND t.slack_channel_id=c.slack_channel_id \
             AND sc.team_id=t.team_id AND sc.slack_channel_id=t.slack_channel_id AND ch.id=sc.channel_id \
             RETURNING json_build_object('team_id',t.team_id,'slack_channel_id',t.slack_channel_id,\
             'channel_id',ch.id,'private',ch.is_private,'deleted',ch.deleted_at IS NOT NULL,\
             'last_ts',sc.last_ts,'synced',t.last_synced_at IS NOT NULL,'token',t.lease_token,'attempts',t.attempts,'running',t.running,\
             'phase',t.phase,'run_oldest',t.run_oldest,'run_max_ts',t.run_max_ts,\
             'page_cursor',t.page_cursor,'pending_threads',t.pending_threads,\
             'known_threads',t.known_threads,'token_user_id',t.token_user_id)::text"
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

/// リースを手放し、`secs` 秒後に次の処理を行う。`bump` なら試行回数を増やす (増えるほど待ちが延びる)
async fn release(
    s: &AppState,
    task: &Task,
    secs: u64,
    error: Option<&str>,
    bump: bool,
) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    query(
        &mut tx,
        "UPDATE slack_channel_syncs SET lease_until=NULL,lease_token=NULL,\
         available_at=now()+make_interval(secs=>$3::integer),last_error=NULLIF($4,''),\
         attempts=CASE WHEN $5::boolean THEN attempts+1 ELSE 0 END,updated_at=now() \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token=$6 \
         RETURNING json_build_object('ok',true)::text",
        &[
            &task.team_id,
            &task.slack_channel_id,
            &secs.to_string(),
            error.unwrap_or_default(),
            if bump { "true" } else { "false" },
            &task.token,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn reset_run(s: &AppState, task: &Task) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    query(
        &mut tx,
        "UPDATE slack_channel_syncs SET running=false,phase='history',run_oldest=NULL,run_max_ts=NULL,\
         page_cursor=NULL,pending_threads='[]',lease_until=NULL,lease_token=NULL,available_at=now(),\
         last_error='invalid_cursor',updated_at=now() \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token=$3 \
         RETURNING json_build_object('ok',true)::text",
        &[&task.team_id, &task.slack_channel_id, &task.token],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Slack Web API
// ---------------------------------------------------------------------------

/// 連携済みメンバーのトークンで Slack を呼ぶ。読めないトークンは飛ばして次のトークンで呼び直す
pub(super) struct Slack {
    client: reqwest::Client,
    base: String,
    key: [u8; 32],
    /// (zoo のユーザー ID, 暗号化したトークン)。使うときに復号する
    tokens: Vec<(String, String)>,
    index: usize,
}

impl Slack {
    /// 1 人のトークンだけで Slack を呼ぶ (ユーザーが自分で始める取り込み用)
    pub(super) fn single(
        s: &AppState,
        key: [u8; 32],
        user: &str,
        enc: &str,
    ) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| AppError::Internal(e.into()))?;
        Ok(Self {
            client,
            base: s.config.slack.api_base_url.clone(),
            key,
            tokens: vec![(user.to_string(), enc.to_string())],
            index: 0,
        })
    }

    fn user(&self) -> Option<&str> {
        self.tokens.get(self.index).map(|(u, _)| u.as_str())
    }

    pub(super) async fn get(
        &mut self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<Value, SyncError> {
        loop {
            let Some((user, enc)) = self.tokens.get(self.index) else {
                return Err(SyncError::NoAccess("no_usable_token".into()));
            };
            let Ok(token) = crypto::decrypt(&self.key, enc) else {
                tracing::warn!(user_id = %user, "Failed to decrypt a Slack token");
                self.index += 1;
                continue;
            };
            let response = self
                .client
                .get(format!("{}/{method}", self.base))
                .query(params)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| SyncError::Failed(format!("{method}: {e}")))?;
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(SyncError::RateLimited(retry_after(&response)));
            }
            if !response.status().is_success() {
                return Err(SyncError::Failed(format!(
                    "{method}: HTTP {}",
                    response.status()
                )));
            }
            let value: Value = response
                .json()
                .await
                .map_err(|e| SyncError::Failed(format!("{method}: invalid response: {e}")))?;
            if value["ok"] == true {
                return Ok(value);
            }
            let error = value["error"].as_str().unwrap_or("unknown_error");
            if error == "ratelimited" {
                return Err(SyncError::RateLimited(60));
            }
            if error == "invalid_cursor" {
                return Err(SyncError::InvalidCursor);
            }
            if ACCESS_ERRORS.contains(&error) {
                self.index += 1;
                if self.index >= self.tokens.len() {
                    return Err(SyncError::NoAccess(error.to_string()));
                }
                continue;
            }
            return Err(SyncError::Failed(format!("{method}: {error}")));
        }
    }
}

fn retry_after(response: &reqwest::Response) -> u64 {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(60)
}

fn parse_messages(value: &Value) -> Vec<SlackMessage> {
    value["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| serde_json::from_value(m.clone()).ok())
        .collect()
}

/// `ts` を (秒, 小数部) にして比べる。形式が妥当でないものは None
fn ts_key(ts: &str) -> Option<(u64, u64)> {
    if !super::is_valid_ts(ts) {
        return None;
    }
    let mut parts = ts.splitn(2, '.');
    let secs = parts.next()?.parse().ok()?;
    let frac = parts.next().unwrap_or("0");
    // 小数部は 9 桁に揃えて比べる
    let padded = format!("{frac:0<9}");
    Some((secs, padded.parse().ok()?))
}

fn max_ts(a: Option<String>, b: &str) -> Option<String> {
    match (a.as_deref().and_then(ts_key), ts_key(b)) {
        (Some(x), Some(y)) if y > x => Some(b.to_string()),
        (None, Some(_)) => Some(b.to_string()),
        _ => a,
    }
}

/// ユーザーが API から取り込みを始めた (カーソルが 0 で、まだ一度も同期が終わっていない) チャンネルの最初の同期か
fn is_initial(task: &Task) -> bool {
    !task.synced && task.last_ts.as_deref() == Some("0")
}

/// 新しい同期の起点。最初の同期では、ZIP に含まれる最近のスレッドの新しい返信も取れるよう、カーソルより
/// スレッドの監視範囲だけ前から取り直す (取り込み済みの投稿は、決定的 ID で重複しない)
fn first_oldest(task: &Task) -> String {
    let cursor = task.last_ts.clone().filter(|t| super::is_valid_ts(t));
    if task.synced {
        return cursor.unwrap_or_else(|| "0".into());
    }
    let horizon = now_secs().saturating_sub(THREAD_WINDOW_SECS);
    match cursor {
        // カーソルがなければ (取り込みが空だったチャンネル) 全履歴から取る
        None => "0".into(),
        Some(c) if ts_key(&c).is_some_and(|(secs, _)| secs <= horizon) => c,
        Some(_) => format!("{horizon}.000000"),
    }
}

/// スレッドの親になれる投稿か。入退室・トピック変更などのシステム通知は、スレッドにできない
/// (`conversations.replies` が `thread_not_found` になる) ので、候補から外す
fn threadable(message: &SlackMessage) -> bool {
    message.is_importable()
        && message
            .subtype
            .as_deref()
            .is_none_or(|t| !(t.starts_with("channel_") || t.starts_with("group_")))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// 同期
// ---------------------------------------------------------------------------

/// 保存する途中経過
struct Progress {
    phase: String,
    oldest: String,
    max_ts: Option<String>,
    cursor: Option<String>,
    pending: Vec<String>,
    /// 次の同期でも返信を確かめるスレッド (`pending` は処理するたびに減るので、別に持つ)
    known: Vec<String>,
}

async fn load_tokens(s: &AppState, task: &Task) -> Result<Vec<(String, String)>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    // 非公開の zoo チャンネルは、そのメンバーのトークンだけを使う。公開なら、メンバーを先にして、ほかの人も使う。
    // 前回読めたトークンを先に試す
    let rows = query(
        &mut tx,
        "SELECT json_build_object('user_id',l.user_id,'token',l.access_token_enc)::text \
         FROM slack_user_links l WHERE l.team_id=$1 \
         AND (NOT $3::boolean OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=$2 AND cu.user_id=l.user_id)) \
         ORDER BY (l.user_id=$4) DESC,\
         EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=$2 AND cu.user_id=l.user_id) DESC,\
         l.updated_at DESC,l.user_id",
        &[
            &task.team_id,
            &task.channel_id,
            if task.private { "true" } else { "false" },
            task.token_user_id.as_deref().unwrap_or(""),
        ],
    )
    .await?;
    tx.commit().await?;
    // 先頭のトークンが使えなくても、残りのトークンを試せるよう、人数で切らずに全員分を返す
    let tokens: Vec<(String, String)> = rows
        .into_iter()
        .filter_map(|row| {
            Some((
                row["user_id"].as_str()?.to_string(),
                row["token"].as_str()?.to_string(),
            ))
        })
        .collect();
    Ok(tokens)
}

async fn sync_channel(s: &AppState, key: &[u8; 32], task: &Task) -> Result<Step, SyncError> {
    if !s.config.slack.team_allowed(&task.team_id) {
        return Ok(Step::Park(86_400, "team_mismatch".into()));
    }
    if task.deleted {
        return Ok(Step::Park(86_400, "channel_deleted".into()));
    }
    let tokens = load_tokens(s, task).await?;
    if tokens.is_empty() {
        return Ok(Step::Park(
            3600,
            "no linked user can read this channel".into(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(e.into()))?;
    let mut slack = Slack {
        client,
        base: s.config.slack.api_base_url.clone(),
        key: *key,
        tokens,
        index: 0,
    };
    let mut db = s.db.clone();

    // 再開のときも、毎回確かめる。保存した途中経過のあとで、取り込み先が公開に変わっていることがある。
    // 非公開の Slack チャンネルの履歴を、公開の zoo チャンネルに入れない
    let info = slack
        .get(
            "conversations.info",
            &[("channel", task.slack_channel_id.as_str())],
        )
        .await?;
    let channel = &info["channel"];
    let slack_private = ["is_private", "is_im", "is_mpim", "is_group"]
        .iter()
        .any(|k| channel[*k] == true);
    if slack_private && !task.private {
        return Ok(Step::Park(
            6 * 3600,
            "private_slack_channel_in_public_channel".into(),
        ));
    }
    let known: Vec<String> = serde_json::from_str(&task.known_threads).unwrap_or_default();
    let mut progress = if task.running {
        Progress {
            phase: task.phase.clone(),
            oldest: task.run_oldest.clone().unwrap_or_else(|| "0".into()),
            max_ts: task.run_max_ts.clone(),
            cursor: task.page_cursor.clone(),
            pending: serde_json::from_str(&task.pending_threads).unwrap_or_default(),
            known,
        }
    } else {
        Progress {
            phase: "history".into(),
            oldest: first_oldest(task),
            max_ts: None,
            cursor: None,
            pending: Vec::new(),
            known,
        }
    };

    let mut ctx = Context {
        team: task.team_id.clone(),
        users: HashMap::new(),
        profiles: HashMap::new(),
        channel_names: HashMap::new(),
        files: FilePolicy::new(s),
    };
    // 同期した投稿の添付は、いまはファイル名を本文に残すだけにする (取得は取り込みのジョブだけが行う)
    ctx.files.token = false;
    {
        let mut tx = db.transaction().await.map_err(AppError::from)?;
        // `<#C…>` は、非公開でない取り込み先の今の名前に直す (取り込みと同じ)
        for row in query(
            &mut tx,
            "SELECT json_build_object('slack',s.slack_channel_id,'name',c.channel_name)::text \
             FROM slack_channels s JOIN channels c ON c.id=s.channel_id \
             WHERE s.team_id=$1 AND c.deleted_at IS NULL AND NOT c.is_private",
            &[&task.team_id],
        )
        .await?
        {
            if let (Some(slack), Some(name)) = (row["slack"].as_str(), row["name"].as_str()) {
                ctx.channel_names
                    .insert(slack.to_string(), name.to_string());
            }
        }
        tx.commit().await.map_err(AppError::from)?;
    }

    // ユーザーが API から取り込みを始めたチャンネルの最初の同期 (カーソルが 0)。全履歴を取るので、返信も、
    // 差分同期の監視範囲 (3 日・最大 20 スレッド) には限らず、返信のある全スレッドから取る
    let initial = is_initial(task);
    for _ in 0..MAX_PAGES_PER_STEP {
        if progress.phase == "history" {
            let mut params = vec![
                ("channel", task.slack_channel_id.as_str()),
                ("oldest", progress.oldest.as_str()),
                ("limit", PAGE_LIMIT),
            ];
            if let Some(cursor) = progress.cursor.as_deref() {
                params.push(("cursor", cursor));
            }
            let page = slack.get("conversations.history", &params).await?;
            let messages = parse_messages(&page);
            let horizon = now_secs().saturating_sub(THREAD_WINDOW_SECS);
            for message in &messages {
                progress.max_ts = max_ts(progress.max_ts.take(), &message.ts);
                // スレッドの親になりうる投稿 (返信でないもの) は、まだ返信がなくても、範囲内なら監視の候補に入れる。
                // 同期のあとで初めて返信が付いても、次回以降の同期で取り込めるようにするため
                let top_level = message
                    .thread_ts
                    .as_deref()
                    .is_none_or(|t| t == message.ts.as_str());
                let recent = ts_key(&message.ts).is_some_and(|(secs, _)| secs >= horizon);
                let candidate = if initial {
                    // 最初の同期は、返信のあるスレッドをすべて取る (範囲内の親は、あとで返信が付くので監視に残す)
                    recent || message.reply_count.unwrap_or(0) > 0
                } else {
                    recent && progress.pending.len() < MAX_CANDIDATES
                };
                if top_level
                    && threadable(message)
                    && candidate
                    && !progress.pending.contains(&message.ts)
                {
                    progress.pending.push(message.ts.clone());
                }
            }
            let next = page["response_metadata"]["next_cursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            let more = page["has_more"] == true && next.is_some();
            progress.cursor = next.filter(|_| more);
            if !more {
                progress.phase = "threads".into();
                // 前回までに見つけたスレッドにも、新しい返信がないか確かめる
                for ts in progress.known.clone() {
                    if !progress.pending.contains(&ts) {
                        progress.pending.push(ts);
                    }
                }
                let horizon = now_secs().saturating_sub(THREAD_WINDOW_SECS);
                if initial {
                    // 古いスレッドも返信を取るが、次回以降に監視するのは、範囲内の新しいスレッドだけ
                    progress
                        .pending
                        .sort_by_key(|ts| std::cmp::Reverse(ts_key(ts)));
                    progress.known = progress
                        .pending
                        .iter()
                        .filter(|ts| ts_key(ts).is_some_and(|(secs, _)| secs >= horizon))
                        .take(MAX_THREADS)
                        .cloned()
                        .collect();
                } else {
                    progress
                        .pending
                        .retain(|ts| ts_key(ts).is_some_and(|(secs, _)| secs >= horizon));
                    progress
                        .pending
                        .sort_by_key(|ts| std::cmp::Reverse(ts_key(ts)));
                    progress.pending.truncate(MAX_THREADS);
                    progress.known = progress.pending.clone();
                }
            }
            store(&mut slack, &mut ctx, task, &progress, &messages, &mut db).await?;
        } else {
            let Some(thread) = progress.pending.first().cloned() else {
                return finish(s, task, &progress).await;
            };
            let mut params = vec![
                ("channel", task.slack_channel_id.as_str()),
                ("ts", thread.as_str()),
                ("oldest", progress.oldest.as_str()),
                ("limit", PAGE_LIMIT),
            ];
            if let Some(cursor) = progress.cursor.as_deref() {
                params.push(("cursor", cursor));
            }
            let page = match slack.get("conversations.replies", &params).await {
                Ok(page) => page,
                // 消されたスレッドなどは、飛ばして次へ進む
                Err(SyncError::Failed(e)) if e.ends_with("thread_not_found") => {
                    progress.pending.remove(0);
                    progress.cursor = None;
                    store(&mut slack, &mut ctx, task, &progress, &[], &mut db).await?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let messages = parse_messages(&page);
            let next = page["response_metadata"]["next_cursor"]
                .as_str()
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            let more = page["has_more"] == true && next.is_some();
            progress.cursor = next.filter(|_| more);
            if !more {
                progress.pending.remove(0);
            }
            store(&mut slack, &mut ctx, task, &progress, &messages, &mut db).await?;
        }
    }
    if progress.phase == "threads" && progress.pending.is_empty() {
        return finish(s, task, &progress).await;
    }
    Ok(Step::More)
}

/// 1 ページ分の投稿と途中経過を、同じトランザクションで保存する
async fn store(
    slack: &mut Slack,
    ctx: &mut Context,
    task: &Task,
    progress: &Progress,
    messages: &[SlackMessage],
    db: &mut toasty::Db,
) -> Result<(), SyncError> {
    // 投稿者の名前を調べる (トランザクションの外で行う。調べられなくても、Slack の ID の名前で作る)
    lookup_profiles(db, slack, ctx, messages).await?;
    let mut importable: Vec<&SlackMessage> =
        messages.iter().filter(|m| m.is_importable()).collect();
    importable.sort_by_key(|m| ts_key(&m.ts));
    let mut tx = db.transaction().await.map_err(AppError::from)?;
    suppress_app_events(&mut tx).await?;
    let pending = serde_json::to_string(&progress.pending).unwrap_or_else(|_| "[]".into());
    let saved = query(
        &mut tx,
        "UPDATE slack_channel_syncs SET running=true,phase=$3,run_oldest=$4,run_max_ts=NULLIF($5,''),\
         page_cursor=NULLIF($6,''),pending_threads=$7,token_user_id=COALESCE(NULLIF($8,''),token_user_id),\
         known_threads=$10,attempts=0,last_error=NULL,lease_until=now()+interval '5 minutes',updated_at=now() \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token=$9 \
         RETURNING json_build_object('ok',true)::text",
        &[
            &task.team_id,
            &task.slack_channel_id,
            &progress.phase,
            &progress.oldest,
            progress.max_ts.as_deref().unwrap_or(""),
            progress.cursor.as_deref().unwrap_or(""),
            &pending,
            slack.user().unwrap_or(""),
            &task.token,
            &serde_json::to_string(&progress.known).unwrap_or_else(|_| "[]".into()),
        ],
    )
    .await?;
    if saved.is_empty() {
        return Err(SyncError::LeaseLost);
    }
    // 同じ区間をほかのトランザクションが同時に取り込んでも、決定的 ID で重複しない
    for message in importable {
        insert_message(
            &mut tx,
            ctx,
            &task.slack_channel_id,
            &task.channel_id,
            "",
            message,
        )
        .await?;
    }
    tx.commit().await.map_err(AppError::from)?;
    Ok(())
}

/// 投稿者とメンションされた人のうち、まだ zoo にいない Slack ユーザーの名前を `users.info` で調べる
async fn lookup_profiles(
    db: &mut toasty::Db,
    slack: &mut Slack,
    ctx: &mut Context,
    messages: &[SlackMessage],
) -> Result<(), SyncError> {
    let mut wanted: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for message in messages.iter().filter(|m| m.is_importable()) {
        let authors = message.author().map(str::to_string).into_iter();
        let mentions = super::convert::mentioned_users(&message.text).into_iter();
        for id in authors.chain(mentions) {
            // bot (B…) は users.info で引けない。ユーザー (U…・W…) だけを調べる
            if (id.starts_with('U') || id.starts_with('W'))
                && !ctx.users.contains_key(&id)
                && !ctx.profiles.contains_key(&id)
                && seen.insert(id.clone())
            {
                wanted.push(id);
            }
        }
    }
    if wanted.is_empty() {
        return Ok(());
    }
    let mut tx = db.transaction().await.map_err(AppError::from)?;
    let known = query(
        &mut tx,
        "SELECT json_build_object('id',slack_user_id)::text FROM slack_users \
         WHERE team_id=$1 AND slack_user_id IN (SELECT jsonb_array_elements_text($2::jsonb))",
        &[&ctx.team, &serde_json::json!(wanted).to_string()],
    )
    .await?;
    tx.commit().await.map_err(AppError::from)?;
    let known: HashSet<String> = known
        .into_iter()
        .filter_map(|r| r["id"].as_str().map(str::to_string))
        .collect();
    // 1 回の要求は最大 30 秒かかる。ページの保存 (リースの延長) まで 5 分のリースを切らさないよう、時間で打ち切る
    let started = std::time::Instant::now();
    for id in wanted
        .into_iter()
        .filter(|id| !known.contains(id))
        .take(MAX_PROFILE_LOOKUPS)
    {
        if started.elapsed() >= PROFILE_LOOKUP_BUDGET {
            break;
        }
        match slack.get("users.info", &[("user", id.as_str())]).await {
            Ok(value) => {
                if let Ok(user) = serde_json::from_value::<SlackUser>(value["user"].clone()) {
                    ctx.profiles.insert(id, user);
                }
            }
            // 名前が引けなくても、同期は止めない (429 は次の呼び出しで返る)
            Err(SyncError::RateLimited(_)) => break,
            Err(_) => {}
        }
    }
    Ok(())
}

/// 同期を終える。カーソルを進め、次の実行を予約する
async fn finish(s: &AppState, task: &Task, progress: &Progress) -> Result<Step, SyncError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await.map_err(AppError::from)?;
    // 新しい投稿があったときだけ、カーソルを進める (逆戻りはしない)
    if let Some(ts) = progress.max_ts.as_deref() {
        query(
            &mut tx,
            "UPDATE slack_channels SET last_ts=$3,updated_at=now() WHERE team_id=$1 AND slack_channel_id=$2 \
             AND (last_ts IS NULL OR last_ts::numeric<$3::numeric) RETURNING json_build_object('ok',true)::text",
            &[&task.team_id, &task.slack_channel_id, ts],
        )
        .await?;
    }
    // API から取り込みを始めたチャンネルの最初の同期では、既存メンバーに過去ログが未読として大量に出ないよう、
    // 既読位置を取り込んだ過去ログの最新 (同期カーソルと、返信を含む取り込み済みの最新の投稿の新しいほう。
    // 取り込みを始めたあとに Zoo で投稿された分は含めない) まで進める。ZIP の取り込みと同じで、それより新しい投稿の未読は残る
    if is_initial(task) {
        query(
            &mut tx,
            "INSERT INTO channel_read_states(user_id,channel_id,last_read_at) \
             SELECT cu.user_id,cu.channel_id,m.latest FROM channel_user cu JOIN users u ON u.id=cu.user_id \
             CROSS JOIN (SELECT GREATEST((to_timestamp(last_ts::numeric) AT TIME ZONE 'UTC')::timestamp(3),\
               COALESCE((SELECT max(x.created_at) FROM messages x WHERE x.channel_id=$1 AND x.deleted_at IS NULL \
                 AND x.created_at<(SELECT (t.created_at AT TIME ZONE 'UTC') FROM slack_channel_syncs t \
                   WHERE t.team_id=$2 AND t.slack_channel_id=$3)),'-infinity'::timestamp)) AS latest \
             FROM slack_channels WHERE team_id=$2 AND slack_channel_id=$3 AND last_ts IS NOT NULL AND last_ts<>'0') m \
             WHERE cu.channel_id=$1 AND NOT u.is_deactivated \
             ON CONFLICT (user_id,channel_id) DO UPDATE \
             SET last_read_at=GREATEST(channel_read_states.last_read_at,EXCLUDED.last_read_at) \
             RETURNING json_build_object('ok',true)::text",
            &[&task.channel_id, &task.team_id, &task.slack_channel_id],
        )
        .await?;
    }
    // 次の同期でも返信を確かめるスレッドは、今回見つけたものと前回のものの、範囲内の新しいもの
    let mut known = progress.known.clone();
    let horizon = now_secs().saturating_sub(THREAD_WINDOW_SECS);
    known.retain(|ts| ts_key(ts).is_some_and(|(secs, _)| secs >= horizon));
    known.sort_by_key(|ts| std::cmp::Reverse(ts_key(ts)));
    known.dedup();
    known.truncate(MAX_THREADS);
    // 前回の同期の終わりのあとに連携した人がいれば、同期中に連携した可能性がある (投稿者・メンション先・リアクション
    // のどれでも、キャッシュしたプレースホルダのまま保存されている)。そのときだけ、ZIP の取り込みと同じ付け替え
    // (`reconcile_team`) をする。付け替えは全ワークスペース共通のロックを取り、チームの投稿本文も走査するので、
    // 連携の変わっていない無更新の同期では呼ばない。前回の終わりの日時は、下の UPDATE の前に読む
    let relinked = !query(
        &mut tx,
        "SELECT json_build_object('found',true)::text FROM slack_users su \
         JOIN slack_channel_syncs t ON t.team_id=su.team_id AND t.slack_channel_id=$2 \
         WHERE su.team_id=$1 AND su.linked AND (t.last_synced_at IS NULL OR su.updated_at>t.last_synced_at) LIMIT 1",
        &[&task.team_id, &task.slack_channel_id],
    )
    .await?
    .is_empty();
    let done = query(
        &mut tx,
        "UPDATE slack_channel_syncs SET running=false,phase='history',run_oldest=NULL,run_max_ts=NULL,\
         page_cursor=NULL,pending_threads='[]',known_threads=$3,attempts=0,last_error=NULL,\
         last_synced_at=now(),available_at=now()+make_interval(secs=>$4::integer),\
         lease_until=NULL,lease_token=NULL,updated_at=now() \
         WHERE team_id=$1 AND slack_channel_id=$2 AND lease_token=$5 \
         RETURNING json_build_object('ok',true)::text",
        &[
            &task.team_id,
            &task.slack_channel_id,
            &serde_json::to_string(&known).unwrap_or_else(|_| "[]".into()),
            &SYNC_INTERVAL_SECS.to_string(),
            &task.token,
        ],
    )
    .await?;
    if done.is_empty() {
        return Err(SyncError::LeaseLost);
    }
    if relinked {
        super::link::reconcile_team(&mut tx, &task.team_id).await?;
    }
    tx.commit().await.map_err(AppError::from)?;
    Ok(Step::Done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_timestamps_numerically() {
        assert!(ts_key("1700000000.000100") < ts_key("1700000000.000200"));
        assert!(ts_key("1700000000.5") > ts_key("1700000000.4999"));
        assert!(ts_key("1700000001") > ts_key("1700000000.999999"));
        assert_eq!(ts_key("1;DROP"), None);
        assert_eq!(
            max_ts(Some("1700000000.000100".into()), "1700000000.000200").as_deref(),
            Some("1700000000.000200")
        );
        assert_eq!(
            max_ts(Some("1700000000.000200".into()), "1700000000.000100").as_deref(),
            Some("1700000000.000200")
        );
        assert_eq!(max_ts(None, "17x"), None);
    }
}
