-- 通知設定の正本を channel_user から分離する (#620)。
-- 旧バックエンドが動いたままでも動くように、列は残し、既存の設定値は新しいテーブルへ写す。
-- 古い列は実際の設定を残さない（ローリング更新中にメンバー一覧から読まれないようにするため）。
-- トリガーは値の移送と古い列の既定値化が済んでから作り、バックフィルを上書きしないようにする。
CREATE TABLE IF NOT EXISTS "user_channel_notification_settings" (
  "user_id" text NOT NULL,
  "channel_id" text NOT NULL,
  "notification_level" text NOT NULL DEFAULT 'mentions',
  CONSTRAINT "user_channel_notification_settings_check" CHECK (notification_level IN ('all', 'mentions', 'mute')),
  CONSTRAINT "user_channel_notification_settings_pkey" PRIMARY KEY ("user_id", "channel_id"),
  CONSTRAINT "user_channel_notification_settings_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT "user_channel_notification_settings_channel_id_fkey" FOREIGN KEY ("channel_id") REFERENCES "channels" ("id") ON UPDATE CASCADE ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS "idx_user_channel_notification_settings_channel_id"
  ON "user_channel_notification_settings" ("channel_id", "user_id");

-- 既存の設定を移す（行が無いメンバーは既定値の mentions として扱う）。
INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level)
SELECT user_id, channel_id, notification_level FROM channel_user
ON CONFLICT (user_id, channel_id) DO NOTHING;
UPDATE channel_user SET notification_level = 'mentions' WHERE notification_level <> 'mentions';

ALTER TABLE user_channel_notification_settings REPLICA IDENTITY FULL;

CREATE OR REPLACE FUNCTION user_channel_notification_settings_sync() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    DELETE FROM user_channel_notification_settings
     WHERE user_id = OLD.user_id AND channel_id = OLD.channel_id;
    RETURN OLD;
  END IF;
  INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level)
  VALUES (NEW.user_id, NEW.channel_id, 'mentions')
  ON CONFLICT (user_id, channel_id) DO NOTHING;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS channel_user_notification_settings_sync ON channel_user;
CREATE TRIGGER channel_user_notification_settings_sync
  AFTER INSERT OR DELETE ON channel_user
  FOR EACH ROW EXECUTE FUNCTION user_channel_notification_settings_sync();

CREATE OR REPLACE FUNCTION channel_user_notification_level_legacy() RETURNS trigger AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    IF NEW.notification_level IS DISTINCT FROM 'mentions' THEN
      INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level)
      VALUES (NEW.user_id, NEW.channel_id, NEW.notification_level)
      ON CONFLICT (user_id, channel_id) DO UPDATE SET notification_level = EXCLUDED.notification_level
       WHERE user_channel_notification_settings.notification_level <> EXCLUDED.notification_level;
    END IF;
    NEW.notification_level := 'mentions';
    RETURN NEW;
  END IF;
  IF NEW.notification_level IS DISTINCT FROM OLD.notification_level THEN
    INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level)
    VALUES (NEW.user_id, NEW.channel_id, NEW.notification_level)
    ON CONFLICT (user_id, channel_id) DO UPDATE SET notification_level = EXCLUDED.notification_level
     WHERE user_channel_notification_settings.notification_level <> EXCLUDED.notification_level;
  END IF;
  NEW.notification_level := OLD.notification_level;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS channel_user_notification_level_legacy ON channel_user;
CREATE TRIGGER channel_user_notification_level_legacy
  BEFORE INSERT OR UPDATE OF notification_level ON channel_user
  FOR EACH ROW EXECUTE FUNCTION channel_user_notification_level_legacy();
