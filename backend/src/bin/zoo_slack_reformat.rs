//! zoo-slack-reformat: 取り込み済みの Slack の投稿に、書式 (`*太字*` など) の変換を当てる 1 回限りの修正。
//!
//! 既定は dry-run で、何も書かずに件数と、変更前後の例を出す。`--apply` を付けたときだけ書き込む。
//! 設定は backend と同じ環境変数 (`DATABASE_URL` または `POSTGRES_*`)。
//!
//! ```text
//! zoo_slack_reformat [--samples N]            件数と例だけを出す (何も書かない)
//! zoo_slack_reformat --channel <Zoo のチャンネル ID> ...   そのチャンネルだけを対象にする (どの形でも付けられる)
//! zoo_slack_reformat --apply > backup.jsonl   書き込む。変更前後を JSON Lines で標準出力に出す
//! zoo_slack_reformat --restore backup.jsonl   backup.jsonl の変更を元に戻す (`-` なら標準入力から読む)
//! ```
//!
//! 標準出力は JSON Lines (`{"id","old","new"}`)、件数などの報告は標準エラーに出す。
//! 出力には投稿の本文が入るので、リポジトリや PR には置かない。
use backend::slack::reformat::{self, Change, Options};
use std::io::Write;

fn parse(line: &str) -> anyhow::Result<Change> {
    let value: serde_json::Value = serde_json::from_str(line)?;
    let field = |name: &str| {
        value[name]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("missing {name}"))
    };
    Ok(Change {
        id: field("id")?,
        old: field("old")?,
        new: field("new")?,
    })
}

fn print(change: &Change) {
    let line = serde_json::json!({"id": change.id, "old": change.old, "new": change.new});
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut apply = false;
    let mut samples = 10;
    let mut restore: Option<String> = None;
    let mut channel: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => apply = true,
            "--samples" => {
                samples = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("--samples needs a number"))?;
            }
            "--channel" => {
                channel = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--channel needs a channel id"))?,
                );
            }
            "--restore" => {
                restore = Some(
                    args.next()
                        .ok_or_else(|| anyhow::anyhow!("--restore needs a file"))?,
                );
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let config = backend::config::Config::from_env();
    let db = backend::db::init_db(&config.database_url).await?;

    if let Some(path) = restore {
        let text = if path == "-" {
            std::io::read_to_string(std::io::stdin())?
        } else {
            std::fs::read_to_string(&path)?
        };
        let changes = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(parse)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let restored = reformat::restore(&db, &changes).await?;
        eprintln!("restored {restored} of {} messages", changes.len());
        return Ok(());
    }

    let report = reformat::run(
        &db,
        &Options {
            apply,
            samples,
            channel,
        },
        |change| {
            if apply {
                print(change);
            }
        },
    )
    .await?;
    if !apply {
        for change in &report.samples {
            print(change);
        }
    }
    eprintln!(
        "{}: channels={} candidates={} changed={} applied={} raced={}",
        if apply { "applied" } else { "dry-run" },
        report.channels,
        report.candidates,
        report.changed,
        report.applied,
        report.raced
    );
    Ok(())
}
