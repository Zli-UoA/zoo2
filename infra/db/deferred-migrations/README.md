# Deferred migrations

`20261201010010_guest_membership_lock_order.sql` replaces
`app_guest_membership_rules()` with the version that always locks the `users`
row (`FOR NO KEY UPDATE`) before checking the single-channel guest rule.

It is intentionally outside `infra/db/migrations` so the standard
`migrate-deploy` image and `compose.stack` migration job do not apply it while
old backend instances are still running. Deploy the new backend (which takes
the same user-row lock in `lock_membership_users` before its member advisory
locks) first. After all old instances have stopped, apply this directory:

```sh
atlas migrate apply --dir file://infra/db/deferred-migrations   --url "$DATABASE_URL" --revisions-schema atlas_schema_revisions_deferred --allow-dirty
```

`--allow-dirty` is required because the target database already carries the
canonical schema and `atlas_schema_revisions`; without it Atlas rejects the
apply with "connected database is not clean". Do not substitute
`--baseline`, which would mark the version as applied without running the
`CREATE OR REPLACE FUNCTION`.

`--revisions-schema` records this directory's progress in its own revision
table (`atlas_schema_revisions_deferred`). Without it, the deferred version
would become the "current" revision of the shared `atlas_schema_revisions`
table, and a later canonical migration could collide with that version number
or be skipped as already applied. The canonical history stays untouched, so
`migrate status` on `infra/db/migrations` keeps reporting only canonical
migrations. The follow-up that moves this file into `infra/db/migrations`
applies the same idempotent `CREATE OR REPLACE FUNCTION` again in the canonical
history and regenerates `atlas.sum`; the deferred revision table can then be
dropped together with the moved file.

## 段階デプロイ中の平文 1 対 1 DM

旧バックエンドと新バックエンドが混在するロールアウト中に、`single` ゲストの
平文 1 対 1 DM を並行して作成・復帰すると、`channels` と `users` のロック順の
違いから PostgreSQL のデッドロック (`deadlock_detected`, SQLSTATE 40P01) が
一時的に発生することがある。データは壊れず、新バックエンドは 1 対 1 DM の
トランザクションで 40P01 を検出すると、そのトランザクションを破棄して 1 回だけ
再試行する。旧バックエンドが処理した要求は、ロールアウトが完了するまで一時的に
失敗し得るため、クライアントの再試行に任せること。

## 適用後のロールバック制約

この移行を適用した DB では、`app_guest_membership_rules()` は single 以外の
ゲストでも常時 `users` 行をロックする。この状態で**旧バックエンド**
（`lock_members` が member advisory lock を先に取り、`users` を後からロックする版）
へロールバックすると、適用を遅らせたのと同じデッドロック (SQLSTATE 40P01) が
再現する。バックエンドを旧版へ戻す必要がある場合は、先にこの移行の効果を戻すこと。
同じ DB に対して canonical 側の条件付き定義
（`infra/db/migrations/20261201010001_guest_membership_no_key_update.sql` の
`CREATE OR REPLACE FUNCTION app_guest_membership_rules()`）を再実行すれば、
トリガー定義は常にロックしない版へ戻り、`atlas_schema_revisions_deferred` の
記録はそのまま残る（記録の削除は不要）。ただし、`20261201010010` の適用記録が
残った状態では `atlas migrate apply --allow-dirty` を再実行しても既適用とみなされ
SQL が再実行されないため、切り戻し後に新バックエンドへ再移行するときは、旧版が
すべて停止したことを確認してから `infra/db/deferred-migrations/20261201010010_guest_membership_lock_order.sql`
の `CREATE OR REPLACE FUNCTION app_guest_membership_rules()` を直接実行し、
`SELECT pg_get_functiondef('app_guest_membership_rules()'::regprocedure);` で常時
`users` 行をロックする定義に戻ったことを確認すること。バックエンドのロールバック中は、
暗号化チャンネルへの所属追加とユーザー無効化が並行して 40P01 で中止され得る
点も `infra/README.md` の段階デプロイの節と同様に扱うこと。

A staging Compose procedure for this command is in `infra/README.md` (the
standard `migrate` image does not contain this directory and the PostgreSQL
port is not published to the host).
