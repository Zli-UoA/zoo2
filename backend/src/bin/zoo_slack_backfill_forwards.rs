//! zoo_slack_backfill_forwards: 過去に取り込み済みの Slack チャンネルを API で再走査し、
//! 転送・引用（`is_share` / `is_msg_unfurl`）の `forwarded_message_ids` 補完と
//! コメントなし転送の取り込み漏れを冪等に修復する 1 回限りの運用コマンド。
//!
//! 既定は dry-run で何も書き込まない。`--apply` を指定したときだけ DB を更新・挿入する。
//!
//! ```text
//! zoo_slack_backfill_forwards [--apply] [--channel <zoo_channel_id|slack_channel_id>] [--oldest <slack_ts>] [--max-pages <N>]
//! ```

use backend::slack::backfill_forwards::{Options, run};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut opts = Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => opts.apply = true,
            "--channel" => {
                opts.channel = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--channel requires a value"))?,
                );
            }
            "--oldest" => {
                let ts = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--oldest requires a Slack timestamp"))?;
                if !backend::slack::is_valid_ts(&ts) {
                    anyhow::bail!("--oldest is not a valid Slack timestamp: {ts}");
                }
                opts.oldest = Some(ts);
            }
            "--max-pages" => {
                let raw = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--max-pages requires a positive integer"))?;
                let n: usize = raw
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --max-pages value '{raw}': {e}"))?;
                if n == 0 {
                    anyhow::bail!("--max-pages must be at least 1");
                }
                opts.max_pages = Some(n);
            }
            "-h" | "--help" => {
                eprintln!(
                    "Usage: zoo_slack_backfill_forwards [--apply] [--channel <id>] [--oldest <ts>] [--max-pages <N>]"
                );
                return Ok(());
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let _telemetry = backend::telemetry::init(
        "zoo-slack-backfill-forwards",
        "backend=info,zoo_slack_backfill_forwards=info,info",
    );
    let config = backend::config::Config::from_env();
    let db = backend::db::init_db(&config.database_url).await?;
    let report = run(&db, &config, &opts).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
