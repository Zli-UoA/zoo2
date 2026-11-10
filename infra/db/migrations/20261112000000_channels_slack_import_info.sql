-- Slack から取り込んだチャンネルの、取り込みの種類 (新しく作った / 既存のチャンネルに追加) と、最後に取り込んだ日時。
-- どちらも NULL 許容の列の追加なので、適用前のバックエンドが動いたままでも壊れない。
ALTER TABLE "channels" ADD COLUMN "slack_import_kind" text NULL, ADD COLUMN "slack_imported_at" timestamp(3) NULL;
ALTER TABLE "channels" ADD CONSTRAINT "channels_slack_import_kind_check" CHECK ("slack_import_kind" IS NULL OR "slack_import_kind" IN ('created', 'existing'));
-- すでに印の付いたチャンネルは、取り込みのジョブの指定から種類を求める (「新しいチャンネル」の指定があれば created)。
-- ジョブが途中で失敗・取り消しになっても、チャンネルと slack_channels の登録は先にコミットされて残るので、
-- ジョブの状態では絞らない。代わりに、取り込み先が「新しいチャンネル」として作られる決定的な ID
-- (backend の slack::channel_id と同じ、sha256("slack-channel:<team>:<Slack のチャンネル>") の先頭 16 バイトの UUID) と
-- 一致することを確かめる (作成の指定のジョブが、チャンネルを作る前に失敗し、あとで既存のチャンネルに取り込んだ場合を除く)。
-- 日時は、メッセージの取り込みが終わった (fetching_files か completed の) ジョブの、最後の終了日時にする。
-- 失敗・取り消しのジョブは、取り込み先を先に記録しても、取り込みが終わっていないので使わない。成功したジョブがなければ NULL のままにする。
UPDATE "channels" c SET
  "slack_import_kind" = CASE WHEN EXISTS (
    SELECT 1 FROM "slack_channels" sc
    JOIN "slack_import_jobs" j ON j."team_id" = sc."team_id"
    WHERE sc."channel_id" = c."id"
      AND sc."channel_id" = encode(substring(sha256(convert_to('slack-channel:' || sc."team_id" || ':' || sc."slack_channel_id", 'UTF8')) from 1 for 16), 'hex')::uuid::text
      AND EXISTS (
      SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(j."mappings"::jsonb) = 'array' THEN j."mappings"::jsonb ELSE '[]'::jsonb END) m
      WHERE m->>'slack_channel_id' = sc."slack_channel_id" AND m->>'action' = 'create'
    )
  ) THEN 'created' ELSE 'existing' END,
  "slack_imported_at" = (
    SELECT (max(COALESCE(j."finished_at", j."updated_at")) AT TIME ZONE 'UTC')::timestamp(3)
    FROM "slack_channels" sc
    JOIN "slack_import_jobs" j ON j."team_id" = sc."team_id" AND j."status" IN ('fetching_files', 'completed')
    WHERE sc."channel_id" = c."id" AND EXISTS (
      SELECT 1 FROM jsonb_array_elements(CASE WHEN jsonb_typeof(j."mappings"::jsonb) = 'array' THEN j."mappings"::jsonb ELSE '[]'::jsonb END) m
      WHERE m->>'slack_channel_id' = sc."slack_channel_id" AND m->>'action' IN ('create', 'existing')
    )
  )
WHERE c."imported_from_slack";
