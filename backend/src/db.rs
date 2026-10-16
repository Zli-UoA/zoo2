use crate::models::{Channel, ChannelUser, Message, Role, User, UserIdentity, UserRole};

pub async fn init_db(database_url: &str) -> Result<toasty::Db, toasty::Error> {
    tracing::info!("Connecting to PostgreSQL via Toasty ORM...");

    let db = toasty::Db::builder()
        .models(toasty::models!(
            User,
            UserIdentity,
            Channel,
            Message,
            ChannelUser,
            Role,
            UserRole
        ))
        .connect(database_url)
        .await?;

    tracing::info!("Successfully connected to PostgreSQL database");
    Ok(db)
}

/// 同一トランザクション内で pg_current_xact_id()::xid::text を実行し、txid を取得する
pub async fn get_current_txid(
    tx: &mut toasty::Transaction<'_>,
) -> Result<String, crate::error::AppError> {
    let rows = toasty::sql::query("SELECT pg_current_xact_id()::xid::text AS txid")
        .exec(tx)
        .await
        .map_err(crate::error::AppError::from)?;

    for row in rows {
        if let toasty::stmt::Value::Record(rec) = row
            && let Some(toasty::stmt::Value::String(txid)) = rec.fields.into_iter().next()
        {
            return Ok(txid);
        }
    }

    Err(crate::error::AppError::Internal(anyhow::anyhow!(
        "Failed to retrieve txid from transaction"
    )))
}
