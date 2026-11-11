-- E2EE の端末と鍵（#354）。新しい表と、DEFAULT 付きの列、既存の行が満たす CHECK だけなので、
-- 適用前のバックエンドが動いたままでも壊れない（既存のチャンネルは encrypted = false）。
-- 暗号化チャンネル。作成時の指定と変更の拒否は #357 で足す。既存の行は false なので後方互換。
ALTER TABLE channels ADD COLUMN encrypted boolean NOT NULL DEFAULT false;
ALTER TABLE channels ADD CONSTRAINT channels_encrypted_private CHECK (NOT encrypted OR is_private);

CREATE TABLE e2ee_devices (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  device_id text NOT NULL,
  display_name text NOT NULL DEFAULT '',
  curve25519_key text NOT NULL UNIQUE,
  ed25519_key text NOT NULL UNIQUE,
  device_keys jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz,
  -- 論理削除。30 日たったらバッチで行を消し、device_id を e2ee_deleted_device_ids に残す。
  deleted_at timestamptz,
  PRIMARY KEY (user_id, device_id)
);
-- 登録の回数制限（1 時間に 30 回）と、30 日の回収のバッチが使う。
CREATE INDEX e2ee_devices_user_created ON e2ee_devices (user_id, created_at);
CREATE INDEX e2ee_devices_deleted ON e2ee_devices (deleted_at) WHERE deleted_at IS NOT NULL;
ALTER TABLE e2ee_devices REPLICA IDENTITY FULL;

-- 削除した端末の device_id の墓標（鍵を持たない。再利用を防ぐためだけに残す）。
CREATE TABLE e2ee_deleted_device_ids (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  device_id text NOT NULL,
  deleted_at timestamptz NOT NULL,
  PRIMARY KEY (user_id, device_id)
);

CREATE TABLE e2ee_one_time_keys (
  user_id text NOT NULL,
  device_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'signed_curve25519'),
  key_id text NOT NULL,
  key_json jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, device_id, algorithm, key_id),
  FOREIGN KEY (user_id, device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);

CREATE TABLE e2ee_fallback_keys (
  user_id text NOT NULL,
  device_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'signed_curve25519'),
  key_id text NOT NULL,
  key_json jsonb NOT NULL,
  used boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, device_id, algorithm),
  FOREIGN KEY (user_id, device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);

-- keys/claim の払い出しの要求の回数（1 日分だけ持ち、古い行はバッチで回収する）。
CREATE TABLE e2ee_one_time_key_claim_counts (
  claimer_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  target_user_id text NOT NULL,
  target_device_id text NOT NULL,
  day date NOT NULL,
  claimed integer NOT NULL DEFAULT 0,
  PRIMARY KEY (claimer_user_id, target_user_id, target_device_id, day),
  FOREIGN KEY (target_user_id, target_device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);
CREATE INDEX e2ee_one_time_key_claim_counts_day ON e2ee_one_time_key_claim_counts (day);

-- クロス署名鍵、署名、TOFU のピン。書き込みの API は #356 で足す。
-- keys/query の応答と、端末の削除で自己署名を消す処理（#354）が読み書きするので、表はここで作る。
CREATE TABLE e2ee_cross_signing_keys (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  usage text NOT NULL CHECK (usage IN ('master', 'self_signing', 'user_signing')),
  public_key text NOT NULL,
  key_json jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, usage)
);

CREATE TABLE e2ee_key_signatures (
  signer_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  signer_key_id text NOT NULL,
  target_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  target_key_id text NOT NULL,
  signature text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (signer_user_id, signer_key_id, target_user_id, target_key_id)
);
CREATE INDEX e2ee_key_signatures_target ON e2ee_key_signatures (target_user_id, target_key_id);

CREATE TABLE e2ee_tofu_pins (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  target_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  master_key text NOT NULL,
  pin_json jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, target_user_id)
);

-- アカウントの無効化（is_deactivated が false から true）と同じトランザクションで、端末をすべて削除する。
-- 経路（guest_kind の更新、App の API など）によらず同じにするため、トリガーで行う。
-- 無効化の UPDATE は users の行を FOR NO KEY UPDATE で持つので、ロックの順（users の行、端末単位のロック）に合う。
-- 端末単位のロックは DELETE /api/e2ee/devices/{device_id} と同じキーを device_id の昇順で取る。
CREATE FUNCTION e2ee_delete_devices_on_deactivate() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
  d text;
BEGIN
  -- 鍵の行も端末ごとに device_id の昇順で消す。keys/claim は宛先を (user_id, device_id) の昇順で
  -- 払い出すので、同じ順で行ロックを取り、並行した claim と行き詰まらない。
  FOR d IN SELECT device_id FROM e2ee_devices
      WHERE user_id = NEW.id AND deleted_at IS NULL ORDER BY device_id COLLATE "C" LOOP
    PERFORM pg_advisory_xact_lock(2, hashtext(NEW.id || ':' || d));
    DELETE FROM e2ee_key_signatures
      WHERE signer_user_id = NEW.id AND target_user_id = NEW.id AND target_key_id = d;
    DELETE FROM e2ee_one_time_keys WHERE user_id = NEW.id AND device_id = d;
    DELETE FROM e2ee_fallback_keys WHERE user_id = NEW.id AND device_id = d;
  END LOOP;
  UPDATE e2ee_devices SET deleted_at = now() WHERE user_id = NEW.id AND deleted_at IS NULL;
  RETURN NULL;
END $$;
CREATE TRIGGER users_e2ee_deactivate AFTER UPDATE OF is_deactivated, guest_kind ON users
  FOR EACH ROW WHEN (NEW.is_deactivated AND NOT OLD.is_deactivated)
  EXECUTE FUNCTION e2ee_delete_devices_on_deactivate();
