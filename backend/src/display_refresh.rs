//! Metadata fan-out runs after commit, one cached row per transaction. Normal
//! message writes keep their synchronous joined projection and never take a
//! global metadata lock. Durable jobs also survive backend restarts.
use crate::{apps::query, error::AppError, state::AppState};

async fn run_one(state: &AppState, scope: bool) -> Result<bool, AppError> {
    let mut db = state.db.clone();
    let mut tx = db.transaction().await?;
    // Reserve a runnable job before choosing its timeout. Retries get a larger
    // budget so an expensive row can eventually finish without pinning the queue.
    let (predicate, initial_ms, maximum_ms, function) = if scope {
        ("NOT IN", 5000_u32, 30000_u32, "expand_next_display_scope")
    } else {
        ("IN", 200_u32, 5000_u32, "refresh_next_display")
    };
    toasty::sql::statement(format!("SET LOCAL statement_timeout='{initial_ms}ms'"))
        .exec(&mut tx)
        .await?;
    let jobs = query(
        &mut tx,
        &format!("SELECT json_build_object('attempts',attempts)::text FROM display_refresh_queue WHERE kind {predicate} ('message','notification') AND retry_at<=statement_timestamp() ORDER BY retry_at,kind,target,generation LIMIT 1 FOR UPDATE SKIP LOCKED"),
        &[],
    )
    .await?;
    let Some(job) = jobs.first() else {
        tx.commit().await?;
        return Ok(false);
    };
    let attempts = job["attempts"].as_u64().unwrap_or(0).min(8) as u32;
    let timeout_ms = (initial_ms * 2_u32.pow(attempts)).min(maximum_ms);
    toasty::sql::statement(format!("SET LOCAL statement_timeout='{timeout_ms}ms'"))
        .exec(&mut tx)
        .await?;
    let rows = query(
        &mut tx,
        &format!("SELECT json_build_object('processed',{function}())::text"),
        &[],
    )
    .await?;
    tx.commit().await?;
    Ok(rows.first().is_some_and(|row| row["processed"] == true))
}

pub async fn refresh_one(state: &AppState) -> Result<bool, AppError> {
    // An expansion failure must not prevent independent row jobs from running.
    let expanded = match run_one(state, true).await {
        Ok(expanded) => expanded,
        Err(error) => {
            tracing::warn!(%error, "Display scope expansion will be retried");
            false
        }
    };
    let refreshed = run_one(state, false).await?;
    Ok(expanded || refreshed)
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
