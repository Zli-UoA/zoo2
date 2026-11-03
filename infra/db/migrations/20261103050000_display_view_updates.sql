SET LOCAL jit=off;

CREATE OR REPLACE VIEW notification_display AS
SELECT n.id, jsonb_build_object(
  'message', CASE WHEN m.deleted_at IS NULL AND m.id IS NOT NULL THEN
    jsonb_build_object('id',m.id,'user_id',m.user_id,'content',left(m.content,281),
      'emojis',COALESCE((SELECT jsonb_object_agg(e.key,e.value)
        FROM jsonb_each(COALESCE(m.display_data->'emojis','{}'::jsonb)) e
        WHERE strpos(left(m.content,281),':'||e.key||':')>0 OR
          badge.reaction->>'emoji' IN (e.key,':'||e.key||':')), '{}'::jsonb)) ELSE NULL END,
  'reactions', CASE WHEN n.notification_type='reaction' AND m.deleted_at IS NULL AND badge.reaction IS NOT NULL
    THEN jsonb_build_array(badge.reaction) ELSE '[]'::jsonb END,
  'actor', jsonb_build_object('id',u.id,'user_name',u.user_name,'display_name',u.display_name,
    'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id),
  'channel', to_jsonb(c)
) AS display_data
FROM notification_events n JOIN users u ON u.id=n.actor_id JOIN channels c ON c.id=n.channel_id
LEFT JOIN messages m ON m.id=n.message_id AND m.channel_id=n.channel_id
LEFT JOIN LATERAL (
  SELECT entry.value AS reaction FROM jsonb_array_elements(COALESCE(m.display_data->'reactions','[]'::jsonb)) AS entry(value)
  WHERE n.notification_type='reaction' AND entry.value->>'user_id'=n.actor_id
  ORDER BY entry.value->>'created_at' DESC,entry.value->>'id' LIMIT 1
) badge ON true
WHERE n.source='channel';

CREATE OR REPLACE FUNCTION sync_message_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  -- Include the row and both roots in the same deterministic lock order.
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.parent_id END
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.parent_id END
  ) targets WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION sync_message_notifications_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT n.id FROM notification_events n
    WHERE n.source='channel' AND n.message_id=NEW.id AND (
      OLD.content IS DISTINCT FROM NEW.content OR OLD.user_id IS DISTINCT FROM NEW.user_id OR
      OLD.deleted_at IS DISTINCT FROM NEW.deleted_at OR
      (OLD.display_data-'reactions') IS DISTINCT FROM (NEW.display_data-'reactions') OR
      n.notification_type='reaction' AND n.actor_id IN (
        SELECT reaction->>'user_id' FROM (
          (SELECT value AS reaction FROM jsonb_array_elements(COALESCE(OLD.display_data->'reactions','[]'))
           EXCEPT SELECT value FROM jsonb_array_elements(COALESCE(NEW.display_data->'reactions','[]')))
          UNION
          (SELECT value FROM jsonb_array_elements(COALESCE(NEW.display_data->'reactions','[]'))
           EXCEPT SELECT value FROM jsonb_array_elements(COALESCE(OLD.display_data->'reactions','[]')))
        ) changed
      )
    ) ORDER BY n.id LOOP
    PERFORM refresh_notification_display(target);
  END LOOP;
  RETURN NULL;
END;
$$;

-- A phase lock lets unrelated row writers run concurrently, and unrelated
-- metadata writers run concurrently, but never crosses those two lock orders.
-- Session locks are held only while entering a phase and are exception-safe;
-- the shared phase token itself is released automatically with the transaction.
CREATE FUNCTION enter_display_group(writer text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE held text := current_setting('zoo.display_writer', true);
        own_key integer; other_key integer;
BEGIN
  IF held='metadata' OR held=writer THEN RETURN; END IF;
  IF held='row' THEN
    RAISE EXCEPTION 'Lock display metadata before modifying messages' USING ERRCODE='40P01';
  END IF;
  own_key := CASE WHEN writer='metadata' THEN 31602 ELSE 31601 END;
  other_key := CASE WHEN writer='metadata' THEN 31601 ELSE 31602 END;
  BEGIN
    PERFORM pg_advisory_lock(2063231026,31600);
    PERFORM pg_advisory_lock(2063231026,other_key);
    PERFORM pg_advisory_xact_lock_shared(2063231026,own_key);
    PERFORM set_config('zoo.display_writer',writer,true);
    PERFORM pg_advisory_unlock(2063231026,other_key);
    PERFORM pg_advisory_unlock(2063231026,31600);
  EXCEPTION WHEN query_canceled OR assert_failure OR OTHERS THEN
    PERFORM pg_advisory_unlock(2063231026,l.objid::integer) FROM pg_locks l
      WHERE l.pid=pg_backend_pid() AND l.locktype='advisory' AND l.classid=2063231026
        AND l.objsubid=2 AND l.objid IN (31600,other_key) AND l.mode='ExclusiveLock' AND l.granted;
    RAISE;
  END;
END;
$$;
CREATE FUNCTION lock_display_metadata() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM enter_display_group(TG_ARGV[0]);
  RETURN NULL;
END;
$$;
CREATE TRIGGER messages_display_gate BEFORE INSERT OR UPDATE OR DELETE ON messages FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('row');
CREATE TRIGGER reactions_display_gate BEFORE INSERT OR UPDATE OR DELETE ON message_reactions FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('row');
CREATE TRIGGER attachments_display_gate BEFORE INSERT OR UPDATE OR DELETE ON attachments FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('row');
CREATE TRIGGER federated_reactions_display_gate BEFORE INSERT OR UPDATE OR DELETE ON apub_reactions FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('row');
CREATE TRIGGER users_display_gate BEFORE UPDATE OF user_name,display_name,avatar_url,is_bot,app_id ON users FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('metadata');
CREATE TRIGGER emojis_display_gate BEFORE INSERT OR UPDATE OR DELETE ON workspace_emojis FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('metadata');
CREATE TRIGGER federated_actors_display_gate BEFORE UPDATE OF user_name,display_name,blocked ON apub_actors FOR EACH STATEMENT EXECUTE FUNCTION lock_display_metadata('metadata');


UPDATE notification_events n SET display_data=v.display_data FROM notification_display v WHERE n.id=v.id AND n.display_data IS DISTINCT FROM v.display_data;
