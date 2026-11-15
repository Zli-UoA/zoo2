-- E2EE: クロス署名、Secret Storage、鍵バックアップ（#356。仕様は docs/e2ee.md の「DB」）。
-- クロス署名鍵と署名は Shape で配る。
ALTER TABLE e2ee_cross_signing_keys REPLICA IDENTITY FULL;
ALTER TABLE e2ee_key_signatures REPLICA IDENTITY FULL;

-- クロス署名鍵の変更の記録。old_public_key は登録し直しの拒否（E2EE_KEY_REUSED）に使う。
CREATE TABLE e2ee_cross_signing_key_changes (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  usage text NOT NULL CHECK (usage IN ('master', 'self_signing', 'user_signing')),
  old_public_key text,
  new_public_key text NOT NULL,
  changed_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX e2ee_cs_key_changes_old ON e2ee_cross_signing_key_changes (user_id, old_public_key)
  WHERE old_public_key IS NOT NULL;

CREATE TABLE e2ee_secret_storage_keys (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  key_id text NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'm.secret_storage.v1.aes-hmac-sha2'),
  iv text NOT NULL,
  mac text NOT NULL,
  is_default boolean NOT NULL DEFAULT false,
  -- 既定の鍵の行で使う。鍵を作り直すときは前の値を引き継いで 1 増やす。
  secrets_revision bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, key_id)
);
CREATE UNIQUE INDEX e2ee_secret_storage_keys_default
  ON e2ee_secret_storage_keys (user_id) WHERE is_default;

CREATE TABLE e2ee_secrets (
  user_id text NOT NULL,
  name text NOT NULL CHECK (name IN ('m.cross_signing.master', 'm.cross_signing.self_signing',
                                     'm.cross_signing.user_signing', 'm.megolm_backup.v1',
                                     'zoo.megolm_backup.previous')),
  key_id text NOT NULL,
  iv text NOT NULL,
  ciphertext text NOT NULL,
  mac text NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, name),
  FOREIGN KEY (user_id, key_id) REFERENCES e2ee_secret_storage_keys ON DELETE CASCADE
);
CREATE INDEX e2ee_secrets_key ON e2ee_secrets (user_id, key_id);

CREATE TABLE e2ee_secret_storage_key_slots (
  -- slot_id（クライアントが作る UUID。AAD に入る）
  id text PRIMARY KEY,
  user_id text NOT NULL,
  key_id text NOT NULL,
  kind text NOT NULL CHECK (kind IN ('passkey', 'recovery')),
  credential_id text,
  prf_salt text,
  nonce text NOT NULL,
  ciphertext text NOT NULL,
  label text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  last_used_at timestamptz,
  FOREIGN KEY (user_id, key_id) REFERENCES e2ee_secret_storage_keys ON DELETE CASCADE,
  CHECK ((kind = 'passkey' AND credential_id IS NOT NULL AND prf_salt IS NOT NULL)
      OR (kind = 'recovery' AND credential_id IS NULL AND prf_salt IS NULL))
);
CREATE UNIQUE INDEX e2ee_slots_credential ON e2ee_secret_storage_key_slots (user_id, key_id, credential_id)
  WHERE kind = 'passkey';
CREATE UNIQUE INDEX e2ee_slots_recovery ON e2ee_secret_storage_key_slots (user_id, key_id)
  WHERE kind = 'recovery';

CREATE TABLE e2ee_key_backup_versions (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  version bigint NOT NULL,
  algorithm text NOT NULL CHECK (algorithm = 'zoo.megolm_backup.v1.curve25519-aes-sha2'),
  auth_data jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  -- 消した版も行を残し、番号と公開鍵の再利用の判定に使う。
  deleted_at timestamptz,
  PRIMARY KEY (user_id, version)
);
-- 過去の版と同じ公開鍵の拒否（E2EE_KEY_REUSED）に使う。
CREATE INDEX e2ee_key_backup_versions_public_key
  ON e2ee_key_backup_versions (user_id, (auth_data ->> 'public_key'));

CREATE TABLE e2ee_key_backup_sessions (
  user_id text NOT NULL,
  version bigint NOT NULL,
  -- チャンネルを消してもバックアップは残すので、外部キーを付けない。
  channel_id text NOT NULL,
  sender_key text NOT NULL,
  session_id text NOT NULL,
  first_message_index integer NOT NULL,
  forwarded_count integer NOT NULL,
  is_verified boolean NOT NULL,
  session_data jsonb NOT NULL,
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id, version, channel_id, sender_key, session_id),
  FOREIGN KEY (user_id, version) REFERENCES e2ee_key_backup_versions ON DELETE CASCADE
);
