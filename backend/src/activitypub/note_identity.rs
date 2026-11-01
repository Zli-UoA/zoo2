//! Resolve cross-actor ID collisions against the Note's HTTPS origin, never its first claimant.
use super::*;
use std::future::Future;
pub(super) struct Proof {
    id: String,
    actor: String,
}
impl Proof {
    pub(super) fn from_note(note: &Value, id: &str) -> Result<Self, AppError> {
        let actor = string(note, "attributedTo")?;
        if note["id"] != id || note["type"] != "Note" || !same_origin(id, actor) {
            return Err(AppError::BadGateway(
                "Canonical Note identity mismatch".into(),
            ));
        }
        Ok(Self {
            id: id.to_owned(),
            actor: actor.to_owned(),
        })
    }
    fn matches(&self, id: &str, actor: &str) -> bool {
        self.id == id && self.actor == actor
    }
}
pub(super) async fn proof_for_activity(
    state: &AppState,
    user: &str,
    activity: &Value,
) -> Result<Option<Proof>, AppError> {
    proof_with_fetch(state, activity, |id| async move {
        transport::fetch_json(state, Some(user), &id).await
    })
    .await
}
async fn proof_with_fetch<F, Fut>(
    state: &AppState,
    activity: &Value,
    fetch: F,
) -> Result<Option<Proof>, AppError>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<Value, AppError>>,
{
    if !matches!(activity["type"].as_str(), Some("Create" | "Update"))
        || activity["object"]["type"] != "Note"
    {
        return Ok(None);
    }
    let id = string(&activity["object"], "id")?;
    let actor = string(activity, "actor")?;
    if !same_origin(id, actor) || activity["object"]["attributedTo"] != actor {
        return Err(AppError::BadRequest("Invalid note attribution".into()));
    }
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    let existing=query(&mut tx,"SELECT json_build_object('actor_id',actor_id,'origin_verified',origin_verified)::text FROM apub_remote_posts WHERE id=$1",&[id]).await?;
    let Some(row) = existing.first() else {
        return Ok(None);
    };
    if row["actor_id"] == actor {
        return Ok(None);
    }
    if row["origin_verified"] == true {
        return Err(AppError::Forbidden(
            "Note ID belongs to another actor".into(),
        ));
    }
    drop(tx); // Never hold a DB connection while a signed GET obtains signing keys.
    let proof = Proof::from_note(&fetch(id.to_owned()).await?, id)?;
    if !proof.matches(id, actor) {
        return Err(AppError::Forbidden(
            "Note ID belongs to another actor".into(),
        ));
    }
    Ok(Some(proof))
}
pub(super) async fn lock(tx: &mut toasty::Transaction<'_>, id: &str) -> Result<(), AppError> {
    query(tx,"SELECT '{}'::text FROM (SELECT pg_advisory_xact_lock(hashtextextended('note-identity:'||$1,0))) locked",&[id]).await?;
    Ok(())
}
pub(super) async fn reconcile(
    tx: &mut toasty::Transaction<'_>,
    id: &str,
    actor: &str,
    proof: Option<&Proof>,
) -> Result<bool, AppError> {
    lock(tx, id).await?;
    let existing=query(tx,"SELECT json_build_object('actor_id',actor_id,'origin_verified',origin_verified)::text FROM apub_remote_posts WHERE id=$1 FOR UPDATE",&[id]).await?;
    let verified = proof.is_some_and(|p| p.matches(id, actor));
    let Some(row) = existing.first() else {
        return Ok(verified);
    };
    if row["actor_id"] == actor {
        if verified {
            toasty::sql::statement(
                "UPDATE apub_remote_posts SET origin_verified=true WHERE id=$1 AND actor_id=$2",
            )
            .bind(id)
            .bind(actor)
            .exec(tx)
            .await?;
        }
        return Ok(verified || row["origin_verified"] == true);
    }
    if row["origin_verified"] == true {
        return Err(AppError::Forbidden(
            "Note ID belongs to another actor".into(),
        ));
    }
    if !verified {
        return Err(AppError::BadGateway(
            "Retry Note identity verification".into(),
        ));
    }
    // Recipient grants and actions on the impostor must not transfer to the real author's Note.
    for sql in [
        "DELETE FROM apub_reactions WHERE post_id=$1",
        "DELETE FROM apub_reposts WHERE post_id=$1",
        "DELETE FROM apub_notifications WHERE post_id=$1",
        "DELETE FROM apub_remote_posts WHERE id=$1",
    ] {
        toasty::sql::statement(sql).bind(id).exec(&mut *tx).await?;
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn origin_proof_requires_exact_note_and_same_origin_author() {
        let id = "https://remote.example/notes/1";
        assert!(
            Proof::from_note(
                &json!({"id":id,"type":"Note","attributedTo":"https://remote.example/users/b"}),
                id
            )
            .unwrap()
            .matches(id, "https://remote.example/users/b")
        );
        for note in [
            json!({"id":"https://remote.example/notes/2","type":"Note","attributedTo":"https://remote.example/users/b"}),
            json!({"id":id,"type":"Note","attributedTo":"https://other.example/users/b"}),
            json!({"id":id,"type":"Person","attributedTo":"https://remote.example/users/b"}),
        ] {
            assert!(Proof::from_note(&note, id).is_err());
        }
    }
    #[tokio::test]
    #[ignore = "requires migrated PostgreSQL and ACTIVITYPUB_BASE_URL"]
    async fn canonical_origin_recovers_a_note_id_without_transferring_recipient_grants()
    -> anyhow::Result<()> {
        let config = crate::config::Config::from_env();
        let mut db = crate::db::init_db(&config.database_url).await?;
        let state = AppState::new(db.clone(), config);
        let user = uuid::Uuid::new_v4().to_string();
        let impostor = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
        let real = format!("https://remote.example/users/{}", uuid::Uuid::new_v4());
        let id = format!("https://remote.example/notes/{}", uuid::Uuid::new_v4());
        toasty::sql::statement("INSERT INTO users(id,user_name,display_name) VALUES($1,$1,$1)")
            .bind(user.as_str())
            .exec(&mut db)
            .await?;
        for actor in [&impostor, &real] {
            toasty::sql::statement("INSERT INTO apub_actors(id,inbox,followers_url,user_name,display_name,key_id,public_key) VALUES($1,$1,$1,'peer','Peer',$1,'test')").bind(actor.as_str()).exec(&mut db).await?;
        }
        toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at) VALUES($1,$2,'Impostor','public',now(),now())").bind(id.as_str()).bind(impostor.as_str()).exec(&mut db).await?;
        toasty::sql::statement("INSERT INTO apub_remote_recipients(post_id,user_id) VALUES($1,$2)")
            .bind(id.as_str())
            .bind(user.as_str())
            .exec(&mut db)
            .await?;
        let activity = json!({"type":"Create","actor":real,"object":{"id":id,"type":"Note","attributedTo":real}});
        let canonical = json!({"id":id,"type":"Note","attributedTo":real});
        let expected_id = id.clone();
        let proof = proof_with_fetch(&state, &activity, |url| async move {
            assert_eq!(url, expected_id);
            Ok(canonical)
        })
        .await?
        .unwrap();
        let mut tx = db.transaction().await?;
        assert!(reconcile(&mut tx, &id, &real, Some(&proof)).await?);
        assert!(
            query(
                &mut tx,
                "SELECT '{}'::text FROM apub_remote_recipients WHERE post_id=$1",
                &[&id]
            )
            .await?
            .is_empty()
        );
        toasty::sql::statement("INSERT INTO apub_remote_posts(id,actor_id,content,visibility,published,updated_at,origin_verified) VALUES($1,$2,'Real','public',now(),now(),true)").bind(id.as_str()).bind(real.as_str()).exec(&mut tx).await?;
        tx.commit().await?;
        let forged = json!({"type":"Create","actor":impostor,"object":{"id":id,"type":"Note","attributedTo":impostor}});
        assert!(
            proof_with_fetch(&state, &forged, |_| async {
                panic!("verified origin ownership must not be replaced by another Actor")
            })
            .await
            .is_err()
        );
        let mut tx = db.transaction().await?;
        let proof = Proof::from_note(&json!({"id":id,"type":"Note","attributedTo":impostor}), &id)?;
        assert!(
            reconcile(&mut tx, &id, &impostor, Some(&proof))
                .await
                .is_err()
        );
        drop(tx);
        toasty::sql::statement("DELETE FROM apub_remote_posts WHERE id=$1")
            .bind(id)
            .exec(&mut db)
            .await?;
        toasty::sql::statement("DELETE FROM users WHERE id=$1")
            .bind(user)
            .exec(&mut db)
            .await?;
        Ok(())
    }
}
