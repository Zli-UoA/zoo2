use backend::{config::Config, slack::sync::claim_sql};
use serde_json::Value;
use tokio_postgres::{NoTls, Transaction};

async fn selected(tx: &Transaction<'_>, wake: bool) -> anyhow::Result<Option<Value>> {
    let token = uuid::Uuid::new_v4().to_string();
    let no_team = String::new();
    Ok(tx
        .query(&claim_sql(wake), &[&token, &no_team])
        .await?
        .first()
        .map(|r| serde_json::from_str(&r.get::<_, String>(0)).unwrap()))
}
#[tokio::test]
async fn split_claims_preserve_wake_priority_backoff_leases_and_eligibility() -> anyhow::Result<()>
{
    let (mut client, connection) =
        tokio_postgres::connect(&Config::from_env().database_url, NoTls).await?;
    tokio::spawn(connection);
    let tx = client.transaction().await?;
    // This transaction rolls back other fixtures' queue entries along with ours.
    tx.batch_execute("DELETE FROM slack_channel_syncs").await?;
    let id = uuid::Uuid::new_v4().to_string();
    let encrypted = format!("{id}-encrypted");
    tx.execute(
        "INSERT INTO users(id,user_name,display_name) VALUES($1::text,$1::text,$1::text)",
        &[&id],
    )
    .await?;
    tx.execute("INSERT INTO channels(id,channel_name,created_user_id,is_private,encrypted) VALUES($1::text,$1::text,$1::text,false,false),($2::text,$2::text,$1,true,true)", &[&id,&encrypted]).await?;
    tx.execute("INSERT INTO slack_channels(team_id,slack_channel_id,channel_id,last_ts) VALUES($1,'due',$1,'1'),($1,'wake',$1,'1')", &[&id]).await?;
    tx.execute("INSERT INTO slack_channel_syncs(team_id,slack_channel_id,available_at) VALUES($1,'due','2020-01-01'),($1,'wake','2035-01-01')", &[&id]).await?;
    tx.execute("INSERT INTO slack_channel_wakes(team_id,slack_channel_id,first_at) VALUES($1,'wake',now()-interval '10 seconds')", &[&id]).await?;
    assert_eq!(
        selected(&tx, true).await?.unwrap()["slack_channel_id"],
        "wake",
        "mature wake precedes the earlier scheduled task"
    );
    assert!(
        selected(&tx, true).await?.is_none(),
        "a leased wake is not claimed twice"
    );
    assert_eq!(
        selected(&tx, false).await?.unwrap()["slack_channel_id"],
        "due"
    );
    tx.execute("UPDATE slack_channel_syncs SET lease_until=NULL,not_before=now()+interval '1 hour' WHERE team_id=$1 AND slack_channel_id='wake'", &[&id]).await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "wake cannot bypass a 429 delay"
    );
    tx.execute("UPDATE slack_channel_syncs SET not_before=NULL,last_synced_at=now() WHERE team_id=$1 AND slack_channel_id='wake'", &[&id]).await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "minimum sync interval remains enforced"
    );
    tx.execute("UPDATE slack_channel_syncs SET last_synced_at=now()-interval '30 seconds' WHERE team_id=$1 AND slack_channel_id='wake'", &[&id]).await?;
    tx.execute(
        "UPDATE slack_channel_wakes SET first_at=now() WHERE team_id=$1",
        &[&id],
    )
    .await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "wake debounce remains enforced"
    );
    tx.execute(
        "UPDATE slack_channel_wakes SET first_at=now()-interval '3 seconds' WHERE team_id=$1",
        &[&id],
    )
    .await?;
    tx.execute(
        "UPDATE slack_channel_syncs SET running=true WHERE team_id=$1 AND slack_channel_id='wake'",
        &[&id],
    )
    .await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "continuations do not take wake priority"
    );
    tx.execute(
        "UPDATE slack_channel_syncs SET running=false WHERE team_id=$1 AND slack_channel_id='wake'",
        &[&id],
    )
    .await?;
    tx.execute(
        "UPDATE slack_channels SET channel_id=$2 WHERE team_id=$1 AND slack_channel_id='wake'",
        &[&id, &encrypted],
    )
    .await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "encrypted channels stay excluded"
    );
    tx.execute("UPDATE slack_channels SET channel_id=$1,last_ts=NULL WHERE team_id=$1 AND slack_channel_id='wake'", &[&id]).await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "unimported channels do not fetch all history"
    );
    tx.execute(
        "UPDATE slack_channels SET last_ts='1' WHERE team_id=$1 AND slack_channel_id='wake'",
        &[&id],
    )
    .await?;
    tx.execute("INSERT INTO slack_import_jobs(id,created_by,status,file_name,file_size,storage_key,team_id) VALUES($1,$1,'analyzing','fixture',0,'fixture',$1)", &[&id]).await?;
    assert!(
        selected(&tx, true).await?.is_none(),
        "an active import pauses wake work"
    );
    tx.execute("UPDATE slack_channel_syncs SET lease_until=NULL WHERE team_id=$1 AND slack_channel_id='due'", &[&id]).await?;
    assert!(
        selected(&tx, false).await?.is_none(),
        "an active import pauses scheduled work too"
    );
    tx.execute(
        "UPDATE slack_import_jobs SET status='fetching_files' WHERE id=$1",
        &[&id],
    )
    .await?;
    assert_eq!(
        selected(&tx, true).await?.unwrap()["slack_channel_id"],
        "wake",
        "file fetching does not block channel sync"
    );
    tx.rollback().await?;
    Ok(())
}
