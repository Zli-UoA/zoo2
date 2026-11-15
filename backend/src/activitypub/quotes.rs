//! One-level, origin-verified quote cards delivered with the Home page, without per-card APIs.
use super::*;

type QuoteFlights = std::collections::HashMap<String, std::sync::Weak<tokio::sync::Mutex<()>>>;
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
    let task = async move {
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

async fn lock_quote(uri: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let mutex = {
        let mut flights = FLIGHTS.lock().await;
        flights.retain(|_, weak| weak.strong_count() > 0);
        let entry = flights.entry(uri.to_owned()).or_default();
        if let Some(mutex) = entry.upgrade() {
            mutex
        } else {
            let mutex = std::sync::Arc::new(tokio::sync::Mutex::new(()));
            *entry = std::sync::Arc::downgrade(&mutex);
            mutex
        }
    };
    mutex.lock_owned().await
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
    if known_original(&mut tx, uri).await? {
        return Ok(None);
    }
    Ok(Some(uri.to_owned()))
}

async fn known_original(tx: &mut toasty::Transaction<'_>, uri: &str) -> Result<bool, AppError> {
    Ok(!query(tx, "SELECT '{}'::text FROM apub_remote_posts WHERE id=$1 AND origin_verified UNION ALL SELECT '{}'::text FROM apub_remote_tombstones WHERE id=$1", &[uri]).await?.is_empty())
}

async fn resolve(state: &AppState, user: &str, uri: &str) -> Result<(), AppError> {
    let mut db = state.db.clone();
    // Multiple inbox recipients share the same public original. Do not hold a
    // database connection while waiting for another recipient's fetch.
    let _guard = lock_quote(uri).await;
    let mut tx = db.transaction().await?;
    if known_original(&mut tx, uri).await? {
        return Ok(());
    }
    drop(tx);
    // Reuse the Announce resolver's signed HTTPS GET, identity and audience checks.
    // Do not resolve quotes on the fetched original: nesting and cycles stop here.
    reposts::resolve_public_note(state, user, uri).await
}

pub(crate) async fn decorate(
    tx: &mut toasty::Transaction<'_>,
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
    let quotes = query(tx, r#"WITH refs AS (
      SELECT id,quote_uri FROM apub_remote_posts WHERE quote_uri IS NOT NULL AND id IN (SELECT jsonb_array_elements_text($1::jsonb))
    ) SELECT json_build_object('id',r.id,'uri',r.quote_uri,'local',starts_with(r.quote_uri,$2 || '/api/activitypub/notes/'),'post',COALESCE(remote.post,local.post))::text FROM refs r
    LEFT JOIN LATERAL (
      SELECT json_build_object('id',p.id,'content',p.content,'user_name',a.user_name || '@' || split_part(a.id,'/',3),'display_name',a.display_name,'created_at',p.published,'emojis',p.emojis,'actor_emojis',a.emojis,'avatar',a.avatar,'media',p.media,'remote',true,'user_id',p.actor_id) AS post
      FROM apub_remote_posts p JOIN apub_actors a ON a.id=p.actor_id
      WHERE p.id=r.quote_uri AND p.origin_verified AND p.deleted_at IS NULL AND NOT a.blocked AND p.visibility IN ('public','unlisted')
    ) remote ON true
    LEFT JOIN LATERAL (
      SELECT json_build_object('id',p.id,'content',p.content,'user_name',u.user_name,'display_name',COALESCE(profile.display_name,u.display_name),'created_at',p.created_at,'emojis',COALESCE((SELECT jsonb_agg(jsonb_build_object('name',':'||e.key||':','url',e.value->>'url','image_id',COALESCE(e.value->>'image_id',''),'local',true)) FROM jsonb_each(message_content_emojis(p.content)) e),'[]'::jsonb),'avatar_url',u.avatar_url,'media',p.media,'channel_id',p.channel_id,'remote',false,'user_id',p.user_id) AS post
      FROM home_posts p JOIN users u ON u.id=p.user_id LEFT JOIN apub_profiles profile ON profile.user_id=u.id LEFT JOIN channels c ON c.id=p.channel_id
      WHERE p.id=CASE WHEN starts_with(r.quote_uri,$2 || '/api/activitypub/notes/') THEN substr(r.quote_uri,length($2 || '/api/activitypub/notes/')+1) END AND p.apub AND p.deleted_at IS NULL AND p.visibility IN ('public','unlisted') AND (p.channel_id IS NULL OR NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL)
    ) local ON true"#, &[&json!(ids).to_string(),&base]).await?;
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
        let stored = one(&mut tx,"SELECT json_build_object('content',content,'verified',origin_verified)::text FROM apub_remote_posts WHERE id=$1",&[&id]).await?;
        assert_eq!(stored["content"], "Canonical original");
        assert_eq!(stored["verified"], true);
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
