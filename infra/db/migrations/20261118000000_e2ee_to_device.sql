-- E2EE: to-device メッセージ（#355。仕様は docs/e2ee.md の「to-device メッセージ」と「DB」）。
-- 宛先の端末ごとのキュー。受信確認（to_device/ack）で消す。生きている端末では時間で回収せず、
-- 件数（1 端末 1000 件）だけで抑える。端末の削除と無効化のときに、その端末宛ての行を消す。
CREATE TABLE e2ee_to_device_messages (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  recipient_user_id text NOT NULL,
  recipient_device_id text NOT NULL,
  sender_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  sender_device_id text NOT NULL,
  txn_id text NOT NULL,
  type text NOT NULL,
  content jsonb NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  FOREIGN KEY (recipient_user_id, recipient_device_id) REFERENCES e2ee_devices ON DELETE CASCADE,
  UNIQUE (sender_user_id, sender_device_id, txn_id, recipient_user_id, recipient_device_id)
);
-- 宛先の端末の Shape、件数の確認、ack が使う。
CREATE INDEX e2ee_to_device_recipient ON e2ee_to_device_messages (recipient_user_id, recipient_device_id, id);
CREATE INDEX e2ee_to_device_created ON e2ee_to_device_messages (created_at);
ALTER TABLE e2ee_to_device_messages REPLICA IDENTITY FULL;

-- ack 後の再送を冪等にするための送信の記録。キューに入れた宛先にだけ書き、30 日で回収する。
CREATE TABLE e2ee_to_device_txns (
  sender_user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  sender_device_id text NOT NULL,
  txn_id text NOT NULL,
  recipient_user_id text NOT NULL,
  recipient_device_id text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (sender_user_id, sender_device_id, txn_id, recipient_user_id, recipient_device_id)
);
CREATE INDEX e2ee_to_device_txns_created ON e2ee_to_device_txns (created_at);

-- アカウントの無効化で、未受信の to-device メッセージも消す（#355）。
CREATE OR REPLACE FUNCTION e2ee_delete_devices_on_deactivate() RETURNS trigger LANGUAGE plpgsql AS $$
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
    -- 未受信の to-device メッセージも消す（#355）。to-device の送信は同じ端末単位のロックのあとに
    -- deleted_at を読み直すので、このあとで削除済みの端末に入り直らない。
    DELETE FROM e2ee_to_device_messages WHERE recipient_user_id = NEW.id AND recipient_device_id = d;
  END LOOP;
  UPDATE e2ee_devices SET deleted_at = now() WHERE user_id = NEW.id AND deleted_at IS NULL;
  RETURN NULL;
END $$;
