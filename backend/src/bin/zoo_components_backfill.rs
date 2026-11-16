//! zoo_components_backfill: 旧形式のカードとブロックを、メッセージ部品（`messages.components` など）に移す 1 回限りの処理。
//!
//! 既定は dry-run で、何も書かずに件数だけを出す。`--apply` を付けたときだけ書き込む。
//! 設定は backend と同じ環境変数（`DATABASE_URL` または `POSTGRES_*`）。
//!
//! ```text
//! zoo_components_backfill                       件数だけを出す（何も書かない）
//! zoo_components_backfill --apply > backup.jsonl   書き込む。変更前後を JSON Lines で標準出力に出す
//! zoo_components_backfill --restore backup.jsonl   backup.jsonl の変更を元に戻す（`-` なら標準入力から読む）
//! ```
//!
//! 出力には投稿の内容が入るので、リポジトリや PR には置かない。Electric の shape への更新が増えるので、
//! 利用の少ない時間に流す。
//!
//! クライアントが部品を描画できるようになってから（Zli-UoA/zoo2#600 を出したあと）流す。`app_ui` の文書は
//! 旧形式を残さず書き換えるので、先に流すと、旧版の Web にはホーム・モーダル・一時メッセージが表示されない。
use backend::components::backfill;
use std::io::Write;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut apply = false;
    let mut restore: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--apply" => apply = true,
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
            .map(backfill::from_line)
            .collect::<anyhow::Result<Vec<_>>>()?;
        let restored = backfill::restore(&db, &changes).await?;
        eprintln!("restored {restored} of {} rows", changes.len());
        return Ok(());
    }
    let report = backfill::run(&db, apply, |change| {
        let mut out = std::io::stdout().lock();
        writeln!(out, "{}", backfill::to_line(change))?;
        out.flush()
    })
    .await?;
    eprintln!(
        "{}: scanned={} changed={} applied={} skipped={}",
        if apply { "applied" } else { "dry-run" },
        report.scanned,
        report.changed,
        report.applied,
        report.skipped
    );
    Ok(())
}
