//! Metadata fan-out runs after commit, one cached row per transaction. Normal
//! message writes keep their synchronous joined projection and never take a
//! global metadata lock. Durable jobs also survive backend restarts.
use crate::{apps::query, error::AppError, state::AppState};

pub async fn refresh_one(state: &AppState) -> Result<bool, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // A busy notification or concurrent writer cannot pin this worker forever.
    // On timeout the transaction rolls back and the durable job remains queued.
    toasty::sql::statement("SET LOCAL statement_timeout='200ms'")
        .exec(&mut tx)
        .await?;
    let rows = query(
        &mut tx,
        "SELECT json_build_object('refreshed',refresh_next_display())::text",
        &[],
    )
    .await?;
    tx.commit().await?;
    Ok(rows.first().is_some_and(|row| row["refreshed"] == true))
}

pub async fn worker(state: AppState) {
    loop {
        match refresh_one(&state).await {
            Ok(true) => tokio::task::yield_now().await,
            Ok(false) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
            Err(error) => {
                tracing::warn!(%error, "Display refresh will be retried");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        }
    }
}
