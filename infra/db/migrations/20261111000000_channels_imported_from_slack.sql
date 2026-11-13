-- Slack から取り込んだチャンネルの印。取り込み先 (新しく作ったチャンネルも、既存のチャンネルも) に付ける。
-- 既定値の付いた NOT NULL の列の追加なので、適用前のバックエンドが動いたままでも壊れない。
ALTER TABLE "channels" ADD COLUMN "imported_from_slack" boolean NOT NULL DEFAULT false;
-- すでに取り込み先として記録されているチャンネルにも付ける
UPDATE "channels" SET "imported_from_slack" = true WHERE "id" IN (SELECT "channel_id" FROM "slack_channels");
