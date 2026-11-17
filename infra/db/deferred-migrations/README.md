# Deferred migrations

`20261129000008_guest_membership_lock_order.sql` replaces
`app_guest_membership_rules()` with the version that always locks the `users`
row (`FOR NO KEY UPDATE`) before checking the single-channel guest rule.

It is intentionally outside `infra/db/migrations` so the standard
`migrate-deploy` image and `compose.stack` migration job do not apply it while
old backend instances are still running. Deploy the new backend (which takes
the same user-row lock in `lock_membership_users` before its member advisory
locks) first. After all old instances have stopped, apply this directory:

```sh
atlas migrate apply --dir file://infra/db/deferred-migrations --url "$DATABASE_URL"
```

A follow-up change should move this file into `infra/db/migrations` with a
version after the latest canonical migration and regenerate `atlas.sum`.
