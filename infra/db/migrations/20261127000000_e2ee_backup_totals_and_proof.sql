-- 鍵バックアップの版ごとのセッションの数と大きさ、アップロードの証明（docs/e2ee.md の鍵バックアップの API）。
-- 後方互換：列は DEFAULT 付きか NULL 許容で足す。数はトリガーが保ち、古い backend のアップロードも数える。
-- upload_key が NULL の版（古い backend が作った版を含む）は、新しい backend が初めて要るときに作る。
-- 古い backend は proven を書かないので、その行は証明のない行（false）として扱われる。
ALTER TABLE e2ee_key_backup_versions
  ADD COLUMN session_count bigint NOT NULL DEFAULT 0,
  ADD COLUMN session_bytes bigint NOT NULL DEFAULT 0,
  ADD COLUMN upload_key text;
ALTER TABLE e2ee_key_backup_sessions
  ADD COLUMN proven boolean NOT NULL DEFAULT false;

-- e2ee_key_backup_versions の session_count と session_bytes を、文ごとにまとめて保つ。
-- アップロードの INSERT ... ON CONFLICT DO UPDATE では、INSERT と UPDATE の両方のトリガーが動く。
CREATE FUNCTION e2ee_key_backup_session_totals() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    UPDATE e2ee_key_backup_versions v
    SET session_count = v.session_count + d.n, session_bytes = v.session_bytes + d.bytes
    FROM (SELECT user_id, version, count(*) AS n, sum(octet_length(session_data::text)) AS bytes
          FROM new_rows GROUP BY user_id, version) d
    WHERE v.user_id = d.user_id AND v.version = d.version;
  ELSIF TG_OP = 'UPDATE' THEN
    UPDATE e2ee_key_backup_versions v
    SET session_count = v.session_count + d.n, session_bytes = v.session_bytes + d.bytes
    FROM (SELECT user_id, version, sum(n) AS n, sum(bytes) AS bytes
          FROM (SELECT user_id, version, 1 AS n, octet_length(session_data::text) AS bytes FROM new_rows
                UNION ALL
                SELECT user_id, version, -1, -octet_length(session_data::text) FROM old_rows) x
          GROUP BY user_id, version) d
    WHERE v.user_id = d.user_id AND v.version = d.version AND (d.n <> 0 OR d.bytes <> 0);
  ELSE
    UPDATE e2ee_key_backup_versions v
    SET session_count = v.session_count - d.n, session_bytes = v.session_bytes - d.bytes
    FROM (SELECT user_id, version, count(*) AS n, sum(octet_length(session_data::text)) AS bytes
          FROM old_rows GROUP BY user_id, version) d
    WHERE v.user_id = d.user_id AND v.version = d.version;
  END IF;
  RETURN NULL;
END;
$$;
CREATE TRIGGER e2ee_key_backup_sessions_insert_totals AFTER INSERT ON e2ee_key_backup_sessions REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION e2ee_key_backup_session_totals();
CREATE TRIGGER e2ee_key_backup_sessions_update_totals AFTER UPDATE ON e2ee_key_backup_sessions REFERENCING OLD TABLE AS old_rows NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION e2ee_key_backup_session_totals();
CREATE TRIGGER e2ee_key_backup_sessions_delete_totals AFTER DELETE ON e2ee_key_backup_sessions REFERENCING OLD TABLE AS old_rows FOR EACH STATEMENT EXECUTE FUNCTION e2ee_key_backup_session_totals();

-- トリガーを作ったあとで数える。CREATE TRIGGER が e2ee_key_backup_sessions の書き込みを待たせるので、
-- 数え漏れも二重の数えもない。
UPDATE e2ee_key_backup_versions v
SET session_count = s.n, session_bytes = s.bytes
FROM (SELECT user_id, version, count(*) AS n, sum(octet_length(session_data::text)) AS bytes
      FROM e2ee_key_backup_sessions GROUP BY user_id, version) s
WHERE v.user_id = s.user_id AND v.version = s.version;
