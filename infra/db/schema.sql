CREATE TABLE users (
  id text NOT NULL,
  user_name text NOT NULL,
  display_name varchar(255) NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  PRIMARY KEY (id),
  CONSTRAINT users_user_name_key UNIQUE (user_name)
);

CREATE TABLE channels (
  id text NOT NULL,
  channel_name varchar(255) NOT NULL,
  description varchar(255),
  created_user_id text NOT NULL,
  is_private boolean NOT NULL,
  is_dm boolean NOT NULL DEFAULT false,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  archive_at timestamp(3),
  deleted_at timestamp(3),
  PRIMARY KEY (id),
  CONSTRAINT channels_channel_name_key UNIQUE (channel_name),
  CONSTRAINT channels_created_user_id_fkey
    FOREIGN KEY (created_user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_channels_is_dm ON channels (is_dm);


CREATE TABLE messages (
  id text NOT NULL,
  user_id text NOT NULL,
  channel_id text NOT NULL,
  content text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  deleted_at timestamp(3),
  parent_id text,
  PRIMARY KEY (id),
  CONSTRAINT messages_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT messages_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT messages_parent_id_fkey
    FOREIGN KEY (parent_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE TABLE channel_user (
  user_id text NOT NULL,
  channel_id text NOT NULL,
  joined_at timestamp(3) NOT NULL,
  PRIMARY KEY (user_id, channel_id),
  CONSTRAINT channel_user_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT channel_user_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE TABLE role (
  id text NOT NULL,
  role_name varchar(255) NOT NULL,
  PRIMARY KEY (id),
  CONSTRAINT role_role_name_key UNIQUE (role_name)
);

CREATE TABLE user_role (
  user_id text NOT NULL,
  role_id text NOT NULL,
  PRIMARY KEY (user_id, role_id),
  CONSTRAINT user_role_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT user_role_role_id_fkey
    FOREIGN KEY (role_id) REFERENCES role (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE TABLE user_identities (
  id text NOT NULL,
  user_id text NOT NULL,
  iss text NOT NULL,
  sub text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  PRIMARY KEY (id),
  CONSTRAINT user_identities_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT user_identities_iss_sub_key UNIQUE (iss, sub)
);

CREATE TABLE message_reactions (
  id text NOT NULL,
  message_id text NOT NULL,
  user_id text NOT NULL,
  emoji text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (id),
  CONSTRAINT message_reactions_unique UNIQUE (message_id, user_id, emoji),
  CONSTRAINT message_reactions_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT message_reactions_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_messages_channel_id ON messages (channel_id);
CREATE INDEX idx_messages_parent_id ON messages (parent_id);
CREATE INDEX idx_message_reactions_message_id ON message_reactions (message_id);
CREATE INDEX idx_message_reactions_user_id ON message_reactions (user_id);

CREATE TABLE notifications (
  id text NOT NULL,
  user_id text NOT NULL,
  actor_id text NOT NULL,
  channel_id text NOT NULL,
  message_id text NOT NULL,
  notification_type text NOT NULL,
  is_read boolean NOT NULL DEFAULT false,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (id),
  CONSTRAINT notifications_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT notifications_actor_id_fkey
    FOREIGN KEY (actor_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT notifications_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT notifications_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_notifications_user_id ON notifications (user_id);
CREATE INDEX idx_notifications_user_id_is_read ON notifications (user_id, is_read);
CREATE INDEX idx_notifications_channel_id ON notifications (channel_id);
CREATE INDEX idx_notifications_message_id ON notifications (message_id);



-- Shared ephemeral typing state; the database clock is authoritative for expiry.
CREATE TABLE typing_presence (
  channel_id text NOT NULL,
  parent_id text,
  user_id text NOT NULL,
  session_id text NOT NULL,
  expires_at timestamptz NOT NULL,
  PRIMARY KEY (channel_id, user_id, session_id),
  CONSTRAINT typing_presence_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT typing_presence_parent_id_fkey
    FOREIGN KEY (parent_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT typing_presence_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_typing_presence_scope ON typing_presence (channel_id, parent_id, expires_at);
CREATE INDEX idx_typing_presence_expires_at ON typing_presence (expires_at);

CREATE TABLE channel_webhooks (
  id text NOT NULL PRIMARY KEY,
  channel_id text NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
  name varchar(80) NOT NULL,
  secret text NOT NULL,
  user_id text NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX idx_channel_webhooks_channel_id ON channel_webhooks (channel_id);

-- Structured integration cards are separate from user-controlled message content.
ALTER TABLE messages ADD COLUMN card text;
ALTER TABLE users ADD COLUMN is_bot boolean NOT NULL DEFAULT false;

CREATE TABLE channel_read_states (
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  channel_id text NOT NULL REFERENCES channels (id) ON UPDATE CASCADE ON DELETE CASCADE,
  last_read_at timestamp(3) NOT NULL,
  PRIMARY KEY (user_id, channel_id)
);

CREATE INDEX idx_messages_channel_unread ON messages (channel_id, created_at)
  WHERE parent_id IS NULL AND deleted_at IS NULL;

CREATE TABLE push_subscriptions (
  id text NOT NULL PRIMARY KEY,
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  endpoint text NOT NULL UNIQUE,
  created_at timestamp(3) NOT NULL
);
CREATE INDEX idx_push_subscriptions_user_id ON push_subscriptions (user_id);

ALTER TABLE push_subscriptions ADD COLUMN p256dh text;
ALTER TABLE push_subscriptions ADD COLUMN auth_secret text;
-- App profiles are public; all other platform tables are server-only.
CREATE TABLE apps (
 id text PRIMARY KEY,
 owner_id text NOT NULL REFERENCES users(id),
 bot_user_id text NOT NULL UNIQUE REFERENCES users(id),
 name varchar(80) NOT NULL,
 description text NOT NULL DEFAULT '',
 icon_url text,
 status text NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','pending','installed','disabled')),
 manifest text NOT NULL DEFAULT '{}',
 revision integer NOT NULL DEFAULT 1,
 created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
 updated_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
ALTER TABLE users ADD COLUMN app_id text REFERENCES apps(id);
ALTER TABLE users ADD COLUMN avatar_url text;
CREATE TABLE app_installations (
 id text PRIMARY KEY,
 app_id text NOT NULL UNIQUE REFERENCES apps(id),
 approved_by text REFERENCES users(id),
 scopes text NOT NULL DEFAULT '[]',
 manifest text NOT NULL DEFAULT '{}',
 revision integer NOT NULL,
 active boolean NOT NULL DEFAULT true,
 installed_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE app_credentials (
 id text PRIMARY KEY,
 installation_id text NOT NULL REFERENCES app_installations(id),
 user_id text REFERENCES users(id),
 token_hash text NOT NULL UNIQUE,
 refresh_hash text UNIQUE,
 scopes text NOT NULL,
 expires_at timestamptz NOT NULL,
 revoked_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE app_oauth_codes (
 code_hash text PRIMARY KEY,
 installation_id text NOT NULL REFERENCES app_installations(id),
 user_id text NOT NULL REFERENCES users(id),
 redirect_uri text NOT NULL,
 code_challenge text NOT NULL,
 scopes text NOT NULL,
 expires_at timestamptz NOT NULL,
 used_at timestamptz
);
CREATE TABLE app_deliveries (
 id text PRIMARY KEY,
 event_id text NOT NULL,
 installation_id text NOT NULL REFERENCES app_installations(id),
 channel_id text REFERENCES channels(id),
 event_type text NOT NULL,
 payload text NOT NULL,
 attempts integer NOT NULL DEFAULT 0,
 available_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
 expires_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP + interval '24 hours',
 lease_until timestamptz,
 lease_token text,
 acked_at timestamptz,
 last_error text,
 created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
 UNIQUE(installation_id,event_id)
);
CREATE INDEX app_deliveries_pending ON app_deliveries(available_at) WHERE acked_at IS NULL;
CREATE TABLE app_audit_log (
 id text PRIMARY KEY,
 app_id text NOT NULL REFERENCES apps(id),
 actor_id text NOT NULL REFERENCES users(id),
 action text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE app_ui (
 id text PRIMARY KEY,
 installation_id text NOT NULL REFERENCES app_installations(id),
 user_id text NOT NULL REFERENCES users(id),
 channel_id text REFERENCES channels(id),
 kind text NOT NULL CHECK(kind IN ('home','modal','ephemeral','trigger')),
 document text NOT NULL DEFAULT '{}',
 expires_at timestamptz,
 updated_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE UNIQUE INDEX app_home_user ON app_ui(installation_id,user_id) WHERE kind='home';
CREATE TABLE app_files (
 id text PRIMARY KEY,
 installation_id text NOT NULL REFERENCES app_installations(id),
 user_id text NOT NULL REFERENCES users(id),
 channel_id text NOT NULL REFERENCES channels(id),
 object_key text NOT NULL UNIQUE,
 name text NOT NULL,
 content_type text NOT NULL,
 size_bytes bigint NOT NULL CHECK(size_bytes BETWEEN 1 AND 26214400),
 completed boolean NOT NULL DEFAULT false,
 created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE app_rate_limits (
 installation_id text NOT NULL REFERENCES app_installations(id),
 window_start timestamptz NOT NULL,
 requests integer NOT NULL,
 PRIMARY KEY(installation_id,window_start)
);
ALTER TABLE messages ADD COLUMN app_blocks text;

-- Business writes and their events commit atomically, including legacy webhook writes.
CREATE FUNCTION enqueue_app_event() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
 row_data jsonb; old_data jsonb; channel text; actor text; event_name text;
 event_uuid text := gen_random_uuid()::text; envelope jsonb; installation record;
 is_bot_actor boolean; membership_change boolean;
BEGIN
 row_data := CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
 old_data := CASE WHEN TG_OP='UPDATE' THEN to_jsonb(OLD) ELSE NULL END;
 IF TG_TABLE_NAME='messages' THEN
   channel := row_data->>'channel_id'; actor := row_data->>'user_id';
   event_name := CASE WHEN TG_OP='INSERT' THEN 'message.created' WHEN TG_OP='DELETE' OR (row_data->>'deleted_at') IS NOT NULL THEN 'message.deleted' ELSE 'message.updated' END;
 ELSIF TG_TABLE_NAME='message_reactions' THEN
   SELECT channel_id INTO channel FROM messages WHERE id=row_data->>'message_id';
   actor := row_data->>'user_id'; event_name := CASE WHEN TG_OP='DELETE' THEN 'reaction.removed' ELSE 'reaction.added' END;
 ELSIF TG_TABLE_NAME='channel_user' THEN
   channel := row_data->>'channel_id'; actor := row_data->>'user_id';
   event_name := CASE WHEN TG_OP='DELETE' THEN 'member.left' ELSE 'member.joined' END;
 ELSIF TG_TABLE_NAME='channels' THEN
   channel := row_data->>'id'; actor := row_data->>'created_user_id';
   event_name := CASE WHEN TG_OP='INSERT' THEN 'channel.created' WHEN TG_OP='DELETE' OR (row_data->>'deleted_at') IS NOT NULL THEN 'channel.deleted' ELSE 'channel.updated' END;
 ELSE RETURN NULL;
 END IF;
 SELECT is_bot INTO is_bot_actor FROM users WHERE id=actor;
 envelope := jsonb_build_object('id',event_uuid,'version',1,'type',event_name,'occurred_at',CURRENT_TIMESTAMP,'actor_id',actor,'channel_id',channel,'data',row_data,'previous',old_data);
 FOR installation IN
 SELECT i.*,a.bot_user_id FROM app_installations i JOIN apps a ON a.id=i.app_id
 WHERE i.active AND a.status='installed'
 AND EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=channel AND cu.user_id=a.bot_user_id)
 AND (CASE WHEN TG_TABLE_NAME IN('messages','message_reactions') THEN i.scopes::jsonb ? 'messages:read' ELSE i.scopes::jsonb ? 'channels:read' END)
 AND (i.manifest::jsonb->'events' ? event_name)
 AND (NOT COALESCE(is_bot_actor,false) OR COALESCE((i.manifest::jsonb->>'include_bot_events')::boolean,false))
 LOOP
 INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload)
 VALUES(gen_random_uuid()::text,event_uuid,installation.id,channel,event_name,envelope::text);
 END LOOP;
 RETURN NULL;
END $$;
CREATE TRIGGER app_message_events AFTER INSERT OR UPDATE OR DELETE ON messages FOR EACH ROW EXECUTE FUNCTION enqueue_app_event();
CREATE TRIGGER app_reaction_events AFTER INSERT OR DELETE ON message_reactions FOR EACH ROW EXECUTE FUNCTION enqueue_app_event();
CREATE TRIGGER app_member_events AFTER INSERT OR DELETE ON channel_user FOR EACH ROW EXECUTE FUNCTION enqueue_app_event();
CREATE TRIGGER app_channel_events AFTER INSERT OR UPDATE OR DELETE ON channels FOR EACH ROW EXECUTE FUNCTION enqueue_app_event();
ALTER TABLE app_installations ADD COLUMN signing_secret text NOT NULL DEFAULT (gen_random_uuid()::text || gen_random_uuid()::text);
CREATE TABLE app_socket_tickets (
 ticket_hash text PRIMARY KEY,
 credential_id text NOT NULL REFERENCES app_credentials(id),
 expires_at timestamptz NOT NULL,
 used_at timestamptz
);

ALTER TABLE messages ADD COLUMN app_attachments text;
ALTER TABLE users ADD COLUMN bot_mentionable boolean NOT NULL DEFAULT false;
CREATE FUNCTION sync_app_profile() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 UPDATE users SET display_name=NEW.name,avatar_url=NEW.icon_url,
 bot_mentionable=(NEW.status='installed' AND NOT COALESCE((NEW.manifest::jsonb->>'notification_only')::boolean,false) AND COALESCE(NEW.manifest::jsonb->'scopes','[]'::jsonb) ? 'messages:read'),
 updated_at=CURRENT_TIMESTAMP WHERE id=NEW.bot_user_id;
 RETURN NULL;
END $$;
CREATE TRIGGER app_profile_updated AFTER UPDATE ON apps FOR EACH ROW EXECUTE FUNCTION sync_app_profile();
-- Mention and link events are selective; the complete message stream is optional.
CREATE FUNCTION enqueue_app_special_message() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE installation record; kind text; event_uuid text; envelope jsonb; kinds text[];
BEGIN
 FOR installation IN
 SELECT i.*,a.bot_user_id,u.user_name FROM app_installations i JOIN apps a ON a.id=i.app_id JOIN users u ON u.id=a.bot_user_id
 WHERE i.active AND a.status='installed' AND i.scopes::jsonb ? 'messages:read'
 AND EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=NEW.channel_id AND cu.user_id=a.bot_user_id)
 AND (NOT (SELECT is_bot FROM users WHERE id=NEW.user_id) OR COALESCE((i.manifest::jsonb->>'include_bot_events')::boolean,false))
 LOOP
 kinds:=ARRAY[]::text[];
 IF iS_mention(NEW.content,installation.user_name) AND installation.manifest::jsonb->'events' ? 'app.mentioned' THEN kinds:=array_append(kinds,'app.mentioned'); END IF;
 IF installation.scopes::jsonb ? 'links:write' AND installation.manifest::jsonb->'events' ? 'link.shared' AND EXISTS(
 SELECT 1 FROM jsonb_array_elements_text(COALESCE(installation.manifest::jsonb->'link_domains','[]'::jsonb)) domain
 WHERE NEW.content ~ ('https://' || replace(domain.value,'.','[.]') || '([/:?#[:space:]]|$)')
 ) THEN kinds:=array_append(kinds,'link.shared'); END IF;
 FOREACH kind IN ARRAY kinds LOOP
 event_uuid:=gen_random_uuid()::text;
 envelope:=jsonb_build_object('id',event_uuid,'version',1,'type',kind,'occurred_at',CURRENT_TIMESTAMP,'actor_id',NEW.user_id,'channel_id',NEW.channel_id,'data',to_jsonb(NEW));
 INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload) VALUES(gen_random_uuid()::text,event_uuid,installation.id,NEW.channel_id,kind,envelope::text);
 END LOOP;
 END LOOP; RETURN NULL;
END $$;
CREATE FUNCTION is_mention(content text, username text) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
 SELECT content ~ ('(^|[^[:alnum:]_.@-])@' || replace(username,'.','[.]') || '($|[^[:alnum:]_.-]|[.]+($|[[:space:]]))')
$$;
CREATE TRIGGER app_special_message_events AFTER INSERT ON messages FOR EACH ROW EXECUTE FUNCTION enqueue_app_special_message();
CREATE FUNCTION enqueue_app_file() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE installation record; event_uuid text:=gen_random_uuid()::text;
BEGIN
 IF NEW.completed AND NOT OLD.completed THEN
 FOR installation IN SELECT i.* FROM app_installations i JOIN apps a ON a.id=i.app_id WHERE i.active AND a.status='installed' AND i.scopes::jsonb ? 'files:read' AND i.manifest::jsonb->'events' ? 'file.created' AND EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=NEW.channel_id AND cu.user_id=a.bot_user_id)
 LOOP
 INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload) VALUES(gen_random_uuid()::text,event_uuid,installation.id,NEW.channel_id,'file.created',jsonb_build_object('id',event_uuid,'version',1,'type','file.created','occurred_at',CURRENT_TIMESTAMP,'actor_id',NEW.user_id,'channel_id',NEW.channel_id,'data',jsonb_build_object('id',NEW.id,'name',NEW.name,'size_bytes',NEW.size_bytes,'content_type',NEW.content_type))::text);
 END LOOP;
 END IF; RETURN NULL;
END $$;
CREATE TRIGGER app_file_created AFTER UPDATE ON app_files FOR EACH ROW EXECUTE FUNCTION enqueue_app_file();
CREATE TABLE app_link_unfurls (
 message_id text NOT NULL REFERENCES messages(id),
 installation_id text NOT NULL REFERENCES app_installations(id),
 url text NOT NULL,
 card text NOT NULL,
 PRIMARY KEY(message_id,installation_id,url)
);
ALTER TABLE messages ADD COLUMN app_unfurls text;
CREATE TABLE app_responses (
 token_hash text PRIMARY KEY,
 installation_id text NOT NULL REFERENCES app_installations(id),
 user_id text NOT NULL REFERENCES users(id),
 channel_id text REFERENCES channels(id),
 expires_at timestamptz NOT NULL,
 used_at timestamptz
);
CREATE TABLE app_versions (
 app_id text NOT NULL REFERENCES apps(id),
 revision integer NOT NULL,
 manifest text NOT NULL,
 created_at timestamptz NOT NULL DEFAULT CURRENT_TIMESTAMP,
 PRIMARY KEY(app_id,revision)
);
CREATE FUNCTION save_app_version() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 INSERT INTO app_versions(app_id,revision,manifest) VALUES(NEW.id,NEW.revision,NEW.manifest) ON CONFLICT DO NOTHING;
 RETURN NULL;
END $$;
CREATE TRIGGER app_version_saved AFTER INSERT OR UPDATE ON apps FOR EACH ROW EXECUTE FUNCTION save_app_version();
CREATE FUNCTION enqueue_app_lifecycle() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE installation record; kind text; event_uuid text:=gen_random_uuid()::text;
BEGIN
 IF NEW.status=OLD.status OR NEW.status NOT IN('installed','disabled') THEN RETURN NULL; END IF;
 kind:=CASE WHEN NEW.status='installed' THEN 'app.installed' ELSE 'app.uninstalled' END;
 SELECT * INTO installation FROM app_installations WHERE app_id=NEW.id;
 IF installation.id IS NOT NULL AND installation.manifest::jsonb->'events' ? kind THEN
 INSERT INTO app_deliveries(id,event_id,installation_id,event_type,payload) VALUES(gen_random_uuid()::text,event_uuid,installation.id,kind,jsonb_build_object('id',event_uuid,'version',1,'type',kind,'occurred_at',CURRENT_TIMESTAMP,'actor_id',NEW.owner_id,'data',jsonb_build_object('app_id',NEW.id,'revision',NEW.revision))::text);
 END IF; RETURN NULL;
END $$;
CREATE TRIGGER app_lifecycle AFTER UPDATE ON apps FOR EACH ROW EXECUTE FUNCTION enqueue_app_lifecycle();

ALTER TABLE messages ADD COLUMN app_origin_id text REFERENCES apps(id);
