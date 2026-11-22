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
    pub fn new(config: CallSfuConfig, http: reqwest::Client) -> Self {
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
    pub fn start_departure_worker(&self, db: toasty::Db) {
        let sfu = self.clone();
        tokio::spawn(async move {
            loop {
                let mut db = db.clone();
                match drain_departures(&sfu, &mut db).await {
                    Ok(n) if n > 0 => continue,
                    Ok(_) => {}
                    Err(e) => tracing::warn!("Failed to close departed SFU sessions: {e}"),
                }
                tokio::select! {
                    _ = sfu.departures_wake.notified() => {}
                    _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                }
            }
        });
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
    attempts: i32,
}

/// 何度試しても閉じられないもの（相手側でもう消えたものなど）は、この回数で諦める。
const MAX_DEPARTURE_ATTEMPTS: i32 = 5;
const DEPARTURE_BATCH: i64 = 50;

/// 退出した人の SFU の音声を止める。
///
/// 探索：`call_sfu_departures` の主キー順に最大 `DEPARTURE_BATCH` 行。行は処理したら消すので、
/// 表の大きさは「まだ閉じていない退出」の数に比例し、履歴は残らない。
async fn drain_departures(sfu: &CallSfu, db: &mut toasty::Db) -> Result<usize, AppError> {
    let mut tx = db.transaction().await?;
    let rows = toasty::sql::query(format!(
        "SELECT json_build_object(
             'id', id, 'session_id', session_id, 'sfu_id', sfu_id, 'sfu_room', sfu_room,
             'cf_session_id', cf_session_id, 'attempts', attempts
         )::text
         FROM call_sfu_departures
         ORDER BY id
         LIMIT {DEPARTURE_BATCH}
         FOR UPDATE SKIP LOCKED"
    ))
    .exec(&mut tx)
    .await?;
    let departures: Vec<Departure> = rows_json(rows);
    if departures.is_empty() {
        tx.commit().await?;
        return Ok(0);
    }

    let mut done = Vec::new();
    let mut retry = Vec::new();
    for d in &departures {
        let result = close_departed(sfu, d).await;
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
        .exec(&mut tx)
        .await?;
    }
    if !retry.is_empty() {
        toasty::sql::statement(
            "UPDATE call_sfu_departures SET attempts = attempts + 1
             WHERE id = ANY(string_to_array($1, ',')::bigint[])",
        )
        .bind(join_ids(&retry))
        .exec(&mut tx)
        .await?;
    }
    tx.commit().await?;
    // 失敗だけが残ったときは、すぐに繰り返さない。
    Ok(done.len())
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
        && let (Some(lk), Some(room)) = (&sfu.livekit, &d.sfu_room)
    {
        lk.remove_participant(room, &d.session_id.to_string())
            .await?;
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
