use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};
use uuid::Uuid;

use crate::{
    error::AppError,
    models::{Channel, ChannelUser, Message, Notification, User, now_utc_millis},
    routes::e2ee_channels::Mentions,
};

/// API 全体の内訳計測（docs/query-design.md）。`notify_timing` 目標の低コスト event として、
/// 各 SQL の「往復を含む実行時間」をメッセージ ID 単位で残す。SQL の計画だけは toasty
/// ドライバから分離できないため、ここでは往復込みの時間として記録する。
/// `debug!` にして既定フィルターでは出力しない。計測するときだけ `notify_timing` ターゲットの
/// debug を有効にした subscriber を置く。無効なら静的なレベルチェックで終わり、挙動は変わらない。
pub(crate) fn emit_timing(stage: &'static str, message_id: &str, began: Instant) {
    tracing::debug!(
        target: "notify_timing",
        stage = stage,
        message_id = message_id,
        elapsed_us = u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX),
    );
}

pub(crate) async fn create_federated_notification(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    actor: &str,
    activity: &str,
    kind: &str,
    post: &str,
) -> Result<(), AppError> {
    toasty::sql::statement("INSERT INTO notification_events(id,user_id,actor_id,activity_id,notification_type,post_id,source) VALUES($1,$2,$3,$4,$5,NULLIF($6,''),'activitypub') ON CONFLICT DO NOTHING")
        .bind(uuid::Uuid::new_v4().to_string()).bind(user).bind(actor).bind(activity).bind(kind).bind(post).exec(tx).await?;
    Ok(())
}
pub struct InboxReadResult {
    pub updated_count: usize,
    /// Only return an Electric acknowledgement when the channel Shape can observe it.
    pub channel_changed: bool,
}

/// Shared inbox mutation for channel and federated notifications. A missing selection
/// means all matching rows; an empty selection is deliberately a no-op.
pub async fn mark_read(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    ids: Option<&[String]>,
    source: Option<&str>,
    channel: Option<&str>,
) -> Result<InboxReadResult, AppError> {
    if ids.is_some_and(|ids| ids.len() > 500) {
        return Err(AppError::BadRequest(
            "Too many notifications selected".into(),
        ));
    }
    let ids_json = serde_json::json!(ids).to_string();
    let rows = toasty::sql::query(
        "UPDATE notification_events SET is_read=true
         WHERE user_id=$1 AND NOT is_read
           AND ($2::jsonb='null'::jsonb OR id IN (
             SELECT jsonb_array_elements_text(CASE WHEN $2::jsonb='null'::jsonb THEN '[]'::jsonb ELSE $2::jsonb END)))
           AND ($3='' OR source=$3) AND ($4='' OR channel_id=$4)
         RETURNING source",
    )
    .bind(user)
    .bind(ids_json)
    .bind(source.unwrap_or_default())
    .bind(channel.unwrap_or_default())
    .exec(tx)
    .await?;
    let channel_changed = rows.iter().any(|row| {
        matches!(row, toasty::stmt::Value::Record(record)
            if matches!(record.fields.first(), Some(toasty::stmt::Value::String(source)) if source == "channel"))
    });
    Ok(InboxReadResult {
        updated_count: rows.len(),
        channel_changed,
    })
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MentionUserCandidate {
    pub raw: String,
    pub trimmed: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MentionTargets {
    pub user_candidates: Vec<MentionUserCandidate>,
    pub has_channel: bool,
    pub has_here: bool,
}

const PUNCTUATION: &[char] = &['.', ',', '!', '?', ':', ';', '。', '、', '！', '？'];

pub fn is_mention_boundary(prev: char) -> bool {
    prev.is_whitespace()
        || matches!(
            prev,
            '(' | '[' | '{' | '<' | '（' | '「' | '『' | '"' | '\'' | '。' | '、'
        )
}

/// メッセージ本文からメンション (@username, @channel, @here) を抽出する
///
/// [P3] @ の直前が行頭・空白・開き括弧などの区切り文字であることを要求する (メールアドレスやURLの除外)。
/// また、Thanks @bob. のように文末に句読点がある場合でもユーザー名候補として照合できるよう、
/// 完全一致候補 (raw) と末尾句読点トリム候補 (trimmed) の双方を保持する。
pub fn extract_mentions(content: &str) -> MentionTargets {
    let mut user_candidates = Vec::new();
    let mut has_channel = false;
    let mut has_here = false;

    let chars: Vec<char> = content.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '@' {
            let is_boundary = i == 0 || is_mention_boundary(chars[i - 1]);
            if is_boundary {
                let start = i + 1;
                let mut end = start;
                while end < len {
                    let c = chars[end];
                    if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                        end += 1;
                    } else {
                        break;
                    }
                }

                if end > start {
                    let token: String = chars[start..end].iter().collect();
                    let trimmed_token = token.trim_end_matches(PUNCTUATION);

                    if trimmed_token.eq_ignore_ascii_case("channel") {
                        has_channel = true;
                    } else if trimmed_token.eq_ignore_ascii_case("here") {
                        has_here = true;
                    } else {
                        let trimmed = if trimmed_token != token && !trimmed_token.is_empty() {
                            Some(trimmed_token.to_string())
                        } else {
                            None
                        };
                        user_candidates.push(MentionUserCandidate {
                            raw: token,
                            trimmed,
                        });
                    }
                    i = end;
                    continue;
                }
            }
        }
        i += 1;
    }

    MentionTargets {
        user_candidates,
        has_channel,
        has_here,
    }
}

/// 1 投稿の通知先に関する運用上限 (docs/query-design.md)。超過しても配信は止めず、警告を出す。
const MAX_NOTIFICATION_TARGETS: usize = 10_000;

/// チャンネルの通知設定を `channel_id` で1クエリだけ読み、user_id -> level にする。
///
/// `idx_user_channel_notification_settings_channel_id (channel_id, user_id)` の範囲走査で
/// 対象チャンネルの行だけを読む。走査はそのチャンネルの設定行数 `S`（`S <= M`）までで、
/// ユーザーごとの再問い合わせはしない。`channel_user.notification_level` は互換用の列なので読まない。
pub(crate) async fn channel_notification_levels(
    tx: &mut toasty::Transaction<'_>,
    channel_id: &str,
) -> Result<HashMap<String, String>, AppError> {
    let rows = crate::apps::query(
        tx,
        "SELECT json_build_object('user_id', user_id, 'level', notification_level)::text \
         FROM user_channel_notification_settings WHERE channel_id = $1",
        &[channel_id],
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let user_id = row["user_id"].as_str()?.to_owned();
            let level = row["level"].as_str()?.to_owned();
            Some((user_id, level))
        })
        .collect())
}

/// スレッドの全参加者を、通知レベルと退出日時 (`left_at`) と一緒に1回の問い合わせで読む (#620)。
///
/// `thread_participants` は主キー `(thread_id, user_id)` の範囲走査でそのスレッドの P 行だけを読み、
/// `left_at` は SQL では除かず Rust 側で判定する（親投稿者の退出確認にも同じ結果を使う）。
/// 通知レベルは `user_channel_notification_settings` を
/// `idx_user_channel_notification_settings_channel_id` で P 行まで参照する。行が無ければ
/// 既定値の `mentions` として扱う。ユーザー本体は読まない
/// (bot・無効化・ゲストの除外は、ほかの通知対象とまとめて後段の1クエリで行う)。
pub fn notification_participants_sql() -> &'static str {
    "SELECT json_build_object('user_id', tp.user_id, \
            'level', COALESCE(s.notification_level, 'mentions'), \
            'left_at', tp.left_at)::text \
     FROM thread_participants tp \
     LEFT JOIN user_channel_notification_settings s \
       ON s.channel_id = $2 AND s.user_id = tp.user_id \
     WHERE tp.thread_id = $1"
}

fn is_muted(levels: &HashMap<String, String>, user_id: &str) -> bool {
    levels.get(user_id).is_some_and(|level| level == "mute")
}

/// メッセージ作成時の通知生成
///
/// 1つのメッセージ投稿に対し、同一ユーザー宛ての通知は1件に集約する
/// (メンション > 返信 > DM > チャンネル投稿)。
/// [P2] 公開チャンネルでは非メンバー投稿者への返信通知も配信する。
/// プライベートチャンネルではチャンネル非所属ユーザー (キック・退出者含む) への通知を除外する。
///
/// 通知レベル (#620) は `user_channel_notification_settings.notification_level` による。
/// `mute` には明示メンションだけを届け、返信・DM・チャンネル投稿は作らない。`all` のメンバーには
/// 上位の種別が付かなければ `channel_message` を1件作る (最下位の優先度)。`mentions` は従来どおり。
///
/// 暗号化チャンネル (#358) では本文を解析せず、送信側が平文で付けた `explicit_mentions` だけから
/// メンションを作る (省略時はメンションなし)。平文チャンネルでは `explicit_mentions` を使わない。
/// `all` でも本文は解析せず、Push は既存の暗号化プレースホルダを使う。
pub async fn create_message_notifications(
    tx: &mut toasty::Transaction<'_>,
    message: &Message,
    actor_id: &str,
    parent_author_id: Option<&str>,
    thread_root_id: Option<&str>,
    explicit_mentions: Option<&Mentions>,
) -> Result<Vec<Notification>, AppError> {
    // チャンネル情報を取得 (公開かプライベートかで通知先ポリシーを判定)
    let began = Instant::now();
    let channel = Channel::get_by_id(tx, message.channel_id.as_str()).await?;
    emit_timing("notif_channel", &message.id, began);

    let (mentions, mentioned_ids) = if channel.encrypted {
        let explicit = explicit_mentions.cloned().unwrap_or_default();
        (
            MentionTargets {
                user_candidates: Vec::new(),
                has_channel: explicit.channel,
                has_here: explicit.here,
            },
            explicit.user_ids,
        )
    } else {
        (extract_mentions(&message.content), Vec::new())
    };

    // チャンネル所属メンバーを1回で取得 (走査は M 行、メンバーごとの再問い合わせはしない)。
    // 通知レベルは別テーブルを channel_id で1回だけ読み、メンバーごとの設定行が無ければ
    // 既定値の mentions として扱う (ローリング更新中でも設定が読めるようにする)。
    let began = Instant::now();
    let channel_users =
        ChannelUser::filter(ChannelUser::fields().channel_id().eq(&message.channel_id))
            .exec(tx)
            .await?;
    emit_timing("notif_members", &message.id, began);
    let mut member_levels: HashMap<String, String> = channel_users
        .into_iter()
        .map(|cu| (cu.user_id, "mentions".to_owned()))
        .collect();
    let began = Instant::now();
    for (user_id, level) in channel_notification_levels(tx, message.channel_id.as_str()).await? {
        if let Some(current) = member_levels.get_mut(&user_id) {
            *current = level;
        }
    }
    emit_timing("notif_levels", &message.id, began);
    let member_ids: HashSet<String> = member_levels.keys().cloned().collect();

    // スレッド参加者 (#620) を1回で読む。退出済みの行も返し、Rust 側で `left_at` を見て除く。
    // 親投稿者の退出判定にも同じ結果を使うため、返信・親投稿のどちらかがあれば取得する。
    // 走査は P 行までで、参加者ごとの再問い合わせはしない。
    let mut participant_levels: HashMap<String, String> = HashMap::new();
    let mut left_participants: HashSet<String> = HashSet::new();
    if let Some(root_id) = thread_root_id {
        let began = Instant::now();
        let participants = crate::apps::query(
            tx,
            notification_participants_sql(),
            &[root_id, message.channel_id.as_str()],
        )
        .await?;
        emit_timing("notif_participants", &message.id, began);
        for participant in participants {
            let Some(user_id) = participant["user_id"].as_str() else {
                continue;
            };
            if !participant["left_at"].is_null() {
                left_participants.insert(user_id.to_owned());
                continue;
            }
            if let Some(level) = participant["level"].as_str() {
                participant_levels.insert(user_id.to_owned(), level.to_owned());
            }
        }
    }

    // 通知対象ユーザーと通知タイプ (user_id -> notification_type)
    let mut targets_to_notify: HashMap<String, &'static str> = HashMap::new();

    // 1. @channel / @here: チャンネル所属メンバー全員 (本人除く)
    if mentions.has_channel || mentions.has_here {
        for member_id in &member_ids {
            if member_id != actor_id {
                targets_to_notify.insert(member_id.clone(), "mention");
            }
        }
    }

    // 2. @username: 該当ユーザーが存在する場合 (本人除く)
    // [P3] raw (完全一致) で引いた後、見つからなければ trimmed (末尾句読点除去) で引く
    for candidate in mentions.user_candidates {
        let user_opt = if let Ok(u) = User::get_by_user_name(tx, candidate.raw.as_str()).await {
            Some(u)
        } else if let Some(ref trimmed) = candidate.trimmed {
            User::get_by_user_name(tx, trimmed.as_str()).await.ok()
        } else {
            None
        };

        if let Some(user) = user_opt
            && user.id != actor_id
            && member_ids.contains(&user.id)
        {
            targets_to_notify.insert(user.id, "mention");
        }
    }

    // 2'. 暗号化チャンネルの `mentions.user_ids`: メンバーだけ (本人除く)
    for user_id in mentioned_ids {
        if user_id != actor_id && member_ids.contains(&user_id) {
            targets_to_notify.insert(user_id, "mention");
        }
    }

    // 3. スレッド返信: 親メッセージの投稿者 (本人除く)
    // [P2] 公開チャンネルなら非メンバー投稿者であっても返信通知を作成する
    // プライベートチャンネルの場合は channel_user 所属を確認し、キック/退出者には通知しない。
    // [P2] 親投稿者がこのスレッドから退出済みなら reply は出さない (退出判定は参加者クエリを再利用)。
    if let Some(parent_id) = parent_author_id
        && parent_id != actor_id
        && !is_muted(&member_levels, parent_id)
        && !left_participants.contains(parent_id)
    {
        let can_notify = if channel.is_private {
            member_ids.contains(parent_id)
        } else {
            channel.deleted_at.is_none()
        };

        if can_notify {
            // 既にメンション対象になっていれば "mention" を優先
            targets_to_notify
                .entry(parent_id.to_string())
                .or_insert("reply");
        }
    }

    // 3'. スレッド参加者 (#620): 参加中 (left_at IS NULL) のメンバーへ reply を出す。
    // 退出済みの行は上で除いた。公開チャンネルは非メンバーも可、非公開は所属を確認する。
    // 本人・bot・無効化ユーザー・権限のないゲストは後段の共通フィルターで除く。
    for (user_id, level) in &participant_levels {
        if user_id == actor_id || level == "mute" {
            continue;
        }
        if channel.is_private && !member_ids.contains(user_id) {
            continue;
        }
        targets_to_notify.entry(user_id.clone()).or_insert("reply");
    }

    // 通常の DM も現在の参加者全員へ通知する。メンション・返信は優先して重複させない。
    // `mute` の参加者には届けない。
    if channel.is_dm && channel.deleted_at.is_none() {
        for member_id in &member_ids {
            if member_id != actor_id && !is_muted(&member_levels, member_id) {
                targets_to_notify.entry(member_id.clone()).or_insert("dm");
            }
        }
    }

    // 4. `all` のメンバーには通常投稿を通知する。優先度は最下位なので、上で入った
    // メンション・返信・DM は上書きしない。暗号化チャンネルでも本文は解析しない。
    if channel.deleted_at.is_none() {
        for (member_id, level) in &member_levels {
            if level == "all" && member_id != actor_id {
                targets_to_notify
                    .entry(member_id.clone())
                    .or_insert("channel_message");
            }
        }
    }

    // 上限は除外前の候補数 T で判定する。除外後に数えると、無効化ユーザー等が
    // 候補に含まれていた事実が隠れ、運用者が監視したい負荷を見逃すため。
    let candidate_targets = targets_to_notify.len();
    if candidate_targets > MAX_NOTIFICATION_TARGETS {
        tracing::warn!(
            target: "notifications",
            channel_id = %message.channel_id,
            message_id = %message.id,
            targets = candidate_targets,
            limit = MAX_NOTIFICATION_TARGETS,
            "notification targets exceed the operational limit"
        );
    }

    // Apps receive events rather than human inbox notifications.
    // 無効化済みのユーザー (Slack から取り込んだ未連携のプレースホルダなど) にも通知しない。
    // [P1] ゲストは公開チャンネルでも現在の channel_user 所属が必須。退出・キックされた
    // ゲストには返信本文を届けない。宛先ごとの再問い合わせを増やさず、この1クエリで判定する。
    // 取り込んだチャンネルの @here・@channel で、メンバーの数だけ通知と配信の問い合わせを作らないため
    if !targets_to_notify.is_empty() {
        let ids = serde_json::json!(targets_to_notify.keys().collect::<Vec<_>>()).to_string();
        // ID 配列を駆動側にして users の主キーを T 回だけ引く (U に比例する Seq Scan を避ける)。
        let began = Instant::now();
        let excluded = crate::apps::query(tx, "SELECT json_build_object('id',u.id,'guest',u.is_guest,'bot',u.is_bot,'deactivated',u.is_deactivated)::text FROM jsonb_array_elements_text($1::jsonb) AS ids(id) JOIN users u ON u.id=ids.id WHERE u.is_bot OR u.is_deactivated OR u.is_guest", &[&ids]).await?;
        emit_timing("notif_excluded", &message.id, began);
        for row in excluded {
            if let Some(id) = row["id"].as_str()
                && (row["bot"] == true
                    || row["deactivated"] == true
                    || (row["guest"] == true && !member_ids.contains(id)))
            {
                targets_to_notify.remove(id);
            }
        }
    }

    // 宛先が数千人でも SQL 往復を1回にするため、ID は Rust で生成してまとめて INSERT する。
    // 双方向トリガーと display_data の投影は FOR EACH ROW なので、1行ずつ動く。
    let mut created_notifications = Vec::with_capacity(targets_to_notify.len());
    if !targets_to_notify.is_empty() {
        let now = now_utc_millis();
        let mut rows = Vec::with_capacity(targets_to_notify.len());
        for (target_user_id, notif_type) in &targets_to_notify {
            let id = Uuid::new_v4().to_string();
            created_notifications.push(Notification {
                id: id.clone(),
                user_id: target_user_id.clone(),
                actor_id: actor_id.to_owned(),
                channel_id: message.channel_id.clone(),
                message_id: Some(message.id.clone()),
                notification_type: (*notif_type).to_owned(),
                is_read: false,
                created_at: now,
            });
            rows.push(serde_json::json!({
                "id": id,
                "user_id": target_user_id,
                "actor_id": actor_id,
                "channel_id": message.channel_id,
                "message_id": message.id,
                "notification_type": notif_type,
                "is_read": false,
                "created_at": now.to_string(),
            }));
        }
        let began = Instant::now();
        let inserted = toasty::sql::statement(
            "INSERT INTO channel_notifications(id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at) \
             SELECT e->>'id', e->>'user_id', e->>'actor_id', e->>'channel_id', e->>'message_id', \
                    e->>'notification_type', (e->>'is_read')::boolean, (e->>'created_at')::timestamp \
             FROM jsonb_array_elements($1::jsonb) AS e",
        )
        .bind(serde_json::json!(rows).to_string())
        .exec(tx)
        .await?;
        emit_timing("notif_insert", &message.id, began);
        if inserted != created_notifications.len() as u64 {
            return Err(AppError::Internal(anyhow::anyhow!(
                "inserted {inserted} notifications for {} recipients",
                created_notifications.len()
            )));
        }
    }

    Ok(created_notifications)
}

/// リアクション追加時の通知生成
///
/// 本人のメッセージに対するリアクションは通知しない。
/// [P1] メッセージ投稿者が現在チャンネルを閲覧可能か確認し、キック・退出済みの場合は通知しない。
pub async fn create_reaction_notification(
    tx: &mut toasty::Transaction<'_>,
    message: &Message,
    actor_id: &str,
) -> Result<Option<Notification>, AppError> {
    // bot と無効化済みのユーザー (Slack から取り込んだ未連携のプレースホルダなど) には通知しない
    let skip = crate::apps::one(
        tx,
        "SELECT json_build_object('skip',is_bot OR is_deactivated)::text FROM users WHERE id=$1",
        &[&message.user_id],
    )
    .await?;
    if message.user_id == actor_id || skip["skip"] == true {
        return Ok(None);
    }

    let channel = Channel::get_by_id(tx, message.channel_id.as_str()).await?;
    // メンバー判定は channel_user の主キー1回。通知レベルは設定テーブルを LEFT JOIN で1回だけ読み、
    // 行が無ければ既定値の mentions として扱う。`mute` にはリアクションを届けない。
    let membership = crate::apps::query(
        tx,
        "SELECT json_build_object('level', COALESCE(s.notification_level, 'mentions'))::text \
         FROM channel_user cu \
         LEFT JOIN user_channel_notification_settings s \
           ON s.user_id = cu.user_id AND s.channel_id = cu.channel_id \
         WHERE cu.channel_id=$1 AND cu.user_id=$2",
        &[message.channel_id.as_str(), message.user_id.as_str()],
    )
    .await?;
    let level = membership.first().and_then(|row| row["level"].as_str());
    if level == Some("mute") || (channel.is_private && level.is_none()) {
        return Ok(None);
    }

    let notification = Notification::create()
        .id(Uuid::new_v4().to_string())
        .user_id(message.user_id.clone())
        .actor_id(actor_id.to_string())
        .channel_id(message.channel_id.clone())
        .message_id(message.id.clone())
        .notification_type("reaction".to_string())
        .is_read(false)
        .created_at(now_utc_millis())
        .exec(tx)
        .await?;

    Ok(Some(notification))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_avatar_shortcodes_do_not_send_mentions() {
        let targets = extract_mentions(":@alice: :@here: @bob");
        let ordinary = extract_mentions("@bob");
        assert_eq!(targets.user_candidates, ordinary.user_candidates);
        assert!(!targets.has_here);
        assert!(!targets.has_channel);
    }

    #[test]
    fn test_extract_mentions_basic() {
        let text = "Hello @alice and @bob, check this out!";
        let targets = extract_mentions(text);

        assert_eq!(targets.user_candidates.len(), 2);
        assert_eq!(targets.user_candidates[0].raw, "alice");
        assert_eq!(targets.user_candidates[0].trimmed, None);
        assert_eq!(targets.user_candidates[1].raw, "bob");
        assert_eq!(targets.user_candidates[1].trimmed, None);
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn test_extract_mentions_punctuation_and_sentence_end() {
        let text = "Thanks @bob. Have you seen @charlie! Also (@dave)";
        let targets = extract_mentions(text);

        assert_eq!(targets.user_candidates.len(), 3);
        assert_eq!(targets.user_candidates[0].raw, "bob.");
        assert_eq!(targets.user_candidates[0].trimmed, Some("bob".to_string()));

        assert_eq!(targets.user_candidates[1].raw, "charlie");
        assert_eq!(targets.user_candidates[1].trimmed, None);

        assert_eq!(targets.user_candidates[2].raw, "dave");
        assert_eq!(targets.user_candidates[2].trimmed, None);
    }

    #[test]
    fn test_extract_mentions_special_with_punctuation() {
        let text = "Attention @channel! and @HERE. please";
        let targets = extract_mentions(text);

        assert!(targets.user_candidates.is_empty());
        assert!(targets.has_channel);
        assert!(targets.has_here);
    }

    #[test]
    fn test_extract_mentions_email_and_url_ignored() {
        let text = "Contact test@example.com or visit https://example.com/@here or @channel_inside";
        let targets = extract_mentions(text);

        // test@example.com -> 前が 't' なので境界でないため無視
        // https://example.com/@here -> 前が '/' なので境界でないため無視
        // @channel_inside -> 前が ' ' なので有効なメンショントークン
        assert!(!targets.has_here);
        assert_eq!(targets.user_candidates.len(), 1);
        assert_eq!(targets.user_candidates[0].raw, "channel_inside");
    }

    #[test]
    fn test_extract_mentions_none() {
        let text = "No mentions here, just normal text";
        let targets = extract_mentions(text);

        assert!(targets.user_candidates.is_empty());
        assert!(!targets.has_channel);
        assert!(!targets.has_here);
    }

    #[test]
    fn test_extract_mentions_japanese_punctuation() {
        let text = "こんにちは。@bob、確認してください、@alice！";
        let targets = extract_mentions(text);

        assert_eq!(targets.user_candidates.len(), 2);
        assert_eq!(targets.user_candidates[0].raw, "bob");
        assert_eq!(targets.user_candidates[1].raw, "alice");
    }
}
