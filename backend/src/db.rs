use crate::models::{Channel, ChannelUser, Message, Role, User, UserRole};

pub async fn init_db(database_url: &str) -> Result<toasty::Db, toasty::Error> {
    tracing::info!("Connecting to PostgreSQL via Toasty ORM...");

    let db = toasty::Db::builder()
        .models(toasty::models!(
            User,
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
