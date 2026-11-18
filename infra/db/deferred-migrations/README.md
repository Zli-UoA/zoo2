# Deferred migrations

`20261129000009_guest_membership_lock_order.sql` replaces
`app_guest_membership_rules()` with the version that always locks the `users`
row (`FOR NO KEY UPDATE`) before checking the single-channel guest rule.

It is intentionally outside `infra/db/migrations` so the standard
`migrate-deploy` image and `compose.stack` migration job do not apply it while
old backend instances are still running. Deploy the new backend (which takes
the same user-row lock in `lock_membership_users` before its member advisory
locks) first. After all old instances have stopped, apply this directory:

```sh
atlas migrate apply --dir file://infra/db/deferred-migrations   --url "$DATABASE_URL" --revisions-schema atlas_schema_revisions_deferred
```

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

A staging Compose procedure for this command is in `infra/README.md` (the
standard `migrate` image does not contain this directory and the PostgreSQL
port is not published to the host).
