-- Slack から取り込んだチャンネルの、取り込みの種類 (新しく作った / 既存のチャンネルに追加) と、最後に取り込んだ日時。
-- どちらも NULL 許容の列の追加なので、適用前のバックエンドが動いたままでも壊れない。
ALTER TABLE "channels" ADD COLUMN "slack_import_kind" text NULL, ADD COLUMN "slack_imported_at" timestamp(3) NULL;
ALTER TABLE "channels" ADD CONSTRAINT "channels_slack_import_kind_check" CHECK ("slack_import_kind" IS NULL OR "slack_import_kind" IN ('created', 'existing'));
-- すでに印の付いたチャンネルは、取り込みのジョブの指定から種類を求める (「新しいチャンネル」の指定があれば created)。
-- ジョブが途中で失敗・取り消しになっても、チャンネルと slack_channels の登録は先にコミットされて残るので、
-- ジョブの状態では絞らない。
-- 日時は、取り込み先として最後に記録した日時にする。
UPDATE "channels" c SET
  "slack_import_kind" = CASE WHEN EXISTS (
    SELECT 1 FROM "slack_channels" sc
    JOIN "slack_import_jobs" j ON j."team_id" = sc."team_id"
    WHERE sc."channel_id" = c."id" AND EXISTS (
      SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(j."mappings"::jsonb) = 'array' THEN j."mappings"::jsonb ELSE '[]'::jsonb END) m
      WHERE m->>'slack_channel_id' = sc."slack_channel_id" AND m->>'action' = 'create'
    )
  ) THEN 'created' ELSE 'existing' END,
  "slack_imported_at" = (SELECT (max(sc."updated_at") AT TIME ZONE 'UTC')::timestamp(3) FROM "slack_channels" sc WHERE sc."channel_id" = c."id")
WHERE c."imported_from_slack";
