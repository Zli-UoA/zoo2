# Historical migrations

Files here are retained for reference only and are not part of Atlas's canonical
`infra/db/migrations` directory. In particular, some branch databases applied
`20261123000001_channel_notes_expiry_index.sql` before main reached migration
`20261126000000`; keeping it in the canonical directory makes Atlas reject main
history as out of order. The later canonical migration
`20261128000002_channel_notes_expiry_index.sql` drops the conflicting index by
name and recreates it with the current `(channel_id, deleted_at, id)` order.
This preserves both applied histories and their stored checksums while fixing
the old index; `IF NOT EXISTS` alone would leave the old definition in place.

## Reproducing the branch histories

The regression was checked on an isolated PostgreSQL 16.15 container with
Atlas `v1.3.9-1824371-canary`. Use a disposable PostgreSQL instance and create
three empty databases (`notes_case1`, `notes_case2`, `notes_case3`):

```sql
CREATE DATABASE notes_case1;
CREATE DATABASE notes_case2;
CREATE DATABASE notes_case3;
CREATE DATABASE notes_schema_dev1;
CREATE DATABASE notes_schema_dev2;
CREATE DATABASE notes_schema_dev3;
```

Build temporary migration directories from `infra/db/migrations/*.sql`. For
example, this shell loop keeps the legacy history through the old expiry
revision for case 1 (use cutoff `20261126010000` for case 2):

```sh
legacy_dir=/tmp/zoo2-notes-legacy
mkdir -p "$legacy_dir"
cp infra/db/migrations/*.sql "$legacy_dir/"
cp infra/db/legacy-migrations/20261123000001_channel_notes_expiry_index.sql "$legacy_dir/"
for path in "$legacy_dir"/2026*.sql; do
  version=${path##*/}
  version=${version%%_*}
  if [[ "$version" -gt "20261123000001" ]]; then
    rm "$path"
  fi
done
rm -f "$legacy_dir/atlas.sum"
atlas migrate hash --dir "file://$legacy_dir"
```

For case 2, replace the comparison version in the loop with
`20261126010000`. Use fresh scratch directories for each cutoff. Apply the
legacy history to the matching URL, then apply the canonical history:

```sh
atlas migrate apply --dir "file://$legacy_dir" --url "$NOTES_CASE1_URL"
atlas migrate apply --dir file://infra/db/migrations --url "$NOTES_CASE1_URL"
atlas migrate apply --dir "file://$legacy_dir" --url "$NOTES_CASE2_URL"
atlas migrate apply --dir file://infra/db/migrations --url "$NOTES_CASE2_URL"
atlas migrate apply --dir file://infra/db/migrations --url "$NOTES_CASE3_URL"
```

Set NOTES_CASE1_URL, NOTES_CASE2_URL, and NOTES_CASE3_URL to the matching
databases on the disposable PostgreSQL instance. Set NOTES_SCHEMA_DEV1_URL,
NOTES_SCHEMA_DEV2_URL, and NOTES_SCHEMA_DEV3_URL to three separate empty dev
databases on the same instance. The migrations applied were:

1. **Old expiry only:** copy the old expiry SQL from this directory into the
   temporary directory and remove migrations newer than
   `20261123000001`. Run `atlas migrate hash --dir file://<legacy-dir>` and
   `atlas migrate apply --dir file://<legacy-dir> --url <case1-url>`. This
   applies 76 files through the old expiry revision. Then apply the current
   canonical directory to the same URL; it applies the remaining 17 files.
2. **Old expiry and later main history:** build the same temporary directory,
   retaining migrations through `20261126010000` and removing the `20261127*`
   and `20261128*` files. Hash it, apply it to `notes_case2` (84 files), then
   apply the current canonical directory (9 files).
3. **Clean current history:** apply the current canonical directory directly
   to `notes_case3` (92 files).

For each database, run this SQL to inspect both the retained historical hash
and the canonical replacement:

```sql
SELECT version, description, hash
FROM atlas_schema_revisions.atlas_schema_revisions
WHERE version IN (
  '20261123000001', '20261123100000', '20261128000002'
)
ORDER BY version;

SELECT max(version) AS current_version, count(*) AS executed_files
FROM atlas_schema_revisions.atlas_schema_revisions;
```

Then run `atlas migrate status --dir file://infra/db/migrations --url
<case-url>`. The observed results were:

| History | Atlas apply result | SQL evidence | Atlas status |
| --- | --- | --- | --- |
| Old expiry only | 76 legacy + 17 canonical files succeeded | old `20261123000001` hash stayed `1oYCJ/km4b0vq3OcL8NQX8b/ng4A8ZnnWcoIVGTIrt0=`; canonical expiry `20261128000002` was added | OK, current `20261128000006`, 93 executed, 0 pending |
| Old expiry + main through `20261126010000` | 84 legacy + 9 canonical files succeeded | old expiry hash stayed `1oYCJ/km4b0vq3OcL8NQX8b/ng4A8ZnnWcoIVGTIrt0=`; the stored `20261123100000` hash stayed `POd7u0AyUQe9zRF52nGhH8JaJRfulbiCx631+eUs57s=`; canonical expiry was added | OK, current `20261128000006`, 93 executed, 0 pending |
| Clean current history | 92 canonical files succeeded | no old expiry revision; canonical expiry hash `v5Tc4BW9V0O+1pRUfvvt97xYWVgoHkuy0iN1mPEL56s=` | OK, current `20261128000006`, 92 executed, 0 pending |

These results show the old revision remains intact where it was applied, and
the corrective `20261128000002` migration converges both legacy histories.
Do not delete or edit rows in `atlas_schema_revisions`; let Atlas apply the
new canonical migration. No manual revision-table rewrite is needed.

Compare each final application schema with the checked-in schema.sql. Give
each comparison its own empty dev database: Atlas imports the SQL file into
that database while computing the diff. The migration revision bookkeeping
schema exists only in the migrated databases, so exclude it from the
application-schema comparison:

```sh
atlas schema diff \
  --exclude atlas_schema_revisions \
  --from "$NOTES_CASE1_URL" \
  --to file://infra/db/schema.sql \
  --dev-url "$NOTES_SCHEMA_DEV1_URL"
atlas schema diff \
  --exclude atlas_schema_revisions \
  --from "$NOTES_CASE2_URL" \
  --to file://infra/db/schema.sql \
  --dev-url "$NOTES_SCHEMA_DEV2_URL"
atlas schema diff \
  --exclude atlas_schema_revisions \
  --from "$NOTES_CASE3_URL" \
  --to file://infra/db/schema.sql \
  --dev-url "$NOTES_SCHEMA_DEV3_URL"
```

On PostgreSQL 16.15 with Atlas v1.3.9-1824371-canary, all three commands
reported "Schemas are synced, no changes to be made." Migration status was
OK for all three final databases; the two legacy histories had 90 executed
files after convergence and the clean current history had 89. No application
schema differences remained. `pg_get_indexdef` returned
`(channel_id, deleted_at, id)` for all three databases.

The explicit regression fixture applied the old-only history first and
confirmed that the index was `(deleted_at, id)`. Running the former
`CREATE INDEX IF NOT EXISTS` statement left that definition unchanged. With
`enable_seqscan=off`, EXPLAIN used a non-covering scan and sort. After the
canonical migration, the same query used an Index Only Scan on
`idx_channel_notes_expired` with `channel_id` as an index condition.

Run the checked-in index definition assertion against each final history with:

```sh
psql "$NOTES_CASE1_URL" -v ON_ERROR_STOP=1 -f infra/db/tests/channel_notes_expiry_index_regression.sql
psql "$NOTES_CASE2_URL" -v ON_ERROR_STOP=1 -f infra/db/tests/channel_notes_expiry_index_regression.sql
psql "$NOTES_CASE3_URL" -v ON_ERROR_STOP=1 -f infra/db/tests/channel_notes_expiry_index_regression.sql
```
