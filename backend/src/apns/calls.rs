use super::*;
use crate::push::CallInvitePush;

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
 JOIN channels c ON c.id=i.channel_id AND c.deleted_at IS NULL AND c.archive_at IS NULL AND NOT c.encrypted
 WHERE i.id=$1::uuid AND i.status='pending' AND i.expires_at>clock_timestamp()
   AND (NOT c.is_private AND NOT c.is_dm AND NOT u.is_guest OR EXISTS(
       SELECT 1 FROM channel_user WHERE user_id=u.id AND channel_id=c.id))
   AND NOT EXISTS(SELECT 1 FROM user_event_settings WHERE user_id=u.id AND dnd_until>now())";

async fn call_payload(
    db: &mut dyn toasty::db::Executor,
    sub: &ApnsSubscription,
    invite: &str,
) -> anyhow::Result<Option<Value>> {
    if !is_voip(sub) {
        return Ok(None);
    }
    let rows = toasty::sql::query(CALL_PAYLOAD_SQL)
        .bind(invite)
        .bind(sub.id.as_str())
        .bind(sub.revocation_hash.as_str())
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

pub(crate) fn dispatch_call_invites(mut db: toasty::Db, invites: Vec<CallInvitePush>) {
    tokio::spawn(async move {
        let result: anyhow::Result<()> = async {
            let configs = delivery_configs()?;
            if configs.is_empty() {
                return Ok(());
            }
            let client = client()?;
            for invite in invites {
                if deliver_call(&client, &configs, &mut db, &invite)
                    .await
                    .is_err()
                {
                    tracing::warn!("iPhoneへの着信を送信できませんでした");
                }
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            tracing::warn!("VoIP通知の設定を読み取れませんでした");
        }
    });
}

async fn deliver_call(
    client: &reqwest::Client,
    configs: &[(&str, ApnsConfig)],
    db: &mut toasty::Db,
    invite: &CallInvitePush,
) -> anyhow::Result<()> {
    // 既存のuser_id索引と登録時の20件制限を使い、全端末や全招待を走査しない。
    let subscriptions = ApnsSubscription::filter(
        ApnsSubscription::fields()
            .user_id()
            .eq(invite.invitee_id.as_str()),
    )
    .exec(db)
    .await?;
    for sub in subscriptions.into_iter().filter(is_voip) {
        let Some((_, config)) = configs.iter().find(|(name, _)| *name == sub.environment) else {
            continue;
        };
        let result: Result<DeliveryOutcome, AppError> = async {
            let request = client
                .post(endpoint(&sub.environment, &sub.device_token)?)
                .version(reqwest::Version::HTTP_2)
                .bearer_auth(config.authorization().await?)
                .header("apns-topic", "works.zli.zoo.voip")
                .header("apns-push-type", "voip")
                .header("apns-priority", "10")
                // オフライン中の着信を後から鳴らさない。取消通知はVoIPとして送らない。
                .header("apns-expiration", "0");
            send_if_allowed(request, call_payload(db, &sub, &invite.id)).await
        }
        .await;
        match result {
            Ok(DeliveryOutcome::Invalid(at)) => {
                if delete_invalid(db, &sub, at).await.is_err() {
                    tracing::warn!("失効したVoIP通知登録を削除できませんでした");
                }
            }
            Ok(_) => {}
            Err(_) => tracing::warn!("端末へVoIP通知を配信できませんでした"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(call_payload(&mut tx, &sub, &invite).await?.is_none());
        toasty::sql::statement(
            "INSERT INTO channel_user(user_id,channel_id,joined_at) VALUES($1,$2,now())",
        )
        .bind(user.as_str())
        .bind(channel.as_str())
        .exec(&mut tx)
        .await?;
        let payload = call_payload(&mut tx, &sub, &invite).await?.unwrap();
        assert_eq!(payload["user_id"], user);
        assert_eq!(payload["invite_id"], invite);
        assert_eq!(payload["caller_name"], "発信者");
        assert_eq!(payload["type"], "call_invite");
        assert!(payload["expires_at"].as_str().unwrap().ends_with('Z'));
        let mut wrong = sub.clone();
        wrong.revocation_hash = digest("別の解除情報");
        assert!(call_payload(&mut tx, &wrong, &invite).await?.is_none());
        wrong = sub.clone();
        wrong.id = PushKind::Alert.registration_id(&wrong.environment, &wrong.device_token);
        assert!(call_payload(&mut tx, &wrong, &invite).await?.is_none());
        for status in ["accepted", "declined", "cancelled", "missed"] {
            toasty::sql::statement("UPDATE call_invites SET status=$2 WHERE id=$1::uuid")
                .bind(invite.as_str())
                .bind(status)
                .exec(&mut tx)
                .await?;
            assert!(call_payload(&mut tx, &sub, &invite).await?.is_none());
        }
        toasty::sql::statement("UPDATE call_invites SET status='pending', expires_at=now()-interval '1 second' WHERE id=$1::uuid")
            .bind(invite.as_str()).exec(&mut tx).await?;
        assert!(call_payload(&mut tx, &sub, &invite).await?.is_none());
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
        assert!(call_payload(&mut tx, &sub, &invite).await?.is_none());
        tx.rollback().await?;
        Ok(())
    }
}
