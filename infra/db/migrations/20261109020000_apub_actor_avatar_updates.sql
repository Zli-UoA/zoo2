-- Create "apub_actor_updates" table
CREATE TABLE "apub_actor_updates" (
  "user_id" text NOT NULL,
  "queued_at" timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("user_id"),
  CONSTRAINT "apub_actor_updates_user_id_fkey" FOREIGN KEY ("user_id") REFERENCES "users" ("id") ON UPDATE NO ACTION ON DELETE CASCADE
);
-- Create index "apub_actor_updates_queued_at" to table: "apub_actor_updates"
CREATE INDEX "apub_actor_updates_queued_at" ON "apub_actor_updates" ("queued_at", "user_id");

CREATE FUNCTION public.enqueue_apub_actor_update(target_user text) RETURNS void
    LANGUAGE sql
    AS $$
  INSERT INTO apub_actor_updates(user_id)
    SELECT u.id FROM users u WHERE u.id=target_user AND NOT u.is_bot
      AND EXISTS(SELECT 1 FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id
        WHERE f.user_id=u.id AND f.direction='incoming' AND f.accepted AND NOT a.blocked)
    ON CONFLICT(user_id) DO UPDATE SET queued_at=EXCLUDED.queued_at;
$$;

CREATE FUNCTION public.enqueue_apub_avatar_update() RETURNS trigger
    LANGUAGE plpgsql
    AS $$
BEGIN
  PERFORM enqueue_apub_actor_update(NEW.id);
  RETURN NULL;
END $$;

CREATE TRIGGER users_apub_avatar_update AFTER UPDATE OF avatar_url ON public.users FOR EACH ROW WHEN ((old.avatar_url IS DISTINCT FROM new.avatar_url)) EXECUTE FUNCTION public.enqueue_apub_avatar_update();

-- Publish avatars already stored before this feature reached federation.
SELECT enqueue_apub_actor_update(id) FROM users WHERE avatar_url IS NOT NULL;
