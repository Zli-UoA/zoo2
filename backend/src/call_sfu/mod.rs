//! 通話の SFU（docs/calls-sfu.md）。
//!
//! 少人数の通話は今までどおり P2P mesh。人数が `threshold` を超え、在室の全員が SFU に対応していれば、
//! 部屋（`call_rooms`）を SFU モードに切り替える。参加者ごとに、LAN の中なら LiveKit、それ以外は
//! Cloudflare Realtime に割り当てる。両方に人がいる部屋は sfu-bridge が中継する。

pub mod cloudflare;
pub mod config;
pub mod livekit;

use std::{net::IpAddr, sync::Arc, time::Duration};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::sync::Notify;
use uuid::Uuid;

pub use config::CallSfuConfig;

use crate::error::AppError;

pub const SFU_LIVEKIT: &str = "livekit";
pub const SFU_CLOUDFLARE: &str = "cloudflare";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomMode {
    Mesh,
    Sfu,
}

impl RoomMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mesh => "mesh",
            Self::Sfu => "sfu",
        }
    }
}

/// SFU の機能。`AppState` に 1 つ置く。設定がなければ `None` で、mesh だけで動く。
#[derive(Clone)]
pub struct CallSfu {
    pub config: Arc<CallSfuConfig>,
    pub cloudflare: cloudflare::CloudflareRealtime,
    pub livekit: Option<livekit::LiveKit>,
    /// sfu-bridge の long-poll を起こす。
    pub bridge_wake: Arc<Notify>,
    /// 退出の後始末の worker を起こす。
    pub departures_wake: Arc<Notify>,
}

impl CallSfu {
    /// SFU の API は Bearer を付けて呼ぶので、リダイレクトを追わない専用のクライアントを使う
    /// （送り先の検査は `config::require_secure_api_url`）。
    pub fn new(config: CallSfuConfig) -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("reqwest client");
        let cloudflare =
            cloudflare::CloudflareRealtime::new(http.clone(), config.cloudflare.clone());
        let livekit = config
            .livekit
            .clone()
            .map(|lk| livekit::LiveKit::new(http, lk));
        Self {
            config: Arc::new(config),
            cloudflare,
            livekit,
            bridge_wake: Arc::new(Notify::new()),
            departures_wake: Arc::new(Notify::new()),
        }
    }

    pub fn bridge_enabled(&self) -> bool {
        self.config.bridge_token.is_some() && self.livekit.is_some()
    }

    /// 部屋の定員。
    pub fn capacity(&self, mode: RoomMode) -> usize {
        capacity(Some(self), mode)
    }

    /// LAN の中から来た人か（LiveKit に届く見込みがあるか）。
    pub fn is_lan_client(&self, ip: Option<IpAddr>) -> bool {
        match (&self.livekit, ip) {
            (Some(lk), Some(ip)) => lk.config().lan_egress.iter().any(|c| c.contains(ip)),
            _ => false,
        }
    }

    /// 退出の後始末の worker を起動する。
    ///
    /// SFU ごとに、確保と処理のループを分ける。片方の API が遅くても（Cloudflare の close が応答待ちのまま、
    /// など）、もう片方の行の確保と処理は待たない。LiveKit の設定がないときは、LiveKit のループを起動しない
    /// （行は消さずに残り、設定を戻せば処理する）。
    pub fn start_departure_worker(&self, db: toasty::Db) {
        let mut wakes = vec![self.spawn_departure_loop(db.clone(), SFU_CLOUDFLARE)];
        if self.livekit.is_some() {
            wakes.push(self.spawn_departure_loop(db, SFU_LIVEKIT));
        }
        // `departures_wake` の 1 回の通知で、すべてのループを起こす。
        let wake = self.departures_wake.clone();
        tokio::spawn(async move {
            loop {
                wake.notified().await;
                for w in &wakes {
                    w.notify_one();
                }
            }
        });
    }

    fn spawn_departure_loop(&self, db: toasty::Db, sfu_id: &'static str) -> Arc<Notify> {
        let wake = Arc::new(Notify::new());
        let sfu = self.clone();
        let woken = wake.clone();
        tokio::spawn(async move {
            loop {
                let mut db = db.clone();
                match drain_departures(&sfu, &mut db, sfu_id).await {
                    Ok(n) if n > 0 => continue,
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(sfu_id, "Failed to close departed SFU sessions: {e}")
                    }
                }
                tokio::select! {
                    _ = woken.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                }
            }
        });
        wake
    }
}

/// 部屋の定員。SFU が無効なら mesh の定員だけ。
pub fn capacity(sfu: Option<&CallSfu>, mode: RoomMode) -> usize {
    match (sfu, mode) {
        (Some(sfu), RoomMode::Sfu) => sfu.config.max_participants,
        _ => crate::routes::calls::MAX_CALL_PARTICIPANTS,
    }
}

/// どの SFU に割り当てるかを決める。
///
/// - `fallback` は、LiveKit に割り当てたが ICE がつながらなかったクライアントの再試行。
/// - bridge がないときは、部屋に Cloudflare の人が 1 人でもいれば全員を Cloudflare にする
///   （LiveKit と Cloudflare に分かれると、お互いの声が届かないため）。
pub fn choose_sfu(
    lan_client: bool,
    livekit_enabled: bool,
    bridge_enabled: bool,
    fallback: bool,
    room_has_cloudflare: bool,
) -> &'static str {
    if !livekit_enabled || !lan_client || fallback {
        return SFU_CLOUDFLARE;
    }
    if !bridge_enabled && room_has_cloudflare {
        return SFU_CLOUDFLARE;
    }
    SFU_LIVEKIT
}

/// `SELECT json_build_object(...)::text` の結果を読む。
pub fn rows_json<T: DeserializeOwned>(rows: Vec<toasty::stmt::Value>) -> Vec<T> {
    rows.into_iter()
        .filter_map(|row| {
            if let toasty::stmt::Value::Record(rec) = row
                && let Some(toasty::stmt::Value::String(json)) = rec.fields.into_iter().next()
            {
                serde_json::from_str(&json).ok()
            } else {
                None
            }
        })
        .collect()
}

#[derive(Debug, Clone, Deserialize)]
struct Departure {
    id: i64,
    session_id: Uuid,
    sfu_id: String,
    sfu_room: Option<String>,
    cf_session_id: Option<String>,
    /// その割り当ての LiveKit の identity（`livekit::participant_identity`）。
    livekit_identity: Option<String>,
    attempts: i32,
}

/// 何度試しても閉じられないもの（相手側でもう消えたものなど）は、この回数で諦める。
const MAX_DEPARTURE_ATTEMPTS: i32 = 5;
const DEPARTURE_BATCH: i64 = 50;
/// SFU ごとに同時に出す要求の数。
const DEPARTURE_CONCURRENCY: usize = 8;
/// 確保の期限。worker が途中で止まっても、この時間の後にほかの worker（再起動後を含む）が取り直す。
/// 1 件は最長で約 20 秒（Cloudflare は照会と close の 2 回、各 10 秒）なので、
/// `DEPARTURE_BATCH / DEPARTURE_CONCURRENCY` 回分（約 140 秒）より長くする。
const DEPARTURE_CLAIM_SECS: i64 = 300;
/// 失敗した行を、次に試すまでの間隔。
const DEPARTURE_RETRY_SECS: i64 = 15;

/// テスト用：退出の後始末の worker が、行を確保しないようにする。本番では使わない。
/// worker は確保の文のあいだ読み取りのロックを持ち、止める側は書き込みのロックで値を変える。
static DEPARTURES_PAUSED: tokio::sync::RwLock<bool> = tokio::sync::RwLock::const_new(false);

/// テスト用：`true` のあいだ、退出の後始末の worker は新しく行を確保しない（すでに確保した行は処理する）。
/// worker が処理して消すと見えなくなる行（誤って記録した後始末）を、テストが確かめるために使う。
///
/// 戻ったときには、確保の文を実行中の worker はいない（書き込みのロックは、実行中の確保が終わるまで待つ）。
/// そのあとに始まる確保は、止めたことを必ず見る。
#[doc(hidden)]
pub async fn pause_departure_worker_for_test(paused: bool) {
    *DEPARTURES_PAUSED.write().await = paused;
}

/// `sfu_id` の SFU に割り当てていた人の、退出の後始末をする。
///
/// 1. 短い 1 文で、その SFU の行を確保する（`claimed_until` を先に延ばす）。行ロックは、この文の中でだけ持つ。
/// 2. ロックもトランザクションも持たずに、同時数を限って API を呼ぶ。
/// 3. 終わった行は消し、失敗した行は `attempts` を増やして、`DEPARTURE_RETRY_SECS` 後まで確保を残す。
///
/// 探索：その SFU の、確保の済んでいない行を id 順に最大 `DEPARTURE_BATCH` 行。読む行数の上限は、
/// `DEPARTURE_BATCH` ＋ その SFU の確保中・再試行待ちの行 ＋ ほかの worker が行ロック中で読み飛ばす行
/// （SKIP LOCKED。worker の数 × `DEPARTURE_BATCH` 以下）＋ 主キーで読む計画のときは、それより古い
/// ほかの SFU の行。この SFU の行が少ないときは idx_call_sfu_departures_sfu (sfu_id, id) を使い、ほかの SFU の
/// 行は読まない。この SFU の行が多いときは主キー順の計画になることがあり、そのときは先に並ぶほかの SFU の
/// 行を除外しながら読む（その数は、ほかの SFU のまだ閉じていない退出の数以下）。行は処理したら消すので、
/// 表の大きさは「まだ閉じていない退出」の数に比例し、履歴は残らない。
async fn drain_departures(
    sfu: &CallSfu,
    db: &mut toasty::Db,
    sfu_id: &str,
) -> Result<usize, AppError> {
    // テスト用の一時停止。確保の文が終わるまで読み取りのロックを持つ。
    let paused = DEPARTURES_PAUSED.read().await;
    if *paused {
        return Ok(0);
    }
    // sfu_id は、パラメーターでなく定数として SQL に書く。パラメーターだと、汎用の計画（generic plan）が
    // idx_call_sfu_departures_sfu を使わず、主キー順にほかの SFU の行まで読むことがある（30 万件の偏りで
    // 30 万行を除外）。値は 2 つの定数のどちらかなので、文字列を組み立てても注入はない。
    let sfu_literal = match sfu_id {
        SFU_LIVEKIT => SFU_LIVEKIT,
        _ => SFU_CLOUDFLARE,
    };
    let rows = toasty::sql::query(format!(
        "WITH picked AS (
             SELECT id FROM call_sfu_departures
             WHERE sfu_id = '{sfu_literal}'
               AND (claimed_until IS NULL OR claimed_until < statement_timestamp())
             ORDER BY id
             LIMIT {DEPARTURE_BATCH}
             FOR UPDATE SKIP LOCKED
         )
         UPDATE call_sfu_departures d
         SET claimed_until = statement_timestamp() + interval '{DEPARTURE_CLAIM_SECS} seconds'
         FROM picked
         WHERE d.id = picked.id
         RETURNING json_build_object(
             'id', d.id, 'session_id', d.session_id, 'sfu_id', d.sfu_id, 'sfu_room', d.sfu_room,
             'cf_session_id', d.cf_session_id, 'livekit_identity', d.livekit_identity,
             'attempts', d.attempts
         )::text"
    ))
    .exec(&mut *db)
    .await?;
    drop(paused);
    let departures: Vec<Departure> = rows_json(rows);
    if departures.is_empty() {
        return Ok(0);
    }

    let processed = departures.len();
    let results = close_all(sfu, departures).await;

    let mut done = Vec::new();
    let mut retry = Vec::new();
    for (d, result) in results {
        match result {
            Ok(()) => done.push(d.id),
            Err(e) if d.attempts + 1 >= MAX_DEPARTURE_ATTEMPTS => {
                tracing::warn!(session_id = %d.session_id, "Giving up closing SFU session: {e}");
                done.push(d.id);
            }
            Err(e) => {
                tracing::debug!(session_id = %d.session_id, "Retrying SFU close later: {e}");
                retry.push(d.id);
            }
        }
    }
    if !done.is_empty() {
        toasty::sql::statement(
            "DELETE FROM call_sfu_departures WHERE id = ANY(string_to_array($1, ',')::bigint[])",
        )
        .bind(join_ids(&done))
        .exec(&mut *db)
        .await?;
    }
    if !retry.is_empty() {
        toasty::sql::statement(format!(
            "UPDATE call_sfu_departures
             SET attempts = attempts + 1,
                 claimed_until = statement_timestamp() + interval '{DEPARTURE_RETRY_SECS} seconds'
             WHERE id = ANY(string_to_array($1, ',')::bigint[])"
        ))
        .bind(join_ids(&retry))
        .exec(&mut *db)
        .await?;
    }
    // 処理した件数を返す。全件が失敗しても、続けて次の未確保の行を確保する。失敗した行は
    // `claimed_until` を先にずらしたので、次の確保では選ばれず、失敗だけが残ったときはすぐに繰り返さない。
    Ok(processed)
}

async fn close_all(
    sfu: &CallSfu,
    departures: Vec<Departure>,
) -> Vec<(Departure, Result<(), AppError>)> {
    use futures_util::StreamExt;
    let sfu = sfu.clone();
    futures_util::stream::iter(departures)
        .map(move |d| {
            let sfu = sfu.clone();
            async move {
                let result = close_departed(&sfu, &d).await;
                (d, result)
            }
        })
        .buffer_unordered(DEPARTURE_CONCURRENCY)
        .collect()
        .await
}

fn join_ids(ids: &[i64]) -> String {
    ids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

async fn close_departed(sfu: &CallSfu, d: &Departure) -> Result<(), AppError> {
    if let Some(cf) = &d.cf_session_id {
        sfu.cloudflare.close_session(cf).await?;
    }
    if d.sfu_id == SFU_LIVEKIT
        && let Some(room) = &d.sfu_room
    {
        // LiveKit の行は、LiveKit の設定があるときだけ確保する（`start_departure_worker`）。
        let Some(lk) = &sfu.livekit else {
            return Err(AppError::Internal(anyhow::anyhow!(
                "LiveKit is not configured"
            )));
        };
        // 外すのは、その割り当ての identity だけ。identity は割り当てごとに新しいので、同じ session_id で
        // 入り直した人（新しい identity）の接続は、この要求が遅れて届いても切れない。ロックは持たない。
        // identity のない行は、identity が session_id だったころの割り当て。
        let session_identity = d.session_id.to_string();
        let identity = d.livekit_identity.as_deref().unwrap_or(&session_identity);
        lk.remove_participant(room, identity).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_clients_use_livekit() {
        assert_eq!(choose_sfu(true, true, true, false, true), SFU_LIVEKIT);
        assert_eq!(choose_sfu(true, true, false, false, false), SFU_LIVEKIT);
    }

    #[test]
    fn outside_or_unknown_clients_use_cloudflare() {
        assert_eq!(choose_sfu(false, true, true, false, false), SFU_CLOUDFLARE);
    }

    #[test]
    fn fallback_moves_to_cloudflare() {
        assert_eq!(choose_sfu(true, true, true, true, false), SFU_CLOUDFLARE);
    }

    #[test]
    fn without_livekit_everyone_uses_cloudflare() {
        assert_eq!(choose_sfu(true, false, false, false, false), SFU_CLOUDFLARE);
    }

    #[test]
    fn without_bridge_a_mixed_room_stays_on_cloudflare() {
        assert_eq!(choose_sfu(true, true, false, false, true), SFU_CLOUDFLARE);
    }

    #[test]
    fn capacity_by_mode() {
        assert_eq!(
            capacity(None, RoomMode::Sfu),
            crate::routes::calls::MAX_CALL_PARTICIPANTS
        );
    }
}
