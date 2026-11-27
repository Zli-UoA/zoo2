-- 通知設定の正本を channel_user から分離する (#620)。
-- 旧バックエンドが動いたままでも動くように、列は残し、既存の設定値は新しいテーブルへ写す。
-- 古い列は実際の設定を残さない（ローリング更新中にメンバー一覧から読まれないようにするため）。
-- 参加・退出トリガーは既存値の移送より前に作る。移送とトリガー作成の間に参加した行を拾うため。
-- CREATE TRIGGER は channel_user への書き込みを待たせるため、作成後の参加行はトリガーで設定行が作られ、
-- 既存行は ON CONFLICT DO NOTHING の補完で埋める（トリガーが書いた値は上書きしない）。
-- 値移送用の BEFORE トリガーは、古い列の既定値化を打ち消さないよう、既定値化の後に作る。
--
-- バックフィルから値移送トリガーの作成まで、channel_user を SHARE ROW EXCLUSIVE でロックして
-- 同じトランザクションで保持する。CREATE TRIGGER 自身も同じロックを取るが、それだけでは
-- バックフィル中に旧バックエンドが旧列へ書いた値が移送トリガーの無い間に失われ、
-- メンバー一覧から実際の設定が読めてしまう。先にロックすれば旧バックエンドの
-- INSERT/UPDATE/DELETE はコミットまで待ち、書込みを取りこぼさずに移行できる。
-- txmode none を使うと途中でロックが解放されるため使わない。
-- 運用影響: ロック保持中は channel_user への参加・退出・通知レベル更新が待たされる（SELECT は可）。
-- バックフィルは channel_user の全行を走査して新テーブルへ書くため、所属行数が多いほど
-- 待ち時間が伸びる。移行の実行時間とロック待ちを監視する。
LOCK TABLE channel_user IN SHARE ROW EXCLUSIVE MODE;
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

ALTER TABLE user_channel_notification_settings REPLICA IDENTITY FULL;

-- 参加・退出を設定テーブルへ反映するトリガーを先に作る。
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

-- 既存の設定を移す（行が無いメンバーは既定値の mentions として扱う）。
-- 上記トリガー作成後に参加した行は設定行が既にあるため、ON CONFLICT でその値を守る。
INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level)
SELECT user_id, channel_id, notification_level FROM channel_user
ON CONFLICT (user_id, channel_id) DO NOTHING;
UPDATE channel_user SET notification_level = 'mentions' WHERE notification_level <> 'mentions';

-- 旧アプリが channel_user.notification_level に書いた値を設定テーブルへ移送する。
-- この BEFORE トリガーを既定値化より前に作ると、UPDATE の NEW 側（mentions）が移送され、
-- 古い列の既定値化が打ち消されるため、ここで作る。
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
  -- UPDATE OF notification_level で発火したときは、旧列が既定値の mentions に戻されているため
  -- OLD と NEW が同じでも指定値を正本へ反映する。旧値との比較で判定すると、旧アプリが
  -- 現在の mentions をそのまま書いたときに正本の all/mute が更新されない。
  -- 同値更新の抑止は正本側の ON CONFLICT ... WHERE で行う。
  INSERT INTO user_channel_notification_settings (user_id, channel_id, notification_level)
  VALUES (NEW.user_id, NEW.channel_id, NEW.notification_level)
  ON CONFLICT (user_id, channel_id) DO UPDATE SET notification_level = EXCLUDED.notification_level
   WHERE user_channel_notification_settings.notification_level <> EXCLUDED.notification_level;
  NEW.notification_level := OLD.notification_level;
  RETURN NEW;
END;
$$ LANGUAGE plpgsql;
DROP TRIGGER IF EXISTS channel_user_notification_level_legacy ON channel_user;
CREATE TRIGGER channel_user_notification_level_legacy
  BEFORE INSERT OR UPDATE OF notification_level ON channel_user
  FOR EACH ROW EXECUTE FUNCTION channel_user_notification_level_legacy();
