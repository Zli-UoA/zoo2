-- Failed jobs remain durable, with bounded backoff and independent row/scope scheduling.

ALTER TABLE display_refresh_queue ADD COLUMN attempts integer NOT NULL DEFAULT 0;

ALTER TABLE display_refresh_queue ADD COLUMN retry_at timestamptz NOT NULL DEFAULT '-infinity';

CREATE INDEX display_refresh_rows_ready_idx ON display_refresh_queue(retry_at,kind,target,generation)
  WHERE kind IN ('message','notification');

CREATE INDEX display_refresh_scopes_ready_idx ON display_refresh_queue(retry_at,kind,target,generation)
  WHERE kind NOT IN ('message','notification');

CREATE OR REPLACE FUNCTION refresh_next_display() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
DECLARE job record;
BEGIN
  FOR job IN SELECT * FROM display_refresh_queue WHERE kind IN ('message','notification') AND retry_at<=statement_timestamp() ORDER BY retry_at,kind,target,generation LIMIT 32 FOR UPDATE SKIP LOCKED LOOP
    BEGIN
    IF job.kind='message' THEN
      IF EXISTS(SELECT 1 FROM messages WHERE id=job.target) THEN
        PERFORM 1 FROM messages WHERE id=job.target FOR NO KEY UPDATE SKIP LOCKED;
        IF NOT FOUND THEN CONTINUE; END IF;
        PERFORM set_config('zoo.display_refresh','metadata',true);
        PERFORM refresh_message_display(job.target);
        PERFORM set_config('zoo.display_refresh','',true);
      END IF;
    ELSE
      IF EXISTS(SELECT 1 FROM notification_events WHERE id=job.target) THEN
        PERFORM 1 FROM notification_events WHERE id=job.target FOR NO KEY UPDATE SKIP LOCKED;
        IF NOT FOUND THEN CONTINUE; END IF;
        PERFORM refresh_notification_display(job.target);
      END IF;
    END IF;
    DELETE FROM display_refresh_queue WHERE kind=job.kind AND target=job.target AND generation=job.generation;
    EXCEPTION WHEN query_canceled OR OTHERS THEN
      UPDATE display_refresh_queue SET attempts=attempts+1,
        retry_at=clock_timestamp()+LEAST(60,power(2,LEAST(job.attempts,6))) * interval '1 second'
        WHERE kind=job.kind AND target=job.target AND generation=job.generation;
      RAISE WARNING 'Display row %/% deferred (SQLSTATE %)',job.kind,job.target,SQLSTATE;
    END;
    RETURN true;
  END LOOP;
  RETURN false;
END;
$$;

CREATE OR REPLACE FUNCTION expand_next_display_scope() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
DECLARE job record; item record; last_id text; processed integer := 0;
BEGIN
  SELECT * INTO job FROM display_refresh_queue
    WHERE kind NOT IN ('message','notification') AND retry_at<=statement_timestamp() ORDER BY retry_at,kind,target,generation LIMIT 1 FOR UPDATE SKIP LOCKED;
  IF NOT FOUND THEN RETURN false; END IF;
  BEGIN
  FOR item IN SELECT * FROM display_scope_targets(job.kind,job.target)
    WHERE row_id>job.cursor_id ORDER BY row_id LIMIT 128 LOOP
    INSERT INTO display_refresh_queue(kind,target) VALUES(item.row_kind,item.row_id) ON CONFLICT DO NOTHING;
    last_id := item.row_id;
    processed := processed+1;
  END LOOP;
  IF processed<128 THEN
    DELETE FROM display_refresh_queue WHERE kind=job.kind AND target=job.target AND generation=job.generation;
  ELSE
    UPDATE display_refresh_queue SET cursor_id=last_id WHERE kind=job.kind AND target=job.target AND generation=job.generation;
  END IF;
  EXCEPTION WHEN query_canceled OR OTHERS THEN
    UPDATE display_refresh_queue SET attempts=attempts+1,
      retry_at=clock_timestamp()+LEAST(60,power(2,LEAST(job.attempts,6))) * interval '1 second'
      WHERE kind=job.kind AND target=job.target AND generation=job.generation;
    RAISE WARNING 'Display scope %/% deferred (SQLSTATE %)',job.kind,job.target,SQLSTATE;
  END;
  RETURN true;
END;
$$;
