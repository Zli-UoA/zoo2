# DB の ER 図

`infra/db/schema.sql` から自動生成しています。直接編集しないでください。

スキーマを変えたら `infra/db/erd.sh` を実行し、更新された `docs/db-erd.md` と `docs/db-erd/` を commit してください。
CI(`ci-db-erd`)が `infra/db/erd.sh --check` で更新漏れを検査します。

図が大きくなりすぎるので、テーブル名の接頭辞で機能ごとに分けています。
別の図のテーブルを参照する外部キーは、相手のテーブルが属性なしの箱で描かれます。

| 図 | 内容 | テーブル数 |
|---|---|---|
| [core](db-erd/core.md) | コア(ユーザー、チャンネル、メッセージ、通知など) | 37 |
| [apps](db-erd/apps.md) | アプリ連携(apps と app_*) | 22 |
| [activitypub](db-erd/activitypub.md) | ActivityPub(apub_*) | 17 |
| [e2ee](db-erd/e2ee.md) | E2EE(e2ee_*) | 16 |
| [slack](db-erd/slack.md) | Slack 取り込み(slack_*) | 6 |
