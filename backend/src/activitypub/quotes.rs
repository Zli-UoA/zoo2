//! One-level, origin-verified quote cards delivered with the Home page, without per-card APIs.
use super::*;

type QuoteFlights =
    std::collections::HashMap<(String, String), std::sync::Weak<tokio::sync::Mutex<()>>>;
static FLIGHTS: std::sync::LazyLock<tokio::sync::Mutex<QuoteFlights>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(QuoteFlights::new()));
static RESOLUTIONS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

fn spawn_resolution_task(
    task: impl std::future::Future<Output = ()> + Send + 'static,
) -> Option<tokio::task::JoinHandle<()>> {
    let permit = RESOLUTIONS.try_acquire().ok()?;
    Some(tokio::spawn(async move {
        let _permit = permit;
        task.await;
    }))
}

pub(super) async fn schedule(
    state: AppState,
    user: String,
    activity: Value,
) -> Result<(), AppError> {
    match activity["type"].as_str() {
        Some("Create" | "Update") if emoji::note_quote_uri(&activity["object"]).is_some() => {}
        Some("Announce") => {}
        _ => return Ok(()),
    }
    // Ordinary Announce activities and cached/local originals must not occupy
    // the scarce network slots. This preparation performs database reads only.
    let Some(uri) = eligible_target(&state, &user, &activity).await? else {
        return Ok(());
    };
    let Some(flight) = claim_quote(&uri, &user).await else {
        return Ok(());
    };
    let task = async move {
        let _flight = flight;
        let result =
            tokio::time::timeout(Duration::from_secs(20), resolve(&state, &user, &uri)).await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::debug!(%error, "Quoted Note could not be resolved"),
            Err(_) => tracing::debug!("Quote resolution timed out"),
        }
    };
    if spawn_resolution_task(task).is_none() {
        tracing::debug!("Quote resolution capacity reached; keeping the original link");
    }
    Ok(())
}

async fn claim_quote(uri: &str, user: &str) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    let mutex = {
        let mut flights = FLIGHTS.lock().await;
        flights.retain(|_, weak| weak.strong_count() > 0);
        let entry = flights
            .entry((uri.to_owned(), user.to_owned()))
            .or_default();
        if let Some(mutex) = entry.upgrade() {
            mutex
        } else {
            let mutex = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            *entry = std::sync::Arc::downgrade(&mutex);
            mutex
        }
    };
    mutex.try_lock_owned().ok()
}

async fn eligible_target(
    state: &AppState,
    user: &str,
    activity: &Value,
) -> Result<Option<String>, AppError> {
    let announce = activity["type"] == "Announce";
    let id = match activity["type"].as_str() {
        Some("Create" | "Update") => string(&activity["object"], "id")?,
        Some("Announce") => activity["object"]
            .as_str()
            .or_else(|| activity["object"]["id"].as_str())
            .ok_or_else(|| AppError::BadRequest("Missing Announce object".into()))?,
        _ => return Ok(None),
    };
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // Only a stored post delivered to this recipient may authorize a quote fetch.
    let accepted = query(&mut tx, "SELECT json_build_object('quote_uri',p.quote_uri)::text FROM apub_remote_posts p JOIN apub_actors a ON a.id=p.actor_id WHERE p.id=$1 AND p.quote_uri IS NOT NULL AND p.deleted_at IS NULL AND NOT a.blocked AND (($5::boolean AND EXISTS(SELECT 1 FROM apub_reposts r JOIN apub_repost_recipients rr ON rr.activity_id=r.activity_id WHERE r.post_id=p.id AND r.activity_id=$4 AND rr.user_id=$2)) OR (NOT $5::boolean AND p.actor_id=$3 AND EXISTS(SELECT 1 FROM apub_remote_recipients WHERE post_id=p.id AND user_id=$2)))", &[id,user,string(activity,"actor")?,string(activity,"id")?,if announce {"true"} else {"false"}]).await?;
    let Some(row) = accepted.first() else {
        return Ok(None);
    };
    let Some(uri) = emoji::note_quote_uri(&json!({"quoteUri":row["quote_uri"]})) else {
        return Ok(None);
    };
    if !announce && emoji::note_quote_uri(&activity["object"]).as_ref() != Some(&uri) {
        return Ok(None);
    }
    let uri = uri.as_str();
    if uri == id || uri.starts_with(&format!("{}/api/activitypub/notes/", base_url()?)) {
        return Ok(None);
    }
    if known_original(&mut tx, user, uri).await? {
        return Ok(None);
    }
    Ok(Some(uri.to_owned()))
}

const REMOTE_QUOTE: &str = include_str!("remote_quote.sql");

async fn known_original(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    uri: &str,
) -> Result<bool, AppError> {
    let sql = format!(
        "SELECT '{{}}'::text WHERE EXISTS(
           SELECT 1 FROM (VALUES ($1::text,$2::text)) target(id,user_id)
           CROSS JOIN LATERAL ({REMOTE_QUOTE}) original
         ) OR EXISTS(SELECT 1 FROM apub_remote_tombstones WHERE id=$1)"
    );
    Ok(!query(tx, &sql, &[uri, user]).await?.is_empty())
}

async fn resolve(state: &AppState, user: &str, uri: &str) -> Result<(), AppError> {
    let mut db = state.db.clone();
    // A concurrent signer may already have saved a public original.
    let mut tx = db.transaction().await?;
    if known_original(&mut tx, user, uri).await? {
        return Ok(());
    }
    drop(tx);
    // Fetch with this recipient's actor signature. Nested quotes stay unresolved.
    let note = transport::fetch_json_with_retry(state, Some(user), uri).await?;
    reposts::store_quoted_note(state, user, uri, &note).await
}

pub(crate) async fn decorate(
    tx: &mut toasty::Transaction<'_>,
    user: &str,
    posts: &mut [Value],
) -> Result<(), AppError> {
    let ids: Vec<&str> = posts
        .iter()
        .flat_map(|p| [p["id"].as_str(), p["parent"]["id"].as_str()])
        .flatten()
        .collect();
    if ids.is_empty() {
        return Ok(());
    }
    let base = base_url().unwrap_or_default();
    let sql = r#"WITH refs AS (
      SELECT p.id,p.quote_uri
      FROM (SELECT DISTINCT jsonb_array_elements_text($1::jsonb) AS id) ids
      CROSS JOIN LATERAL (
        SELECT id,quote_uri FROM apub_remote_posts WHERE id=ids.id AND quote_uri IS NOT NULL OFFSET 0
      ) p
    ) SELECT json_build_object('id',r.id,'uri',r.quote_uri,'local',starts_with(r.quote_uri,$2 || '/api/activitypub/notes/'),'post',COALESCE(remote.post,local.post))::text FROM refs r
    LEFT JOIN LATERAL (
      SELECT json_build_object('id',p.id,'content',p.content,'user_name',p.user_name || '@' || split_part(p.actor_id,'/',3),'display_name',p.display_name,'created_at',p.published,'emojis',p.emojis,'actor_emojis',p.actor_emojis,'avatar',p.avatar,'media',p.media,'remote',true,'user_id',p.actor_id) AS post
      FROM (VALUES (r.quote_uri,$3::text)) target(id,user_id)
      CROSS JOIN LATERAL (/* remote_quote */ LIMIT 1) p
      OFFSET 0
    ) remote ON true
    LEFT JOIN LATERAL (
      SELECT json_build_object('id',p.id,'content',p.content,'user_name',u.user_name,'display_name',COALESCE(profile.display_name,u.display_name),'created_at',p.created_at,'emojis',COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'local',true)) FROM jsonb_each(message_content_emojis(p.content)) e),'[]'::jsonb),'avatar_url',u.avatar_url,'media',p.media,'channel_id',p.channel_id,'remote',false,'user_id',p.user_id) AS post
      FROM home_posts p JOIN users u ON u.id=p.user_id LEFT JOIN apub_profiles profile ON profile.user_id=u.id LEFT JOIN channels c ON c.id=p.channel_id
      WHERE p.id=CASE WHEN starts_with(r.quote_uri,$2 || '/api/activitypub/notes/') THEN substr(r.quote_uri,length($2 || '/api/activitypub/notes/')+1) END AND p.apub AND p.deleted_at IS NULL AND p.visibility IN ('public','unlisted') AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)
      OFFSET 0
    ) local ON true"#.replace("/* remote_quote */", REMOTE_QUOTE);
    let quotes = query(tx, &sql, &[&json!(ids).to_string(), &base, user]).await?;
    if quotes.is_empty() {
        return Ok(());
    }
    let originals: std::collections::HashMap<String, Value> = quotes
        .iter()
        .filter_map(|q| q.get("post").filter(|p| p.is_object()))
        .filter_map(|p| Some((p["id"].as_str()?.to_owned(), p.clone())))
        .collect();
    let mut originals: Vec<Value> = originals.into_values().collect();
    if !originals.is_empty() {
        crate::routes::home_images::decorate(tx, &mut originals).await?;
        remember_display_emojis(tx, &originals).await?;
    }
    let originals: std::collections::HashMap<String, Value> = originals
        .into_iter()
        .filter_map(|p| Some((p["id"].as_str()?.to_owned(), p)))
        .collect();
    let quotes: std::collections::HashMap<String, Value> = quotes
        .into_iter()
        .filter_map(|mut q| {
            let id = q["id"].as_str()?.to_owned();
            q.as_object_mut()?.remove("id");
            if let Some(post) = q["post"]["id"].as_str().and_then(|id| originals.get(id)) {
                q["post"] = post.clone();
            }
            Some((id, q))
        })
        .collect();
    for row in posts {
        if let Some(quote) = row["id"].as_str().and_then(|id| quotes.get(id)) {
            row["quote"] = quote.clone();
        }
        if let Some(quote) = row["parent"]["id"].as_str().and_then(|id| quotes.get(id)) {
            row["parent"]["quote"] = quote.clone();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn optional_fetches_run_separately_and_do_not_queue_beyond_capacity() {
        let gate = std::sync::Arc::new(tokio::sync::Semaphore::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let gate = gate.clone();
            handles.push(
                spawn_resolution_task(async move {
                    let _permit = gate.acquire().await.unwrap();
                })
                .expect("the first four optional fetches are admitted"),
            );
        }
        assert!(spawn_resolution_task(async {}).is_none());
        gate.add_permits(4);
        for handle in handles {
            handle.await.unwrap();
        }
        spawn_resolution_task(async {})
            .expect("completed work releases capacity")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn only_the_same_signer_is_deduplicated() {
        let original = "https://quoted.example/notes/id";
        let first = claim_quote(original, "first").await.unwrap();
        assert!(claim_quote(original, "first").await.is_none());
        assert!(claim_quote(original, "second").await.is_some());
        drop(first);
        assert!(claim_quote(original, "first").await.is_some());
    }

    #[tokio::test]
    #[ignore = "requires migrated PostgreSQL and ACTIVITYPUB_BASE_URL"]
    async fn followers_only_quotes_are_cached_per_authorized_signer() -> anyhow::Result<()> {
        let config = crate::config::Config::from_env();
        let mut db = crate::db::init_db(&config.database_url).await?;
        let state = AppState::new(db.clone(), config);
        let user = uuid::Uuid::new_v4().to_string();
        let other = uuid::Uuid::new_v4().to_string();
        let author = format!("https://quoted.example/users/{}", uuid::Uuid::new_v4());
        let id = format!("{author}/notes/original");
        let followers = format!("{author}/followers");
        for recipient in [&user, &other] {
            toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
                .bind(recipient.as_str())
                .exec(&mut db)
                .await?;
        }
        toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$2,'author','Author',$1,'cached')")
            .bind(author.as_str()).bind(followers.as_str()).exec(&mut db).await?;
        let note = json!({"id":id,"type":"Note","attributedTo":author,"content":"Followers only","published":"2026-01-02T00:00:00Z","to":[followers],"inReplyTo":{"id":format!("{author}/notes/parent")}});
        assert!(
            reposts::store_quoted_note(&state, &user, &id, &note)
                .await
                .is_err()
        );
        toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) VALUES($1,$2,'outgoing',$2,true)")
            .bind(user.as_str()).bind(author.as_str()).exec(&mut db).await?;
        let mut direct = note.clone();
        direct["to"] = json!([actor_id(&user)?]);
        assert!(
            reposts::store_quoted_note(&state, &user, &id, &direct)
                .await
                .is_err()
        );
        // A private response older than an unverified public Inbox copy cannot promote it.
        let stale_id = format!("{author}/notes/stale");
        let mut stale_response = note.clone();
        stale_response["id"] = json!(stale_id);
        toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at) VALUES($1,$2,'Unverified public copy','public','2026-01-01','2026-01-03')")
            .bind(stale_id.as_str()).bind(author.as_str()).exec(&mut db).await?;
        reposts::store_quoted_note(&state, &user, &stale_id, &stale_response).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &user, &stale_id).await?);
        let stale = one(
            &mut tx,
            "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
            &[&stale_id],
        )
        .await?;
        assert_eq!(stale["origin_verified"], false);
        assert_eq!(stale["content"], "Unverified public copy");
        drop(tx);
        reposts::store_quoted_note(&state, &user, &id, &note).await?;

        toasty::sql::statement("INSERT INTO apub_follows(user_id,actor_id,direction,activity_id,accepted) VALUES($1,$2,'outgoing',$3,true)")
            .bind(other.as_str()).bind(author.as_str()).bind(format!("{author}/second-follow")).exec(&mut db).await?;
        let quote_id = format!("{author}/notes/quote");
        toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,quote_uri) VALUES($1,$2,'Quote','public',now(),now(),$3)")
            .bind(quote_id.as_str()).bind(author.as_str()).bind(id.as_str()).exec(&mut db).await?;
        let mut tx = db.transaction().await?;
        assert!(known_original(&mut tx, &user, &id).await?);
        assert!(!known_original(&mut tx, &other, &id).await?);
        let post = one(
            &mut tx,
            "SELECT row_to_json(p)::text FROM apub_quote_originals p WHERE id=$1",
            &[&id],
        )
        .await?;
        assert_eq!(post["parent_id"], format!("{author}/notes/parent"));
        assert!(
            query(
                &mut tx,
                "SELECT '{}'::text FROM apub_remote_recipients WHERE post_id=$1",
                &[&id]
            )
            .await?
            .is_empty()
        );
        let mut rows = [json!({"id":quote_id})];
        decorate(&mut tx, &user, &mut rows).await?;
        assert_eq!(rows[0]["quote"]["post"]["content"], "Followers only");
        decorate(&mut tx, &other, &mut rows).await?;
        assert!(rows[0]["quote"]["post"].is_null());
        toasty::sql::statement("UPDATE apub_follows SET accepted=false WHERE user_id=$1 AND actor_id=$2 AND direction='outgoing'")
            .bind(user.as_str()).bind(author.as_str()).exec(&mut tx).await?;
        decorate(&mut tx, &user, &mut rows).await?;
        assert!(rows[0]["quote"]["post"].is_null());
        tx.rollback().await?;
        reposts::store_quoted_note(&state, &other, &id, &note).await?;
        let mut tx = db.transaction().await?;
        assert!(known_original(&mut tx, &other, &id).await?);
        decorate(&mut tx, &other, &mut rows).await?;
        assert_eq!(rows[0]["quote"]["post"]["content"], "Followers only");
        tx.rollback().await?;

        let mut tx = db.transaction().await?;
        let actor = one(
            &mut tx,
            "SELECT row_to_json(a)::text FROM apub_actors a WHERE id=$1",
            &[&author],
        )
        .await?;
        drop(tx);
        let duplicate = json!({"id":format!("{author}/unchanged-update"),"type":"Update","actor":author,"object":note});
        inbox::receive_verified(&state, &other, &actor, &duplicate).await?;
        let mut tx = db.transaction().await?;
        assert!(known_original(&mut tx, &user, &id).await?);
        drop(tx);

        let mut corrected = note.clone();
        corrected["content"] = json!("Corrected without updated");
        let correction = json!({"id":format!("{author}/correction"),"type":"Update","actor":author,"object":corrected});
        inbox::receive_verified(&state, &other, &actor, &correction).await?;
        let mut tx = db.transaction().await?;
        for recipient in [&user, &other] {
            assert!(!known_original(&mut tx, recipient, &id).await?);
            decorate(&mut tx, recipient, &mut rows).await?;
            assert!(rows[0]["quote"]["post"].is_null());
        }
        drop(tx);
        reposts::store_quoted_note(&state, &user, &id, &corrected).await?;
        let mut tx = db.transaction().await?;
        decorate(&mut tx, &user, &mut rows).await?;
        assert_eq!(
            rows[0]["quote"]["post"]["content"],
            "Corrected without updated"
        );
        assert!(!known_original(&mut tx, &other, &id).await?);
        drop(tx);

        let mut revised = note.clone();
        revised["content"] = json!("Updated before Create");
        revised["updated"] = json!("2026-01-02T01:00:00Z");
        let early_update = json!({"id":format!("{author}/early-update"),"type":"Update","actor":author,"object":revised});
        inbox::receive_verified(&state, &other, &actor, &early_update).await?;
        reposts::store_quoted_note(&state, &user, &id, &note).await?;
        let mut tx = db.transaction().await?;
        assert!(
            query(
                &mut tx,
                "SELECT '{}'::text FROM apub_remote_posts WHERE id=$1",
                &[&id]
            )
            .await?
            .is_empty()
        );
        assert!(!known_original(&mut tx, &user, &id).await?);
        assert!(!known_original(&mut tx, &other, &id).await?);
        drop(tx);
        reposts::store_quoted_note(&state, &user, &id, &revised).await?;
        let mut tx = db.transaction().await?;
        decorate(&mut tx, &user, &mut rows).await?;
        assert_eq!(rows[0]["quote"]["post"]["content"], "Updated before Create");
        assert!(!known_original(&mut tx, &other, &id).await?);
        drop(tx);

        let mut inbox_note = note.clone();
        inbox_note["content"] = json!("Delivered public copy");
        inbox_note["to"] = json!([PUBLIC]);
        let create = json!({"id":format!("{author}/create"),"type":"Create","actor":author,"object":inbox_note});
        inbox::receive_verified(&state, &other, &actor, &create).await?;
        let mut tx = db.transaction().await?;
        let delivered = one(&mut tx, "SELECT row_to_json(r)::text FROM apub_remote_recipients r WHERE post_id=$1 AND user_id=$2", &[&id,&other]).await?;
        assert_eq!(delivered["parent_id"], format!("{author}/notes/parent"));
        drop(tx);
        let mut fetched_update = note.clone();
        fetched_update["content"] = json!("New private copy");
        fetched_update["published"] = json!("2026-01-01T00:00:00Z");
        fetched_update["updated"] = json!("2026-01-02T12:00:00Z");
        reposts::store_quoted_note(&state, &user, &id, &fetched_update).await?;
        let mut tx = db.transaction().await?;
        let delivered_copy = one(
            &mut tx,
            "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
            &[&id],
        )
        .await?;
        assert_eq!(delivered_copy["content"], "Delivered public copy");
        assert_eq!(delivered_copy["visibility"], "public");
        decorate(&mut tx, &other, &mut rows).await?;
        assert!(rows[0]["quote"]["post"].is_null());
        decorate(&mut tx, &user, &mut rows).await?;
        assert_eq!(rows[0]["quote"]["post"]["content"], "New private copy");
        assert!(
            rows[0]["quote"]["post"]["created_at"]
                .as_str()
                .unwrap()
                .starts_with("2026-01-01")
        );
        drop(tx);

        // A newer Inbox version has a narrower audience and must hide old grants.
        let local_parent = uuid::Uuid::new_v4().to_string();
        toasty::sql::statement("INSERT INTO home_posts(id,user_id,content,apub,visibility) VALUES($1,$2,'Parent',true,'public')")
            .bind(local_parent.as_str()).bind(other.as_str()).exec(&mut db).await?;
        let mut direct_update = note.clone();
        direct_update["content"] = json!("Direct reply");
        direct_update["to"] = json!([actor_id(&other)?]);
        direct_update["updated"] = json!("2026-01-03T00:00:00Z");
        direct_update["inReplyTo"] = json!(format!(
            "{}/api/activitypub/notes/{local_parent}",
            base_url()?
        ));
        let update = json!({"id":format!("{author}/update"),"type":"Update","actor":author,"object":direct_update});
        inbox::receive_verified(&state, &other, &actor, &update).await?;
        reposts::store_quoted_note(&state, &other, &id, &note).await?;
        let mut same_timestamp = note.clone();
        same_timestamp["updated"] = direct_update["updated"].clone();
        same_timestamp["inReplyTo"] = direct_update["inReplyTo"].clone();
        reposts::store_quoted_note(&state, &other, &id, &same_timestamp).await?;
        let mut tx = db.transaction().await?;
        for recipient in [&user, &other] {
            assert!(!known_original(&mut tx, recipient, &id).await?);
            decorate(&mut tx, recipient, &mut rows).await?;
            assert!(rows[0]["quote"]["post"].is_null());
        }
        let post = one(
            &mut tx,
            "SELECT row_to_json(p)::text FROM apub_remote_posts p WHERE id=$1",
            &[&id],
        )
        .await?;
        assert_eq!(post["content"], "Direct reply");
        tx.rollback().await?;

        let mut without_updated = note.clone();
        without_updated["published"] = json!("2026-01-04T00:00:00Z");
        reposts::store_quoted_note(&state, &user, &id, &without_updated).await?;
        let mut public = without_updated.clone();
        public["to"] = json!([PUBLIC]);
        public["content"] = json!("Public again with the same timestamp");
        reposts::store_public_note(&state, &user, &id, &public).await?;
        let mut tx = db.transaction().await?;
        assert!(
            reposts::remote_repost_target(&mut tx, &id, true)
                .await?
                .is_some()
        );
        for recipient in [&user, &other] {
            decorate(&mut tx, recipient, &mut rows).await?;
            assert_eq!(rows[0]["quote"]["post"]["content"], public["content"]);
        }
        drop(tx);
        let mut revised_public = public.clone();
        revised_public["content"] = json!("Corrected public copy");
        let public_update = json!({"id":format!("{author}/public-correction"),"type":"Update","actor":author,"object":revised_public});
        inbox::receive_verified(&state, &other, &actor, &public_update).await?;
        reposts::store_public_note(&state, &user, &id, &public).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &other, &id).await?);
        assert!(
            reposts::remote_repost_target(&mut tx, &id, false)
                .await?
                .is_none()
        );
        drop(tx);
        reposts::store_public_note(&state, &user, &id, &revised_public).await?;
        let mut tx = db.transaction().await?;
        decorate(&mut tx, &other, &mut rows).await?;
        assert_eq!(
            rows[0]["quote"]["post"]["content"],
            revised_public["content"]
        );
        drop(tx);

        let restricted_again = json!({"id":format!("{author}/restricted-again"),"type":"Update","actor":author,"object":without_updated});
        inbox::receive_verified(&state, &other, &actor, &restricted_again).await?;
        // An older public GET completing after the restrictive Update cannot publish it again.
        reposts::store_public_note(&state, &user, &id, &public).await?;
        let mut tx = db.transaction().await?;
        assert!(
            reposts::remote_repost_target(&mut tx, &id, false)
                .await?
                .is_none()
        );
        for recipient in [&user, &other] {
            decorate(&mut tx, recipient, &mut rows).await?;
            assert!(rows[0]["quote"]["post"].is_null());
        }
        drop(tx);

        // A fresh private response at that timestamp cannot revive a previous signer's grant.
        reposts::store_quoted_note(&state, &other, &id, &without_updated).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &user, &id).await?);
        decorate(&mut tx, &other, &mut rows).await?;
        assert_eq!(
            rows[0]["quote"]["post"]["content"],
            without_updated["content"]
        );
        drop(tx);

        // Only the Note's audience authorizes quotes, even if the envelope reaches former followers.
        let mut narrower = without_updated.clone();
        narrower["to"] = json!([actor_id(&other)?]);
        let envelope = json!({"id":format!("{author}/narrower"),"type":"Update","actor":author,"to":[followers],"object":narrower});
        inbox::receive_verified(&state, &other, &actor, &envelope).await?;
        reposts::store_quoted_note(&state, &user, &id, &without_updated).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &user, &id).await?);
        assert!(!known_original(&mut tx, &other, &id).await?);
        assert!(
            reposts::remote_repost_target(&mut tx, &id, false)
                .await?
                .is_none()
        );
        drop(tx);

        // A signed correction to a delivered private Note must beat the old Inbox payload.
        let delivered_id = format!("{author}/notes/delivered");
        let mut delivered_note = note.clone();
        delivered_note["id"] = json!(delivered_id);
        let delivered_create = json!({"id":format!("{author}/deliver-private"),"type":"Create","actor":author,"object":delivered_note});
        inbox::receive_verified(&state, &other, &actor, &delivered_create).await?;
        // The first quote lookup may occur after an equal-time Inbox correction.
        let mut before_first_fetch = delivered_note.clone();
        before_first_fetch["content"] = json!("Corrected before first fetch");
        let first_correction = json!({"id":format!("{author}/correct-before-fetch"),"type":"Update","actor":author,"object":before_first_fetch});
        inbox::receive_verified(&state, &other, &actor, &first_correction).await?;
        reposts::store_quoted_note(&state, &user, &delivered_id, &delivered_note).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &user, &delivered_id).await?);
        drop(tx);
        reposts::store_quoted_note(&state, &user, &delivered_id, &before_first_fetch).await?;
        let mut tx = db.transaction().await?;
        assert!(known_original(&mut tx, &user, &delivered_id).await?);
        drop(tx);
        toasty::sql::statement("UPDATE apub_remote_posts SET quote_uri=$2 WHERE id=$1")
            .bind(quote_id.as_str())
            .bind(delivered_id.as_str())
            .exec(&mut db)
            .await?;
        let mut delivered_correction = delivered_note.clone();
        delivered_correction["content"] = json!("Corrected delivered private Note");
        let delivered_update = json!({"id":format!("{author}/correct-delivered"),"type":"Update","actor":author,"object":delivered_correction});
        inbox::receive_verified(&state, &other, &actor, &delivered_update).await?;
        reposts::store_quoted_note(&state, &user, &delivered_id, &delivered_note).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &user, &delivered_id).await?);
        drop(tx);
        reposts::store_quoted_note(&state, &user, &delivered_id, &delivered_correction).await?;
        let mut tx = db.transaction().await?;
        decorate(&mut tx, &user, &mut rows).await?;
        assert_eq!(
            rows[0]["quote"]["post"]["content"],
            delivered_correction["content"]
        );
        let delivered_copy = one(
            &mut tx,
            "SELECT json_build_object('content',content)::text FROM apub_remote_posts WHERE id=$1",
            &[&delivered_id],
        )
        .await?;
        assert_eq!(delivered_copy["content"], delivered_note["content"]);
        drop(tx);

        let narrowed_id = format!("{author}/notes/narrowed-before-fetch");
        let mut originally_followers = note.clone();
        originally_followers["id"] = json!(narrowed_id);
        let create = json!({"id":format!("{author}/create-before-narrow"),"type":"Create","actor":author,"object":originally_followers});
        inbox::receive_verified(&state, &other, &actor, &create).await?;
        let mut narrow = originally_followers.clone();
        narrow["to"] = json!([actor_id(&other)?]);
        let update = json!({"id":format!("{author}/narrow-before-fetch"),"type":"Update","actor":author,"to":[followers],"object":narrow});
        inbox::receive_verified(&state, &other, &actor, &update).await?;
        reposts::store_quoted_note(&state, &user, &narrowed_id, &originally_followers).await?;
        let mut tx = db.transaction().await?;
        assert!(!known_original(&mut tx, &user, &narrowed_id).await?);
        drop(tx);

        let conflicting_actor = format!("{author}/other-actor");
        let conflicting_id = format!("{author}/notes/conflicting-observation");
        toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'peer','Peer',$1,'cached')")
            .bind(conflicting_actor.as_str()).exec(&mut db).await?;
        toasty::sql::statement("INSERT INTO apub_quote_originals(id,actor_id,content,published,updated_at,emojis,media,superseded,observed_audience) VALUES($1,$2,'Unverified observation','2026-01-01','2026-01-03','[]','[]',true,'direct')")
            .bind(conflicting_id.as_str()).bind(conflicting_actor.as_str()).exec(&mut db).await?;
        let mut canonical = note.clone();
        canonical["id"] = json!(conflicting_id);
        reposts::store_quoted_note(&state, &user, &conflicting_id, &canonical).await?;
        let mut tx = db.transaction().await?;
        assert!(known_original(&mut tx, &user, &conflicting_id).await?);
        drop(tx);

        let conflicting_public_id = format!("{author}/notes/conflicting-public-observation");
        toasty::sql::statement("INSERT INTO apub_quote_originals(id,actor_id,content,published,updated_at,emojis,media,superseded,observed_audience) VALUES($1,$2,'Unverified observation','2026-01-01','2026-01-03','[]','[]',true,'direct')")
            .bind(conflicting_public_id.as_str()).bind(conflicting_actor.as_str()).exec(&mut db).await?;
        let mut canonical_public = note.clone();
        canonical_public["id"] = json!(conflicting_public_id);
        canonical_public["to"] = json!([PUBLIC]);
        reposts::store_public_note(&state, &user, &conflicting_public_id, &canonical_public)
            .await?;
        toasty::sql::statement("UPDATE apub_remote_posts SET quote_uri=$2 WHERE id=$1")
            .bind(quote_id.as_str())
            .bind(conflicting_public_id.as_str())
            .exec(&mut db)
            .await?;
        let mut tx = db.transaction().await?;
        assert!(
            reposts::remote_repost_target(&mut tx, &conflicting_public_id, true)
                .await?
                .is_some()
        );
        decorate(&mut tx, &other, &mut rows).await?;
        assert_eq!(
            rows[0]["quote"]["post"]["content"],
            canonical_public["content"]
        );
        drop(tx);
        canonical_public["to"] = json!([followers]);
        let restrict = json!({"id":format!("{author}/restrict-conflicting-public"),"type":"Update","actor":author,"object":canonical_public});
        inbox::receive_verified(&state, &other, &actor, &restrict).await?;
        let mut tx = db.transaction().await?;
        assert!(
            reposts::remote_repost_target(&mut tx, &conflicting_public_id, false)
                .await?
                .is_none()
        );
        assert!(
            reposts::remote_repost_target(&mut tx, &conflicting_public_id, true)
                .await?
                .is_none()
        );
        decorate(&mut tx, &other, &mut rows).await?;
        assert!(rows[0]["quote"]["post"].is_null());
        drop(tx);

        for (sql, key) in [
            (
                "DELETE FROM apub_quote_originals WHERE id=$1",
                &conflicting_public_id,
            ),
            (
                "DELETE FROM apub_remote_posts WHERE id=$1",
                &conflicting_public_id,
            ),
            (
                "DELETE FROM apub_quote_originals WHERE id=$1",
                &conflicting_id,
            ),
            ("DELETE FROM apub_actors WHERE id=$1", &conflicting_actor),
            ("DELETE FROM apub_quote_originals WHERE id=$1", &narrowed_id),
            ("DELETE FROM apub_remote_posts WHERE id=$1", &narrowed_id),
            (
                "DELETE FROM apub_quote_originals WHERE id=$1",
                &delivered_id,
            ),
            ("DELETE FROM apub_remote_posts WHERE id=$1", &delivered_id),
            ("DELETE FROM users WHERE id=$1", &user),
            ("DELETE FROM users WHERE id=$1", &other),
            ("DELETE FROM apub_remote_posts WHERE id=$1", &quote_id),
            ("DELETE FROM apub_remote_posts WHERE id=$1", &stale_id),
            ("DELETE FROM apub_remote_posts WHERE id=$1", &id),
            ("DELETE FROM apub_quote_originals WHERE id=$1", &id),
            ("DELETE FROM apub_actors WHERE id=$1", &author),
        ] {
            toasty::sql::statement(sql)
                .bind(key.as_str())
                .exec(&mut db)
                .await?;
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires migrated PostgreSQL and ACTIVITYPUB_BASE_URL"]
    async fn quote_originals_require_origin_and_public_audience_before_storage()
    -> anyhow::Result<()> {
        let config = crate::config::Config::from_env();
        let mut db = crate::db::init_db(&config.database_url).await?;
        let state = AppState::new(db.clone(), config);
        let author = format!("https://quoted.example/users/{}", uuid::Uuid::new_v4());
        let id = format!("{author}/notes/original");
        toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'author','Author',$1,'cached')").bind(author.as_str()).exec(&mut db).await?;
        toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at) VALUES($1,$2,'Unverified cached content','public','2026-01-01','2026-01-01')").bind(id.as_str()).bind(author.as_str()).exec(&mut db).await?;
        let note = json!({"id":id,"type":"Note","attributedTo":author,"content":"Canonical original","published":"2026-01-02T00:00:00Z","to":[PUBLIC]});
        reposts::store_public_note(&state, "unused", &id, &note).await?;
        let mut tx = db.transaction().await?;
        let stored = one(&mut tx,"SELECT json_build_object('content',content,'verified',origin_verified,'published',published::date)::text FROM apub_remote_posts WHERE id=$1",&[&id]).await?;
        assert_eq!(stored["content"], "Canonical original");
        assert_eq!(stored["verified"], true);
        assert_eq!(stored["published"], "2026-01-02");
        drop(tx);
        let mut private = note.clone();
        private["to"] = json!([]);
        let mut impostor = note.clone();
        impostor["attributedTo"] = json!("https://impostor.example/users/a");
        let mut other_id = note.clone();
        other_id["id"] = json!(format!("{author}/different"));
        for invalid in [private, impostor, other_id] {
            assert!(
                reposts::store_public_note(&state, "unused", &id, &invalid)
                    .await
                    .is_err()
            );
        }
        toasty::sql::statement("UPDATE apub_remote_posts SET deleted_at=now() WHERE id=$1")
            .bind(id.as_str())
            .exec(&mut db)
            .await?;
        reposts::store_public_note(&state, "unused", &id, &note).await?;
        let mut tx = db.transaction().await?;
        assert_eq!(one(&mut tx,"SELECT json_build_object('deleted',deleted_at IS NOT NULL)::text FROM apub_remote_posts WHERE id=$1",&[&id]).await?["deleted"],true);
        Ok(())
    }
}
