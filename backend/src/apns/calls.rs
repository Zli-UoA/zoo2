use super::*;
use crate::push::CallInvitePush;
use futures_util::{StreamExt, stream};

#[cfg(test)]
#[path = "calls_budget_test.rs"]
mod budget_tests;

// 招待、端末、利用者、チャンネルは各主キー1行、会員資格・DNDも一意キー各1行に限定する。
const CALL_PAYLOAD_SQL: &str = "WITH device AS MATERIALIZED (
 SELECT user_id, revocation_hash FROM apns_subscriptions WHERE id=$2
) SELECT json_build_object(
    'type', 'call_invite', 'invite_id', i.id::text, 'channel_id', i.channel_id,
    'user_id', i.invitee_id, 'inviter_id', i.inviter_id,
    'caller_name', left(COALESCE(NULLIF(actor.display_name,''),actor.user_name),120),
    'expires_at', to_char(i.expires_at AT TIME ZONE 'UTC','YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"'),
    'aps', json_build_object('content-available',1))::text
 FROM call_invites i
 JOIN device s ON s.user_id=i.invitee_id AND s.revocation_hash=$3
 JOIN users u ON u.id=i.invitee_id AND NOT u.is_deactivated
 JOIN users actor ON actor.id=i.inviter_id AND NOT actor.is_deactivated
 JOIN channels c ON c.id=i.channel_id AND c.deleted_at IS NULL AND c.archive_at IS NULL
 WHERE i.id=$1::uuid AND i.status='pending' AND i.expires_at>clock_timestamp()
   AND (NOT c.is_private AND NOT c.is_dm AND NOT u.is_guest OR EXISTS(
       SELECT 1 FROM channel_user WHERE user_id=u.id AND channel_id=c.id))
   AND NOT EXISTS(SELECT 1 FROM user_event_settings WHERE user_id=u.id AND dnd_until>now())
   AND (NOT $4::bool OR NOT EXISTS(
       SELECT 1 FROM call_rooms r WHERE r.channel_id=c.id AND r.mode='sfu'))";

async fn call_payload(
    db: &mut dyn toasty::db::Executor,
    sub: &ApnsSubscription,
    invite: &str,
    sfu_enabled: bool,
) -> anyhow::Result<Option<Value>> {
    if !is_voip(sub) {
        return Ok(None);
    }
    let rows = toasty::sql::query(CALL_PAYLOAD_SQL)
        .bind(invite)
        .bind(sub.id.as_str())
        .bind(sub.revocation_hash.as_str())
        .bind(sfu_enabled)
        .exec(db)
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let toasty::stmt::Value::String(body) = &row.as_record_unwrap()[0] else {
        anyhow::bail!("着信の内容を読み取れませんでした");
    };
    Ok(Some(serde_json::from_str(body)?))
}

/// iOS への着信（VoIP）を送る。`sfu_enabled` のとき、SFU モードの部屋への招待は送らない。
///
/// iOS は SFU に対応しておらず、SFU の部屋には入れない（409 `CALL_SFU_UNSUPPORTED`、docs/calls-sfu.md）。
/// 着信を鳴らしても応答できないので、iOS の端末には知らせない（Web・Android の通知は今までどおり送る）。
/// 部屋のモードは、端末ごとの送信直前の照会（`CALL_PAYLOAD_SQL` の `$4`）で見る。照会の失敗は、APNs の
/// 失敗と分けて記録し（`deliver_device`）、その端末には送らず、ほかの宛先の配信は止めない。
pub(crate) fn dispatch_call_invites(
    db: toasty::Db,
    invites: Vec<CallInvitePush>,
    sfu_enabled: bool,
) {
    tokio::spawn(async move {
        let result: anyhow::Result<()> = async {
            let configs = delivery_configs()?;
            if configs.is_empty() {
                return Ok(());
            }
            let client = client()?;
            deliver_calls(db, invites, |db, sub, invite| {
                deliver_device(&client, &configs, db, sub, invite, sfu_enabled)
            })
            .await;
            Ok(())
        }
        .await;
        if result.is_err() {
            tracing::warn!("VoIP通知の設定を読み取れませんでした");
        }
    });
}

async fn deliver_calls<F, Fut>(db: toasty::Db, invites: Vec<CallInvitePush>, send: F)
where
    F: Fn(toasty::Db, ApnsSubscription, String) -> Fut + Clone,
    Fut: Future<Output = ()>,
{
    // 1回の招待は最大5人・各20登録。遅い端末が他の宛先の30秒期限を消費しないようにする。
    stream::iter(invites)
        .for_each_concurrent(crate::routes::calls::MAX_CALL_PARTICIPANTS - 1, |invite| {
            let mut db = db.clone();
            let send = send.clone();
            async move {
                let subscriptions = ApnsSubscription::filter(
                    ApnsSubscription::fields()
                        .user_id()
                        .eq(invite.invitee_id.as_str()),
                )
                .exec(&mut db)
                .await;
                let Ok(subscriptions) = subscriptions else {
                    tracing::warn!("iPhoneの着信先を取得できませんでした");
                    return;
                };
                stream::iter(subscriptions.into_iter().filter(is_voip))
                    .for_each_concurrent(20, |sub| send(db.clone(), sub, invite.id.clone()))
                    .await;
            }
        })
        .await;
}

async fn deliver_device(
    client: &reqwest::Client,
    configs: &[(&str, ApnsConfig)],
    mut db: toasty::Db,
    sub: ApnsSubscription,
    invite: String,
    sfu_enabled: bool,
) {
    let Some((_, config)) = configs.iter().find(|(name, _)| *name == sub.environment) else {
        return;
    };
    let result: Result<DeliveryOutcome, AppError> = async {
        let request = call_request(
            client,
            &sub.environment,
            &sub.device_token,
            &config.authorization().await?,
        )?;
        // 照会の失敗は、APNs への送信の失敗と分けて記録する。
        let payload = async {
            let payload = call_payload(&mut db, &sub, &invite, sfu_enabled).await;
            if let Err(e) = &payload {
                tracing::warn!(error = ?e, "VoIP通知の着信の内容をDBから照会できませんでした");
            }
            payload
        };
        send_if_allowed(request, payload).await
    }
    .await;
    match result {
        Ok(DeliveryOutcome::Invalid(at)) => {
            if delete_invalid(&mut db, &sub, at).await.is_err() {
                tracing::warn!("失効したVoIP通知登録を削除できませんでした");
            }
        }
        Ok(_) => {}
        Err(_) => tracing::warn!("端末へVoIP通知を配信できませんでした"),
    }
}

fn call_request(
    client: &reqwest::Client,
    environment: &str,
    token: &str,
    authorization: &str,
) -> anyhow::Result<reqwest::RequestBuilder> {
    Ok(client
        .post(endpoint(environment, token)?)
        .version(reqwest::Version::HTTP_2)
        .bearer_auth(authorization)
        .header("apns-topic", "works.zli.zoo.voip")
        .header("apns-push-type", "voip")
        .header("apns-priority", "10")
        // オフライン中の着信を後から鳴らさない。取消通知はVoIPとして送らない。
        .header("apns-expiration", "0"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn voip_request_preserves_delivery_contract() -> anyhow::Result<()> {
        let client = client()?;
        for (environment, host) in [
            ("sandbox", "api.sandbox.push.apple.com"),
            ("production", "api.push.apple.com"),
        ] {
            let request =
                call_request(&client, environment, "aabb", "dummy-authorization")?.build()?;
            assert_eq!(request.method(), reqwest::Method::POST);
            assert_eq!(request.version(), reqwest::Version::HTTP_2);
            assert_eq!(
                request.url().as_str(),
                format!("https://{host}/3/device/aabb")
            );
            for (name, value) in [
                ("apns-topic", "works.zli.zoo.voip"),
                ("apns-push-type", "voip"),
                ("apns-priority", "10"),
                ("apns-expiration", "0"),
                ("authorization", "Bearer dummy-authorization"),
            ] {
                assert_eq!(request.headers()[name], value);
            }
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "検証用PostgreSQLで遅い端末が他の着信を止めないことを確認"]
    async fn slow_devices_do_not_block_other_invitees() -> anyhow::Result<()> {
        let mut db = crate::db::init_db(&crate::config::Config::from_env().database_url).await?;
        let users = [
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        ];
        let mut tx = db.transaction().await?;
        for user in &users {
            toasty::sql::statement(
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'着信テスト')",
            )
            .bind(user.as_str())
            .exec(&mut tx)
            .await?;
            for _ in 0..2 {
                let token = digest(&uuid::Uuid::new_v4().to_string());
                ApnsSubscription::create()
                    .id(PushKind::Voip.registration_id("sandbox", &token))
                    .user_id(user.as_str())
                    .device_token(token)
                    .environment("sandbox")
                    .revocation_hash(digest(user))
                    .created_at(crate::models::now_utc_millis())
                    .exec(&mut tx)
                    .await?;
            }
        }
        tx.commit().await?;
        let invites = users
            .iter()
            .map(|user| CallInvitePush {
                id: uuid::Uuid::new_v4().to_string(),
                invitee_id: user.clone(),
                channel_id: String::new(),
                inviter_id: String::new(),
            })
            .collect();
        let (started, mut received) = tokio::sync::mpsc::channel(4);
        let release = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let delivery = deliver_calls(db.clone(), invites, |_, sub, _| {
            let started = started.clone();
            let release = release.clone();
            async move {
                started.send(sub.user_id).await.unwrap();
                release.acquire().await.unwrap().forget();
            }
        });
        let observe = async {
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                let mut targets = Vec::new();
                for _ in 0..4 {
                    targets.push(received.recv().await.unwrap());
                }
                for user in &users {
                    anyhow::ensure!(targets.iter().filter(|id| *id == user).count() == 2);
                }
                Ok::<_, anyhow::Error>(())
            })
            .await;
            // 失敗時も待機中の配信を解放し、テストの登録を回収できるようにする。
            release.add_permits(4);
            result?
        };
        let (_, result) = tokio::join!(delivery, observe);
        for user in &users {
            toasty::sql::statement("DELETE FROM users WHERE id=$1")
                .bind(user.as_str())
                .exec(&mut db)
                .await?;
        }
        result
    }

    #[test]
    fn registration_kind_preserves_alert_identity() {
        assert_eq!(
            PushKind::Alert.registration_id("sandbox", "aabb"),
            digest("sandbox:aabb")
        );
        assert_eq!(
            PushKind::Voip.registration_id("sandbox", "aabb"),
            digest("voip:sandbox:aabb")
        );
        assert_ne!(
            PushKind::Alert.registration_id("sandbox", "aabb"),
            PushKind::Voip.registration_id("sandbox", "aabb")
        );
    }

    #[tokio::test]
    #[ignore = "検証用PostgreSQLで着信の認可・取消・期限を確認"]
    async fn call_delivery_rechecks_invitation_and_device() -> anyhow::Result<()> {
        let mut db = crate::db::init_db(&crate::config::Config::from_env().database_url).await?;
        let mut tx = db.transaction().await?;
        let user = uuid::Uuid::new_v4().to_string();
        let actor = uuid::Uuid::new_v4().to_string();
        let channel = uuid::Uuid::new_v4().to_string();
        let invite = uuid::Uuid::new_v4().to_string();
        for id in [&user, &actor] {
            toasty::sql::statement(
                "INSERT INTO users(id,user_name,display_name) VALUES($1,$1,'発信者')",
            )
            .bind(id.as_str())
            .exec(&mut tx)
            .await?;
        }
        toasty::sql::statement("INSERT INTO channels(id,channel_name,created_user_id,is_private,is_dm) VALUES($1,'通話',$2,true,true)")
            .bind(channel.as_str()).bind(actor.as_str()).exec(&mut tx).await?;
        toasty::sql::statement("INSERT INTO call_invites(id,channel_id,inviter_id,invitee_id,expires_at) VALUES($1::uuid,$2,$3,$4,now()+interval '30 seconds')")
            .bind(invite.as_str()).bind(channel.as_str()).bind(actor.as_str()).bind(user.as_str()).exec(&mut tx).await?;
        let token = digest(&user);
        let sub = ApnsSubscription::create()
            .id(PushKind::Voip.registration_id("sandbox", &token))
            .user_id(user.as_str())
            .device_token(token)
            .environment("sandbox")
            .revocation_hash(digest(&actor))
            .created_at(crate::models::now_utc_millis())
            .exec(&mut tx)
            .await?;
        assert!(call_payload(&mut tx, &sub, &invite, false).await?.is_none());
        toasty::sql::statement(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
        )
        .bind(user.as_str())
        .bind(channel.as_str())
        .exec(&mut tx)
        .await?;
        let payload = call_payload(&mut tx, &sub, &invite, false).await?.unwrap();
        assert_eq!(payload["user_id"], user);
        assert_eq!(payload["invite_id"], invite);
        assert_eq!(payload["caller_name"], "発信者");
        assert_eq!(payload["type"], "call_invite");
        assert!(payload["expires_at"].as_str().unwrap().ends_with('Z'));
        // SFU モードの部屋への招待は、SFU が有効なときだけ iOS に送らない。
        toasty::sql::statement("INSERT INTO call_rooms(channel_id,mode) VALUES($1,'sfu')")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        assert!(call_payload(&mut tx, &sub, &invite, true).await?.is_none());
        assert!(call_payload(&mut tx, &sub, &invite, false).await?.is_some());
        toasty::sql::statement("UPDATE call_rooms SET mode='mesh' WHERE channel_id=$1")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        assert!(call_payload(&mut tx, &sub, &invite, true).await?.is_some());
        // チャンネルの暗号化はメッセージ本文の設定であり、通話招待を抑制しない。
        toasty::sql::statement("UPDATE channels SET encrypted=true WHERE id=$1")
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        assert_eq!(
            call_payload(&mut tx, &sub, &invite, false).await?,
            Some(payload.clone())
        );
        toasty::sql::statement("DELETE FROM channel_user WHERE user_id=$1 AND channel_id=$2")
            .bind(user.as_str())
            .bind(channel.as_str())
            .exec(&mut tx)
            .await?;
        assert!(call_payload(&mut tx, &sub, &invite, false).await?.is_none());
        toasty::sql::statement(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
        )
        .bind(user.as_str())
        .bind(channel.as_str())
        .exec(&mut tx)
        .await?;
        let mut wrong = sub.clone();
        wrong.revocation_hash = digest("別の解除情報");
        assert!(
            call_payload(&mut tx, &wrong, &invite, false)
                .await?
                .is_none()
        );
        wrong = sub.clone();
        wrong.id = PushKind::Alert.registration_id(&wrong.environment, &wrong.device_token);
        assert!(
            call_payload(&mut tx, &wrong, &invite, false)
                .await?
                .is_none()
        );
        for status in ["accepted", "declined", "cancelled", "missed"] {
            toasty::sql::statement("UPDATE call_invites SET status=$2 WHERE id=$1::uuid")
                .bind(invite.as_str())
                .bind(status)
                .exec(&mut tx)
                .await?;
            assert!(call_payload(&mut tx, &sub, &invite, false).await?.is_none());
        }
        toasty::sql::statement("UPDATE call_invites SET status='pending', expires_at=now()-interval '1 second' WHERE id=$1::uuid")
            .bind(invite.as_str()).exec(&mut tx).await?;
        assert!(call_payload(&mut tx, &sub, &invite, false).await?.is_none());
        toasty::sql::statement(
            "UPDATE call_invites SET expires_at=now()+interval '30 seconds' WHERE id=$1::uuid",
        )
        .bind(invite.as_str())
        .exec(&mut tx)
        .await?;
        toasty::sql::statement(
            "INSERT INTO user_event_settings(user_id,dnd_until) VALUES($1,now()+interval '1 hour')",
        )
        .bind(user.as_str())
        .exec(&mut tx)
        .await?;
        assert!(call_payload(&mut tx, &sub, &invite, false).await?.is_none());
        tx.rollback().await?;
        Ok(())
    }
}
