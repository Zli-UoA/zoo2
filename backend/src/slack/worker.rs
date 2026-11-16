//! Slack 取り込みジョブのワーカー。
//!
//! 本番は複数台で動くため、`apps::delivery` と同じく `FOR UPDATE SKIP LOCKED` とリースで
//! 1 ジョブを 1 台だけが進める。処理中はリースを延長し続け、止まったらリース切れ後に
//! 別の台が同じ段階をやり直す。書き込みは決定的 ID と `ON CONFLICT DO NOTHING` で冪等。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use serde::Deserialize;
use serde_json::json;

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
/// リースを失ったとき (別の台が同じジョブを処理している) のエラー。一意制約違反の Conflict と区別する
const LEASE_LOST: &str = "Slack import lease was lost";
const LEASE_SQL: &str = "interval '2 minutes'";
const HEARTBEAT: Duration = Duration::from_secs(30);
/// 終了の合図で手放したジョブを、別のワーカーが取れるようになるまでの間隔 (終了処理中の古い台が取り直さないため)
/// 手放すときに、データベースを待つ最長の時間 (超えたら、リースが切れるのを待つ)
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);
const HANDOFF_DELAY_SQL: &str = "interval '30 seconds'";
/// カードの説明文の上限 (`MessageCard::encode` と同じ)
const MAX_CARD_DESCRIPTION: usize = 4000;
/// 添付 1 件あたりの取得失敗の上限
const MAX_FILE_ATTEMPTS: i64 = 3;
/// 429 の `Retry-After` の上限 (秒)
const MAX_RETRY_AFTER_SECS: u64 = 300;
/// 行単位の取得 (ジョブに紐づかない添付) のリース。1 件の最悪時間 (120 秒 × 最大 4 リクエスト + 保存) より長くする
const FILE_LEASE_SQL: &str = "interval '10 minutes'";
/// 行単位の取得で、取る行がないときに待つ時間
const FILE_IDLE: Duration = Duration::from_secs(30);

/// Lock the importer before any target-channel row is updated when this job can create a
/// channel. `create_channel` inserts the importer into `channel_user`, whose trigger locks users.
async fn lock_creator_for_channel_creation(
    tx: &mut toasty::Transaction<'_>,
    mappings: &[Mapping],
    creator: &str,
) -> Result<(), AppError> {
    if mappings
        .iter()
        .any(|mapping| matches!(mapping.action, MappingAction::Create { .. }))
    {
        crate::routes::e2ee_channels::lock_membership_users(tx, &[creator]).await?;
    }
    Ok(())
}

/// 終了の合図 (SIGTERM など)。`true` になったら、新しいジョブを取らず、処理中の段階を捨ててジョブを手放す。
pub type Stop = tokio::sync::watch::Receiver<bool>;

/// 合図が来るまで待つ。送り手が先に落ちた (合図が来ない) ときは、永久に待つ
async fn stopped(stop: &mut Stop) {
    loop {
        if *stop.borrow() {
            return;
        }
        if stop.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// 終了の合図を受けたら、処理中のジョブを手放す。リースが切れるのを待たずに、別のワーカーが再開できる。
/// `attempts` は、デプロイによる中断で上限に近づかないよう 1 戻す (取得のときに増えた分)。
/// 古い台が、まだ終了処理中に取り直さないよう、少し先から取れるようにする。
/// トークンが違う (リースを奪われた) ときは何もしない。手放したら true
pub async fn release_job(s: &AppState, id: &str, token: &str) -> Result<bool, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let rows = query(
        &mut tx,
        &format!(
            "UPDATE slack_import_jobs SET attempts=GREATEST(attempts-1,0),handoffs=handoffs+1,\
             lease_until=NULL,lease_token=NULL,available_at=now()+{HANDOFF_DELAY_SQL},updated_at=now() \
             WHERE id=$1 AND lease_token=$2 \
             RETURNING json_build_object('id',id)::text"
        ),
        &[id, token],
    )
    .await?;
    tx.commit().await?;
    Ok(!rows.is_empty())
}

pub async fn worker(s: AppState) {
    // 合図を送らない (止まらない) ワーカー。送り手をここで持ち続ける
    let (_keep, stop) = tokio::sync::watch::channel(false);
    worker_until(s, stop).await;
}

/// `stop` が `true` になるまで動くワーカー。合図を受けたら、処理中のジョブを手放して戻る
pub async fn worker_until(s: AppState, mut stop: Stop) {
    // 前のプロセスが取り込みの途中で止まると、ZIP が残る（Kubernetes の emptyDir は
    // コンテナの再起動では消えない）。残ったままだと、取り直したジョブの ZIP と合わせて
    // 容量の上限を超えることがあるので、始める前に片付ける。
    let dir = s.config.slack.import_tmp_dir.clone();
    match blocking(move || remove_orphan_zips(&dir).map_err(|e| AppError::Internal(e.into()))).await
    {
        Ok(0) => {}
        Ok(removed) => tracing::info!(removed, "Removed leftover Slack import ZIPs"),
        Err(e) => tracing::warn!(error = %e, "Failed to clean up leftover Slack import ZIPs"),
    }
    // ジョブに紐づかない添付 (差分同期の添付など) の取得。ジョブの処理は長くかかるので、別のタスクで動かす。
    // Bot トークンか、ユーザートークン (連携) を使えるときだけ動かす。終わるときは、取りかけの行を手放すのを待つ
    let mut files =
        file_worker_enabled(&s).then(|| tokio::spawn(orphan_files_worker(s.clone(), stop.clone())));
    let finish = |files: Option<tokio::task::JoinHandle<()>>| async move {
        if let Some(files) = files {
            let _ = tokio::time::timeout(RELEASE_TIMEOUT, files).await;
        }
    };
    let mut ticks = tokio::time::interval(Duration::from_secs(2));
    let mut passes = 0u32;
    loop {
        tokio::select! {
            _ = ticks.tick() => {}
            _ = stopped(&mut stop) => return finish(files.take()).await,
        }
        // 添付の取得が (panic などで) 終わったら、このワーカーも終わる。呼び出し側が再起動する
        if files.as_ref().is_some_and(|f| f.is_finished()) {
            tracing::error!("The Slack file worker stopped unexpectedly");
            return;
        }
        passes = passes.wrapping_add(1);
        match run_once_until(&s, &mut stop).await {
            Ok(Step::Stopped) => return finish(files.take()).await,
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "Slack import worker failed"),
        }
        if passes.is_multiple_of(1800)
            && let Err(e) = cleanup(&s).await
        {
            tracing::warn!(error = %e, "Slack import cleanup failed");
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

/// `run_once_until` の結果
pub enum Step {
    /// 実行できるジョブがなかった
    Idle,
    /// ジョブを 1 段階進めた (失敗して、やり直しを待つ場合も含む)
    Ran,
    /// 終了の合図を受けて、ジョブを手放した (手放すジョブがなくても、合図を受けたときは、これを返す)
    Stopped,
}

/// 実行できるジョブを 1 件取り、1 段階進める。ジョブがあれば true。
pub async fn run_once(s: &AppState) -> Result<bool, AppError> {
    let (_keep, mut stop) = tokio::sync::watch::channel(false);
    Ok(matches!(run_once_until(s, &mut stop).await?, Step::Ran))
}

/// `run_once` と同じだが、`stop` が `true` になったら、段階を捨ててジョブを手放す。
/// 段階は、トランザクションごとに完結し、冪等なので、途中で捨ててよい (書き込み中のトランザクションは、
/// 捨てるとロールバックされる)。ZIP の取得・展開のような、止められない処理も、ここで待たずに捨てる。
pub async fn run_once_until(s: &AppState, stop: &mut Stop) -> Result<Step, AppError> {
    if *stop.borrow() {
        return Ok(Step::Stopped);
    }
    let Some(job) = claim(s).await? else {
        return Ok(Step::Idle);
    };
    let lease = Lease::start(s.clone(), job.id.clone(), job.token.clone());
    let step_name = job.status.clone();
    let span = tracing::info_span!(
        "slack_import_step",
        import_id = %job.id,
        step = %step_name,
        attempt = job.attempts
    );
    let started = std::time::Instant::now();
    span.in_scope(|| {
        tracing::info!(import_id = %job.id, status = %step_name, attempt = job.attempts, "Slack import step started");
    });

    use tracing::Instrument;
    let step = async {
        match step_name.as_str() {
            "analyzing" => analyze(s, &job, &lease).await,
            "importing" => import(s, &job, &lease).await,
            "fetching_files" => fetch_files(s, &job, &lease).await,
            _ => Ok(()),
        }
    }
    .instrument(span.clone());
    let result = tokio::select! {
        result = step => result,
        _ = stopped(stop) => {
            // 段階の future はここで捨てられる (トランザクションはロールバックされる)
            drop(lease);
            match tokio::time::timeout(RELEASE_TIMEOUT, release_job(s, &job.id, &job.token)).await {
                Ok(Ok(true)) => span.in_scope(|| tracing::info!(import_id = %job.id, "Released the Slack import job for shutdown")),
                Ok(Ok(false)) => span.in_scope(|| tracing::warn!(import_id = %job.id, "Slack import lease was already lost at shutdown")),
                Ok(Err(e)) => span.in_scope(|| tracing::warn!(import_id = %job.id, error = %e, "Failed to release the Slack import job; the lease will expire")),
                Err(_) => span.in_scope(|| tracing::warn!(import_id = %job.id, "Timed out releasing the Slack import job; the lease will expire")),
            }
            crate::metrics::record_slack_import_step(&step_name, "stopped", started.elapsed().as_secs_f64());
            return Ok(Step::Stopped);
        }
    };
    drop(lease);
    if let Err(e) = result {
        span.in_scope(
            || tracing::warn!(import_id = %job.id, error = %e, "Slack import step failed"),
        );
        crate::metrics::record_slack_import_step(
            &step_name,
            "err",
            started.elapsed().as_secs_f64(),
        );
        fail(s, &job, &e).await?;
    } else {
        crate::metrics::record_slack_import_step(&step_name, "ok", started.elapsed().as_secs_f64());
    }
    Ok(Step::Ran)
}

async fn claim(s: &AppState) -> Result<Option<Job>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let token = uuid::Uuid::new_v4().to_string();
    // 処理中にプロセスが落ちると fail() を通らないので、試行回数の上限はここでも見る
    let exhausted = query(
        &mut tx,
        "UPDATE slack_import_jobs SET status='failed',finished_at=now(),lease_until=NULL,lease_token=NULL,\
         updated_at=now(),error=COALESCE(error,'The import stopped repeatedly and was given up') \
         WHERE status IN ('analyzing','importing','fetching_files') AND attempts>=$1::integer \
         AND (lease_until IS NULL OR lease_until<now()) \
         RETURNING json_build_object('key',CASE WHEN source_id IS NULL THEN storage_key END)::text",
        &[&MAX_ATTEMPTS.to_string()],
    )
    .await?;
    let rows = query(
        &mut tx,
        &format!(
            "WITH c AS (SELECT id FROM slack_import_jobs \
             WHERE status IN ('analyzing','importing','fetching_files') AND available_at<=now() \
             AND (lease_until IS NULL OR lease_until<now()) AND attempts<{MAX_ATTEMPTS} \
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
    for job in exhausted {
        discard_zip(s, job["key"].as_str().unwrap_or_default()).await;
    }
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
            return Err(AppError::Conflict(LEASE_LOST.into()));
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

pub(super) async fn exec(
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
pub(super) async fn suppress_app_events(tx: &mut toasty::Transaction<'_>) -> Result<(), AppError> {
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
    let lost = matches!(error, AppError::Conflict(message) if message == LEASE_LOST);
    if lost {
        return Ok(());
    }
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let failed = query(
        &mut tx,
        "UPDATE slack_import_jobs SET error=$3,lease_until=NULL,lease_token=NULL,updated_at=now(),\
         status=CASE WHEN $4::boolean OR attempts>=$5::integer THEN 'failed' ELSE status END,\
         finished_at=CASE WHEN $4::boolean OR attempts>=$5::integer THEN now() END,\
         available_at=now()+make_interval(secs=>LEAST(600,10*power(2,attempts))::integer) \
         WHERE id=$1 AND lease_token=$2 \
         RETURNING json_build_object('failed',status='failed',\
         'key',CASE WHEN source_id IS NULL THEN storage_key END)::text",
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
    // 解析に失敗したエクスポートは使えないので ZIP を消す (取り込みのジョブは ZIP を共有しているので消さない)
    for row in failed.iter().filter(|r| r["failed"] == true) {
        discard_zip(s, row["key"].as_str().unwrap_or_default()).await;
    }
    Ok(())
}

/// ストレージに置いた ZIP を消す (失敗しても取り込みの結果には影響しない)
async fn discard_zip(s: &AppState, key: &str) {
    if key.is_empty() {
        return;
    }
    if let Err(e) = delete_storage_object(&s.s3_client, &s.config.s3.bucket, key).await {
        tracing::warn!(key, error = %e, "Failed to delete a Slack export");
    }
}

/// 新しいエクスポートが解析済みになった古いエクスポートを片付け、ZIP を消す。
/// 取り込み中のジョブが使っているものは、終わってから片付ける。
pub async fn retire_superseded(s: &AppState) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    // 先に行をロックして、取り込みの開始 (import::create_run) と入れ違わないようにする
    let candidates = query(
        &mut tx,
        "SELECT json_build_object('id',o.id)::text FROM slack_import_jobs o \
         WHERE o.source_id IS NULL AND o.status='ready' AND EXISTS(SELECT 1 FROM slack_import_jobs n \
         WHERE n.source_id IS NULL AND n.status='ready' AND n.team_id=o.team_id AND (n.created_at,n.id)>(o.created_at,o.id)) \
         FOR UPDATE OF o SKIP LOCKED",
        &[],
    )
    .await?;
    let mut keys = Vec::new();
    for candidate in candidates {
        keys.extend(
            query(
                &mut tx,
                "UPDATE slack_import_jobs SET status='cancelled',finished_at=now(),updated_at=now() \
                 WHERE id=$1 AND status='ready' AND NOT EXISTS(SELECT 1 FROM slack_import_jobs r \
                 WHERE r.source_id=$1 AND r.status='importing') \
                 RETURNING json_build_object('key',storage_key)::text",
                &[candidate["id"].as_str().unwrap_or_default()],
            )
            .await?,
        );
    }
    tx.commit().await?;
    for row in keys {
        discard_zip(s, row["key"].as_str().unwrap_or_default()).await;
    }
    Ok(())
}

/// アップロードされないまま残ったジョブを取り消し、置きかけの ZIP を消す
pub async fn cleanup(s: &AppState) -> Result<(), AppError> {
    retire_superseded(s).await?;
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let stale = query(
        &mut tx,
        "UPDATE slack_import_jobs SET status='cancelled',finished_at=now(),updated_at=now() \
         WHERE status='uploading' AND created_at<now()-interval '1 day' \
         RETURNING json_build_object('key',storage_key)::text",
        &[],
    )
    .await?;
    // アップロード中に取り消したエクスポートは、取り消した後でブラウザからの PUT が終わって
    // ZIP が残ることがある。署名付き URL の期限と、それまでに始まった PUT が終わる分 (1 日) を待ってから、
    // もう一度だけ消す (消したら storage_key を空にして、次からは対象にしない)
    let late = query(
        &mut tx,
        "UPDATE slack_import_jobs j SET storage_key='',updated_at=now() \
         FROM (SELECT id,storage_key FROM slack_import_jobs \
               WHERE source_id IS NULL AND status='cancelled' AND storage_key<>'' \
               AND updated_at<now()-make_interval(secs=>$1::double precision) FOR UPDATE SKIP LOCKED) o \
         WHERE j.id=o.id RETURNING json_build_object('key',o.storage_key)::text",
        &[&(s.config.s3.presigned_put_expiry_secs + LATE_UPLOAD_GRACE_SECS).to_string()],
    )
    .await?;
    tx.commit().await?;
    for row in stale.into_iter().chain(late) {
        discard_zip(s, row["key"].as_str().unwrap_or_default()).await;
    }
    Ok(())
}

/// 取り消したエクスポートの ZIP を、署名付き URL の期限からさらにこれだけ待ってから消し直す
const LATE_UPLOAD_GRACE_SECS: u64 = 24 * 60 * 60;

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| AppError::Internal(e.into()))?
}

/// 取り込んだ投稿の本文を入れるカード (先頭) の見出し
pub const IMPORTED_TEXT_CARD_TITLE: &str = "Slack";

const TEMP_ZIP_PREFIX: &str = "zoo-slack-import-";
const TEMP_ZIP_SUFFIX: &str = ".zip";

/// ストレージから落とした ZIP の一時ファイル。破棄時に消す。
///
/// 使っている間はファイルに排他ロック（flock）をかけておく。ロックはプロセスが
/// 終わると OS が外すので、ロックを取れるファイルは持ち主が止まった残りだと分かる。
struct TempZip {
    path: PathBuf,
    _lock: std::fs::File,
}

impl TempZip {
    /// ロックをかけた空のファイルを作り、書き込み用のハンドルと一緒に返す
    fn create(dir: &Path) -> std::io::Result<(Self, std::fs::File)> {
        let path = dir.join(format!(
            "{TEMP_ZIP_PREFIX}{}{TEMP_ZIP_SUFFIX}",
            uuid::Uuid::new_v4()
        ));
        // Slack の過去ログを含むので、同じホストのほかのユーザーから読めないようにする
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&path)?;
        if let Err(e) = file.try_lock() {
            let _ = std::fs::remove_file(&path);
            return Err(std::io::Error::other(e));
        }
        // 複製したハンドルは同じロックを共有するので、書き込み側を閉じてもロックは残る
        let writer = file.try_clone()?;
        Ok((Self { path, _lock: file }, writer))
    }
}

impl Drop for TempZip {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `dir` に残っている一時 ZIP のうち、どのプロセスも使っていないものを消す。
/// 既定の一時ディレクトリはほかのプロセスと共有することがあるので、ロックが取れた
/// （持ち主のプロセスが終わっている）ファイルだけを消す。消した数を返す。
fn remove_orphan_zips(dir: &Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(TEMP_ZIP_PREFIX) || !name.ends_with(TEMP_ZIP_SUFFIX) {
            continue;
        }
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Ok(file) = std::fs::File::open(entry.path()) else {
            continue;
        };
        if file.try_lock().is_ok() {
            std::fs::remove_file(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

async fn download(s: &AppState, key: &str) -> Result<TempZip, AppError> {
    let dir = s.config.slack.import_tmp_dir.clone();
    let (tmp, writer) = blocking(move || {
        TempZip::create(&dir).map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "Failed to create the Slack export in SLACK_IMPORT_TMP_DIR ({}): {e}",
                dir.display()
            ))
        })
    })
    .await?;
    let object = s
        .s3_client
        .get_object()
        .bucket(&s.config.s3.bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to read the Slack export: {e}")))?;
    let mut reader = object.body.into_async_read();
    let mut file = tokio::fs::File::from_std(writer);
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
        entries.push(serde_json::to_value(&channel).map_err(|e| AppError::Internal(e.into()))?);
    }
    // 以前の取り込み先や名前の衝突は、取り込むときの状態で決まるので読み出し時に足す (import::annotate_channels)
    let analysis = json!({ "team_id": team, "user_count": user_count, "channels": entries });
    exec(
        &mut tx,
        "UPDATE slack_import_jobs SET status='ready',team_id=$3,analysis=$4,\
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
    // 同じワークスペースの古いエクスポートは置き換える
    retire_superseded(s).await
}

// ---------------------------------------------------------------------------
// 取り込み
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub(super) struct ZooUser {
    pub(super) id: String,
    pub(super) user_name: String,
}

pub(super) struct Context {
    pub(super) team: String,
    pub(super) users: HashMap<String, ZooUser>,
    pub(super) profiles: HashMap<String, SlackUser>,
    /// ZIP 内の全選択チャンネルから先に集めた、profileのないユーザーの表示名候補。
    pub(super) fallback_names: HashMap<String, String>,
    pub(super) channel_names: HashMap<String, String>,
    pub(super) files: FilePolicy,
    /// この Slack アプリの ID (`SLACK_APP_ID`)。このアプリが投稿したメッセージは取り込まない
    pub(super) app_id: Option<String>,
}

/// 添付ファイルを取得して添付にできるかを、取り込みの時点で判定する (`fetch_one` と同じ条件)
pub(super) struct FilePolicy {
    /// 添付を取得するトークンがあるか
    pub(super) token: bool,
    pub(super) max_size: u64,
    pub(super) base_url: String,
}

impl FilePolicy {
    pub(super) fn new(s: &AppState) -> Self {
        Self {
            token: s.config.slack.import_bot_token.is_some(),
            max_size: s.config.s3.max_file_size,
            base_url: s.config.slack.files_base_url.clone(),
        }
    }

    /// 添付として取得するファイルか。取り込めないファイル (外部サービスの共有など)、トークンがなくて保留する
    /// ファイル、サイズ・形式・URL で取得をスキップするファイルは偽
    pub(super) fn will_fetch(&self, file: &super::export::SlackFile) -> bool {
        let mimetype = file
            .mimetype
            .as_deref()
            .filter(|m| !m.is_empty())
            .unwrap_or("application/octet-stream");
        file.is_importable()
            && self.token
            && allowed_file_url(
                file.url_private_download.as_deref().unwrap_or_default(),
                &self.base_url,
            )
            && file.size.unwrap_or(0).max(0) as u64 <= self.max_size
            && crate::routes::attachments::validate_upload_mime(mimetype).is_ok()
    }
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
    pub(super) async fn user(
        &mut self,
        tx: &mut toasty::Transaction<'_>,
        slack_id: &str,
        fallback_name: Option<&str>,
    ) -> Result<ZooUser, AppError> {
        if let Some(user) = self.users.get(slack_id) {
            return Ok(user.clone());
        }
        let profile = self.profiles.get(slack_id).cloned();
        // ZIP全体の候補は日付ファイル単位の候補より優先する。これで最初のmentionや
        // channel membership作成前に名前が確定し、後の日付で本文参照先が変わらない。
        let fallback_name = self
            .fallback_names
            .get(slack_id)
            .map(String::as_str)
            .or(fallback_name);
        let user = ensure_user(tx, &self.team, slack_id, profile.as_ref(), fallback_name).await?;
        self.users.insert(slack_id.to_string(), user.clone());
        Ok(user)
    }
}

/// Resolve every Slack member before channel membership is written. New
/// placeholders use deterministic IDs, so concurrent imports must create them
/// in that same ID order even when the archive lists members differently.
pub(super) async fn channel_users(
    ctx: &mut Context,
    tx: &mut toasty::Transaction<'_>,
    members: &[String],
) -> Result<Vec<ZooUser>, AppError> {
    channel_users_with_additional_users(ctx, tx, members, &[]).await
}

/// Resolve members while acquiring any additional actor rows in the same sorted user lock set.
/// Callers that later create a channel must include its creator here, because the creator may
/// not be present in Slack's current membership list.
pub(super) async fn channel_users_with_additional_users(
    ctx: &mut Context,
    tx: &mut toasty::Transaction<'_>,
    members: &[String],
    additional_user_ids: &[&str],
) -> Result<Vec<ZooUser>, AppError> {
    if members.is_empty() && additional_user_ids.is_empty() {
        return Ok(Vec::new());
    }
    // Serialize mapping reads with link/reconcile. Otherwise an import can resolve a
    // placeholder, let link move its memberships, then insert the stale placeholder ID.
    super::link::lock_links(tx).await?;
    resolve_users_in_id_order_with_additional_users(
        ctx,
        tx,
        members,
        &HashMap::new(),
        additional_user_ids,
    )
    .await
}

/// Resolve a batch using the same deterministic placeholder locks for API and
/// ZIP import paths. Callers that write channel membership use channel_users
/// so mapping reads also serialize with link/reconcile.
async fn resolve_users_in_id_order_with_fallbacks(
    ctx: &mut Context,
    tx: &mut toasty::Transaction<'_>,
    members: &[String],
    fallbacks: &HashMap<String, String>,
) -> Result<Vec<ZooUser>, AppError> {
    resolve_users_in_id_order_with_additional_users(ctx, tx, members, fallbacks, &[]).await
}

async fn resolve_users_in_id_order_with_additional_users(
    ctx: &mut Context,
    tx: &mut toasty::Transaction<'_>,
    members: &[String],
    fallbacks: &HashMap<String, String>,
    additional_user_ids: &[&str],
) -> Result<Vec<ZooUser>, AppError> {
    let mut ordered: Vec<(String, &str)> = members
        .iter()
        .map(|slack_id| (super::user_id(&ctx.team, slack_id), slack_id.as_str()))
        .collect();
    ordered.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    ordered.dedup_by(|left, right| left.0 == right.0);
    // A missing users row cannot be row-locked. Serialize placeholder creation
    // by its deterministic ID before the INSERT ... ON CONFLICT path; competing
    // channel imports then observe the first transaction's committed mapping.
    for (id, _) in &ordered {
        let lock_key = format!("slack-import-user:{id}");
        one(
            tx,
            "SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended($1,0)) IS NULL)::text",
            &[&lock_key],
        )
        .await?;
    }
    // Resolve the persisted mapping before changing any user row. `ensure_user` may update a
    // generic placeholder's display name; acquire every mapped Zoo user row first, in the same
    // order as E2EE membership transitions, so that update cannot take a high-ID placeholder
    // before the lower-ID real participant.
    let mut user_ids = Vec::with_capacity(ordered.len());
    for (placeholder_id, slack_id) in &ordered {
        let mapped = query(
            tx,
            "SELECT json_build_object('id',user_id)::text FROM slack_users WHERE team_id=$1 AND slack_user_id=$2",
            &[&ctx.team, slack_id],
        )
        .await?
        .into_iter()
        .next()
        .and_then(|row| row["id"].as_str().map(str::to_owned));
        user_ids.push(mapped.unwrap_or_else(|| placeholder_id.clone()));
    }
    user_ids.extend(additional_user_ids.iter().map(|id| (*id).to_owned()));
    let user_refs: Vec<&str> = user_ids.iter().map(String::as_str).collect();
    crate::routes::e2ee_channels::lock_membership_users(tx, &user_refs).await?;

    for (_, slack_id) in ordered {
        // Context survives across the ZIP's per-channel and per-day transactions. Re-read
        // slack_users under the link lock so a Slack link committed between transactions
        // replaces an earlier cached placeholder before this transaction writes content.
        ctx.users.remove(slack_id);
        ctx.user(tx, slack_id, fallbacks.get(slack_id).map(String::as_str))
            .await?;
    }
    members
        .iter()
        .map(|slack_id| {
            ctx.users.get(slack_id).cloned().ok_or_else(|| {
                AppError::Internal(anyhow::anyhow!("Slack import user was not resolved"))
            })
        })
        .collect()
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
        "SELECT json_build_object('id',u.id,'user_name',u.user_name,'linked',s.linked,'name',s.name,'real_name',s.real_name,'profile_source',s.profile_source)::text FROM slack_users s \
         JOIN users u ON u.id=s.user_id WHERE s.team_id=$1 AND s.slack_user_id=$2",
        &[team, slack_id],
    )
    .await?
    .into_iter()
    .next()
    {
        // A placeholder username can already be referenced by imported message bodies.
        // Keep it stable across later ZIP imports: the mention token has no stored identity,
        // so rewriting it after the token becomes reusable could alter an unrelated user's
        // mention. Preserve the later fallback in display_name and Slack metadata.
        let fallback_name = fallback_name.filter(|name| !name.is_empty());
        let source = found["profile_source"].as_str().unwrap_or("unknown");
        let generic_legacy_row = source == "unknown"
            && found["name"].as_str() == Some(slack_id)
            && found["real_name"].as_str().unwrap_or_default().is_empty();
        let should_refresh = profile.is_some()
            || (fallback_name.is_some() && (source == "fallback" || generic_legacy_row));
        if found["linked"] == false && should_refresh {
            let id = found["id"].as_str().unwrap_or_default();
            let handle = profile
                .map(|p| p.name.as_str())
                .filter(|name| !name.is_empty())
                .or(fallback_name)
                .unwrap_or(slack_id);
            let display = profile
                .map(SlackUser::display_name)
                .or(fallback_name)
                .unwrap_or(handle);
            let display = sanitize_for_display_name(display, handle);
            let real_name = profile.map(|p| p.real_name.as_str()).unwrap_or("");
            let avatar = profile
                .map(|p| p.profile.image_72.as_str())
                .filter(|url| url.starts_with("https://"))
                .unwrap_or("");
            let is_bot = profile.map(|p| p.is_bot).unwrap_or(false) || slack_id.starts_with('B');
            exec(
                tx,
                "UPDATE users SET display_name=$2,is_bot=$3::boolean,avatar_url=NULLIF($4,'') WHERE id=$1",
                &[id, &display, if is_bot { "true" } else { "false" }, avatar],
            )
            .await?;
            exec(
                tx,
                "UPDATE slack_users SET name=$3,real_name=$4,profile_source=CASE WHEN $5::boolean THEN 'profile' ELSE 'fallback' END WHERE team_id=$1 AND slack_user_id=$2 AND NOT linked",
                &[team, slack_id, handle, real_name, if profile.is_some() { "true" } else { "false" }],
            )
            .await?;
            return Ok(ZooUser {
                id: id.to_string(),
                user_name: found["user_name"].as_str().unwrap_or_default().to_string(),
            });
        }
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

    // 実在の Zoo のユーザー名を先に取らないよう、プレースホルダには slack- を付ける
    let base = sanitize_for_user_name(
        &format!("slack-{}", sanitize_for_user_name(handle, &id)),
        &id,
    );
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
        "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name,profile_source) \
         VALUES($1,$2,$3,false,$4,$5,$6) ON CONFLICT (team_id,slack_user_id) DO NOTHING",
        &[
            team,
            slack_id,
            &id,
            handle,
            real_name,
            if profile.is_some() { "profile" } else if fallback_name.is_some() { "fallback" } else { "unknown" },
        ],
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

/// 前の試行の再開位置 (チャンネルごとに、取り込んだ最後の日付ファイル名) と、処理済みの件数。
/// 記録がなければ、最初からなので、件数も 0
async fn load_progress(
    db: &mut toasty::Db,
    id: &str,
) -> Result<(HashMap<String, String>, u64), AppError> {
    let mut tx = db.transaction().await?;
    let row = one(
        &mut tx,
        "SELECT json_build_object('progress',import_progress,'processed',processed_messages)::text \
         FROM slack_import_jobs WHERE id=$1",
        &[id],
    )
    .await?;
    tx.commit().await?;
    let progress: HashMap<String, String> = row["progress"]
        .as_str()
        .and_then(|v| serde_json::from_str(v).ok())
        .unwrap_or_default();
    let processed = if progress.is_empty() {
        0
    } else {
        row["processed"].as_u64().unwrap_or(0)
    };
    Ok((progress, processed))
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
    // ZIP の展開と JSON の解析は重いので、async のワーカースレッドを止めないよう別スレッドで行う
    let path = tmp.path.clone();
    let (mut archive, slack_users, slack_channels) = blocking(move || {
        let mut archive = ExportArchive::open(&path)?;
        let users = archive.users()?;
        let channels = archive.channels()?;
        Ok((archive, users, channels))
    })
    .await?;
    let slack_channels: HashMap<String, SlackChannel> = slack_channels
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
        fallback_names: HashMap::new(),
        channel_names: HashMap::new(),
        files: FilePolicy::new(s),
        app_id: s.config.slack.app_id.clone(),
    };
    let mut db = s.db.clone();
    let admin = {
        let mut tx = db.transaction().await?;
        let admin = crate::auth::rbac::is_workspace_admin(
            &mut tx,
            &job.created_by,
            s.config.dev_auth_enabled,
        )
        .await?;
        tx.commit().await?;
        admin
    };
    // 取り込みを始められるのは管理者だけ。ユーザーが自分で取り込む方式 (ZIP) をやめたので、管理者でない人が
    // 作ったジョブ (廃止前に作られて残っているもの、管理者でなくなった人のもの) は、権限の確認なしでは実行せず失敗にする
    if !admin {
        return Err(AppError::BadRequest(
            "Only workspace admins can import from Slack".into(),
        ));
    }

    // プレースホルダは、選んだチャンネルのメンバー・投稿者・リアクション・メンションに出る人だけを作る
    // (users.json の全員を作ると、大きいワークスペースで users の同期量が増えるため)

    // 取り込み先チャンネルを決める
    let mut targets = Vec::new();
    {
        let mut tx = db.transaction().await?;
        suppress_app_events(&mut tx).await?;
        // 別のジョブが同じ Slack チャンネルを並行して別の取り込み先へ入れないよう、
        // 確認と登録を Slack チャンネル単位で排他する。デッドロックしないよう ID の順に取る。
        let mut locked: Vec<&str> = mappings
            .iter()
            .filter(|m| !matches!(m.action, MappingAction::Skip))
            .map(|m| m.slack_channel_id.as_str())
            .collect();
        locked.sort_unstable();
        locked.dedup();
        for slack_channel in locked {
            one(
                &mut tx,
                "SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended($1,0)) IS NULL)::text",
                &[&format!("slack-channel:{team}:{slack_channel}")],
            )
            .await?;
        }
        // A created channel inserts the importing administrator into channel_user. The
        // membership trigger locks users, so lock the administrator before this transaction
        // updates any existing target channel. Otherwise a note writer can hold users FOR SHARE
        // while waiting for that channel, and this import can hold the channel while waiting for
        // the administrator's users row.
        lock_creator_for_channel_creation(&mut tx, &mappings, &job.created_by).await?;
        // 以前の取り込みで入れたチャンネルへの <#C…> も、取り込み先の今の名前に変換する。
        // 今回選んだチャンネルは、下の対応付けで上書きする。非公開の取り込み先は、参加していない人にも
        // 見える本文へ今の名前を書かないよう、Slack の本文にある名前のままにする。
        for row in query(
            &mut tx,
            "SELECT json_build_object('slack',s.slack_channel_id,'name',c.channel_name)::text \
             FROM slack_channels s JOIN channels c ON c.id=s.channel_id \
             WHERE s.team_id=$1 AND c.deleted_at IS NULL AND NOT c.is_private",
            &[&team],
        )
        .await?
        {
            if let (Some(slack), Some(name)) = (row["slack"].as_str(), row["name"].as_str()) {
                ctx.channel_names
                    .insert(slack.to_string(), name.to_string());
            }
        }
        for mapping in &mappings {
            let Some(channel) = slack_channels.get(&mapping.slack_channel_id) else {
                continue;
            };
            // 開始後に別のジョブが同じチャンネルを取り込んだ場合も、取り込み先はずらさない
            super::import::ensure_same_target(&mut tx, &team, mapping)
                .await
                .map_err(|e| AppError::BadRequest(e.to_string()))?;
            // 以前の取り込みで作ったチャンネルや既存のチャンネルに入れるなら、開始後に外されていないか確かめる
            let reused = match &mapping.action {
                MappingAction::Skip => continue,
                MappingAction::Existing { channel_id } => Some(channel_id.clone()),
                MappingAction::Create { .. } => {
                    let own = super::channel_id(&team, &channel.id);
                    let exists = !query(
                        &mut tx,
                        "SELECT json_build_object('id',id)::text FROM channels WHERE id=$1",
                        &[&own],
                    )
                    .await?
                    .is_empty();
                    exists.then_some(own)
                }
            };
            if let Some(channel_id) = &reused {
                ensure_plaintext_allowed(&mut tx, channel_id).await?;
            } else if matches!(mapping.action, MappingAction::Create { .. })
                && crate::apps::events::guest(&mut tx, &job.created_by).await?
            {
                // 開始した後でゲストになった場合も、通常のチャンネル作成と同じく新しいチャンネルは作らない
                return Err(AppError::BadRequest(format!(
                    "Guests cannot create channels by importing from Slack ({})",
                    channel.id
                )));
            }
            let (target, private) = match &mapping.action {
                MappingAction::Skip => continue,
                MappingAction::Create {
                    name, is_private, ..
                } => {
                    // 非公開チャンネルは常に非公開で作る (start で拒否済みだが、古いジョブにも備える)。
                    // 以前作ったチャンネルを使うときは、その実際の公開・非公開を返す
                    let private = channel.is_private || is_private.unwrap_or(false);
                    create_channel(&mut tx, &team, channel, name, private, &job.created_by).await?
                }
                MappingAction::Existing { channel_id } => {
                    let found = one(
                        &mut tx,
                        "SELECT json_build_object('id',id,'name',channel_name,'is_private',is_private)::text \
                         FROM channels WHERE id=$1 AND NOT is_dm AND deleted_at IS NULL",
                        &[channel_id],
                    )
                    .await
                    .map_err(|_| {
                        AppError::BadRequest(format!("Channel {channel_id} no longer exists"))
                    })?;
                    if channel.is_private && found["is_private"] != true {
                        return Err(AppError::BadRequest(format!(
                            "Private Slack channel {} must be imported into a private channel",
                            channel.id
                        )));
                    }
                    (
                        (
                            channel_id.clone(),
                            found["name"].as_str().unwrap_or_default().to_string(),
                        ),
                        found["is_private"] == true,
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
            if !private {
                ctx.channel_names
                    .insert(channel.id.clone(), target.1.clone());
            }
            let create = matches!(mapping.action, MappingAction::Create { .. });
            // 取り込み先 (新しく作ったチャンネルも、既存のチャンネルも) に、Slack から取り込んだ印と、取り込みの種類を付ける。
            // 種類は最初の取り込みのものを残す (以前の取り込みで作ったチャンネルへの再取り込みは created のまま)。
            // 最後に取り込んだ日時は、このチャンネルの取り込みが終わったときに更新する (下)
            exec(
                &mut tx,
                "UPDATE channels SET imported_from_slack=true,slack_import_kind=COALESCE(slack_import_kind,$2) WHERE id=$1",
                &[&target.0, if create { "created" } else { "existing" }],
            )
            .await?;
            // 取り込みが終わったあとにアーカイブするか (Slack でアーカイブ済みのチャンネルか、管理者の指定)
            let archive = create
                && (channel.is_archived
                    || matches!(mapping.action, MappingAction::Create { archive: true, .. }));
            targets.push((channel.clone(), target.0, archive));
        }
        tx.commit().await?;
    }

    // 日付ごとにtransactionを分ける前に、選択したZIP範囲のusername fallbackを解決する。
    // アーカイブ全体は保持せず、1日分ずつ解析する。
    let fallback_channels: Vec<SlackChannel> = targets
        .iter()
        .map(|(channel, _, _)| channel.clone())
        .collect();
    let app_id = ctx.app_id.clone();
    let (returned, fallback_names) = blocking(move || {
        let mut archive = archive;
        let fallback_names =
            collect_archive_fallback_names(&mut archive, &fallback_channels, app_id.as_deref())?;
        Ok((archive, fallback_names))
    })
    .await?;
    archive = returned;
    ctx.fallback_names = fallback_names;

    // 前の試行が、日付ファイルまで取り込んで止まったなら、その続きから始める。日付ファイルは、取り込みの
    // トランザクションごとに、この記録と同時にコミットするので、記録にある分は、取り込み済み。
    // 記録がない (古いジョブや最初の試行) ときは、処理済みの件数も数え直す
    let (mut progress, mut processed) = load_progress(&mut db, &job.id).await?;
    for (channel, channel_id, _) in &targets {
        lease.check()?;
        {
            let mut tx = db.transaction().await?;
            suppress_app_events(&mut tx).await?;
            lock_zip_target_advisory(&mut tx, &team, &channel.id).await?;
            let imported_users = channel_users(&mut ctx, &mut tx, &channel.members).await?;
            let membership_users: Vec<&str> = imported_users
                .iter()
                .map(|member| member.id.as_str())
                .collect();
            // The Slack-target advisory lock is already held. Keep users -> channel row order
            // consistent with encryption conversion and Channel Notes.
            // Checking plaintext first takes KEY SHARE on channels; the membership trigger then
            // waits for users NO KEY UPDATE. Notes and conversion can hold users while waiting
            // for the channel, forming a deadlock with this import. Lock every imported member
            // in stable ID order before checking/locking the target channel.
            crate::routes::e2ee_channels::lock_membership_users(&mut tx, &membership_users).await?;
            // メンバーを足す直前にも、実行者がまだ取り込み先にメンバーを足せるか確かめる
            ensure_zip_target_allowed(&mut tx, &team, &channel.id, channel_id, channel.is_private)
                .await?;
            for user in &imported_users {
                // 連携したゲストは、管理者が決めたチャンネルにだけ参加する (取り込みでは足さない)。
                // 未連携のプレースホルダ (guest_kind='deactivated' で is_guest も立つ) は、連携時に
                // 本人へ移せるよう参加情報を残す。
                exec(
                    &mut tx,
                    "INSERT INTO channel_user(user_id,channel_id,joined_at) SELECT $1,$2,CURRENT_TIMESTAMP \
                     WHERE NOT EXISTS(SELECT 1 FROM users WHERE id=$1 AND guest_kind IN ('single','multi')) \
                     ON CONFLICT DO NOTHING",
                    &[&user.id, channel_id],
                )
                .await?;
            }
            tx.commit().await?;
        }

        let done = progress.get(&channel.id).cloned();
        for entry in archive.day_files(channel) {
            // 日付ファイルは、名前の昇順に処理する。取り込み済みの分は飛ばす
            if done.as_deref().is_some_and(|done| entry.as_str() <= done) {
                continue;
            }
            lease.check()?;
            let (returned, messages) = blocking({
                let entry = entry.clone();
                move || {
                    let mut archive = archive;
                    let messages = archive.messages(&entry)?;
                    Ok((archive, messages))
                }
            })
            .await?;
            archive = returned;
            let mut tx = db.transaction().await?;
            suppress_app_events(&mut tx).await?;
            let importable_messages: Vec<&SlackMessage> = messages
                .iter()
                .filter(|message| message.is_importable())
                .collect();
            lock_zip_target_advisory(&mut tx, &team, &channel.id).await?;
            resolve_message_users(&mut tx, &mut ctx, &channel.id, &importable_messages).await?;
            // Keep the stable Slack-channel lock before user rows and the target channel.
            // Revalidate the mapping and privacy/deletion/encryption state in every save
            // transaction so a mid-import visibility change cannot leak private history.
            ensure_zip_target_allowed(&mut tx, &team, &channel.id, channel_id, channel.is_private)
                .await?;
            // 編集・削除の履歴を含むエクスポートの記録 (`message_changed` / `message_deleted`) は、投稿として取り込まない。
            // 以前の取り込みで、これらを投稿として入れてしまった分 (編集した日時の重複した投稿、編集してから削除した
            // 投稿の本文) と、削除された元の投稿 (記録はエクスポートに残らない) は、ここで論理削除する。
            // 保存済みの ZIP から再取り込みするだけで片づく (存在しなければ何もしない)
            //
            // - 削除された元の投稿の普通の記録 (`ts` が `original.ts` / `original_ts` と同じ) が同じファイルにあれば、
            //   先に INSERT してから、最後に論理削除する。初回の取り込みと再取り込みで結果が同じになり、その投稿への
            //   返信は、削除された親のスレッドに残る (取り込まないと、返信の親が見つからず、チャンネル直下に出てしまう)。
            // - イベントの `ts` と同じ `ts` の本物の投稿が同じファイルにあれば、その投稿は消さない (ID が同じになるため)。
            let deleted_originals: std::collections::HashSet<&str> = messages
                .iter()
                .filter_map(|m| m.deleted_original_ts())
                .collect();
            let real_ts: std::collections::HashSet<&str> = messages
                .iter()
                .filter(|m| m.is_importable())
                .map(|m| m.ts.as_str())
                .collect();
            let mut last_ts = None;
            for message in &importable_messages {
                insert_message(&mut tx, &mut ctx, &channel.id, channel_id, &job.id, message)
                    .await?;
                last_ts = Some(message.ts.as_str());
                processed += 1;
            }
            // 削除する行は、決定的な ID の行のうち、作成日時が Slack の投稿時刻と同じ行だけ (`id_taken_by_other` と同じ判定)
            let mut stale: Vec<serde_json::Value> = Vec::new();
            for event in messages.iter().filter(|m| m.is_history_event()) {
                // `ts` が元の投稿の `ts` と同じ形式の記録は、本物の投稿の ID と同じになるので消さない
                if crate::slack::is_valid_ts(&event.ts)
                    && !real_ts.contains(event.ts.as_str())
                    && event.original_message_ts() != Some(event.ts.as_str())
                {
                    stale.push(serde_json::json!({"id": super::message_id(&team, &channel.id, &event.ts), "ts": event.ts}));
                }
            }
            for original in deleted_originals
                .iter()
                .filter(|ts| crate::slack::is_valid_ts(ts))
            {
                stale.push(serde_json::json!({"id": super::message_id(&team, &channel.id, original), "ts": original}));
            }
            if !stale.is_empty() {
                exec(
                    &mut tx,
                    "UPDATE messages m SET deleted_at=CURRENT_TIMESTAMP \
                     FROM jsonb_to_recordset($1::jsonb) AS s(id text,ts text) \
                     WHERE m.id=s.id AND m.channel_id=$2 AND m.deleted_at IS NULL \
                     AND m.created_at=(to_timestamp(s.ts::numeric) AT TIME ZONE 'UTC')::timestamp(3)",
                    &[&serde_json::json!(stale).to_string(), channel_id],
                )
                .await?;
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
            // 取り込んだ日付ファイルの記録は、投稿と同じトランザクションでコミットする (再開位置)
            progress.insert(channel.id.clone(), entry);
            exec(
                &mut tx,
                "UPDATE slack_import_jobs SET processed_messages=$3::bigint,import_progress=$4,updated_at=now() \
                 WHERE id=$1 AND lease_token=$2",
                &[
                    &job.id,
                    &job.token,
                    &processed.to_string(),
                    &serde_json::to_string(&progress).map_err(|e| AppError::Internal(e.into()))?,
                ],
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
        // このチャンネルの取り込みが終わった日時 (UTC。`timestamp` の列はほかの列と同じく UTC で持つ)
        exec(
            &mut tx,
            "UPDATE channels SET slack_imported_at=(CURRENT_TIMESTAMP AT TIME ZONE 'UTC') WHERE id=$1",
            &[channel_id],
        )
        .await?;
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
    // Slack でアーカイブ済みのチャンネルと、管理者がアーカイブを指定したチャンネルは、作った取り込み先も最後に
    // まとめてアーカイブする。途中でアーカイブすると、再開したときに取り込み先の確認 (`ensure_plaintext_allowed`) で止まるため。
    for (_channel, channel_id, archive) in &targets {
        if *archive {
            exec(
                &mut tx,
                "UPDATE channels SET archive_at=CURRENT_TIMESTAMP WHERE id=$1 AND archive_at IS NULL",
                &[channel_id],
            )
            .await?;
        }
    }
    exec(
        &mut tx,
        "UPDATE slack_import_jobs SET status=$3,processed_messages=$4::bigint,total_files=$5::bigint,\
         processed_files=(SELECT count(*) FROM slack_files WHERE job_id=$1 AND status<>'pending'),\
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
    // ZIP はエクスポートとして残し、管理者が消すか新しい ZIP に置き換わるまで使う
    Ok(())
}

/// 開始のあとに暗号化チャンネルになった場合 (#367) も、ジョブの実行時に書き込みを止める (#358)。
/// チャンネルの行を `FOR KEY SHARE` で読むので、暗号化への切り替え (行を `FOR UPDATE` でロックする) と直列になり、
/// 同じトランザクションのあとの書き込みが、切り替えのあとに平文を入れることはない
pub(super) async fn ensure_plaintext_allowed(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
) -> Result<(), AppError> {
    if crate::routes::e2ee_channels::lock_encryption(tx, channel_id).await? {
        return Err(crate::routes::e2ee_channels::plaintext_forbidden());
    }
    Ok(())
}

/// Serialize each ZIP data transaction with remapping of this Slack channel.
/// The lock order is Slack-channel advisory lock -> users -> target channel.
pub(super) async fn lock_zip_target_advisory(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_channel_id: &str,
) -> Result<(), AppError> {
    one(
        tx,
        "SELECT json_build_object('locked',pg_advisory_xact_lock(hashtextextended($1,0)) IS NULL)::text",
        &[&format!("slack-channel:{team}:{slack_channel_id}")],
    )
    .await?;
    Ok(())
}

/// Lock and revalidate the ZIP's selected destination at the point each data
/// transaction is saved. Call after membership users are locked.
async fn ensure_zip_target_allowed(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_channel_id: &str,
    channel_id: &str,
    slack_private: bool,
) -> Result<(), AppError> {
    let mapping = query(
        tx,
        "SELECT json_build_object('channel_id',channel_id)::text FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2 FOR SHARE",
        &[team, slack_channel_id],
    )
    .await?
    .into_iter()
    .next();
    if mapping.as_ref().and_then(|row| row["channel_id"].as_str()) != Some(channel_id) {
        return Err(AppError::Conflict(format!(
            "Slack channel {slack_channel_id} no longer maps to the selected Zoo channel"
        )));
    }

    let target = query(
        tx,
        "SELECT json_build_object('private',is_private,'deleted',deleted_at IS NOT NULL,'encrypted',encrypted)::text FROM channels WHERE id=$1 FOR SHARE",
        &[channel_id],
    )
    .await?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::BadRequest(format!("Channel {channel_id} no longer exists")))?;
    if target["deleted"] == true {
        return Err(AppError::BadRequest(format!(
            "Channel {channel_id} no longer exists"
        )));
    }
    if slack_private && target["private"] != true {
        return Err(AppError::BadRequest(format!(
            "Private Slack channel {slack_channel_id} must be imported into a private channel"
        )));
    }
    if target["encrypted"] == true {
        return Err(crate::routes::e2ee_channels::plaintext_forbidden());
    }
    Ok(())
}

pub(super) async fn create_channel(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    channel: &SlackChannel,
    name: &str,
    is_private: bool,
    admin: &str,
) -> Result<((String, String), bool), AppError> {
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
    let inserted = query(
        tx,
        "INSERT INTO channels(id,channel_name,description,created_user_id,is_private,created_at) \
         VALUES($1,$2,NULLIF($3,''),$4,$5::boolean,\
         CASE WHEN $6::bigint>0 THEN (to_timestamp($6::bigint) AT TIME ZONE 'UTC')::timestamp(3) ELSE CURRENT_TIMESTAMP END) \
         ON CONFLICT DO NOTHING RETURNING json_build_object('id',id)::text",
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
    // 決定的な ID のチャンネルがすでにあるのに、この Slack チャンネルの取り込み先として記録されていなければ、
    // 取り込みで作ったものではない (`POST /api/channels` は任意の ID を受け付ける)。他人のチャンネルを再利用して
    // 履歴を流し込まないよう、名前を出さずに断る。存在の確認と INSERT を別の文にすると、その間に作られた
    // チャンネルを見逃すので、INSERT 自身の結果 (実際に作れたか) で判定する
    if inserted.is_empty()
        && query(
            tx,
            "SELECT json_build_object('ok',true)::text FROM slack_channels \
             WHERE team_id=$1 AND slack_channel_id=$2 AND channel_id=$3",
            &[team, &channel.id, &id],
        )
        .await?
        .is_empty()
    {
        // 上の名前の確認のあと、INSERT までの間に同じ名前のチャンネルが作られた (名前の UNIQUE との競合)。
        // DB のエラーでジョブを失敗させず、名前が使われていることを伝える。ジョブの中では、名前の確認と同じく
        // `BadRequest` にする (API の事前確認 `import.rs` は 409 を返すが、ジョブでは `BadRequest` だけが
        // 再試行しない恒久的な失敗になる。`Conflict` はリースの喪失にも使うので避ける)
        if !query(
            tx,
            "SELECT json_build_object('id',id)::text FROM channels WHERE channel_name=$1 AND id<>$2",
            &[name, &id],
        )
        .await?
        .is_empty()
        {
            return Err(AppError::BadRequest(format!(
                "Channel name '{name}' is already taken"
            )));
        }
        return Err(AppError::BadRequest(format!(
            "Slack channel {} cannot be imported into a new channel because a channel with the same ID already exists",
            channel.id
        )));
    }
    // 作成した管理者もメンバーに入れる (zoo でチャンネルを作ったときと同じ)
    exec(
        tx,
        "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,CURRENT_TIMESTAMP) ON CONFLICT DO NOTHING",
        &[admin, &id],
    )
    .await?;
    let row = one(
        tx,
        "SELECT json_build_object('name',channel_name,'is_private',is_private)::text FROM channels WHERE id=$1",
        &[&id],
    )
    .await?;
    // 以前公開で取り込んだチャンネルが残っていたら、非公開の履歴は入れない
    if is_private && row["is_private"] != true {
        return Err(AppError::BadRequest(format!(
            "Slack channel {} was imported into a public channel; it cannot be imported as private",
            channel.id
        )));
    }
    Ok((
        (id, row["name"].as_str().unwrap_or(name).to_string()),
        row["is_private"] == true,
    ))
}

/// 投稿のリアクションを入れる (すでにあるものは変えない)。投稿が zoo になければ何もしない
pub(super) async fn insert_reactions(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    id: &str,
    message: &SlackMessage,
) -> Result<(), AppError> {
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
                    &super::reaction_id(id, &reactor.id, &emoji),
                    id,
                    &reactor.id,
                    &emoji,
                    &message.ts,
                ],
            )
            .await?;
        }
    }
    Ok(())
}

/// Resolve all users touched by a ZIP message batch in one stable order before
/// any message, mention, or reaction is inserted. The ZIP importer processes a
/// day file in one transaction; resolving IDs one at a time lets two imports
/// create the same placeholders in opposite orders and deadlock.
pub(super) async fn resolve_message_users(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    slack_channel: &str,
    messages: &[&SlackMessage],
) -> Result<(), AppError> {
    // Serialize the mapping snapshot and profile writes with link/reconcile. This
    // must precede user-row locks: linking takes slack_users before users.
    super::link::lock_links(tx).await?;
    let mut members = Vec::new();
    let mut fallbacks: HashMap<String, (u8, String)> = HashMap::new();
    for message in messages {
        if ctx.app_id.is_some() && message.app_id == ctx.app_id {
            continue;
        }
        if mapped(tx, &ctx.team, slack_channel, &message.ts)
            .await?
            .is_some_and(|(_, origin)| origin == "zoo")
        {
            continue;
        }
        let author = message.author().unwrap_or_default();
        members.push(author.to_string());
        // An archive profile takes precedence in ensure_user. When it is absent, preserve
        // the same fallback order used by insert_message: username, then bot_profile.name.
        if let Some((priority, name)) = message_fallback_name(message) {
            let replace = fallbacks
                .get(author)
                .is_none_or(|(existing_priority, _)| priority < *existing_priority);
            if replace {
                fallbacks.insert(author.to_string(), (priority, name.to_string()));
            }
        }
        members.extend(super::convert::mentioned_users(&message.text));
        for reaction in &message.reactions {
            if super::convert::reaction_emoji(&reaction.name).is_some() {
                members.extend(reaction.users.iter().cloned());
            }
        }
    }
    let fallbacks = fallbacks
        .into_iter()
        .map(|(user, (_, fallback))| (user, fallback))
        .collect();
    resolve_users_in_id_order_with_fallbacks(ctx, tx, &members, &fallbacks).await?;
    Ok(())
}

/// Pre-resolve a specific set of Slack users (with optional author fallbacks)
/// under the link and user-row locks before any `channels` row lock is taken.
pub(super) async fn resolve_users(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    slack_ids: &[String],
    fallbacks: &HashMap<String, String>,
) -> Result<(), AppError> {
    if slack_ids.is_empty() {
        return Ok(());
    }
    // Serialize mapping reads and profile updates with link/reconcile before
    // taking any user-row locks (linking takes slack_users before users).
    super::link::lock_links(tx).await?;
    resolve_users_in_id_order_with_fallbacks(ctx, tx, slack_ids, fallbacks).await?;
    Ok(())
}

/// Collect final fallback names before any ZIP user placeholder or message is written.
fn collect_archive_fallback_names<R: std::io::Read + std::io::Seek>(
    archive: &mut ExportArchive<R>,
    channels: &[SlackChannel],
    app_id: Option<&str>,
) -> Result<HashMap<String, String>, AppError> {
    let mut fallbacks: HashMap<String, (u8, String)> = HashMap::new();
    for channel in channels {
        for entry in archive.day_files(channel) {
            for message in archive.messages(&entry)? {
                if !message.is_importable()
                    || message
                        .app_id
                        .as_deref()
                        .is_some_and(|message_app| Some(message_app) == app_id)
                {
                    continue;
                }
                let Some(author) = message.author() else {
                    continue;
                };
                let Some((priority, name)) = message_fallback_name(&message) else {
                    continue;
                };
                let replace = fallbacks
                    .get(author)
                    .is_none_or(|(existing_priority, _)| priority < *existing_priority);
                if replace {
                    fallbacks.insert(author.to_string(), (priority, name.to_string()));
                }
            }
        }
    }
    Ok(fallbacks
        .into_iter()
        .map(|(user, (_, name))| (user, name))
        .collect())
}

pub(super) fn message_fallback_name(message: &SlackMessage) -> Option<(u8, &str)> {
    message
        .username
        .as_deref()
        .filter(|name| !name.is_empty())
        .map(|name| (0, name))
        .or_else(|| {
            message
                .bot_profile
                .as_ref()
                .map(|profile| profile.name.as_str())
                .filter(|name| !name.is_empty())
                .map(|name| (1, name))
        })
}

/// 投稿の表示用の内容。取り込み (`insert_message`) と、編集の反映 (`sync` のヒント) で共有する
pub(super) struct Rendered {
    pub(super) content: String,
    /// エンコード済みのカード (なければ空)
    pub(super) card: String,
    /// 本文が、取得しない添付のファイル名で補ったものか
    pub(super) filled: bool,
}

/// 投稿の本文とカードを作る。メンションされた人のプレースホルダも作る
pub(super) async fn render_message(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    message: &SlackMessage,
) -> Result<Rendered, AppError> {
    // メンションされた人のプレースホルダを先に作り、本文の <@U…> を解決できるようにする
    for mentioned in super::convert::mentioned_users(&message.text) {
        ctx.user(tx, &mentioned, None).await?;
    }
    let mut cards = message.cards();
    let mut content = mrkdwn_to_mfm(&message.text, &*ctx);
    // カードのある投稿は、チャットでは本文 (content) を表示せずカードだけを表示する。
    // トップレベルの本文もカードの先頭に置いて、本文とカードの両方を読めるようにする。
    // カードの説明は MFM として描画しないので、リンクは `label (URL)` の形にする。
    if !cards.is_empty() && !content.trim().is_empty() {
        let mut card = crate::cards::text_card(
            super::convert::mrkdwn_to_text(&message.text, &*ctx)
                .chars()
                .take(MAX_CARD_DESCRIPTION)
                .collect(),
        );
        card.title = IMPORTED_TEXT_CARD_TITLE.into();
        cards.insert(0, card);
    }
    let card = crate::cards::encode_cards(&cards)?.unwrap_or_default();
    // 本文がカードにしかない bot 投稿は、カードの内容を本文にも入れる (通知・一覧・旧クライアント用)。
    // カードにできなかったときも、fallback などの文字列を残して投稿が欠けないようにする。
    if content.trim().is_empty() {
        content = crate::cards::cards_content(&cards);
    }
    if content.trim().is_empty() {
        content = super::convert::truncate_content(message.attachments_text());
    }
    // 本文のないファイルだけの投稿は、添付として取得しないファイルの名前を本文に残す。取り込めないファイル
    // (外部サービスの共有・削除済みなど) や、取得をスキップ (サイズ・形式) または保留 (トークンなし) する
    // ファイルだけの投稿が空にならないようにする。取得する添付は添付として表示するので、名前は入れない
    // 補った名前の行は `slack_files.in_content` に覚える (投稿者が書いた本文と区別して、後から更新するため)
    let mut filled = false;
    if content.trim().is_empty() {
        content = super::convert::truncate_content(
            message.files_text(|file| !ctx.files.will_fetch(file)),
        );
        filled = !content.trim().is_empty();
    }
    Ok(Rendered {
        content,
        card,
        filled,
    })
}

/// Slack の `ts` に結び付いた Zoo のメッセージ (`slack_message_map`)。(メッセージ ID, 出どころ `zoo` / `slack`)
pub(super) async fn mapped(
    tx: &mut toasty::Transaction<'_>,
    team: &str,
    slack_channel: &str,
    ts: &str,
) -> Result<Option<(String, String)>, AppError> {
    let rows = query(
        tx,
        "SELECT json_build_object('id',message_id,'origin',origin)::text FROM slack_message_map \
         WHERE team_id=$1 AND slack_channel_id=$2 AND ts=$3",
        &[team, slack_channel, ts],
    )
    .await?;
    Ok(rows.first().and_then(|r| {
        Some((
            r["id"].as_str()?.to_string(),
            r["origin"].as_str()?.to_string(),
        ))
    }))
}

/// 決定的な ID の行が、取り込みで作った行ではないか。取り込みで作った行は、取り込み先のチャンネルにあり、
/// 作成日時が Slack の投稿時刻と同じ (API で作る投稿の作成日時はサーバーが決めるので、これと一致させられない)。
pub(super) async fn id_taken_by_other(
    tx: &mut toasty::Transaction<'_>,
    id: &str,
    channel_id: &str,
    ts: &str,
) -> Result<bool, AppError> {
    Ok(!query(
        tx,
        "SELECT json_build_object('id',id)::text FROM messages WHERE id=$1 \
         AND (channel_id<>$2 OR created_at<>(to_timestamp($3::numeric) AT TIME ZONE 'UTC')::timestamp(3))",
        &[id, channel_id, ts],
    )
    .await?
    .is_empty())
}

pub(super) async fn insert_message(
    tx: &mut toasty::Transaction<'_>,
    ctx: &mut Context,
    slack_channel: &str,
    channel_id: &str,
    job_id: &str,
    message: &SlackMessage,
) -> Result<(), AppError> {
    // ループ防止: このアプリが投稿したメッセージ (#294 のブリッジが Zoo から Slack へ出したもの) は、取り込まない
    if ctx.app_id.is_some() && message.app_id == ctx.app_id {
        return Ok(());
    }
    // `slack_message_map` に行があれば、その Zoo のメッセージを使う。Zoo で作った投稿 (`zoo`) は、Slack から取り込まない
    let own = mapped(tx, &ctx.team, slack_channel, &message.ts).await?;
    if own.as_ref().is_some_and(|(_, origin)| origin == "zoo") {
        return Ok(());
    }
    let author = message.author().unwrap_or_default();
    let fallback = message
        .username
        .as_deref()
        .or(message.bot_profile.as_ref().map(|b| b.name.as_str()))
        .filter(|n| !n.is_empty());
    let user = ctx.user(tx, author, fallback).await?;
    let derived = own.is_none();
    let id = own
        .map(|(id, _)| id)
        .unwrap_or_else(|| super::message_id(&ctx.team, slack_channel, &message.ts));
    // 決定的な ID の行がすでにあるのに、別のチャンネルの行か、Slack の投稿時刻と違う時刻の行なら、取り込みで作った行
    // ではない (同じ ID を先に使われた)。その行には、投稿もリアクションも添付も結び付けず、この投稿を飛ばす
    if derived && id_taken_by_other(tx, &id, channel_id, &message.ts).await? {
        tracing::warn!(
            message_id = %id,
            slack_channel,
            ts = %message.ts,
            "A Slack message was not imported because its ID is already used by another message"
        );
        return Ok(());
    }
    // スレッドの親が Zoo で作った投稿 (`slack_message_map`) なら、その Zoo のメッセージが親になる。
    // 決定的な ID の親は、Slack の投稿時刻の行だけを親にする (`parent_ts`)
    let mut parent_ts = "";
    let parent = match message.parent_ts() {
        Some(ts) => match mapped(tx, &ctx.team, slack_channel, ts).await? {
            Some((id, _)) => id,
            None => {
                parent_ts = ts;
                super::message_id(&ctx.team, slack_channel, ts)
            }
        },
        None => String::new(),
    };
    let edited = message
        .edited
        .as_ref()
        .map(|e| e.ts.as_str())
        .filter(|ts| super::is_valid_ts(ts))
        .unwrap_or("");
    let Rendered {
        content,
        card,
        filled,
    } = render_message(tx, ctx, message).await?;
    // Slack の履歴には、削除済みの親への返信も含まれる。スレッド化のトリガーは通常、削除済みの親への返信を
    // 拒否するので、取り込みの間だけ許可する (再取り込みや、後から見つかった返信も削除された親のスレッドに残る)。
    one(
        tx,
        "SELECT json_build_object('ok',set_config('zoo.slack_import','on',true))::text",
        &[],
    )
    .await?;
    // 期間を区切った ZIP では返信だけが先に入ることがある。後から親を取り込んだら、返信をスレッドに戻す。
    exec(
        tx,
        "INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,parent_id,card) \
         VALUES($1,$2,$3,$4,(to_timestamp($5::numeric) AT TIME ZONE 'UTC')::timestamp(3),\
         CASE WHEN $6='' THEN NULL ELSE (to_timestamp($6::numeric) AT TIME ZONE 'UTC')::timestamp(3) END,\
         (SELECT m.id FROM messages m WHERE m.id=NULLIF($7,'') AND m.channel_id=$3 AND m.parent_id IS NULL \
          AND ($9='' OR m.created_at=(to_timestamp($9::numeric) AT TIME ZONE 'UTC')::timestamp(3))),\
         NULLIF($8,'')) \
         ON CONFLICT (id) DO UPDATE SET parent_id=EXCLUDED.parent_id \
         WHERE messages.parent_id IS NULL AND EXCLUDED.parent_id IS NOT NULL \
         AND messages.channel_id=EXCLUDED.channel_id \
         AND ($10='' OR messages.created_at=EXCLUDED.created_at) \
         AND NOT EXISTS(SELECT 1 FROM messages r WHERE r.parent_id=messages.id)",
        &[
            &id,
            &user.id,
            channel_id,
            &content,
            &message.ts,
            edited,
            &parent,
            &card,
            parent_ts,
            // 決定的な ID の行は、作成日時が同じ行だけを更新する (Zoo の投稿との対応 `slack_message_map` がある行は除く)
            if derived { "1" } else { "" },
        ],
    )
    .await?;
    // 確認と INSERT の間に、別の行が同じ ID で確定していた場合も、その行には何も結び付けない
    if derived && id_taken_by_other(tx, &id, channel_id, &message.ts).await? {
        tracing::warn!(
            message_id = %id,
            slack_channel,
            ts = %message.ts,
            "A Slack message was not imported because its ID is already used by another message"
        );
        return Ok(());
    }
    // 差分同期・イベントで取り込んだ投稿は、`slack_message_map` に `slack` で残す (直近 24 時間の再確認が、
    // Slack から消えた投稿を見つけるのに使う。ZIP の取り込みは、件数が多いので残さない。古い行は掃除が消す)
    // スレッドの返信 (`thread_ts` が自分の `ts` と違う) は残さない。履歴 (`conversations.history`) は返信を返さないので、
    // 再確認が「消えた」と誤って判定する。親を Zoo で結び付けられたか (`parent_id`) とは別に、Slack での種類で決める
    if job_id.is_empty() && message.parent_ts().is_none() {
        exec(
            tx,
            "INSERT INTO slack_message_map(team_id,slack_channel_id,ts,message_id,origin) \
             SELECT $1,$2,$3,$4,'slack' WHERE EXISTS(SELECT 1 FROM messages WHERE id=$4) ON CONFLICT DO NOTHING",
            &[&ctx.team, slack_channel, &message.ts, &id],
        )
        .await?;
    }

    // 親だけを後から取り込んだときも、先に入っていた返信をスレッドに戻す
    if parent.is_empty() && !message.replies.is_empty() {
        let replies: Vec<serde_json::Value> = message
            .replies
            .iter()
            .filter(|r| r.ts != message.ts && super::is_valid_ts(&r.ts))
            .map(|r| serde_json::json!({"id": super::message_id(&ctx.team, slack_channel, &r.ts), "ts": r.ts}))
            .collect();
        exec(
            tx,
            "UPDATE messages m SET parent_id=$1 FROM jsonb_to_recordset($3::jsonb) AS r(id text,ts text) \
             WHERE m.id=r.id AND m.channel_id=$2 AND m.parent_id IS NULL AND m.id<>$1 \
             AND m.created_at=(to_timestamp(r.ts::numeric) AT TIME ZONE 'UTC')::timestamp(3) \
             AND EXISTS(SELECT 1 FROM messages p WHERE p.id=$1 AND p.parent_id IS NULL) \
             AND NOT EXISTS(SELECT 1 FROM messages c WHERE c.parent_id=m.id)",
            &[&id, channel_id, &serde_json::to_string(&replies).unwrap_or_default()],
        )
        .await?;
    }

    insert_reactions(tx, ctx, &id, message).await?;

    for file in message.files.iter().filter(|f| f.is_importable()) {
        let url = file.url_private_download.as_deref().unwrap_or_default();
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
             name,mimetype,size,url_private_download,in_content) \
             SELECT $1,$2,NULLIF($3,''),$4,$5,$6,$7,$8,$9::bigint,$10,$11::boolean WHERE EXISTS(SELECT 1 FROM messages WHERE id=$4) \
             ON CONFLICT (team_id,slack_file_id,message_id) DO UPDATE SET job_id=COALESCE(EXCLUDED.job_id,slack_files.job_id),\
             status='pending',attempts=0,error=NULL,lease_until=NULL,lease_token=NULL,updated_at=now() \
             WHERE slack_files.status IN ('pending','failed') AND $3<>''",
            &[
                &ctx.team,
                &file.id,
                job_id,
                &id,
                channel_id,
                &user.id,
                &name,
                mimetype,
                &file.size.unwrap_or(0).max(0).to_string(),
                url,
                if filled && !ctx.files.will_fetch(file) {
                    "true"
                } else {
                    "false"
                },
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
    /// 行単位の取得で、取ったときの `job_id` (ジョブの取得では使わない)
    #[serde(default)]
    job_id: Option<String>,
    /// 行単位の取得で、リースの持ち主 (ジョブの取得では使わない)
    #[serde(default)]
    lease_token: String,
}

enum FileOutcome {
    Done(String),
    Skipped(String),
    /// Slack が 429 を返した。試行回数に数えず、この秒数だけ待って取り直す (行単位の取得だけが扱う)
    RetryLater(u64),
    /// どのトークンでも、Slack がログイン画面を返した (権限がない)。取り直しても変わらない
    Denied,
}

/// 添付の取得に使うクライアント。リダイレクトは `get_following_redirects` が 1 回ずつ確かめるので、自動では追従しない
fn file_client() -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AppError::Internal(e.into()))
}

/// 取得の結果 (`done` / `skipped`) に合わせて、投稿の本文のファイル名を更新する。ジョブの取得と、行単位の取得で共通。
/// 呼び出し側が、`slack_files` の状態を更新したあとの同じトランザクションで呼ぶ
async fn record_file_result(
    tx: &mut toasty::Transaction<'_>,
    file: &PendingFile,
    status: &str,
) -> Result<(), AppError> {
    // 暗号化に切り替えたチャンネル (#367) の取り込み済みの投稿は、本文を変えない (DB のトリガーも、切り替える前の
    // 平文の行に新しい本文を入れることを拒否する)。
    // 本文のファイル名は、取り込み時に補った行だけを `slack_files.in_content` で覚えて更新する。
    // 投稿者が書いた本文 (ファイル名と同じ文字列でも) には触れない。取り込み本体と同じく、過去ログの更新を
    // App へ配信しない
    if status == "skipped" {
        // 実際のサイズが上限を超えたなど、取得してからスキップした添付は、本文が空 (または補った名前だけ)
        // の投稿に限り、その名前を本文に足す。添付だけの投稿が空にならないようにする
        suppress_app_events(tx).await?;
        let added = query(
            tx,
            "UPDATE messages m SET content=CASE WHEN m.content='' THEN f.name ELSE m.content||E'\\n'||f.name END,\
             updated_at=m.updated_at FROM slack_files f \
             WHERE f.team_id=$1 AND f.slack_file_id=$3 AND f.message_id=$2 AND NOT f.in_content AND m.id=$2 \
             AND NOT EXISTS(SELECT 1 FROM channels c WHERE c.id=m.channel_id AND c.encrypted) \
             AND (m.content='' OR EXISTS(SELECT 1 FROM slack_files o WHERE o.team_id=$1 AND o.message_id=$2 AND o.in_content)) \
             RETURNING json_build_object('ok',true)::text",
            &[&file.team_id, &file.message_id, &file.slack_file_id],
        )
        .await?;
        if !added.is_empty() {
            exec(
                tx,
                "UPDATE slack_files SET in_content=true WHERE team_id=$1 AND slack_file_id=$3 AND message_id=$2",
                &[&file.team_id, &file.message_id, &file.slack_file_id],
            )
            .await?;
        }
    }
    if status == "done" {
        // 取り込み時に補ったファイル名 (トークンなし・サイズ・URL など) は、後から取得できたら添付と
        // 二重に出ないよう本文から外す。同名の添付が複数あっても、外すのは 1 行だけにする
        // (取得できなかった側の名前を本文に残す)
        suppress_app_events(tx).await?;
        exec(
            tx,
            "UPDATE messages m SET content=COALESCE((SELECT array_to_string(array_agg(t.l ORDER BY t.n),E'\\n') \
             FROM unnest(string_to_array(m.content,E'\\n')) WITH ORDINALITY AS t(l,n) \
             WHERE t.n<>COALESCE((SELECT min(u.n) FROM unnest(string_to_array(m.content,E'\\n')) WITH ORDINALITY AS u(l,n) \
             WHERE u.l=f.name),0)),''),\
             updated_at=m.updated_at FROM slack_files f \
             WHERE f.team_id=$1 AND f.slack_file_id=$3 AND f.message_id=$2 AND f.in_content AND m.id=$2 \
             AND NOT EXISTS(SELECT 1 FROM channels c WHERE c.id=m.channel_id AND c.encrypted)",
            &[&file.team_id, &file.message_id, &file.slack_file_id],
        )
        .await?;
        exec(
            tx,
            "UPDATE slack_files SET in_content=false WHERE team_id=$1 AND slack_file_id=$3 AND message_id=$2",
            &[&file.team_id, &file.message_id, &file.slack_file_id],
        )
        .await?;
    }
    Ok(())
}

async fn fetch_files(s: &AppState, job: &Job, lease: &Lease) -> Result<(), AppError> {
    let mut db = s.db.clone();
    if let Some(token) = s.config.slack.import_bot_token.as_deref() {
        let client = file_client()?;
        loop {
            lease.check()?;
            let mut tx = db.transaction().await?;
            let batch = query(
                &mut tx,
                "SELECT json_build_object('team_id',team_id,'slack_file_id',slack_file_id,\
                 'message_id',message_id,'channel_id',channel_id,'uploader_id',uploader_id,'name',name,\
                 'mimetype',mimetype,'size',size,'url',url_private_download,'attempts',attempts)::text \
                 FROM slack_files WHERE job_id=$1 AND status='pending' \
                 AND (lease_until IS NULL OR lease_until<now()) ORDER BY created_at,slack_file_id,message_id LIMIT 20",
                &[&job.id],
            )
            .await?;
            tx.commit().await?;
            if batch.is_empty() {
                break;
            }
            for value in batch {
                lease.check()?;
                let mut file: PendingFile =
                    serde_json::from_value(value).map_err(|e| AppError::Internal(e.into()))?;
                // 行単位の取得と同じリースを取ってから取得する。行単位の取得が取っている行は、取らない
                // (同じファイルを並行して取得して、結果を取り違えないため。取れなかった行は、次の選び直しから外れる)
                let lease_token = uuid::Uuid::new_v4().to_string();
                let mut tx = db.transaction().await?;
                let claimed = query(
                    &mut tx,
                    &format!(
                        "UPDATE slack_files SET lease_token=$4,lease_until=now()+{FILE_LEASE_SQL} \
                         WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$3 AND job_id=$5 AND status='pending' \
                         AND (lease_until IS NULL OR lease_until<now()) \
                         RETURNING json_build_object('ok',true)::text"
                    ),
                    &[
                        &file.team_id,
                        &file.slack_file_id,
                        &file.message_id,
                        &lease_token,
                        &job.id,
                    ],
                )
                .await?;
                tx.commit().await?;
                if claimed.is_empty() {
                    continue;
                }
                file.lease_token = lease_token;
                let outcome = fetch_one(s, &client, &[token.to_string()], &file).await;
                let mut tx = db.transaction().await?;
                // 投稿の編集 (messages → slack_files の順) と同じ順にロックして、行き違いのデッドロックを避ける
                query(
                    &mut tx,
                    "SELECT json_build_object('ok',true)::text FROM messages WHERE id=$1 FOR UPDATE",
                    &[&file.message_id],
                )
                .await?;
                // 429 を待って取り直すのは行単位の取得だけ。ジョブの取得は、これまでどおり失敗として数える
                let outcome = match outcome {
                    Ok(FileOutcome::RetryLater(_)) => Err(AppError::BadGateway(
                        "Slack file download returned 429 Too Many Requests".into(),
                    )),
                    // 従来と同じ文言にする (再取得の SQL が、この文言で行を選ぶ)
                    Ok(FileOutcome::Denied) => Err(AppError::BadGateway(
                        "Slack file download failed: redirected to a URL that is not allowed"
                            .into(),
                    )),
                    other => other,
                };
                let (status, attachment, error) = match outcome {
                    Ok(FileOutcome::Done(attachment)) => ("done", attachment, String::new()),
                    Ok(FileOutcome::Skipped(reason)) => ("skipped", String::new(), reason),
                    Ok(FileOutcome::RetryLater(_) | FileOutcome::Denied) => {
                        unreachable!("mapped to an error above")
                    }
                    Err(e) if file.attempts + 1 >= MAX_FILE_ATTEMPTS => {
                        ("failed", String::new(), e.to_string())
                    }
                    Err(e) => ("pending", String::new(), e.to_string()),
                };
                // 取得中に再取り込みが行を別のジョブへ移していたら (試行回数も戻っている)、古い結果は書かない。
                // 添付は決定的 ID で作るので、ここで書かなくても新しいジョブの取得と重複しない。
                let updated = query(
                    &mut tx,
                    "UPDATE slack_files SET status=$3,attachment_id=NULLIF($4,''),error=NULLIF($5,''),\
                     lease_token=NULL,lease_until=NULL,\
                     attempts=attempts+CASE WHEN $3 IN ('pending','failed') THEN 1 ELSE 0 END,updated_at=now() \
                     WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$6 AND job_id=$7 AND status='pending' \
                     AND lease_token=$8 \
                     RETURNING json_build_object('ok',true)::text",
                    &[
                        &file.team_id,
                        &file.slack_file_id,
                        status,
                        &attachment,
                        &error,
                        &file.message_id,
                        &job.id,
                        &file.lease_token,
                    ],
                )
                .await?;
                if updated.is_empty() {
                    tx.commit().await?;
                    continue;
                }
                record_file_result(&mut tx, &file, status).await?;
                exec(
                    &mut tx,
                    // 全件を数え直さず、処理済みになった分だけ進める
                    "UPDATE slack_import_jobs SET processed_files=processed_files+CASE WHEN $3='pending' THEN 0 ELSE 1 END,\
                     updated_at=now() WHERE id=$1 AND lease_token=$2",
                    &[&job.id, &job.token, status],
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
        // 行単位の取得が先に取った行 (書き込みを飛ばした行) があっても合うよう、完了時に数え直す
        "UPDATE slack_import_jobs SET status='completed',finished_at=now(),attempts=0,error=NULL,\
         processed_files=(SELECT count(*) FROM slack_files WHERE job_id=$1 AND status<>'pending'),\
         lease_until=NULL,lease_token=NULL,updated_at=now() WHERE id=$1 AND lease_token=$2",
        &[&job.id, &job.token],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// 添付ファイルの行単位の取得 (ジョブに紐づかない添付)
// ---------------------------------------------------------------------------
//
// 差分同期で取り込んだ投稿の添付 (`job_id` なし) や、取り消された・失敗した・トークンなしで完了したジョブの
// 添付は、ジョブの取得 (`fetch_files`) では取得されない。`slack_files` の行ごとにリースを取って、1 件ずつ取得する。
// 取得そのもの (`fetch_one`) と結果の書き込み (`record_file_result`) は、ジョブの取得と同じ。
// 進行中のジョブ (`analyzing`/`importing`/`fetching_files`) や、まだ始まっていないジョブの行は、ジョブの取得に任せる。

/// 行単位の取得を動かすか。Bot トークンがあるか、連携したユーザーのトークンを復号できる (OAuth と暗号化キー) とき
fn file_worker_enabled(s: &AppState) -> bool {
    s.config.slack.import_bot_token.is_some() || user_tokens_enabled(s)
}

fn user_tokens_enabled(s: &AppState) -> bool {
    s.config.slack.oauth_enabled() && s.config.slack.encryption_key().is_some()
}

/// 候補のユーザーがいないとき (まだ再連携が進んでいない)、行を戻して次に試すまで待つ秒数
const NO_USER_TOKEN_WAIT_SECS: u64 = 3 * 3600;
const DENIED_NOTE: &str = "Slack file is not visible to any linked user or the bot";
const NO_USER_TOKEN_NOTE: &str = "no linked user with files:read yet";

/// 1 行の取得に使うトークン (ユーザー → Bot の順)。値は復号したもので、ログやエラーに出さない
struct FileTokens {
    tokens: Vec<String>,
    /// `tokens` のうち、先頭から数えたユーザーのトークンの数 (残りは Bot)
    users: usize,
    /// ユーザーのトークンを使える行か。連携が増えれば取れる見込みがあるのは、この行だけ
    eligible: bool,
    /// 同じワークスペースに、`files:read` を持つ連携が 1 人でもいるか (使える行だけ調べる)。
    /// いなければ、再連携が進むまで待つ。いるのに、この行のトークンがない (メンバーが連携していない) なら、待たない
    workspace_has_links: bool,
    /// 復号できなかった連携があった (キーの不一致など)。ユーザーのトークンを試せていないので、権限なしとは確定しない
    decrypt_failed: bool,
}

/// 行の URL が、この行のファイルのものか。ユーザーのトークンは、このパスにだけ付けて送る。
/// ZIP の URL は管理者が上げたものなので、別のワークスペース・別のファイルの URL にユーザーの権限を使わない
fn is_own_file_url(url: &str, team: &str, file_id: &str) -> bool {
    reqwest::Url::parse(url)
        .map(|u| {
            u.path()
                .starts_with(&format!("/files-pri/{team}-{file_id}/"))
        })
        .unwrap_or(false)
}

/// 行の取得に使うトークンを集める。ユーザーのトークンは、差分同期の行 (`job_id` なし) だけに使う。
/// ZIP の取り込みの行は、ファイル ID と URL を ZIP の提供者が決めるので、ユーザーの権限で取らせない (Bot だけ)。
/// ユーザーは、同じワークスペースの連携のうち、`files:read` を持つ人。
/// 非公開チャンネルは、そのメンバーだけ。前回その Slack チャンネルを読めた人、メンバー、新しい連携の順。
/// 人数では切らない (先頭の人が見えなくても、残りの人で取れるように)。
async fn file_tokens(s: &AppState, file: &PendingFile) -> Result<FileTokens, AppError> {
    let mut tokens = Vec::new();
    let own_url = file.job_id.is_none()
        && file.url.as_deref().is_some_and(|u| {
            allowed_file_url(u, &s.config.slack.files_base_url)
                && is_own_file_url(u, &file.team_id, &file.slack_file_id)
        });
    let key = s
        .config
        .slack
        .oauth_enabled()
        .then(|| s.config.slack.encryption_key())
        .flatten();
    let eligible = own_url && s.config.slack.team_allowed(&file.team_id) && key.is_some();
    // 連携の有無は、候補より先に調べる (あとで連携が増えても、待つ側に倒れるだけで、権限なしと確定しない)
    let mut workspace_has_links = false;
    if eligible {
        let mut db = s.db.clone();
        let mut tx = db.transaction().await?;
        workspace_has_links = !query(
            &mut tx,
            "SELECT json_build_object('ok',true)::text FROM slack_user_links l WHERE l.team_id=$1 \
             AND 'files:read'=ANY(string_to_array(replace(l.scopes,' ',''),',')) LIMIT 1",
            &[&file.team_id],
        )
        .await?
        .is_empty();
        tx.commit().await?;
    }
    let mut decrypt_failed = false;
    if eligible && let Some(key) = key {
        let mut db = s.db.clone();
        let mut tx = db.transaction().await?;
        let rows = query(
            &mut tx,
            "SELECT json_build_object('user_id',l.user_id,'token',l.access_token_enc)::text \
             FROM slack_user_links l JOIN channels ch ON ch.id=$2 \
             WHERE l.team_id=$1 AND 'files:read'=ANY(string_to_array(replace(l.scopes,' ',''),',')) \
             AND (NOT ch.is_private OR EXISTS(SELECT 1 FROM channel_user cu \
                  WHERE cu.channel_id=ch.id AND cu.user_id=l.user_id)) \
             ORDER BY EXISTS(SELECT 1 FROM slack_channels sc \
                  JOIN slack_channel_syncs sy ON sy.team_id=sc.team_id AND sy.slack_channel_id=sc.slack_channel_id \
                  WHERE sc.team_id=$1 AND sc.channel_id=$2 AND sy.token_user_id=l.user_id) DESC,\
             EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=ch.id AND cu.user_id=l.user_id) DESC,\
             l.updated_at DESC,l.user_id",
            &[&file.team_id, &file.channel_id],
        )
        .await?;
        tx.commit().await?;
        for row in rows {
            let (Some(user), Some(enc)) = (row["user_id"].as_str(), row["token"].as_str()) else {
                continue;
            };
            match super::crypto::decrypt(&key, enc) {
                Ok(token) => tokens.push(token),
                Err(_) => {
                    decrypt_failed = true;
                    tracing::warn!(user_id = %user, "Failed to decrypt a Slack token");
                }
            }
        }
    }
    let users = tokens.len();
    if let Some(bot) = s.config.slack.import_bot_token.as_deref() {
        tokens.push(bot.to_string());
    }
    Ok(FileTokens {
        tokens,
        users,
        eligible,
        workspace_has_links,
        decrypt_failed,
    })
}

/// 行単位の取得のワーカー。取得に使えるトークンがなければ何もせず戻る (呼び出し側で、使えるときだけ起動する)。
/// `stop` が `true` になったら、取りかけの行を手放して戻る
pub async fn orphan_files_worker(s: AppState, mut stop: Stop) {
    if !file_worker_enabled(&s) {
        return;
    }
    let client = match file_client() {
        Ok(client) => client,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build the Slack file client");
            return;
        }
    };
    loop {
        let wait = match orphan_files_step(&s, &client, &mut stop, "").await {
            // 次の行をすぐ取る。Slack に続けて要求しすぎないよう、少しだけ間を置く
            Ok(FileStep::Done) => Duration::from_millis(200),
            Ok(FileStep::Idle) => FILE_IDLE,
            Ok(FileStep::Wait(secs)) => Duration::from_secs(secs),
            Ok(FileStep::Stopped) => return,
            Err(e) => {
                tracing::warn!(error = %e, "Slack file worker failed");
                Duration::from_secs(10)
            }
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stopped(&mut stop) => return,
        }
    }
}

/// 行単位の取得を 1 件だけ進める (テスト用)。`team` が空でなければ、そのワークスペースの行だけを取る。
/// 取る行がなかったら `false`
pub async fn orphan_files_once(s: &AppState, team: &str) -> Result<bool, AppError> {
    if !file_worker_enabled(s) {
        return Ok(false);
    }
    let client = file_client()?;
    // 合図を送らない (止まらない) 取得。送り手をここで持ち続ける
    let (_keep, mut stop) = tokio::sync::watch::channel(false);
    Ok(!matches!(
        orphan_files_step(s, &client, &mut stop, team).await?,
        FileStep::Idle
    ))
}

pub enum FileStep {
    /// 1 件を処理した
    Done,
    /// 取る行がなかった
    Idle,
    /// Slack が 429 を返した。この秒数だけ待つ
    Wait(u64),
    /// 終了の合図を受けて、取りかけの行を手放した
    Stopped,
}

/// 取得を繰り返し失敗した (リースが切れても結果を書けなかった) 行を `failed` にする。
/// 取り直すたびに試行回数が増えるので、落ちる行を永久に取り直さない
async fn fail_exhausted_files(s: &AppState, team: &str) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    let condition = "status='pending' AND lease_token IS NOT NULL AND lease_until<now() AND attempts>=$2::integer \
                     AND ($1='' OR team_id=$1)";
    let limit = MAX_FILE_ATTEMPTS.to_string();
    // 終わったジョブの行は、ジョブの `processed_files` も数え直す。結果を書くときと同じく、ジョブの行を先にロックする
    let jobs: Vec<String> = query(
        &mut tx,
        &format!(
            "SELECT json_build_object('job',job_id)::text FROM slack_files WHERE job_id IS NOT NULL AND {condition} \
             GROUP BY job_id ORDER BY job_id"
        ),
        &[team, &limit],
    )
    .await?
    .into_iter()
    .filter_map(|row| row["job"].as_str().map(str::to_string))
    .collect();
    for job in &jobs {
        query(
            &mut tx,
            "SELECT json_build_object('ok',true)::text FROM slack_import_jobs WHERE id=$1 FOR UPDATE",
            &[job],
        )
        .await?;
    }
    exec(
        &mut tx,
        &format!(
            "UPDATE slack_files SET status='failed',error='Fetching the file stopped repeatedly and was given up',\
             lease_token=NULL,lease_until=NULL,updated_at=now() WHERE {condition}"
        ),
        &[team, &limit],
    )
    .await?;
    for job in &jobs {
        exec(
            &mut tx,
            "UPDATE slack_import_jobs SET processed_files=(SELECT count(*) FROM slack_files WHERE job_id=$1 AND status<>'pending') \
             WHERE id=$1",
            &[job],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn claim_file(
    s: &AppState,
    team: &str,
    token: &str,
    bot: bool,
) -> Result<Option<PendingFile>, AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    // Bot トークンがなければ、差分同期の行 (`job_id` なし) だけを取る。ジョブの行は、ユーザーのトークンでは取らない
    // (Bot を設定するまで、未取得のまま残す)。
    // 削除済みと暗号化 (平文の添付を置けない) チャンネルの行は取らない。リースが切れた行と、失敗後の待ちが終わった行を取る。
    // ジョブの行はロックしない (ハートビートと競合しないよう、`FOR UPDATE OF f`)
    let rows = query(
        &mut tx,
        &format!(
            "WITH c AS (SELECT f.team_id,f.slack_file_id,f.message_id FROM slack_files f \
             JOIN channels ch ON ch.id=f.channel_id \
             WHERE f.status='pending' AND ($1='' OR f.team_id=$1) AND f.attempts<$3::integer \
             AND (f.lease_until IS NULL OR f.lease_until<now()) \
             AND ch.deleted_at IS NULL AND NOT COALESCE(ch.encrypted,false) \
             AND ($4::boolean OR f.job_id IS NULL) \
             AND (f.job_id IS NULL OR EXISTS(SELECT 1 FROM slack_import_jobs j WHERE j.id=f.job_id \
                  AND j.status IN ('completed','failed','cancelled'))) \
             ORDER BY f.created_at,f.slack_file_id,f.message_id LIMIT 1 FOR UPDATE OF f SKIP LOCKED) \
             UPDATE slack_files f SET lease_token=$2,lease_until=now()+{FILE_LEASE_SQL},attempts=f.attempts+1,updated_at=now() \
             FROM c WHERE f.team_id=c.team_id AND f.slack_file_id=c.slack_file_id AND f.message_id=c.message_id \
             RETURNING json_build_object('team_id',f.team_id,'slack_file_id',f.slack_file_id,\
             'message_id',f.message_id,'channel_id',f.channel_id,'uploader_id',f.uploader_id,'name',f.name,\
             'mimetype',f.mimetype,'size',f.size,'url',f.url_private_download,'attempts',f.attempts,\
             'job_id',f.job_id,'lease_token',f.lease_token)::text"
        ),
        &[
            team,
            token,
            &MAX_FILE_ATTEMPTS.to_string(),
            if bot { "true" } else { "false" },
        ],
    )
    .await?;
    tx.commit().await?;
    rows.into_iter()
        .next()
        .map(|value| serde_json::from_value(value).map_err(|e| AppError::Internal(e.into())))
        .transpose()
}

/// 取った行を手放す。停止・429・結果を書けなかったときに、リースの持ち主だけが戻す。
/// `retry` は取得を試行に数えない (取るときに増やした分を戻す)。`secs` だけ待ってから、次に取れるようにする
async fn release_file(
    s: &AppState,
    file: &PendingFile,
    retry: bool,
    secs: u64,
    note: &str,
) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    exec(
        &mut tx,
        "UPDATE slack_files SET lease_token=NULL,\
         lease_until=CASE WHEN $5::bigint=0 THEN NULL ELSE now()+make_interval(secs=>$5::bigint) END,\
         attempts=GREATEST(attempts-CASE WHEN $4::boolean THEN 1 ELSE 0 END,0),\
         error=COALESCE(NULLIF($7,''),error),updated_at=now() \
         WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$3 AND lease_token=$6 AND status='pending'",
        &[
            &file.team_id,
            &file.slack_file_id,
            &file.message_id,
            if retry { "true" } else { "false" },
            &secs.to_string(),
            &file.lease_token,
            note,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn orphan_files_step(
    s: &AppState,
    client: &reqwest::Client,
    stop: &mut Stop,
    team: &str,
) -> Result<FileStep, AppError> {
    if *stop.borrow() {
        return Ok(FileStep::Stopped);
    }
    fail_exhausted_files(s, team).await?;
    let lease_token = uuid::Uuid::new_v4().to_string();
    let Some(file) = claim_file(
        s,
        team,
        &lease_token,
        s.config.slack.import_bot_token.is_some(),
    )
    .await?
    else {
        return Ok(FileStep::Idle);
    };
    let tokens = file_tokens(s, &file).await?;
    // 取得の途中で合図を受けたら、取得を捨てて行を手放す (添付は決定的 ID で作るので、取り直しても重複しない)
    let outcome = tokio::select! {
        outcome = fetch_one(s, client, &tokens.tokens, &file) => outcome,
        _ = stopped(stop) => {
            release_file(s, &file, true, 0, "").await?;
            return Ok(FileStep::Stopped);
        }
    };
    let (status, attachment, error) = match outcome {
        Ok(FileOutcome::RetryLater(secs)) => {
            release_file(s, &file, true, secs, "").await?;
            tracing::info!(secs, "Slack file download rate limited");
            return Ok(FileStep::Wait(secs));
        }
        Ok(FileOutcome::Done(attachment)) => ("done", attachment, String::new()),
        Ok(FileOutcome::Skipped(reason)) => ("skipped", String::new(), reason),
        // どのトークンにも見えなかった。ユーザーのトークンを使える行で、ワークスペースに `files:read` を持つ連携がまだ
        // 1 人もいないなら (再連携が進むまで)、試行に数えず待つ。連携がある (ユーザーも試した、またはこのチャンネルの
        // メンバーが連携していない)、またはユーザーのトークンを使えない行は、取り直しても変わらないので、
        // すぐ `failed` にする
        Ok(FileOutcome::Denied) if tokens.eligible && !tokens.workspace_has_links => {
            release_file(s, &file, true, NO_USER_TOKEN_WAIT_SECS, NO_USER_TOKEN_NOTE).await?;
            return Ok(FileStep::Done);
        }
        // 復号できない連携があると、ユーザーのトークンを試せていない。権限なしとは確定せず、通常の取り直しにする
        Ok(FileOutcome::Denied) if tokens.decrypt_failed && file.attempts < MAX_FILE_ATTEMPTS => (
            "pending",
            String::new(),
            "a linked user's Slack token could not be decrypted".to_string(),
        ),
        Ok(FileOutcome::Denied) => ("failed", String::new(), DENIED_NOTE.to_string()),
        // 取るときに試行回数を増やしてあるので、`attempts` は今回を含む
        Err(e) if file.attempts >= MAX_FILE_ATTEMPTS => ("failed", String::new(), e.to_string()),
        Err(e) => ("pending", String::new(), e.to_string()),
    };
    // 結果の書き込みで失敗しても、試行に数えず、行を手放して取り直す
    match write_file_result(s, &file, status, &attachment, &error).await {
        Ok(()) => {
            if status == "failed" {
                tracing::warn!(
                    slack_file_id = %file.slack_file_id,
                    channel = %file.channel_id,
                    error = %error,
                    "Giving up on a Slack file"
                );
            }
            Ok(FileStep::Done)
        }
        Err(e) => {
            if let Err(release) = release_file(s, &file, true, 0, "").await {
                tracing::warn!(error = %release, "Failed to release a Slack file");
            }
            Err(e)
        }
    }
}

async fn write_file_result(
    s: &AppState,
    file: &PendingFile,
    status: &str,
    attachment: &str,
    error: &str,
) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    // `insert_message` と同じ順 (messages → slack_files) にロックして、同期との行き違いのデッドロックを避ける
    query(
        &mut tx,
        "SELECT json_build_object('ok',true)::text FROM messages WHERE id=$1 FOR UPDATE",
        &[&file.message_id],
    )
    .await?;
    // 終わったジョブの行は、結果を書くときに、ジョブの `processed_files` も数え直す。同時に書く行の結果を
    // 取りこぼさないよう、ジョブの行を先にロックする (messages → ジョブ → slack_files の順)
    if let Some(job_id) = file.job_id.as_deref() {
        query(
            &mut tx,
            "SELECT json_build_object('ok',true)::text FROM slack_import_jobs WHERE id=$1 FOR UPDATE",
            &[job_id],
        )
        .await?;
    }
    // 取得中に再取り込みが行を別のジョブへ移していたら (リースも消える)、古い結果は書かない。
    // 失敗して再試行するときは、リースの期限を「次に試す時刻」にして、短い間に使い切らないようにする
    let updated = query(
        &mut tx,
        "UPDATE slack_files SET status=$4,attachment_id=NULLIF($5,''),error=NULLIF($6,''),lease_token=NULL,\
         lease_until=CASE WHEN $4='pending' THEN now()+make_interval(mins=>(attempts*10)::integer) END,\
         updated_at=now() \
         WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$3 AND lease_token=$7 AND status='pending' \
         AND job_id IS NOT DISTINCT FROM NULLIF($8,'') \
         RETURNING json_build_object('ok',true)::text",
        &[
            &file.team_id,
            &file.slack_file_id,
            &file.message_id,
            status,
            attachment,
            error,
            &file.lease_token,
            file.job_id.as_deref().unwrap_or(""),
        ],
    )
    .await?;
    if !updated.is_empty() {
        record_file_result(&mut tx, file, status).await?;
        if let Some(job_id) = file.job_id.as_deref() {
            exec(
                &mut tx,
                "UPDATE slack_import_jobs SET processed_files=(SELECT count(*) FROM slack_files WHERE job_id=$1 AND status<>'pending') \
                 WHERE id=$1",
                &[job_id],
            )
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// 取得を許可した URL (`SLACK_FILES_BASE_URL` の下) へのリダイレクトだけを、最大 3 回まで追従して取得する。
/// `Authorization` (Bot トークン) は、最初の URL と同じオリジン (scheme・ホスト・ポート) にだけ付ける。
/// クライアントは自動の追従をしない (`Policy::none`) ので、ここで 1 回ずつ確かめる
///
/// Slack のドメイン (`*.slack.com`、許可した URL のホストを除く) へのリダイレクトは、ログイン画面に飛ばされた
/// (このトークンには、ファイルが見えない) ことのサインなので、`None` を返す。ほかの許可外 URL は、エラーにする。
/// 許可は広げない (`Authorization` は許可した URL にだけ送る)
async fn get_following_redirects(
    client: &reqwest::Client,
    url: &str,
    token: &str,
    base: &str,
) -> Result<Option<reqwest::Response>, AppError> {
    let failed =
        |detail: String| AppError::BadGateway(format!("Slack file download failed: {detail}"));
    let origin = reqwest::Url::parse(url).map_err(|e| failed(e.to_string()))?;
    let mut current = origin.clone();
    for _ in 0..=3 {
        let mut request = client.get(current.clone());
        if current.origin() == origin.origin() {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| failed(e.to_string()))?;
        if !response.status().is_redirection() {
            return Ok(Some(response));
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| failed("a redirect without a location".into()))?;
        let next = current.join(location).map_err(|e| failed(e.to_string()))?;
        if !allowed_file_url(next.as_str(), base) {
            if is_login_redirect(&next, base) {
                return Ok(None);
            }
            // リダイレクト先の URL は、元の URL を含むことがあるので、エラーには入れない
            return Err(failed("redirected to a URL that is not allowed".into()));
        }
        current = next;
    }
    Err(failed("too many redirects".into()))
}

fn is_login_redirect(next: &reqwest::Url, base: &str) -> bool {
    let base_host = reqwest::Url::parse(base)
        .ok()
        .and_then(|b| b.host_str().map(str::to_ascii_lowercase));
    next.scheme() == "https"
        && next.host_str().is_some_and(|h| {
            let h = h.to_ascii_lowercase();
            h.ends_with(".slack.com") && Some(&h) != base_host.as_ref()
        })
}

/// 本文を `max` バイトまで読む。累計が上限を超えたら、残りを読まずに `None` を返す
async fn read_limited<S, B, E>(mut stream: S, max: u64) -> Result<Option<Vec<u8>>, E>
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
{
    let mut buffer = Vec::new();
    while let Some(chunk) = futures_util::StreamExt::next(&mut stream).await {
        let chunk = chunk?;
        let chunk = chunk.as_ref();
        if (buffer.len() + chunk.len()) as u64 > max {
            return Ok(None);
        }
        buffer.extend_from_slice(chunk);
    }
    Ok(Some(buffer))
}

/// 添付を取得してよい URL か (`SLACK_FILES_BASE_URL` の下か)。Bot トークンを付けて送るので、
/// 文字列の前方一致ではなく、scheme・ホスト・ポートを URL として比べ、パスはディレクトリ単位で比べる
/// (`https://files.slack.com` と設定しても `https://files.slack.com.evil.example/` は通さない)。
fn allowed_file_url(url: &str, base: &str) -> bool {
    let (Ok(url), Ok(base)) = (reqwest::Url::parse(url), reqwest::Url::parse(base)) else {
        return false;
    };
    let mut prefix = base.path().to_string();
    if !prefix.ends_with('/') {
        prefix.push('/');
    }
    url.scheme() == base.scheme()
        && url.host_str().is_some()
        && url.host_str() == base.host_str()
        && url.port_or_known_default() == base.port_or_known_default()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with(&prefix)
}

/// 取得中の行のリースを延ばす (リースの持ち主だけ)
async fn renew_file_lease(s: &AppState, file: &PendingFile) -> Result<(), AppError> {
    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    exec(
        &mut tx,
        &format!(
            "UPDATE slack_files SET lease_until=now()+{FILE_LEASE_SQL} \
             WHERE team_id=$1 AND slack_file_id=$2 AND message_id=$3 AND lease_token=$4 AND status='pending'"
        ),
        &[
            &file.team_id,
            &file.slack_file_id,
            &file.message_id,
            &file.lease_token,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// `tokens` を前から順に試して、最初に取れたものを使う。トークンごとに、ログイン画面 (権限なし) は次へ進み、
/// 一時的な失敗 (5xx・通信) も次へ進む (全部だめなら、その失敗を返す)。429 は、残りを試さずに待つ。
/// 全部が権限なしなら `Denied`。ユーザーのトークンは、呼び出し側が、その行のファイルの URL のときだけ渡す
async fn fetch_one(
    s: &AppState,
    client: &reqwest::Client,
    tokens: &[String],
    file: &PendingFile,
) -> Result<FileOutcome, AppError> {
    let Some(url) = file
        .url
        .as_deref()
        .filter(|u| allowed_file_url(u, &s.config.slack.files_base_url))
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
    // 取り込みのあとに暗号化へ切り替えたチャンネル (#367) には、平文の添付を置かない (行単位の取得は、
    // 取る行を選ぶときにも外している)。取得の途中で切り替わったら、添付の INSERT を DB のトリガーが拒否し、
    // 次の試行でここに来る
    {
        let mut db = s.db.clone();
        let mut tx = db.transaction().await?;
        let encrypted =
            crate::routes::e2ee_channels::is_encrypted_channel(&mut tx, &file.channel_id).await?;
        tx.commit().await?;
        if encrypted {
            return Ok(FileOutcome::Skipped("Encrypted channel".into()));
        }
    }

    let mut transient = None;
    let mut found = None;
    for (i, token) in tokens.iter().enumerate() {
        // トークンを順に試すと、リース (10 分) を超えうるので、次のトークンの前に延ばす
        if i > 0 {
            renew_file_lease(s, file).await?;
        }
        let response =
            match get_following_redirects(client, url, token, &s.config.slack.files_base_url).await
            {
                Ok(Some(response)) => response,
                Ok(None) => continue,
                Err(e) => {
                    transient = Some(e);
                    continue;
                }
            };
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let secs = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(60)
                .clamp(1, MAX_RETRY_AFTER_SECS);
            return Ok(FileOutcome::RetryLater(secs));
        }
        if !response.status().is_success() {
            transient = Some(AppError::BadGateway(format!(
                "Slack file download returned {}",
                response.status()
            )));
            continue;
        }
        // トークンの権限が足りないと、Slack はログイン画面の HTML を返す
        let is_html = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html") && !file.mimetype.starts_with("text/html"));
        if is_html {
            continue;
        }
        found = Some(response);
        break;
    }
    let Some(response) = found else {
        // 一時的な失敗が 1 つでもあれば、取り直す。全部が権限なしのときだけ、確定させる
        return match transient {
            Some(e) => Err(e),
            None => Ok(FileOutcome::Denied),
        };
    };
    if response.content_length().is_some_and(|len| len > max) {
        return Ok(FileOutcome::Skipped(format!("File exceeds {max} bytes")));
    }
    // サイズが記録されていない・Content-Length がない応答でも、上限を超える分をメモリに読み込まないよう、
    // チャンク単位で読み、累計が上限を超えた時点で中断する
    let Some(buffer) = read_limited(response.bytes_stream(), max)
        .await
        .map_err(|e| AppError::BadGateway(format!("Slack file download failed: {e}")))?
    else {
        return Ok(FileOutcome::Skipped(format!("File exceeds {max} bytes")));
    };

    // 同じファイルを共有するメッセージごとに添付を作る
    let attachment = super::attachment_id(
        &file.team_id,
        &format!("{}:{}", file.slack_file_id, file.message_id),
    );
    let key = format!("attachments/{}/{attachment}", file.channel_id);
    let size = buffer.len();
    s.s3_client
        .put_object()
        .bucket(&s.config.s3.bucket)
        .key(&key)
        .content_type(&file.mimetype)
        .body(buffer.into())
        .send()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to store Slack file: {e}")))?;

    let mut db = s.db.clone();
    let mut tx = db.transaction().await?;
    // 取得の途中で取り込み先が暗号化に切り替わっていたら (#367)、平文の添付の行を作らず、置いたオブジェクトも消す。
    // チャンネルの行を FOR KEY SHARE で読むので、この確認から INSERT までが切り替えと直列になる。
    // 保存先のキーは同じファイルなら同じなので、前の取得が切り替えの前に行を作っていたら (取り直し)、その添付の
    // 実体を消さず、その行を結果にする (切り替える前の添付は残す)
    if crate::routes::e2ee_channels::lock_encryption(&mut tx, &file.channel_id).await? {
        let registered = !query(
            &mut tx,
            "SELECT json_build_object('ok',true)::text FROM attachments WHERE id=$1",
            &[&attachment],
        )
        .await?
        .is_empty();
        tx.rollback().await?;
        if registered {
            return Ok(FileOutcome::Done(attachment));
        }
        if let Err(e) = s
            .s3_client
            .delete_object()
            .bucket(&s.config.s3.bucket)
            .key(&key)
            .send()
            .await
        {
            tracing::warn!(
                key,
                "Failed to remove a Slack file stored for an encrypted channel: {e}"
            );
        }
        return Ok(FileOutcome::Skipped("Encrypted channel".into()));
    }
    exec(
        &mut tx,
        "INSERT INTO attachments(id,message_id,uploader_id,channel_id,file_name,file_size,content_type,storage_key,created_at,is_sensitive) \
         SELECT $1,m.id,COALESCE((SELECT sf.uploader_id FROM slack_files sf WHERE sf.team_id=$9 AND sf.slack_file_id=$10 AND sf.message_id=$2),$3),$4,$5,$6::bigint,$7,$8,m.created_at,(SELECT is_nsfw FROM channels WHERE id=$4) FROM messages m WHERE m.id=$2 \
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
            &file.team_id,
            &file.slack_file_id,
        ],
    )
    .await?;
    tx.commit().await?;
    Ok(FileOutcome::Done(attachment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn later_profile_replaces_fallback_metadata_without_changing_mentions()
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
        anyhow::ensure!(
            database.contains("test") || database.contains("rereview"),
            "refusing non-test database `{database}`"
        );
        let db = crate::db::init_db(&database_url).await?;
        let team = format!("TPROFILE{}", uuid::Uuid::new_v4().simple());
        let slack_id = format!(
            "U{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .to_ascii_uppercase()
        );
        let user_id = super::super::user_id(&team, &slack_id);
        let linked_slack_id = format!(
            "L{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .to_ascii_uppercase()
        );
        let linked_user_id = uuid::Uuid::new_v4().to_string();
        {
            let mut setup_db = db.clone();
            let mut setup = setup_db.transaction().await?;
            exec(
                &mut setup,
                "INSERT INTO users(id,user_name,display_name,avatar_url) VALUES($1,$2,'Linked original','https://avatars.example/original.png')",
                &[&linked_user_id, &format!("profile_linked_{}", uuid::Uuid::new_v4().simple())],
            ).await?;
            exec(
                &mut setup,
                "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name) VALUES($1,$2,$3,true,'linked-original','Linked Original')",
                &[&team, &linked_slack_id, &linked_user_id],
            ).await?;
            setup.commit().await?;
        }
        let result: anyhow::Result<()> = async {
            let mut ctx = Context {
                team: team.clone(), users: HashMap::new(), profiles: HashMap::new(),
                fallback_names: HashMap::from([(slack_id.clone(), "deploybot".into())]),
                channel_names: HashMap::new(),
                files: FilePolicy { token: false, max_size: 0, base_url: String::new() },
                app_id: None,
            };
            let first = {
                let mut task_db = db.clone();
                let mut tx = task_db.transaction().await?;
                let users = channel_users(&mut ctx, &mut tx, std::slice::from_ref(&slack_id)).await?;
                tx.commit().await?;
                users[0].clone()
            };
            let mention_user_name = first.user_name.clone();
            ctx.profiles.insert(slack_id.clone(), serde_json::from_value(json!({
                "id": slack_id, "name": "alice", "real_name": "Alice Example",
                "profile": {"display_name": "Alice", "image_72": "https://avatars.example/alice.png"}
            }))?);
            ctx.profiles.insert(linked_slack_id.clone(), serde_json::from_value(json!({
                "id": linked_slack_id, "name": "changed-linked", "real_name": "Changed Linked",
                "profile": {"display_name": "Changed Linked", "image_72": "https://avatars.example/changed.png"}
            }))?);
            let second = {
                let mut task_db = db.clone();
                let mut tx = task_db.transaction().await?;
                let users = channel_users(&mut ctx, &mut tx, &[slack_id.clone(), linked_slack_id.clone()]).await?;
                tx.commit().await?;
                users[0].clone()
            };
            anyhow::ensure!(second.user_name == mention_user_name, "profile refresh changed the mention username");
            let mut verify_db = db.clone();
            let mut tx = verify_db.transaction().await?;
            let row = query(&mut tx, "SELECT json_build_object('display_name',u.display_name,'avatar_url',u.avatar_url,'name',s.name,'real_name',s.real_name,'profile_source',s.profile_source)::text FROM users u JOIN slack_users s ON s.user_id=u.id WHERE u.id=$1", &[&user_id]).await?;
            let linked = query(&mut tx, "SELECT json_build_object('display_name',u.display_name,'avatar_url',u.avatar_url,'name',s.name,'real_name',s.real_name,'linked',s.linked)::text FROM users u JOIN slack_users s ON s.user_id=u.id WHERE u.id=$1", &[&linked_user_id]).await?;
            tx.commit().await?;
            anyhow::ensure!(row.len() == 1, "placeholder disappeared");
            anyhow::ensure!(row[0]["display_name"] == "Alice", "profile display name was not applied: {}", row[0]);
            anyhow::ensure!(row[0]["avatar_url"] == "https://avatars.example/alice.png", "profile avatar was not applied: {}", row[0]);
            anyhow::ensure!(row[0]["name"] == "alice" && row[0]["real_name"] == "Alice Example", "Slack profile metadata was not applied: {}", row[0]);
            anyhow::ensure!(linked.len() == 1 && linked[0]["linked"] == true && linked[0]["display_name"] == "Linked original" && linked[0]["avatar_url"] == "https://avatars.example/original.png" && linked[0]["name"] == "linked-original" && linked[0]["real_name"] == "Linked Original", "profile refresh changed the linked Zoo account: {linked:?}");
            Ok(())
        }.await;
        let mut cleanup_db = db.clone();
        let mut cleanup = cleanup_db.transaction().await?;
        exec(&mut cleanup, "DELETE FROM users WHERE id=$1", &[&user_id]).await?;
        exec(
            &mut cleanup,
            "DELETE FROM users WHERE id=$1",
            &[&linked_user_id],
        )
        .await?;
        cleanup.commit().await?;
        result
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn channel_user_name_refresh_locks_real_and_placeholder_users_before_update()
    -> anyhow::Result<()> {
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
            database.contains("test") || database.contains("rereview"),
            "refusing non-test database `{database}`"
        );
        let db = crate::db::init_db(&database_url).await?;
        let team = format!("TLOCK{}", uuid::Uuid::new_v4().simple());
        let candidate_a = "U-LOCK-A";
        let candidate_b = "U-LOCK-B";
        let placeholder_a = super::super::user_id(&team, candidate_a);
        let placeholder_b = super::super::user_id(&team, candidate_b);
        let (generic_slack, generic_id, linked_slack) = if placeholder_a < placeholder_b {
            (candidate_a, placeholder_a, candidate_b)
        } else {
            (candidate_b, placeholder_b, candidate_a)
        };
        let low_user = loop {
            let random = uuid::Uuid::new_v4().to_string();
            let candidate = format!("00000000-{}", &random[9..]);
            if candidate < generic_id {
                break candidate;
            }
        };
        let username = format!("lock_test_{}", uuid::Uuid::new_v4().simple());
        {
            let mut fixture_db = db.clone();
            let mut tx = fixture_db.transaction().await?;
            exec(
                &mut tx,
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Low ID real user')",
                &[&low_user, &username],
            )
            .await?;
            exec(
                &mut tx,
                "INSERT INTO users(id,user_name,display_name,guest_kind) VALUES($1,$2,'Generic placeholder','deactivated')",
                &[&generic_id, &format!("slack_generic_{}", uuid::Uuid::new_v4().simple())],
            )
            .await?;
            exec(
                &mut tx,
                "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name) VALUES($1,$2,$3,false,$2,'')",
                &[&team, generic_slack, &generic_id],
            )
            .await?;
            exec(
                &mut tx,
                "INSERT INTO slack_users(team_id,slack_user_id,user_id,linked,name,real_name) VALUES($1,$2,$3,true,'linked','Linked Full Name')",
                &[&team, linked_slack, &low_user],
            )
            .await?;
            tx.commit().await?;
        }

        // Model E2EE conversion holding the lower-ID real participant, then requesting
        // the higher-ID placeholder while the import resolves and refreshes the generic name.
        let (e2ee_connection, e2ee_driver) =
            tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(e2ee_driver);
        let e2ee_pid: i32 = e2ee_connection
            .query_one("SELECT pg_backend_pid()", &[])
            .await?
            .get(0);
        let (e2ee_locked_tx, e2ee_locked_rx) = tokio::sync::oneshot::channel();
        let (release_e2ee_tx, release_e2ee_rx) = tokio::sync::oneshot::channel();
        let low_for_e2ee = low_user.clone();
        let generic_for_e2ee = generic_id.clone();
        let e2ee = tokio::spawn(async move {
            let mut connection = e2ee_connection;
            let tx = connection.transaction().await?;
            tx.query_one(
                "SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE",
                &[&low_for_e2ee],
            )
            .await?;
            let _ = e2ee_locked_tx.send(());
            release_e2ee_rx
                .await
                .map_err(|_| anyhow::anyhow!("E2EE release signal dropped"))?;
            tx.query_one(
                "SELECT id FROM users WHERE id=$1 FOR NO KEY UPDATE",
                &[&generic_for_e2ee],
            )
            .await?;
            tx.commit().await?;
            Ok::<(), anyhow::Error>(())
        });
        e2ee_locked_rx.await?;

        let import_db = db.clone();
        let team_for_import = team.clone();
        let (pid_tx, pid_rx) = tokio::sync::oneshot::channel();
        let import = tokio::spawn(async move {
            let mut import_db = import_db;
            let mut tx = import_db.transaction().await?;
            let pid: i32 = query(
                &mut tx,
                "SELECT json_build_object('pid',pg_backend_pid())::text",
                &[],
            )
            .await?[0]["pid"]
                .as_i64()
                .unwrap() as i32;
            let _ = pid_tx.send(pid);
            let mut ctx = Context {
                team: team_for_import,
                users: HashMap::new(),
                profiles: HashMap::new(),
                fallback_names: HashMap::from([(
                    generic_slack.to_string(),
                    "late-name".to_string(),
                )]),
                channel_names: HashMap::new(),
                files: FilePolicy {
                    token: false,
                    max_size: 0,
                    base_url: String::new(),
                },
                app_id: None,
            };
            let members = vec![generic_slack.to_string(), linked_slack.to_string()];
            let resolved = channel_users(&mut ctx, &mut tx, &members).await?;
            let ids: Vec<&str> = resolved.iter().map(|user| user.id.as_str()).collect();
            crate::routes::e2ee_channels::lock_membership_users(&mut tx, &ids).await?;
            tx.commit().await?;
            Ok::<(), anyhow::Error>(())
        });
        let import_pid = pid_rx.await?;
        let (observer, observer_driver) =
            tokio_postgres::connect(&database_url, tokio_postgres::NoTls).await?;
        tokio::spawn(observer_driver);
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let waiting: bool = observer
                    .query_one(
                        "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE pid=$1 AND wait_event_type='Lock' AND $2=ANY(pg_blocking_pids(pid)))",
                        &[&import_pid, &e2ee_pid],
                    )
                    .await?
                    .get(0);
                if waiting {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Ok::<(), tokio_postgres::Error>(())
        })
        .await??;

        let _ = release_e2ee_tx.send(());
        let (import_result, e2ee_result) =
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                tokio::join!(import, e2ee)
            })
            .await
            .map_err(|_| anyhow::anyhow!("user lock-order reproduction timed out"))?;

        // This dedicated database contains only disposable test data; clean it even if one
        // side of the reproduction reports PostgreSQL's deadlock error.
        let mut cleanup_db = db.clone();
        let mut cleanup = cleanup_db.transaction().await?;
        exec(
            &mut cleanup,
            "DELETE FROM slack_users WHERE team_id=$1",
            &[&team],
        )
        .await?;
        exec(
            &mut cleanup,
            "DELETE FROM users WHERE id IN ($1,$2)",
            &[&low_user, &generic_id],
        )
        .await?;
        cleanup.commit().await?;
        import_result.map_err(|error| anyhow::anyhow!("import task panicked: {error}"))??;
        e2ee_result.map_err(|error| anyhow::anyhow!("E2EE task panicked: {error}"))??;
        Ok(())
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn zip_username_from_later_day_keeps_earlier_mentions_resolvable_after_link()
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
        anyhow::ensure!(
            database.contains("test") || database.contains("rereview"),
            "refusing non-test database"
        );
        let mut db = crate::db::init_db(&database_url).await?;
        let team = format!("T{}", uuid::Uuid::new_v4().simple());
        let channel_slack_id = format!("C{}", uuid::Uuid::new_v4().simple());
        let slack_user = format!(
            "U{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .to_ascii_uppercase()
        );
        let channel_id = uuid::Uuid::new_v4().to_string();
        let zoo_channel_name = format!("mention_link_{}", uuid::Uuid::new_v4().simple());
        let linked_user = uuid::Uuid::new_v4().to_string();
        let linked_name = format!("linked_{}", uuid::Uuid::new_v4().simple());
        let zip = super::super::export::tests::build_zip(
            "",
            &[
                (
                    "users.json",
                    serde_json::json!([{"id":"U-SEED","team_id":team,"name":"seed"}]),
                ),
                (
                    "channels.json",
                    serde_json::json!([{"id":channel_slack_id,"name":"mentions","members":[slack_user]}]),
                ),
                (
                    "mentions/2024-01-01.json",
                    serde_json::json!([
                        {"ts":"1704070000.000100","user":"U-OTHER","text":format!("hello <@{slack_user}>")}
                    ]),
                ),
                (
                    "mentions/2024-01-02.json",
                    serde_json::json!([
                        {"ts":"1704156400.000100","user":slack_user,"bot_profile":{"name":"lower-priority"},"text":"fallback only"},
                        {"ts":"1704156401.000100","user":slack_user,"username":"target-name","text":"later post"}
                    ]),
                ),
            ],
        );
        let zip_path = std::env::temp_dir().join(format!(
            "zoo-slack-mention-link-{}.zip",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&zip_path, zip)?;
        let mut archive = ExportArchive::open(&zip_path)?;
        let profiles = archive
            .users()?
            .into_iter()
            .map(|user| (user.id.clone(), user))
            .collect();
        let channel = archive.channels()?.into_iter().next().unwrap();
        let entries = archive.day_files(&channel);
        anyhow::ensure!(
            entries.len() == 2
                && entries[0].ends_with("2024-01-01.json")
                && entries[1].ends_with("2024-01-02.json"),
            "expected separate ordered day files"
        );
        let fallback_names =
            collect_archive_fallback_names(&mut archive, std::slice::from_ref(&channel), None)?;
        let day_one = archive.messages(&entries[0])?;
        let day_two = archive.messages(&entries[1])?;
        std::fs::remove_file(&zip_path)?;
        let mut ctx = Context {
            team: team.clone(),
            users: HashMap::new(),
            profiles,
            fallback_names,
            channel_names: HashMap::new(),
            files: FilePolicy {
                token: false,
                max_size: 0,
                base_url: String::new(),
            },
            app_id: None,
        };
        {
            let mut tx = db.transaction().await?;
            exec(
                &mut tx,
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Linked account')",
                &[&linked_user, &linked_name],
            )
            .await?;
            exec(&mut tx, "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,false)", &[&channel_id, &zoo_channel_name, &linked_user]).await?;
            exec(
                &mut tx,
                "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id) VALUES($1,$2,$3)",
                &[&team, &channel.id, &channel_id],
            )
            .await?;
            let refs: Vec<&SlackMessage> = day_one.iter().collect();
            resolve_message_users(&mut tx, &mut ctx, &channel.id, &refs).await?;
            insert_message(&mut tx, &mut ctx, &channel.id, &channel_id, "", &day_one[0]).await?;
            tx.commit().await?;
        }
        {
            let mut tx = db.transaction().await?;
            let refs: Vec<&SlackMessage> = day_two.iter().collect();
            resolve_message_users(&mut tx, &mut ctx, &channel.id, &refs).await?;
            for message in &day_two {
                insert_message(&mut tx, &mut ctx, &channel.id, &channel_id, "", message).await?;
            }
            tx.commit().await?;
        }
        let mention_id = super::super::message_id(&team, &channel.id, &day_one[0].ts);
        let placeholder = super::super::user_id(&team, &slack_user);
        let before_link = query(&mut db.transaction().await?, "SELECT json_build_object('content',m.content,'user_name',u.user_name)::text FROM messages m JOIN users u ON u.id=$2 WHERE m.id=$1", &[&mention_id, &placeholder]).await?;
        let content = before_link[0]["content"].as_str().unwrap();
        let final_name = before_link[0]["user_name"].as_str().unwrap();
        anyhow::ensure!(
            content.contains(&format!("@{final_name}")),
            "earlier mention must use the final pre-link name; content={content}, user_name={final_name}"
        );
        {
            let mut tx = db.transaction().await?;
            super::super::link::reassign(&mut tx, &team, &slack_user, "target-name", &linked_user)
                .await?;
            tx.commit().await?;
        }
        let linked = query(
            &mut db.transaction().await?,
            "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
            &[&mention_id],
        )
        .await?;
        anyhow::ensure!(
            linked[0]["content"]
                .as_str()
                .unwrap()
                .contains(&format!("@{linked_name}")),
            "actual link must rewrite the imported mention to the linked account name"
        );
        Ok(())
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn separate_zip_import_keeps_existing_placeholder_mentions_resolvable_after_link()
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
        anyhow::ensure!(
            database.contains("test") || database.contains("rereview"),
            "refusing non-test database"
        );
        let mut db = crate::db::init_db(&database_url).await?;
        let team = format!("T{}", uuid::Uuid::new_v4().simple());
        let slack_channel = format!("C{}", uuid::Uuid::new_v4().simple());
        let slack_user = format!(
            "U{}",
            uuid::Uuid::new_v4()
                .simple()
                .to_string()
                .to_ascii_uppercase()
        );
        let channel_id = uuid::Uuid::new_v4().to_string();
        let linked_user = uuid::Uuid::new_v4().to_string();
        let linked_name = format!("linked_{}", uuid::Uuid::new_v4().simple());
        let channel_name = format!("zip_reimport_{}", uuid::Uuid::new_v4().simple());
        let placeholder = super::super::user_id(&team, &slack_user);
        let test_result: anyhow::Result<()> = async {
            let first_zip = super::super::export::tests::build_zip(
            "",
            &[
                (
                    "users.json",
                    serde_json::json!([{"id":"U-SEED","team_id":team,"name":"seed"}]),
                ),
                (
                    "channels.json",
                    serde_json::json!([{"id":slack_channel,"name":"mentions","members":[slack_user]}]),
                ),
                (
                    "mentions/2024-01-01.json",
                    serde_json::json!([
                        {"ts":"1704070000.000100","user":"U-OTHER","text":format!("hello <@{slack_user}>")}
                    ]),
                ),
            ],
        );
        let second_zip = super::super::export::tests::build_zip(
            "",
            &[
                (
                    "users.json",
                    serde_json::json!([
                        {"id":"U-SEED","team_id":team,"name":"seed"},
                        {"id":slack_user,"team_id":team,"name":"profile-handle","real_name":"Profile Full Name","profile":{"display_name":"Profile Display"}}
                    ]),
                ),
                (
                    "channels.json",
                    serde_json::json!([{"id":slack_channel,"name":"mentions","members":[slack_user]}]),
                ),
                (
                    "mentions/2024-01-02.json",
                    serde_json::json!([
                        {"ts":"1704156400.000100","user":slack_user,"username":"target-name","text":"later post"}
                    ]),
                ),
            ],
        );
        type ParsedZip = (
            HashMap<String, SlackUser>,
            SlackChannel,
            Vec<SlackMessage>,
            HashMap<String, String>,
        );
        let parse_zip = |bytes: Vec<u8>| -> anyhow::Result<ParsedZip> {
            let mut archive = ExportArchive::new(Cursor::new(bytes))?;
            let profiles = archive
                .users()?
                .into_iter()
                .map(|u| (u.id.clone(), u))
                .collect();
            let channel = archive.channels()?.into_iter().next().unwrap();
            let entries = archive.day_files(&channel);
            let messages = archive.messages(&entries[0])?;
            let fallback_names =
                collect_archive_fallback_names(&mut archive, std::slice::from_ref(&channel), None)?;
            Ok((profiles, channel, messages, fallback_names))
        };
        let (profiles1, channel, first_messages, fallbacks1) = parse_zip(first_zip)?;
        let (profiles2, _, second_messages, fallbacks2) = parse_zip(second_zip)?;
        let make_context = |profiles, fallback_names| Context {
            team: team.clone(),
            users: HashMap::new(),
            profiles,
            fallback_names,
            channel_names: HashMap::new(),
            files: FilePolicy {
                token: false,
                max_size: 0,
                base_url: String::new(),
            },
            app_id: None,
        };
        let mut ctx1 = make_context(profiles1, fallbacks1);
        let mention_id = super::super::message_id(&team, &channel.id, &first_messages[0].ts);
        {
            let mut tx = db.transaction().await?;
            exec(
                &mut tx,
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Linked account')",
                &[&linked_user, &linked_name],
            )
            .await?;
            exec(&mut tx, "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,false)", &[&channel_id, &channel_name, &linked_user]).await?;
            // insert_message records the Slack timestamp in slack_message_map, whose
            // channel foreign key requires the corresponding Slack channel mapping.
            exec(
                &mut tx,
                "INSERT INTO slack_channels(team_id,slack_channel_id,channel_id) VALUES($1,$2,$3)",
                &[&team, &channel.id, &channel_id],
            )
            .await?;
            let members = channel.members.clone();
            channel_users(&mut ctx1, &mut tx, &members).await?;
            let refs: Vec<&SlackMessage> = first_messages.iter().collect();
            resolve_message_users(&mut tx, &mut ctx1, &channel.id, &refs).await?;
            insert_message(
                &mut tx,
                &mut ctx1,
                &channel.id,
                &channel_id,
                "",
                &first_messages[0],
            )
            .await?;
            tx.commit().await?;
        }
        let old_name = query(
            &mut db.transaction().await?,
            "SELECT json_build_object('name',user_name)::text FROM users WHERE id=$1",
            &[&placeholder],
        )
        .await?[0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut ctx2 = make_context(profiles2, fallbacks2);
        {
            let mut tx = db.transaction().await?;
            let refs: Vec<&SlackMessage> = second_messages.iter().collect();
            resolve_message_users(&mut tx, &mut ctx2, &channel.id, &refs).await?;
            insert_message(
                &mut tx,
                &mut ctx2,
                &channel.id,
                &channel_id,
                "",
                &second_messages[0],
            )
            .await?;
            tx.commit().await?;
        }
        let current_name = query(
            &mut db.transaction().await?,
            "SELECT json_build_object('name',user_name,'display',display_name)::text FROM users WHERE id=$1",
            &[&placeholder],
        )
        .await?[0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        anyhow::ensure!(
            current_name == old_name,
            "later ZIP fallback must not change a username referenced in earlier content"
        );
        let display_name = query(
            &mut db.transaction().await?,
            "SELECT json_build_object('display',display_name)::text FROM users WHERE id=$1",
            &[&placeholder],
        )
        .await?[0]["display"]
            .as_str()
            .unwrap()
            .to_owned();
        anyhow::ensure!(
            display_name == "Profile Display",
            "later ZIP profile must take precedence for generic placeholder display name"
        );
        let slack_name = query(
            &mut db.transaction().await?,
            "SELECT json_build_object('name',name)::text FROM slack_users WHERE team_id=$1 AND slack_user_id=$2",
            &[&team, &slack_user],
        )
        .await?[0]["name"]
            .as_str()
            .unwrap()
            .to_owned();
        anyhow::ensure!(
            slack_name == "profile-handle",
            "later ZIP profile must take precedence for generic placeholder Slack name"
        );
        {
            let mut tx = db.transaction().await?;
            super::super::link::reassign(&mut tx, &team, &slack_user, "target-name", &linked_user)
                .await?;
            tx.commit().await?;
        }
        let content = query(
            &mut db.transaction().await?,
            "SELECT json_build_object('content',content)::text FROM messages WHERE id=$1",
            &[&mention_id],
        )
        .await?[0]["content"]
            .as_str()
            .unwrap()
            .to_owned();
        anyhow::ensure!(
            content.contains(&format!("@{linked_name}")),
            "link must rewrite old placeholder mention: {content}"
        );
        anyhow::ensure!(
            !content.contains(&format!("@{old_name}")),
            "old placeholder mention must not remain: {content}"
        );
        Ok(())
        }
        .await;
        {
            let mut tx = db.transaction().await?;
            // Remove message_map rows through their message FK before dropping the
            // Slack channel mapping and its backing Zoo channel.
            exec(
                &mut tx,
                "DELETE FROM messages WHERE channel_id=$1",
                &[&channel_id],
            )
            .await?;
            exec(
                &mut tx,
                "DELETE FROM channel_user WHERE channel_id=$1",
                &[&channel_id],
            )
            .await?;
            exec(
                &mut tx,
                "DELETE FROM slack_channels WHERE team_id=$1 AND slack_channel_id=$2",
                &[&team, &slack_channel],
            )
            .await?;
            exec(&mut tx, "DELETE FROM channels WHERE id=$1", &[&channel_id]).await?;
            exec(
                &mut tx,
                "DELETE FROM slack_users WHERE team_id=$1",
                &[&team],
            )
            .await?;
            exec(
                &mut tx,
                "DELETE FROM users WHERE id IN ($1,$2,$3)",
                &[
                    &linked_user,
                    &placeholder,
                    &super::super::user_id(&team, "U-OTHER"),
                ],
            )
            .await?;
            tx.commit().await?;
        }
        test_result
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn zip_user_resolution_refreshes_linked_mapping_between_transactions()
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
        anyhow::ensure!(
            database.contains("test") || database.contains("rereview"),
            "refusing non-test database"
        );
        let mut db = crate::db::init_db(&database_url).await?;
        let team = format!("TLINK{}", uuid::Uuid::new_v4().simple());
        let slack_id = format!("U{}", uuid::Uuid::new_v4().simple());
        let channel_id = uuid::Uuid::new_v4().to_string();
        let real_user = uuid::Uuid::new_v4().to_string();
        let zip = super::super::export::tests::build_zip(
            "",
            &[
                (
                    "users.json",
                    serde_json::json!([{"id":"U-SEED","team_id":team,"name":"seed"}]),
                ),
                (
                    "channels.json",
                    serde_json::json!([{"id":"C-LINK","name":"linked","members":[slack_id]}]),
                ),
                (
                    "linked/2024-01-01.json",
                    serde_json::json!([{"ts":"1704070000.000100","user":slack_id,"username":"link-name","text":"day file"}]),
                ),
            ],
        );
        let zip_path = std::env::temp_dir().join(format!(
            "zoo-slack-link-test-{}.zip",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::write(&zip_path, &zip)?;
        let mut archive = ExportArchive::open(&zip_path)?;
        let profiles = archive
            .users()?
            .into_iter()
            .map(|user| (user.id.clone(), user))
            .collect();
        let channel = archive.channels()?.into_iter().next().unwrap();
        let entry = archive.day_files(&channel).into_iter().next().unwrap();
        let messages = archive.messages(&entry)?;
        std::fs::remove_file(&zip_path)?;
        let mut ctx = Context {
            team: team.clone(),
            users: HashMap::new(),
            profiles,
            fallback_names: HashMap::new(),
            channel_names: HashMap::new(),
            files: FilePolicy {
                token: false,
                max_size: 0,
                base_url: String::new(),
            },
            app_id: None,
        };
        {
            let mut tx = db.transaction().await?;
            exec(
                &mut tx,
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$2,'Linked user')",
                &[
                    &real_user,
                    &format!("linked_{}", uuid::Uuid::new_v4().simple()),
                ],
            )
            .await?;
            let resolved = channel_users(&mut ctx, &mut tx, &channel.members).await?;
            exec(&mut tx, "INSERT INTO channels(id,channel_name,created_user_id,is_private) VALUES($1,$2,$3,false)", &[&channel_id, &format!("linked_{}", uuid::Uuid::new_v4().simple()), &real_user]).await?;
            exec(
                &mut tx,
                "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
                &[&resolved[0].id, &channel_id],
            )
            .await?;
            tx.commit().await?;
        }
        {
            let mut tx = db.transaction().await?;
            super::super::link::reassign(&mut tx, &team, &slack_id, "linked", &real_user).await?;
            tx.commit().await?;
        }
        {
            let mut tx = db.transaction().await?;
            let message_refs: Vec<&SlackMessage> = messages.iter().collect();
            resolve_message_users(&mut tx, &mut ctx, &channel.id, &message_refs).await?;
            let resolved = ctx.users.get(&slack_id).unwrap();
            anyhow::ensure!(
                resolved.id == real_user,
                "ZIP user cache returned stale placeholder {} after Slack linking",
                resolved.id
            );
            // Model a day-file transaction that fails after resolving the author. The committed
            // membership must already belong to the linked Zoo account.
            tx.rollback().await?;
        }
        let rows = query(&mut db.transaction().await?, "SELECT json_build_object('ok',true)::text FROM channel_user WHERE user_id=$1 AND channel_id=$2", &[&real_user, &channel_id]).await?;
        anyhow::ensure!(
            rows.len() == 1,
            "linked user's imported channel membership was lost"
        );
        Ok(())
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn concurrent_channel_imports_create_common_users_in_id_order() -> anyhow::Result<()> {
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

        let baseline_team = format!("TBASE{}", uuid::Uuid::new_v4().simple());
        let order_team = format!("TORDER{}", uuid::Uuid::new_v4().simple());
        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let slack_ids = [format!("U-A-{suffix}"), format!("U-B-{suffix}")];
        let order_suffix = uuid::Uuid::new_v4()
            .simple()
            .to_string()
            .to_ascii_uppercase();
        let order_slack_ids = [
            format!("UAUTHOR{order_suffix}"),
            format!("UMENTION{order_suffix}"),
            format!("UREACTION{order_suffix}"),
        ];
        let creator = uuid::Uuid::new_v4().to_string();
        let channel_id = uuid::Uuid::new_v4().to_string();
        let result: anyhow::Result<()> = async {

        // Reproduce the pre-fix ZIP path: each transaction creates its first
        // unseen user, then asks for the other's user in the opposite order.
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let mut baseline = Vec::new();
        for (first, second, actor) in [
            (&slack_ids[0], &slack_ids[1], "left"),
            (&slack_ids[1], &slack_ids[0], "right"),
        ] {
            let mut task_db = db.clone();
            let task_team = baseline_team.clone();
            let first = first.clone();
            let second = second.clone();
            let fallback = format!("{actor}_{}", uuid::Uuid::new_v4().simple());
            let task_barrier = barrier.clone();
            baseline.push(tokio::spawn(async move {
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
                ctx.user(&mut tx, &first, Some(&fallback)).await?;
                task_barrier.wait().await;
                let failure = ctx
                    .user(&mut tx, &second, Some(&fallback))
                    .await
                    .err()
                    .map(|error| format!("{error:?}"));
                let failed = failure.is_some();
                if failed {
                    tx.rollback().await?;
                } else {
                    tx.commit().await?;
                }
                Ok::<Option<String>, anyhow::Error>(failure)
            }));
        }
        let mut failures = Vec::new();
        for task in baseline {
            failures.push(tokio::time::timeout(Duration::from_secs(5), task).await???);
        }
        anyhow::ensure!(
            failures
                .iter()
                .flatten()
                .any(|error| error.contains("deadlock detected")),
            "expected PostgreSQL deadlock_detected (40P01) for the old unordered path: {failures:?}"
        );

        {
            let mut setup_db = db.clone();
            let mut setup = setup_db.transaction().await?;
            toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$2,$2)")
                .bind(creator.as_str())
                .bind(format!("zip_batch_owner_{}", uuid::Uuid::new_v4().simple()))
                .exec(&mut setup)
                .await?;
            toasty::sql::statement(
                "INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,$2,$3,false,false)",
            )
            .bind(channel_id.as_str())
            .bind(format!("zip_batch_channel_{}", uuid::Uuid::new_v4().simple()))
            .bind(creator.as_str())
            .exec(&mut setup)
            .await?;
            setup.commit().await?;

            let author = order_slack_ids[0].clone();
            let mention = order_slack_ids[1].clone();
            let reactor = order_slack_ids[2].clone();
            let first_message: SlackMessage = serde_json::from_value(serde_json::json!({
                "ts":"1710000000.000001",
                "user":author,
                "text":format!("hello <@{mention}>"),
                "reactions":[{"name":"thumbsup","users":[reactor]}]
            }))?;
            let author = order_slack_ids[2].clone();
            let mention = order_slack_ids[1].clone();
            let reactor = order_slack_ids[0].clone();
            let second_message: SlackMessage = serde_json::from_value(serde_json::json!({
                "ts":"1710000000.000002",
                "user":author,
                "text":format!("hello <@{mention}>"),
                "reactions":[{"name":"thumbsup","users":[reactor]}]
            }))?;
            let gate = std::sync::Arc::new(tokio::sync::Barrier::new(2));
            let mut tasks = Vec::new();
            for message in [first_message, second_message] {
                let mut task_db = db.clone();
                let task_team = order_team.clone();
                let task_gate = gate.clone();
                let task_channel = channel_id.clone();
                tasks.push(tokio::spawn(async move {
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
                    let messages = [&message];
                    resolve_message_users(&mut tx, &mut ctx, "CZIP", &messages)
                        .await
                        .map_err(|error| {
                            anyhow::anyhow!("resolving ZIP message users: {error:?}")
                        })?;
                    insert_message(
                        &mut tx,
                        &mut ctx,
                        "CZIP",
                        &task_channel,
                        "zip-test",
                        &message,
                    )
                    .await
                    .map_err(|error| anyhow::anyhow!("inserting ZIP message: {error:?}"))?;
                    tx.commit()
                        .await
                        .map_err(|error| anyhow::anyhow!("committing ZIP message: {error:?}"))?;
                    Ok::<(), anyhow::Error>(())
                }));
            }
            for task in tasks {
                tokio::time::timeout(Duration::from_secs(5), task).await???;
            }
            let mut verify_db = db.clone();
            let mut verify = verify_db.transaction().await?;
            let counts = query(
                &mut verify,
                "SELECT json_build_object('users',(SELECT count(*) FROM slack_users WHERE team_id=$1),'messages',(SELECT count(*) FROM messages WHERE channel_id=$2),'reactions',(SELECT count(*) FROM message_reactions r JOIN messages m ON m.id=r.message_id WHERE m.channel_id=$2))::text",
                &[&order_team, &channel_id],
            )
            .await?;
            verify.commit().await?;
            anyhow::ensure!(
                counts[0]["users"] == 3,
                "author, mention, and reactor are resolved: {}",
                counts[0]
            );
            anyhow::ensure!(counts[0]["messages"] == 2, "both ZIP messages are imported");
            anyhow::ensure!(counts[0]["reactions"] == 2, "both reactions are imported");
        }
        Ok(())
        }
        .await;

        // Keep the dedicated database clean even when an assertion or a spawned
        // transaction fails. All rows use these test-specific IDs.
        let cleanup_result: anyhow::Result<()> = async {
            let test_users: Vec<String> = slack_ids
                .iter()
                .map(|id| crate::slack::user_id(&baseline_team, id))
                .chain(
                    order_slack_ids
                        .iter()
                        .map(|id| crate::slack::user_id(&order_team, id)),
                )
                .chain(std::iter::once(creator.clone()))
                .collect();
            let mut cleanup_db = db.clone();
            let mut cleanup = cleanup_db.transaction().await?;
            exec(&mut cleanup, "DELETE FROM message_reactions WHERE message_id IN (SELECT id FROM messages WHERE channel_id=$1)", &[&channel_id]).await?;
            exec(&mut cleanup, "DELETE FROM messages WHERE channel_id=$1", &[&channel_id]).await?;
            exec(&mut cleanup, "DELETE FROM channel_read_states WHERE channel_id=$1", &[&channel_id]).await?;
            exec(&mut cleanup, "DELETE FROM channel_user WHERE channel_id=$1", &[&channel_id]).await?;
            exec(&mut cleanup, "DELETE FROM channels WHERE id=$1", &[&channel_id]).await?;
            exec(&mut cleanup, "DELETE FROM slack_message_map WHERE team_id IN ($1,$2)", &[&baseline_team, &order_team]).await?;
            exec(&mut cleanup, "DELETE FROM slack_users WHERE team_id IN ($1,$2)", &[&baseline_team, &order_team]).await?;
            for user in &test_users {
                exec(&mut cleanup, "DELETE FROM users WHERE id=$1", &[user]).await?;
            }
            cleanup.commit().await?;
            Ok(())
        }
        .await;
        cleanup_result?;
        result
    }

    #[tokio::test]
    async fn read_limited_stops_at_the_limit() {
        let chunks = |n: usize| {
            futures_util::stream::iter((0..n).map(|_| Ok::<_, std::io::Error>(vec![b'x'; 8])))
        };
        // 上限ちょうどまでは読める
        assert_eq!(
            read_limited(chunks(2), 16).await.unwrap().unwrap().len(),
            16
        );
        assert!(read_limited(chunks(3), 16).await.unwrap().is_none());
        // 上限を超えた時点で止まり、後ろのチャンクは読まない (エラーも見ない)
        let stream = futures_util::stream::iter(vec![
            Ok(vec![b'x'; 32]),
            Err(std::io::Error::other("never read")),
        ]);
        assert!(read_limited(stream, 16).await.unwrap().is_none());
    }

    #[test]
    fn allowed_file_url_compares_as_url() {
        for base in ["https://files.slack.com", "https://files.slack.com/"] {
            assert!(allowed_file_url(
                "https://files.slack.com/files-pri/T1-F1/a.png",
                base
            ));
            assert!(!allowed_file_url(
                "https://files.slack.com.evil.example/files-pri/a.png",
                base
            ));
            assert!(!allowed_file_url(
                "https://evil.example/https://files.slack.com/",
                base
            ));
            assert!(!allowed_file_url(
                "http://files.slack.com/files-pri/a.png",
                base
            ));
            assert!(!allowed_file_url(
                "https://files.slack.com:8443/files-pri/a.png",
                base
            ));
            assert!(!allowed_file_url(
                "https://user@files.slack.com/files-pri/a.png",
                base
            ));
            assert!(!allowed_file_url("not a url", base));
        }
        // パスはディレクトリ単位で比べる (テストのモックなど)
        assert!(allowed_file_url(
            "http://127.0.0.1:9/files/a",
            "http://127.0.0.1:9/files"
        ));
        assert!(!allowed_file_url(
            "http://127.0.0.1:9/files-evil/a",
            "http://127.0.0.1:9/files"
        ));
    }

    #[test]
    fn remove_orphan_zips_keeps_zips_in_use() {
        let dir = std::env::temp_dir().join(format!("zoo-slack-tmp-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let orphan = dir.join(format!("{TEMP_ZIP_PREFIX}orphan{TEMP_ZIP_SUFFIX}"));
        std::fs::write(&orphan, b"left behind").unwrap();
        let other = dir.join("unrelated.zip");
        std::fs::write(&other, b"not ours").unwrap();
        let (in_use, _writer) = TempZip::create(&dir).unwrap();
        // Slack の過去ログを含むので、ほかのユーザーからは読めない
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&in_use.path)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        assert_eq!(remove_orphan_zips(&dir).unwrap(), 1);
        assert!(!orphan.exists());
        assert!(in_use.path.exists());
        assert!(other.exists());

        // 使い終わったファイルは破棄時に消える
        let path = in_use.path.clone();
        drop(in_use);
        assert!(!path.exists());
        assert_eq!(remove_orphan_zips(&dir.join("missing")).unwrap(), 0);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn zip_import_creator_lock_prevents_note_channel_deadlock() -> anyhow::Result<()> {
        dotenvy::dotenv().ok();
        dotenvy::from_filename("../.env").ok();
        let database_url = std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
        let mut config = crate::config::Config::from_env();
        config.database_url = database_url;
        let mut db = crate::db::init_db(&config.database_url).await?;
        let creator = uuid::Uuid::new_v4().to_string();
        let existing = uuid::Uuid::new_v4().to_string();
        let created = uuid::Uuid::new_v4().to_string();
        {
            let mut tx = db.transaction().await?;
            toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$2,$2)")
                .bind(creator.as_str())
                .bind(format!("slack_lock_{}", uuid::Uuid::new_v4().simple()))
                .exec(&mut tx)
                .await?;
            for channel in [&existing, &created] {
                toasty::sql::statement(
                    "INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,$2,$3,false,false)",
                )
                .bind(channel.as_str())
                .bind(format!("slack_lock_{}", uuid::Uuid::new_v4().simple()))
                .bind(creator.as_str())
                .exec(&mut tx)
                .await?;
            }
            toasty::sql::statement(
                "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
            )
            .bind(creator.as_str())
            .bind(existing.as_str())
            .exec(&mut tx)
            .await?;
            tx.commit().await?;
        }

        // Match the note write's users SHARE -> channel UPDATE lock order.
        let mut note_db = db.clone();
        let mut note_tx = note_db.transaction().await?;
        toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'")
            .exec(&mut note_tx)
            .await?;
        toasty::sql::query("SELECT id FROM users WHERE id=$1 FOR SHARE")
            .bind(creator.as_str())
            .exec(&mut note_tx)
            .await?;

        let import_db = db.clone();
        let import_creator = creator.clone();
        let import_existing = existing.clone();
        let import_created = created.clone();
        let mapping = Mapping {
            slack_channel_id: "new-slack-channel".into(),
            action: MappingAction::Create {
                name: "new-channel".into(),
                is_private: None,
                archive: false,
            },
        };
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let mut db = import_db;
            let mut tx = db.transaction().await?;
            toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'")
                .exec(&mut tx)
                .await?;
            let _ = started_tx.send(());
            lock_creator_for_channel_creation(&mut tx, &[mapping], &import_creator).await?;
            toasty::sql::statement("UPDATE channels SET imported_from_slack=true WHERE id=$1")
                .bind(import_existing.as_str())
                .exec(&mut tx)
                .await?;
            toasty::sql::statement(
                "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now()) ON CONFLICT DO NOTHING",
            )
            .bind(import_creator.as_str())
            .bind(import_created.as_str())
            .exec(&mut tx)
            .await?;
            tx.commit().await?;
            Ok::<_, AppError>(())
        });
        started_rx.await?;
        let mut waiting_for_creator = false;
        for _ in 0..100 {
            let mut monitor_db = db.clone();
            let mut monitor_tx = monitor_db.transaction().await?;
            let rows = query(
                &mut monitor_tx,
                "SELECT json_build_object('waiting',EXISTS(SELECT 1 FROM pg_stat_activity WHERE wait_event_type='Lock' AND query LIKE '%FROM users WHERE id = $1 FOR NO KEY UPDATE%'))::text",
                &[],
            )
            .await?;
            monitor_tx.commit().await?;
            if rows.first().is_some_and(|row| row["waiting"] == true) {
                waiting_for_creator = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            waiting_for_creator,
            "the worker did not wait for the creator row before updating channels"
        );
        toasty::sql::query("SELECT id FROM channels WHERE id=$1 FOR UPDATE")
            .bind(existing.as_str())
            .exec(&mut note_tx)
            .await?;
        note_tx.commit().await?;
        tokio::time::timeout(Duration::from_secs(5), worker).await???;

        let mut tx = db.transaction().await?;
        toasty::sql::statement("DELETE FROM channel_user WHERE user_id=$1")
            .bind(creator.as_str())
            .exec(&mut tx)
            .await?;
        for channel in [&existing, &created] {
            toasty::sql::statement("DELETE FROM channels WHERE id=$1")
                .bind(channel.as_str())
                .exec(&mut tx)
                .await?;
        }
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(creator.as_str())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    #[ignore = "requires an explicitly configured disposable SLACK_IMPORT_TEST_DATABASE_URL"]
    #[tokio::test]
    async fn zip_import_locks_members_before_plaintext_channel_check() -> anyhow::Result<()> {
        dotenvy::dotenv().ok();
        dotenvy::from_filename("../.env").ok();
        let database_url = std::env::var("SLACK_IMPORT_TEST_DATABASE_URL")?;
        let mut config = crate::config::Config::from_env();
        config.database_url = database_url;
        let mut db = crate::db::init_db(&config.database_url).await?;
        let member = uuid::Uuid::new_v4().to_string();
        let channel = uuid::Uuid::new_v4().to_string();
        {
            let mut tx = db.transaction().await?;
            toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$2,$2)")
                .bind(member.as_str())
                .bind(format!("slack_lock_{}", uuid::Uuid::new_v4().simple()))
                .exec(&mut tx)
                .await?;
            toasty::sql::statement(
                "INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,$2,$3,true,false)",
            )
            .bind(channel.as_str())
            .bind(format!("slack_lock_{}", uuid::Uuid::new_v4().simple()))
            .bind(member.as_str())
            .exec(&mut tx)
            .await?;
            toasty::sql::statement(
                "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
            )
            .bind(member.as_str())
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
            tx.commit().await?;
        }

        // A note writer takes users SHARE before channels SHARE. The ZIP path must wait for
        // users before acquiring the target channel, so the note writer can finish without a
        // users <-> channel deadlock.
        let mut note_db = db.clone();
        let mut note_tx = note_db.transaction().await?;
        toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'")
            .exec(&mut note_tx)
            .await?;
        toasty::sql::query("SELECT id FROM users WHERE id=$1 FOR SHARE")
            .bind(member.as_str())
            .exec(&mut note_tx)
            .await?;

        let import_db = db.clone();
        let import_member = member.clone();
        let import_channel = channel.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let importer = tokio::spawn(async move {
            let mut db = import_db;
            let mut tx = db.transaction().await?;
            toasty::sql::statement("SET LOCAL deadlock_timeout='100ms'")
                .exec(&mut tx)
                .await?;
            let _ = started_tx.send(());
            crate::routes::e2ee_channels::lock_membership_users(&mut tx, &[&import_member]).await?;
            ensure_plaintext_allowed(&mut tx, &import_channel).await?;
            tx.commit().await?;
            Ok::<_, AppError>(())
        });
        started_rx.await?;
        let mut waiting_for_member = false;
        for _ in 0..100 {
            let mut monitor_db = db.clone();
            let mut monitor_tx = monitor_db.transaction().await?;
            let rows = query(
                &mut monitor_tx,
                "SELECT json_build_object('waiting',EXISTS(SELECT 1 FROM pg_stat_activity WHERE wait_event_type='Lock' AND query LIKE '%FROM users WHERE id = $1 FOR NO KEY UPDATE%'))::text",
                &[],
            )
            .await?;
            monitor_tx.commit().await?;
            if rows.first().is_some_and(|row| row["waiting"] == true) {
                waiting_for_member = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(waiting_for_member, "ZIP import did not lock members first");
        toasty::sql::query("SELECT id FROM channels WHERE id=$1 FOR SHARE")
            .bind(channel.as_str())
            .exec(&mut note_tx)
            .await?;
        note_tx.commit().await?;
        tokio::time::timeout(Duration::from_secs(5), importer).await???;

        let mut tx = db.transaction().await?;
        toasty::sql::statement("DELETE FROM channel_user WHERE user_id=$1")
            .bind(member.as_str())
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("DELETE FROM channels WHERE id=$1")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(member.as_str())
            .exec(&mut tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}
