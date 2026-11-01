CREATE TABLE users (
  id text NOT NULL,
  user_name text NOT NULL,
  display_name varchar(255) NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  CONSTRAINT "User_pkey" PRIMARY KEY (id),
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
  CONSTRAINT "Channel_pkey" PRIMARY KEY (id),
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
  CONSTRAINT "Message_pkey" PRIMARY KEY (id),
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
  CONSTRAINT "ChannelUser_pkey" PRIMARY KEY (user_id, channel_id),
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
  CONSTRAINT "Role_pkey" PRIMARY KEY (id),
  CONSTRAINT role_role_name_key UNIQUE (role_name)
);

CREATE TABLE user_role (
  user_id text NOT NULL,
  role_id text NOT NULL,
  CONSTRAINT "UserRole_pkey" PRIMARY KEY (user_id, role_id),
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

-- Extended event sources, resource state and delivery authorization.
CREATE TABLE app_event_types(name text PRIMARY KEY, required_scope text, audience text NOT NULL);
INSERT INTO app_event_types VALUES('message.created','messages:read','channel');
INSERT INTO app_event_types VALUES('message.updated','messages:read','channel');
INSERT INTO app_event_types VALUES('message.deleted','messages:read','channel');
INSERT INTO app_event_types VALUES('message.replied','messages:read','channel');
INSERT INTO app_event_types VALUES('message.thread_broadcast','messages:read','channel');
INSERT INTO app_event_types VALUES('message.emote','messages:read','channel');
INSERT INTO app_event_types VALUES('channel.history_changed','messages:read','channel');
INSERT INTO app_event_types VALUES('user.typing','messages:read','channel');
INSERT INTO app_event_types VALUES('member.joined','channels:read','channel');
INSERT INTO app_event_types VALUES('member.left','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.created','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.updated','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.deleted','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.archived','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.unarchived','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.renamed','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.purpose_changed','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.topic_changed','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.privacy_changed','channels:read','channel');
INSERT INTO app_event_types VALUES('channel.posting_permissions_changed','channels:read','channel');
INSERT INTO app_event_types VALUES('conversation.created','channels:read','channel');
INSERT INTO app_event_types VALUES('conversation.opened','channels:read','self');
INSERT INTO app_event_types VALUES('conversation.closed','channels:read','self');
INSERT INTO app_event_types VALUES('conversation.marked','channels:read','self');
INSERT INTO app_event_types VALUES('user.joined','users:read','workspace');
INSERT INTO app_event_types VALUES('user.changed','users:read','workspace');
INSERT INTO app_event_types VALUES('user.profile_changed','users:read','workspace');
INSERT INTO app_event_types VALUES('user.status_changed','users:read','workspace');
INSERT INTO app_event_types VALUES('user.presence_changed','users:read','workspace');
INSERT INTO app_event_types VALUES('user.guest_status_changed','users:read','workspace');
INSERT INTO app_event_types VALUES('bot.added','users:read','workspace');
INSERT INTO app_event_types VALUES('bot.changed','users:read','workspace');
INSERT INTO app_event_types VALUES('user.preferences_changed','preferences:read','self');
INSERT INTO app_event_types VALUES('user.manual_presence_changed','preferences:read','self');
INSERT INTO app_event_types VALUES('user.dnd_changed','preferences:read','self');
INSERT INTO app_event_types VALUES('user.dnd_changed_public','users:read','workspace');
INSERT INTO app_event_types VALUES('pin.added','pins:read','channel');
INSERT INTO app_event_types VALUES('pin.removed','pins:read','channel');
INSERT INTO app_event_types VALUES('star.added','stars:read','self');
INSERT INTO app_event_types VALUES('star.removed','stars:read','self');
INSERT INTO app_event_types VALUES('emoji.changed','emoji:read','workspace');
INSERT INTO app_event_types VALUES('usergroup.created','usergroups:read','workspace');
INSERT INTO app_event_types VALUES('usergroup.updated','usergroups:read','workspace');
INSERT INTO app_event_types VALUES('usergroup.members_changed','usergroups:read','workspace');
INSERT INTO app_event_types VALUES('usergroup.self_added','usergroups:read','self');
INSERT INTO app_event_types VALUES('usergroup.self_removed','usergroups:read','self');
INSERT INTO app_event_types VALUES('workspace.renamed','workspace:read','workspace');
INSERT INTO app_event_types VALUES('workspace.preferences_changed','workspace:read','workspace');
INSERT INTO app_event_types VALUES('workspace.profile_changed','workspace:read','workspace');
INSERT INTO app_event_types VALUES('workspace.profile_deleted','workspace:read','workspace');
INSERT INTO app_event_types VALUES('workspace.profile_reordered','workspace:read','workspace');
INSERT INTO app_event_types VALUES('app.requested','apps:read','workspace');
INSERT INTO app_event_types VALUES('app.deleted','apps:read','workspace');
INSERT INTO app_event_types VALUES('app.installed_workspace','apps:read','workspace');
INSERT INTO app_event_types VALUES('app.uninstalled_workspace','apps:read','workspace');
INSERT INTO app_event_types VALUES('commands.changed','apps:read','workspace');
INSERT INTO app_event_types VALUES('file.created','files:read','channel');
INSERT INTO app_event_types VALUES('file.changed','files:read','channel');
INSERT INTO app_event_types VALUES('file.deleted','files:read','channel');
INSERT INTO app_event_types VALUES('file.public','files:read','channel');
INSERT INTO app_event_types VALUES('file.shared','files:read','channel');
INSERT INTO app_event_types VALUES('file.unshared','files:read','channel');
INSERT INTO app_event_types VALUES('message.metadata_posted','metadata:read','channel');
INSERT INTO app_event_types VALUES('message.metadata_updated','metadata:read','channel');
INSERT INTO app_event_types VALUES('message.metadata_deleted','metadata:read','channel');
INSERT INTO app_event_types VALUES('invite.requested','invites:read','workspace');
INSERT INTO app_event_types VALUES('reminder.created','reminders:read','self');
INSERT INTO app_event_types VALUES('reminder.fired','reminders:read','self');
INSERT INTO app_event_types VALUES('app.installed',NULL,'app');
INSERT INTO app_event_types VALUES('app.uninstalled',NULL,'app');
INSERT INTO app_event_types VALUES('tokens.revoked',NULL,'app');
INSERT INTO app_event_types VALUES('assistant.started',NULL,'app');
INSERT INTO app_event_types VALUES('assistant.context_changed',NULL,'app');
INSERT INTO app_event_types VALUES('assistant.stopped',NULL,'app');
INSERT INTO app_event_types VALUES('assistant.title_changed',NULL,'app');
INSERT INTO app_event_types VALUES('app.context_changed',NULL,'app');
INSERT INTO app_event_types VALUES('user.connection_requested',NULL,'app');
INSERT INTO app_event_types VALUES('function.executed',NULL,'app');
INSERT INTO app_event_types VALUES('entity.details_requested',NULL,'app');
INSERT INTO app_event_types VALUES('entity.comments_requested',NULL,'app');
INSERT INTO app_event_types VALUES('app.rate_limited',NULL,'control');
INSERT INTO app_event_types VALUES('app.mentioned','messages:read','channel');
INSERT INTO app_event_types VALUES('reaction.added','messages:read','channel');
INSERT INTO app_event_types VALUES('reaction.removed','messages:read','channel');
INSERT INTO app_event_types VALUES('command.invoked',NULL,'app');
INSERT INTO app_event_types VALUES('action.invoked',NULL,'app');
INSERT INTO app_event_types VALUES('view.submitted',NULL,'app');
INSERT INTO app_event_types VALUES('shortcut.invoked',NULL,'app');
INSERT INTO app_event_types VALUES('link.shared','links:write','channel');
INSERT INTO app_event_types VALUES('home.opened',NULL,'app');
CREATE TABLE app_event_aliases(alias text NOT NULL,event text NOT NULL REFERENCES app_event_types(name),conversation_kind text NOT NULL,PRIMARY KEY(alias,event,conversation_kind));
INSERT INTO app_event_aliases VALUES('agent_session_stopped','assistant.stopped','any');
INSERT INTO app_event_aliases VALUES('agent_session_title_changed','assistant.title_changed','any');
INSERT INTO app_event_aliases VALUES('app_context_changed','app.context_changed','any');
INSERT INTO app_event_aliases VALUES('app_deleted','app.deleted','any');
INSERT INTO app_event_aliases VALUES('app_home_opened','home.opened','any');
INSERT INTO app_event_aliases VALUES('app_installed','app.installed_workspace','any');
INSERT INTO app_event_aliases VALUES('app_mention','app.mentioned','any');
INSERT INTO app_event_aliases VALUES('app_rate_limited','app.rate_limited','any');
INSERT INTO app_event_aliases VALUES('app_requested','app.requested','any');
INSERT INTO app_event_aliases VALUES('app_uninstalled','app.uninstalled','any');
INSERT INTO app_event_aliases VALUES('assistant_thread_context_changed','assistant.context_changed','any');
INSERT INTO app_event_aliases VALUES('assistant_thread_started','assistant.started','any');
INSERT INTO app_event_aliases VALUES('bot_added','bot.added','any');
INSERT INTO app_event_aliases VALUES('bot_changed','bot.changed','any');
INSERT INTO app_event_aliases VALUES('channel_archive','channel.archived','public');
INSERT INTO app_event_aliases VALUES('channel_created','channel.created','public');
INSERT INTO app_event_aliases VALUES('channel_deleted','channel.deleted','public');
INSERT INTO app_event_aliases VALUES('channel_history_changed','channel.history_changed','public');
INSERT INTO app_event_aliases VALUES('channel_joined','member.joined','public');
INSERT INTO app_event_aliases VALUES('channel_left','member.left','public');
INSERT INTO app_event_aliases VALUES('channel_marked','conversation.marked','public');
INSERT INTO app_event_aliases VALUES('channel_rename','channel.renamed','public');
INSERT INTO app_event_aliases VALUES('channel_unarchive','channel.unarchived','public');
INSERT INTO app_event_aliases VALUES('commands_changed','commands.changed','any');
INSERT INTO app_event_aliases VALUES('dnd_updated','user.dnd_changed','any');
INSERT INTO app_event_aliases VALUES('dnd_updated_user','user.dnd_changed_public','any');
INSERT INTO app_event_aliases VALUES('emoji_changed','emoji.changed','any');
INSERT INTO app_event_aliases VALUES('entity_comments_requested','entity.comments_requested','any');
INSERT INTO app_event_aliases VALUES('entity_details_requested','entity.details_requested','any');
INSERT INTO app_event_aliases VALUES('file_change','file.changed','any');
INSERT INTO app_event_aliases VALUES('file_created','file.created','any');
INSERT INTO app_event_aliases VALUES('file_deleted','file.deleted','any');
INSERT INTO app_event_aliases VALUES('file_public','file.public','any');
INSERT INTO app_event_aliases VALUES('file_shared','file.shared','any');
INSERT INTO app_event_aliases VALUES('file_unshared','file.unshared','any');
INSERT INTO app_event_aliases VALUES('function_executed','function.executed','any');
INSERT INTO app_event_aliases VALUES('group_archive','channel.archived','private');
INSERT INTO app_event_aliases VALUES('group_close','conversation.closed','private');
INSERT INTO app_event_aliases VALUES('group_deleted','channel.deleted','private');
INSERT INTO app_event_aliases VALUES('group_history_changed','channel.history_changed','private');
INSERT INTO app_event_aliases VALUES('group_joined','member.joined','private');
INSERT INTO app_event_aliases VALUES('group_left','member.left','private');
INSERT INTO app_event_aliases VALUES('group_marked','conversation.marked','private');
INSERT INTO app_event_aliases VALUES('group_open','conversation.opened','private');
INSERT INTO app_event_aliases VALUES('group_rename','channel.renamed','private');
INSERT INTO app_event_aliases VALUES('group_unarchive','channel.unarchived','private');
INSERT INTO app_event_aliases VALUES('im_close','conversation.closed','im');
INSERT INTO app_event_aliases VALUES('im_created','conversation.created','im');
INSERT INTO app_event_aliases VALUES('im_history_changed','channel.history_changed','im');
INSERT INTO app_event_aliases VALUES('im_marked','conversation.marked','im');
INSERT INTO app_event_aliases VALUES('im_open','conversation.opened','im');
INSERT INTO app_event_aliases VALUES('invite_requested','invite.requested','any');
INSERT INTO app_event_aliases VALUES('link_shared','link.shared','any');
INSERT INTO app_event_aliases VALUES('manual_presence_change','user.manual_presence_changed','any');
INSERT INTO app_event_aliases VALUES('member_joined_channel','member.joined','any');
INSERT INTO app_event_aliases VALUES('member_left_channel','member.left','any');
INSERT INTO app_event_aliases VALUES('message','message.created','any');
INSERT INTO app_event_aliases VALUES('message','message.updated','any');
INSERT INTO app_event_aliases VALUES('message','message.deleted','any');
INSERT INTO app_event_aliases VALUES('message','message.replied','any');
INSERT INTO app_event_aliases VALUES('message','message.thread_broadcast','any');
INSERT INTO app_event_aliases VALUES('message','message.emote','any');
INSERT INTO app_event_aliases VALUES('message.app_home','message.created','app_dm');
INSERT INTO app_event_aliases VALUES('message.app_home','message.updated','app_dm');
INSERT INTO app_event_aliases VALUES('message.app_home','message.deleted','app_dm');
INSERT INTO app_event_aliases VALUES('message.app_home','message.replied','app_dm');
INSERT INTO app_event_aliases VALUES('message.app_home','message.thread_broadcast','app_dm');
INSERT INTO app_event_aliases VALUES('message.app_home','message.emote','app_dm');
INSERT INTO app_event_aliases VALUES('message.channels','message.created','public');
INSERT INTO app_event_aliases VALUES('message.channels','message.updated','public');
INSERT INTO app_event_aliases VALUES('message.channels','message.deleted','public');
INSERT INTO app_event_aliases VALUES('message.channels','message.replied','public');
INSERT INTO app_event_aliases VALUES('message.channels','message.thread_broadcast','public');
INSERT INTO app_event_aliases VALUES('message.channels','message.emote','public');
INSERT INTO app_event_aliases VALUES('message.groups','message.created','private');
INSERT INTO app_event_aliases VALUES('message.groups','message.updated','private');
INSERT INTO app_event_aliases VALUES('message.groups','message.deleted','private');
INSERT INTO app_event_aliases VALUES('message.groups','message.replied','private');
INSERT INTO app_event_aliases VALUES('message.groups','message.thread_broadcast','private');
INSERT INTO app_event_aliases VALUES('message.groups','message.emote','private');
INSERT INTO app_event_aliases VALUES('message.im','message.created','im');
INSERT INTO app_event_aliases VALUES('message.im','message.updated','im');
INSERT INTO app_event_aliases VALUES('message.im','message.deleted','im');
INSERT INTO app_event_aliases VALUES('message.im','message.replied','im');
INSERT INTO app_event_aliases VALUES('message.im','message.thread_broadcast','im');
INSERT INTO app_event_aliases VALUES('message.im','message.emote','im');
INSERT INTO app_event_aliases VALUES('message.mpim','message.created','mpim');
INSERT INTO app_event_aliases VALUES('message.mpim','message.updated','mpim');
INSERT INTO app_event_aliases VALUES('message.mpim','message.deleted','mpim');
INSERT INTO app_event_aliases VALUES('message.mpim','message.replied','mpim');
INSERT INTO app_event_aliases VALUES('message.mpim','message.thread_broadcast','mpim');
INSERT INTO app_event_aliases VALUES('message.mpim','message.emote','mpim');
INSERT INTO app_event_aliases VALUES('message_metadata_deleted','message.metadata_deleted','any');
INSERT INTO app_event_aliases VALUES('message_metadata_posted','message.metadata_posted','any');
INSERT INTO app_event_aliases VALUES('message_metadata_updated','message.metadata_updated','any');
INSERT INTO app_event_aliases VALUES('pin_added','pin.added','any');
INSERT INTO app_event_aliases VALUES('pin_removed','pin.removed','any');
INSERT INTO app_event_aliases VALUES('pref_change','user.preferences_changed','any');
INSERT INTO app_event_aliases VALUES('presence_change','user.presence_changed','any');
INSERT INTO app_event_aliases VALUES('reaction_added','reaction.added','any');
INSERT INTO app_event_aliases VALUES('reaction_removed','reaction.removed','any');
INSERT INTO app_event_aliases VALUES('star_added','star.added','any');
INSERT INTO app_event_aliases VALUES('star_removed','star.removed','any');
INSERT INTO app_event_aliases VALUES('subteam_created','usergroup.created','any');
INSERT INTO app_event_aliases VALUES('subteam_members_changed','usergroup.members_changed','any');
INSERT INTO app_event_aliases VALUES('subteam_self_added','usergroup.self_added','any');
INSERT INTO app_event_aliases VALUES('subteam_self_removed','usergroup.self_removed','any');
INSERT INTO app_event_aliases VALUES('subteam_updated','usergroup.updated','any');
INSERT INTO app_event_aliases VALUES('team_join','user.joined','any');
INSERT INTO app_event_aliases VALUES('team_pref_change','workspace.preferences_changed','any');
INSERT INTO app_event_aliases VALUES('team_profile_change','workspace.profile_changed','any');
INSERT INTO app_event_aliases VALUES('team_profile_delete','workspace.profile_deleted','any');
INSERT INTO app_event_aliases VALUES('team_profile_reorder','workspace.profile_reordered','any');
INSERT INTO app_event_aliases VALUES('team_rename','workspace.renamed','any');
INSERT INTO app_event_aliases VALUES('tokens_revoked','tokens.revoked','any');
INSERT INTO app_event_aliases VALUES('user_change','user.changed','any');
INSERT INTO app_event_aliases VALUES('user_connection','user.connection_requested','any');
INSERT INTO app_event_aliases VALUES('user_guest_status_changed','user.guest_status_changed','any');
INSERT INTO app_event_aliases VALUES('user_profile_changed','user.profile_changed','any');
INSERT INTO app_event_aliases VALUES('user_status_changed','user.status_changed','any');
INSERT INTO app_event_aliases VALUES('user_typing','user.typing','any');
INSERT INTO app_event_aliases VALUES('message.assistant_app_thread','assistant.started','any');
INSERT INTO app_event_aliases VALUES('message.bot_message','message.created','bot');
INSERT INTO app_event_aliases VALUES('message.channel_archive','channel.archived','public');
INSERT INTO app_event_aliases VALUES('message.channel_convert_to_private','channel.privacy_changed','private');
INSERT INTO app_event_aliases VALUES('message.channel_convert_to_public','channel.privacy_changed','public');
INSERT INTO app_event_aliases VALUES('message.channel_join','member.joined','public');
INSERT INTO app_event_aliases VALUES('message.channel_leave','member.left','public');
INSERT INTO app_event_aliases VALUES('message.channel_name','channel.renamed','public');
INSERT INTO app_event_aliases VALUES('message.channel_posting_permissions','channel.posting_permissions_changed','public');
INSERT INTO app_event_aliases VALUES('message.channel_purpose','channel.purpose_changed','public');
INSERT INTO app_event_aliases VALUES('message.channel_topic','channel.topic_changed','public');
INSERT INTO app_event_aliases VALUES('message.channel_unarchive','channel.unarchived','public');
INSERT INTO app_event_aliases VALUES('message.file_mention','file.shared','any');
INSERT INTO app_event_aliases VALUES('message.file_share','file.shared','any');
INSERT INTO app_event_aliases VALUES('message.group_archive','channel.archived','private');
INSERT INTO app_event_aliases VALUES('message.group_join','member.joined','private');
INSERT INTO app_event_aliases VALUES('message.group_leave','member.left','private');
INSERT INTO app_event_aliases VALUES('message.group_name','channel.renamed','private');
INSERT INTO app_event_aliases VALUES('message.group_purpose','channel.purpose_changed','private');
INSERT INTO app_event_aliases VALUES('message.group_topic','channel.topic_changed','private');
INSERT INTO app_event_aliases VALUES('message.group_unarchive','channel.unarchived','private');
INSERT INTO app_event_aliases VALUES('message.me_message','message.emote','any');
INSERT INTO app_event_aliases VALUES('message.message_changed','message.updated','any');
INSERT INTO app_event_aliases VALUES('message.message_deleted','message.deleted','any');
INSERT INTO app_event_aliases VALUES('message.message_replied','message.replied','any');
INSERT INTO app_event_aliases VALUES('message.pinned_item','pin.added','any');
INSERT INTO app_event_aliases VALUES('message.reminder_add','reminder.created','any');
INSERT INTO app_event_aliases VALUES('message.reply_broadcast','message.thread_broadcast','any');
INSERT INTO app_event_aliases VALUES('message.thread_broadcast','message.thread_broadcast','any');
INSERT INTO app_event_aliases VALUES('message.unpinned_item','pin.removed','any');

ALTER TABLE users ADD COLUMN is_guest boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN guest_kind text NOT NULL DEFAULT 'none' CHECK(guest_kind IN('none','single','multi','deactivated'));
ALTER TABLE users ADD COLUMN is_deactivated boolean NOT NULL DEFAULT false;
ALTER TABLE users ADD COLUMN status_text text NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN status_emoji text NOT NULL DEFAULT '';
ALTER TABLE users ADD COLUMN status_expires_at timestamptz;
ALTER TABLE users ADD COLUMN profile_fields text NOT NULL DEFAULT '{}';
ALTER TABLE channels ADD COLUMN topic text NOT NULL DEFAULT '';
ALTER TABLE channels ADD COLUMN posting_policy text NOT NULL DEFAULT 'members' CHECK(posting_policy IN('members','managers'));
ALTER TABLE messages ADD COLUMN message_kind text NOT NULL DEFAULT 'message' CHECK(message_kind IN('message','emote'));
ALTER TABLE messages ADD COLUMN broadcast boolean NOT NULL DEFAULT false;
ALTER TABLE messages ADD COLUMN metadata text;
ALTER TABLE apps ADD COLUMN deleted_at timestamptz;
ALTER TABLE app_installations ADD COLUMN endpoint_verified_at timestamptz;
ALTER TABLE app_deliveries ADD COLUMN required_scope text;
ALTER TABLE app_deliveries ADD COLUMN subject_user_id text REFERENCES users(id);
ALTER TABLE app_deliveries ADD COLUMN access_user_id text REFERENCES users(id);
ALTER TABLE app_deliveries ADD COLUMN allow_departed boolean NOT NULL DEFAULT false;
CREATE TABLE app_event_limits(installation_id text NOT NULL REFERENCES app_installations(id),window_start timestamptz NOT NULL,events bigint NOT NULL DEFAULT 0,PRIMARY KEY(installation_id,window_start));
CREATE TABLE user_event_settings(
 user_id text PRIMARY KEY REFERENCES users(id),preferences text NOT NULL DEFAULT '{}',
 presence text NOT NULL DEFAULT 'away' CHECK(presence IN('active','away')),
 presence_override text NOT NULL DEFAULT 'auto' CHECK(presence_override IN('auto','away')),
 active_until timestamptz,dnd_until timestamptz
);
CREATE TABLE conversation_views(user_id text NOT NULL REFERENCES users(id),channel_id text NOT NULL REFERENCES channels(id),is_open boolean NOT NULL DEFAULT true,PRIMARY KEY(user_id,channel_id));
CREATE TABLE channel_pins(channel_id text NOT NULL REFERENCES channels(id),item_type text NOT NULL CHECK(item_type IN('message','file')),item_id text NOT NULL,created_by text NOT NULL REFERENCES users(id),created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(channel_id,item_type,item_id));
CREATE TABLE saved_items(user_id text NOT NULL REFERENCES users(id),item_type text NOT NULL CHECK(item_type IN('message','channel','file')),item_id text NOT NULL,channel_id text REFERENCES channels(id),created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(user_id,item_type,item_id));
CREATE TABLE workspace_emojis(name text PRIMARY KEY,url text NOT NULL,created_by text NOT NULL REFERENCES users(id),updated_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE workspace_settings(id text PRIMARY KEY CHECK(id='workspace'),name text NOT NULL DEFAULT 'Zoo',preferences text NOT NULL DEFAULT '{}',profile_fields text NOT NULL DEFAULT '[]');
INSERT INTO workspace_settings(id) VALUES('workspace');
CREATE TABLE usergroups(id text PRIMARY KEY,name text NOT NULL,handle text NOT NULL UNIQUE,description text NOT NULL DEFAULT '',active boolean NOT NULL DEFAULT true,created_by text NOT NULL REFERENCES users(id),created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE usergroup_members(group_id text NOT NULL REFERENCES usergroups(id),user_id text NOT NULL REFERENCES users(id),PRIMARY KEY(group_id,user_id));
CREATE TABLE workspace_invite_requests(id text PRIMARY KEY,email text NOT NULL,requested_by text NOT NULL REFERENCES users(id),channel_id text REFERENCES channels(id),status text NOT NULL DEFAULT 'pending' CHECK(status IN('pending','approved','rejected')),resolved_by text REFERENCES users(id),created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE user_reminders(id text PRIMARY KEY,user_id text NOT NULL REFERENCES users(id),channel_id text REFERENCES channels(id),content text NOT NULL,due_at timestamptz NOT NULL,fired_at timestamptz,completed_at timestamptz);
CREATE INDEX reminders_due ON user_reminders(due_at) WHERE fired_at IS NULL AND completed_at IS NULL;
CREATE TABLE assistant_sessions(id text PRIMARY KEY,installation_id text NOT NULL REFERENCES app_installations(id),user_id text NOT NULL REFERENCES users(id),actor_id text NOT NULL REFERENCES users(id),channel_id text NOT NULL REFERENCES channels(id),parent_id text REFERENCES messages(id),title text NOT NULL DEFAULT '',context text NOT NULL DEFAULT '{}',state text NOT NULL DEFAULT 'running' CHECK(state IN('running','stopped')),created_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE app_view_contexts(installation_id text NOT NULL REFERENCES app_installations(id),user_id text NOT NULL REFERENCES users(id),channel_id text REFERENCES channels(id),context text NOT NULL DEFAULT '{}',PRIMARY KEY(installation_id,user_id));
CREATE TABLE app_user_connections(installation_id text NOT NULL REFERENCES app_installations(id),user_id text NOT NULL REFERENCES users(id),state text NOT NULL CHECK(state IN('requested','connected','disconnected')),PRIMARY KEY(installation_id,user_id));
CREATE TABLE app_function_executions(id text PRIMARY KEY,installation_id text NOT NULL REFERENCES app_installations(id),actor_id text NOT NULL REFERENCES users(id),channel_id text REFERENCES channels(id),name text NOT NULL,inputs text NOT NULL DEFAULT '{}',outputs text,error text,state text NOT NULL DEFAULT 'pending' CHECK(state IN('pending','completed','failed')),created_at timestamptz NOT NULL DEFAULT now());
ALTER TABLE app_files ADD COLUMN description text NOT NULL DEFAULT '';
ALTER TABLE app_files ADD COLUMN deleted_at timestamptz;
ALTER TABLE app_files ADD COLUMN public_token_hash text UNIQUE;
CREATE TABLE app_file_shares(file_id text NOT NULL REFERENCES app_files(id),channel_id text NOT NULL REFERENCES channels(id),created_by text NOT NULL REFERENCES users(id),PRIMARY KEY(file_id,channel_id));
INSERT INTO app_file_shares SELECT id,channel_id,user_id FROM app_files WHERE completed;

-- A subscription may use native names or documented Slack semantic aliases.
CREATE FUNCTION app_event_matches(manifest text,event_name text,channel text,actor text,identity text DEFAULT NULL) RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT COALESCE(manifest::jsonb->'events' ? event_name,false) OR EXISTS(
  SELECT 1 FROM app_event_aliases x WHERE x.event=event_name AND manifest::jsonb->'events' ? x.alias AND (x.alias NOT IN('channel_joined','channel_left','group_joined','group_left') OR actor=identity) AND (
   x.conversation_kind='any' OR
   x.conversation_kind='bot' AND EXISTS(SELECT 1 FROM users WHERE id=actor AND is_bot) OR
   EXISTS(SELECT 1 FROM channels c WHERE c.id=channel AND (
    x.conversation_kind='public' AND NOT c.is_private AND NOT c.is_dm OR
    x.conversation_kind='private' AND c.is_private AND NOT c.is_dm OR
    (x.conversation_kind='im' OR x.conversation_kind='app_dm' AND EXISTS(SELECT 1 FROM channel_user cu JOIN users u ON u.id=cu.user_id WHERE cu.channel_id=c.id AND u.is_bot AND u.id=identity)) AND c.is_dm AND (SELECT count(*) FROM channel_user cu WHERE cu.channel_id=c.id)<=2 OR
    x.conversation_kind='mpim' AND c.is_dm AND (SELECT count(*) FROM channel_user cu WHERE cu.channel_id=c.id)>2
   ))
  )
 )
$$;

-- Find an identity that still grants access. Personal events require that exact user's consent.
CREATE FUNCTION app_event_access(installation text,scope_name text,channel text,subject text,departed text DEFAULT NULL,terminal boolean DEFAULT false) RETURNS text LANGUAGE sql STABLE AS $$
 SELECT identity.user_id FROM app_installations i JOIN apps a ON a.id=i.app_id
 CROSS JOIN LATERAL (
  SELECT a.bot_user_id AS user_id,true AS bot
  UNION ALL
  SELECT t.user_id,false FROM app_credentials t JOIN users u ON u.id=t.user_id
   WHERE t.installation_id=i.id AND t.revoked_at IS NULL AND t.expires_at>now() AND NOT u.is_deactivated
   AND (scope_name IS NULL OR t.scopes::jsonb ? scope_name)
 ) identity
 WHERE i.id=installation AND ((i.active AND a.status='installed' AND a.deleted_at IS NULL) OR terminal AND scope_name IS NULL)
 AND (scope_name IS NULL OR i.scopes::jsonb ? scope_name)
 AND (subject IS NULL OR identity.user_id=subject)
 AND (channel IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=channel
  AND (c.deleted_at IS NULL OR terminal)
  AND (identity.user_id=departed OR scope_name='channels:read' AND NOT c.is_private AND NOT c.is_dm AND NOT EXISTS(SELECT 1 FROM users WHERE id=identity.user_id AND is_guest) OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=identity.user_id))))
 ORDER BY identity.bot DESC LIMIT 1
$$;

CREATE FUNCTION app_emit(event_name text,event_data jsonb,actor text,channel text DEFAULT NULL,subject text DEFAULT NULL,target_installation text DEFAULT NULL,departed text DEFAULT NULL) RETURNS void LANGUAGE plpgsql AS $$
DECLARE definition app_event_types; installation record; identity text; effective_subject text; event_uuid text:=md5(CASE WHEN event_name='app.rate_limited' THEN date_trunc('hour',now())::text||COALESCE(target_installation,'') ELSE pg_current_xact_id()::text||event_data::text||COALESCE(actor,'')||COALESCE(channel,'')||COALESCE(subject,'') END||event_name)::uuid::text; count_now bigint; terminal boolean; envelope jsonb;
BEGIN
 SELECT * INTO STRICT definition FROM app_event_types WHERE name=event_name;
 terminal:=event_name IN('app.uninstalled','app.deleted','tokens.revoked','channel.deleted');
 FOR installation IN SELECT i.*,a.bot_user_id FROM app_installations i JOIN apps a ON a.id=i.app_id
 WHERE (target_installation IS NULL OR i.id=target_installation)
 AND (definition.audience NOT IN('app','control') OR target_installation IS NOT NULL)
 AND (app_event_matches(i.manifest,event_name,channel,actor,app_event_access(i.id,definition.required_scope,channel,subject,departed,terminal))
 OR event_name IN('member.joined','member.left') AND app_event_access(i.id,definition.required_scope,channel,actor,departed,terminal)=actor AND app_event_matches(i.manifest,event_name,channel,actor,actor))
 LOOP
  identity:=app_event_access(installation.id,definition.required_scope,channel,subject,departed,terminal);
  effective_subject:=subject;
  IF NOT app_event_matches(installation.manifest,event_name,channel,actor,identity) THEN
   effective_subject:=actor;
   identity:=app_event_access(installation.id,definition.required_scope,channel,effective_subject,departed,terminal);
  END IF;
  IF identity IS NULL THEN CONTINUE; END IF;
  -- Suppression applies only to bot-authored message/typing loops, never channel or bot lifecycle changes.
  IF (event_name LIKE 'message.%' OR event_name IN('app.mentioned','user.typing'))
   AND EXISTS(SELECT 1 FROM users WHERE id=actor AND is_bot)
   AND NOT COALESCE((installation.manifest::jsonb->>'include_bot_events')::boolean,false)
   AND NOT COALESCE(installation.manifest::jsonb->'events' ? 'message.bot_message',false) THEN CONTINUE; END IF;
  IF EXISTS(SELECT 1 FROM app_deliveries WHERE installation_id=installation.id AND event_id=event_uuid) THEN CONTINUE; END IF;
  IF event_name<>'app.rate_limited' THEN
   -- Append-only accounting avoids taking installation locks in source transactions.
   SELECT COALESCE((SELECT events FROM app_event_limits WHERE installation_id=installation.id AND window_start=date_trunc('hour',now())),0)+count(*)+1 INTO count_now
    FROM app_deliveries WHERE installation_id=installation.id AND created_at>=date_trunc('hour',now()) AND event_type<>'app.rate_limited';
   IF count_now>30000 THEN
    IF app_event_matches(installation.manifest,'app.rate_limited',NULL,NULL) THEN
     PERFORM app_emit('app.rate_limited',jsonb_build_object('installation_id',installation.id,'window_start',date_trunc('hour',now()),'limit',30000),NULL,NULL,NULL,installation.id);
    END IF;
    CONTINUE;
   END IF;
  END IF;
  envelope:=jsonb_build_object('id',event_uuid,'version',1,'type',event_name,'occurred_at',now(),'actor_id',actor,'channel_id',channel,'subject_user_id',effective_subject,'data',event_data);
  INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload,required_scope,subject_user_id,access_user_id,allow_departed)
   VALUES(gen_random_uuid()::text,event_uuid,installation.id,channel,event_name,envelope::text,definition.required_scope,effective_subject,identity,COALESCE(identity=departed,false)) ON CONFLICT(installation_id,event_id) DO NOTHING;
 END LOOP;
END $$;

CREATE FUNCTION app_delivery_allowed(delivery app_deliveries) RETURNS boolean LANGUAGE sql STABLE AS $$
 SELECT app_event_access(delivery.installation_id,
 COALESCE(delivery.required_scope,(SELECT required_scope FROM app_event_types WHERE name=delivery.event_type)),
 delivery.channel_id,delivery.subject_user_id,
 CASE WHEN delivery.allow_departed THEN delivery.access_user_id END,
 delivery.event_type IN('app.uninstalled','app.deleted','tokens.revoked','channel.deleted')) IS NOT NULL
$$;

-- Source events commit with their mutations; no endpoint can publish arbitrary events.
DROP TRIGGER app_message_events ON messages;
DROP TRIGGER app_reaction_events ON message_reactions;
DROP TRIGGER app_member_events ON channel_user;
DROP TRIGGER app_channel_events ON channels;
DROP TRIGGER app_special_message_events ON messages;
DROP TRIGGER app_file_created ON app_files;
DROP TRIGGER app_lifecycle ON apps;

CREATE FUNCTION app_message_options() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE options jsonb:=COALESCE(NULLIF(current_setting('zoo.message_options',true),''),'{}')::jsonb;
BEGIN
 NEW.message_kind:=COALESCE(options->>'kind','message');
 NEW.broadcast:=COALESCE((options->>'broadcast')::boolean,false);
 NEW.metadata:=CASE WHEN options ? 'metadata' AND options->'metadata'<>'null'::jsonb THEN (options->'metadata')::text END;
 RETURN NEW;
END $$;
CREATE TRIGGER app_message_options BEFORE INSERT ON messages FOR EACH ROW EXECUTE FUNCTION app_message_options();
CREATE FUNCTION app_message_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb; row_data jsonb; old_data jsonb; installation record;
BEGIN
 row_data:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
 old_data:=CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END;
 data:=row_data||jsonb_build_object('previous',old_data);
 IF TG_OP='INSERT' THEN
  PERFORM app_emit('message.created',data,NEW.user_id,NEW.channel_id);
  IF NEW.parent_id IS NOT NULL THEN PERFORM app_emit('message.replied',jsonb_build_object('parent_id',NEW.parent_id,'reply',row_data),NEW.user_id,NEW.channel_id);END IF;
  IF NEW.broadcast THEN PERFORM app_emit('message.thread_broadcast',data,NEW.user_id,NEW.channel_id);END IF;
  IF NEW.message_kind='emote' THEN PERFORM app_emit('message.emote',data,NEW.user_id,NEW.channel_id);END IF;
  IF NEW.metadata IS NOT NULL THEN PERFORM app_emit('message.metadata_posted',jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb),NEW.user_id,NEW.channel_id);END IF;
  FOR installation IN SELECT i.*,a.bot_user_id,u.user_name FROM app_installations i JOIN apps a ON a.id=i.app_id JOIN users u ON u.id=a.bot_user_id WHERE i.active AND a.status='installed'
  LOOP
   IF is_mention(NEW.content,installation.user_name) THEN PERFORM app_emit('app.mentioned',data,NEW.user_id,NEW.channel_id,NULL,installation.id);END IF;
   IF EXISTS(SELECT 1 FROM jsonb_array_elements_text(COALESCE(installation.manifest::jsonb->'link_domains','[]')) domain WHERE NEW.content ~ ('https://' || replace(domain.value,'.','[.]') || '([/:?#[:space:]]|$)')) THEN
    PERFORM app_emit('link.shared',data,NEW.user_id,NEW.channel_id,NULL,installation.id);
   END IF;
  END LOOP;
 ELSIF TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND old_data->>'deleted_at' IS NULL THEN
  PERFORM app_emit('message.deleted',data,row_data->>'user_id',row_data->>'channel_id');
  IF old_data->>'metadata' IS NOT NULL THEN PERFORM app_emit('message.metadata_deleted',jsonb_build_object('id',OLD.id,'previous_metadata',OLD.metadata::jsonb),OLD.user_id,OLD.channel_id);END IF;
 ELSIF row_data IS DISTINCT FROM old_data THEN
  PERFORM app_emit('message.updated',data,NEW.user_id,NEW.channel_id);
  IF NEW.metadata IS DISTINCT FROM OLD.metadata THEN
   PERFORM app_emit(CASE WHEN NEW.metadata IS NULL THEN 'message.metadata_deleted' WHEN OLD.metadata IS NULL THEN 'message.metadata_posted' ELSE 'message.metadata_updated' END,
    jsonb_build_object('id',NEW.id,'metadata',NEW.metadata::jsonb,'previous_metadata',OLD.metadata::jsonb),NEW.user_id,NEW.channel_id);
  END IF;
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER app_message_events AFTER INSERT OR UPDATE OR DELETE ON messages FOR EACH ROW EXECUTE FUNCTION app_message_events_v2();
CREATE FUNCTION app_channel_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE row_data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;channel text:=row_data->>'id';actor text:=COALESCE(NULLIF(current_setting('zoo.actor_id',true),''),row_data->>'created_user_id');
BEGIN
 IF TG_OP='INSERT' AND NOT NEW.is_private AND NOT NEW.is_dm THEN
  PERFORM app_emit(CASE WHEN NEW.is_dm THEN 'conversation.created' ELSE 'channel.created' END,row_data,actor,channel);
 ELSIF TG_OP='INSERT' THEN NULL;
 ELSIF TG_OP='DELETE' OR row_data->>'deleted_at' IS NOT NULL AND to_jsonb(OLD)->>'deleted_at' IS NULL THEN
  PERFORM app_emit('channel.deleted',row_data,actor,channel);
 ELSE
  IF OLD.archive_at IS DISTINCT FROM NEW.archive_at THEN PERFORM app_emit(CASE WHEN NEW.archive_at IS NULL THEN 'channel.unarchived' ELSE 'channel.archived' END,row_data,actor,channel);END IF;
  IF OLD.channel_name<>NEW.channel_name THEN PERFORM app_emit('channel.renamed',row_data||jsonb_build_object('previous_name',OLD.channel_name),actor,channel);END IF;
  IF OLD.description IS DISTINCT FROM NEW.description THEN PERFORM app_emit('channel.purpose_changed',row_data,actor,channel);END IF;
  IF OLD.topic IS DISTINCT FROM NEW.topic THEN PERFORM app_emit('channel.topic_changed',row_data,actor,channel);END IF;
  IF OLD.is_private<>NEW.is_private THEN PERFORM app_emit('channel.privacy_changed',row_data||jsonb_build_object('previous_is_private',OLD.is_private),actor,channel);END IF;
  IF OLD.posting_policy<>NEW.posting_policy THEN PERFORM app_emit('channel.posting_permissions_changed',row_data,actor,channel);END IF;
  IF to_jsonb(OLD) IS DISTINCT FROM row_data THEN PERFORM app_emit('channel.updated',row_data,actor,channel);END IF;
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER app_channel_events AFTER INSERT OR UPDATE OR DELETE ON channels FOR EACH ROW EXECUTE FUNCTION app_channel_events_v2();
CREATE FUNCTION app_member_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE row_data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
BEGIN
 PERFORM app_emit(CASE WHEN TG_OP='DELETE' THEN 'member.left' ELSE 'member.joined' END,row_data,row_data->>'user_id',row_data->>'channel_id',NULL,NULL,CASE WHEN TG_OP='DELETE' THEN row_data->>'user_id' END);
 IF TG_OP='INSERT' THEN
  PERFORM app_emit(CASE WHEN c.is_dm THEN 'conversation.created' ELSE 'channel.created' END,to_jsonb(c),c.created_user_id,c.id) FROM channels c WHERE c.id=NEW.channel_id AND (c.is_private OR c.is_dm) AND c.created_at >= date_trunc('millisecond',transaction_timestamp() AT TIME ZONE 'UTC');
 END IF;
 RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;
-- BEFORE DELETE captures the last member's access before it disappears.
CREATE TRIGGER app_member_deleted BEFORE DELETE ON channel_user FOR EACH ROW EXECUTE FUNCTION app_member_events_v2();
CREATE TRIGGER app_member_events AFTER INSERT ON channel_user FOR EACH ROW EXECUTE FUNCTION app_member_events_v2();

CREATE FUNCTION app_reaction_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;channel text;
BEGIN
 SELECT channel_id INTO channel FROM messages WHERE id=data->>'message_id';
 PERFORM app_emit(CASE WHEN TG_OP='DELETE' THEN 'reaction.removed' ELSE 'reaction.added' END,data,data->>'user_id',channel);
 RETURN NULL;
END $$;
CREATE TRIGGER app_reaction_events AFTER INSERT OR DELETE ON message_reactions FOR EACH ROW EXECUTE FUNCTION app_reaction_events_v2();
CREATE FUNCTION app_typing_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' OR OLD.expires_at<=now() OR NEW.parent_id IS DISTINCT FROM OLD.parent_id THEN
  PERFORM app_emit('user.typing',jsonb_build_object('user_id',NEW.user_id,'parent_id',NEW.parent_id,'expires_at',NEW.expires_at),NEW.user_id,NEW.channel_id);
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_typing_events AFTER INSERT OR UPDATE ON typing_presence FOR EACH ROW EXECUTE FUNCTION app_typing_events();
CREATE FUNCTION app_read_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.last_read_at IS DISTINCT FROM OLD.last_read_at THEN PERFORM app_emit('conversation.marked',jsonb_build_object('last_read_at',NEW.last_read_at),NEW.user_id,NEW.channel_id,NEW.user_id);END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_read_events AFTER INSERT OR UPDATE ON channel_read_states FOR EACH ROW EXECUTE FUNCTION app_read_events();
CREATE FUNCTION app_view_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.is_open IS DISTINCT FROM OLD.is_open THEN PERFORM app_emit(CASE WHEN NEW.is_open THEN 'conversation.opened' ELSE 'conversation.closed' END,jsonb_build_object('user_id',NEW.user_id,'is_open',NEW.is_open),NEW.user_id,NEW.channel_id,NEW.user_id);END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_view_events AFTER INSERT OR UPDATE ON conversation_views FOR EACH ROW EXECUTE FUNCTION app_view_events();
CREATE FUNCTION app_guest_rules() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 NEW.is_guest:=NEW.guest_kind<>'none';NEW.is_deactivated:=NEW.guest_kind='deactivated';
 IF NEW.guest_kind='single' AND (SELECT count(*) FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.user_id=NEW.id AND NOT c.is_dm AND c.deleted_at IS NULL)>1 THEN RAISE EXCEPTION USING ERRCODE='23514',MESSAGE='Single-channel guests can join only one channel';END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER app_guest_rules BEFORE INSERT OR UPDATE OF guest_kind ON users FOR EACH ROW EXECUTE FUNCTION app_guest_rules();
CREATE FUNCTION app_guest_membership_rules() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM users WHERE id=NEW.user_id AND guest_kind='single') THEN PERFORM 1 FROM users WHERE id=NEW.user_id FOR UPDATE;END IF;
 IF EXISTS(SELECT 1 FROM users WHERE id=NEW.user_id AND guest_kind='single')
 AND EXISTS(SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_dm)
 AND EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.user_id=NEW.user_id AND cu.channel_id<>NEW.channel_id AND NOT c.is_dm AND c.deleted_at IS NULL) THEN RAISE EXCEPTION USING ERRCODE='23514',MESSAGE='Single-channel guests can join only one channel';END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER app_guest_membership_rules BEFORE INSERT ON channel_user FOR EACH ROW EXECUTE FUNCTION app_guest_membership_rules();
CREATE FUNCTION app_user_events() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=jsonb_build_object('id',NEW.id,'user_name',NEW.user_name,'display_name',NEW.display_name,'is_bot',NEW.is_bot,'app_id',NEW.app_id,'avatar_url',NEW.avatar_url,'guest_kind',NEW.guest_kind,'status_text',NEW.status_text,'status_emoji',NEW.status_emoji,'status_expires_at',NEW.status_expires_at,'profile_fields',NEW.profile_fields::jsonb);
BEGIN
 IF TG_OP='INSERT' AND NOT NEW.is_bot THEN PERFORM app_emit('user.joined',data,NEW.id);
 ELSIF TG_OP='UPDATE' THEN
  IF NEW.is_bot AND OLD.app_id IS NULL AND NEW.app_id IS NOT NULL THEN PERFORM app_emit('bot.added',data,NEW.id);
  ELSIF (to_jsonb(NEW)-'updated_at') IS DISTINCT FROM (to_jsonb(OLD)-'updated_at') THEN
   PERFORM app_emit(CASE WHEN NEW.is_bot THEN 'bot.changed' ELSE 'user.changed' END,data,NEW.id);
  END IF;
  IF NOT NEW.is_bot AND (NEW.display_name,NEW.user_name,NEW.avatar_url,NEW.profile_fields) IS DISTINCT FROM (OLD.display_name,OLD.user_name,OLD.avatar_url,OLD.profile_fields) THEN PERFORM app_emit('user.profile_changed',data,NEW.id);END IF;
  IF (NEW.status_text,NEW.status_emoji,NEW.status_expires_at) IS DISTINCT FROM (OLD.status_text,OLD.status_emoji,OLD.status_expires_at) THEN PERFORM app_emit('user.status_changed',data,NEW.id);END IF;
  IF OLD.guest_kind<>NEW.guest_kind THEN PERFORM app_emit('user.guest_status_changed',data||jsonb_build_object('previous_guest_kind',OLD.guest_kind),NEW.id);END IF;
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_user_events AFTER INSERT OR UPDATE ON users FOR EACH ROW EXECUTE FUNCTION app_user_events();
CREATE FUNCTION app_user_setting_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.preferences IS DISTINCT FROM OLD.preferences THEN PERFORM app_emit('user.preferences_changed',NEW.preferences::jsonb,NEW.user_id,NULL,NEW.user_id);END IF;
 IF TG_OP='INSERT' OR NEW.presence<>OLD.presence THEN PERFORM app_emit('user.presence_changed',jsonb_build_object('user_id',NEW.user_id,'presence',NEW.presence),NEW.user_id);END IF;
 IF TG_OP='INSERT' OR NEW.presence_override<>OLD.presence_override THEN PERFORM app_emit('user.manual_presence_changed',jsonb_build_object('presence',NEW.presence_override),NEW.user_id,NULL,NEW.user_id);END IF;
 IF TG_OP='INSERT' AND NEW.dnd_until IS NOT NULL OR TG_OP='UPDATE' AND NEW.dnd_until IS DISTINCT FROM OLD.dnd_until THEN
  PERFORM app_emit('user.dnd_changed',jsonb_build_object('user_id',NEW.user_id,'dnd_until',NEW.dnd_until,'enabled',COALESCE(NEW.dnd_until>now(),false)),NEW.user_id,NULL,NEW.user_id);
  PERFORM app_emit('user.dnd_changed_public',jsonb_build_object('user_id',NEW.user_id,'enabled',COALESCE(NEW.dnd_until>now(),false)),NEW.user_id);
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_user_setting_events AFTER INSERT OR UPDATE ON user_event_settings FOR EACH ROW EXECUTE FUNCTION app_user_setting_events();
CREATE FUNCTION app_collection_events() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;kind text;actor text;subject text;channel text;
BEGIN
 IF TG_OP='UPDATE' AND to_jsonb(OLD)=data THEN RETURN NULL;END IF;
 IF TG_TABLE_NAME='channel_pins' THEN kind:=CASE WHEN TG_OP='DELETE' THEN 'pin.removed' ELSE 'pin.added' END;actor:=data->>'created_by';channel:=data->>'channel_id';
 ELSIF TG_TABLE_NAME='saved_items' THEN kind:=CASE WHEN TG_OP='DELETE' THEN 'star.removed' ELSE 'star.added' END;actor:=data->>'user_id';subject:=actor;channel:=data->>'channel_id';
 ELSIF TG_TABLE_NAME='workspace_emojis' THEN kind:='emoji.changed';actor:=data->>'created_by';data:=data||jsonb_build_object('subtype',CASE WHEN TG_OP='DELETE' THEN 'remove' WHEN TG_OP='INSERT' THEN 'add' ELSE 'change' END);
 ELSIF TG_TABLE_NAME='workspace_invite_requests' THEN IF TG_OP<>'INSERT' THEN RETURN NULL;END IF;kind:='invite.requested';actor:=data->>'requested_by';
 ELSIF TG_TABLE_NAME='usergroups' THEN kind:=CASE WHEN TG_OP='INSERT' THEN 'usergroup.created' ELSE 'usergroup.updated' END;actor:=data->>'created_by';
 END IF;
 PERFORM app_emit(kind,data,actor,channel,subject);RETURN NULL;
END $$;
CREATE TRIGGER app_pin_events AFTER INSERT OR DELETE ON channel_pins FOR EACH ROW EXECUTE FUNCTION app_collection_events();
CREATE TRIGGER app_star_events AFTER INSERT OR DELETE ON saved_items FOR EACH ROW EXECUTE FUNCTION app_collection_events();
CREATE TRIGGER app_emoji_events AFTER INSERT OR UPDATE OR DELETE ON workspace_emojis FOR EACH ROW EXECUTE FUNCTION app_collection_events();
CREATE TRIGGER app_invite_events AFTER INSERT ON workspace_invite_requests FOR EACH ROW EXECUTE FUNCTION app_collection_events();
CREATE TRIGGER app_usergroup_events AFTER INSERT OR UPDATE ON usergroups FOR EACH ROW EXECUTE FUNCTION app_collection_events();
CREATE FUNCTION app_group_member_events() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
BEGIN
 PERFORM app_emit('usergroup.members_changed',data||jsonb_build_object('added',CASE WHEN TG_OP='INSERT' THEN jsonb_build_array(data->>'user_id') ELSE '[]'::jsonb END,'removed',CASE WHEN TG_OP='DELETE' THEN jsonb_build_array(data->>'user_id') ELSE '[]'::jsonb END),data->>'user_id');
 PERFORM app_emit(CASE WHEN TG_OP='DELETE' THEN 'usergroup.self_removed' ELSE 'usergroup.self_added' END,data,data->>'user_id',NULL,data->>'user_id');RETURN NULL;
END $$;
CREATE TRIGGER app_group_member_events AFTER INSERT OR DELETE ON usergroup_members FOR EACH ROW EXECUTE FUNCTION app_group_member_events();
CREATE FUNCTION app_workspace_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.name<>NEW.name THEN PERFORM app_emit('workspace.renamed',jsonb_build_object('name',NEW.name,'previous_name',OLD.name),NULL);END IF;
 IF OLD.preferences<>NEW.preferences THEN PERFORM app_emit('workspace.preferences_changed',NEW.preferences::jsonb,NULL);END IF;
 IF OLD.profile_fields<>NEW.profile_fields THEN
  IF EXISTS(SELECT 1 FROM jsonb_array_elements(OLD.profile_fields::jsonb) prior_field WHERE NOT EXISTS(SELECT 1 FROM jsonb_array_elements(NEW.profile_fields::jsonb) fresh WHERE fresh->>'id'=prior_field->>'id')) THEN PERFORM app_emit('workspace.profile_deleted',jsonb_build_object('fields',NEW.profile_fields::jsonb,'previous_fields',OLD.profile_fields::jsonb),NULL);END IF;
  IF (SELECT jsonb_agg(x->>'id' ORDER BY x->>'id') FROM jsonb_array_elements(OLD.profile_fields::jsonb) x) IS NOT DISTINCT FROM (SELECT jsonb_agg(x->>'id' ORDER BY x->>'id') FROM jsonb_array_elements(NEW.profile_fields::jsonb) x) AND (SELECT jsonb_agg(x->>'id') FROM jsonb_array_elements(OLD.profile_fields::jsonb) x) IS DISTINCT FROM (SELECT jsonb_agg(x->>'id') FROM jsonb_array_elements(NEW.profile_fields::jsonb) x) THEN PERFORM app_emit('workspace.profile_reordered',jsonb_build_object('fields',NEW.profile_fields::jsonb),NULL);END IF;
  PERFORM app_emit('workspace.profile_changed',jsonb_build_object('fields',NEW.profile_fields::jsonb),NULL);
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_workspace_events AFTER UPDATE ON workspace_settings FOR EACH ROW EXECUTE FUNCTION app_workspace_events();
CREATE FUNCTION app_reminder_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' THEN PERFORM app_emit('reminder.created',to_jsonb(NEW),NEW.user_id,NEW.channel_id,NEW.user_id);
 ELSIF OLD.fired_at IS NULL AND NEW.fired_at IS NOT NULL THEN PERFORM app_emit('reminder.fired',to_jsonb(NEW),NEW.user_id,NEW.channel_id,NEW.user_id);END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_reminder_events AFTER INSERT OR UPDATE ON user_reminders FOR EACH ROW EXECUTE FUNCTION app_reminder_events();
CREATE FUNCTION app_assistant_events() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=to_jsonb(NEW)||jsonb_build_object('context',NEW.context::jsonb);
BEGIN
 IF TG_OP='INSERT' THEN PERFORM app_emit('assistant.started',data,NEW.actor_id,NEW.channel_id,NULL,NEW.installation_id);
 ELSE
  IF OLD.title<>NEW.title THEN PERFORM app_emit('assistant.title_changed',data||jsonb_build_object('previous_title',OLD.title),NEW.actor_id,NEW.channel_id,NULL,NEW.installation_id);END IF;
  IF OLD.context<>NEW.context THEN PERFORM app_emit('assistant.context_changed',data,NEW.actor_id,NEW.channel_id,NULL,NEW.installation_id);END IF;
  IF OLD.state<>NEW.state AND NEW.state='stopped' THEN PERFORM app_emit('assistant.stopped',data,NEW.actor_id,NEW.channel_id,NULL,NEW.installation_id);END IF;
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_assistant_events AFTER INSERT OR UPDATE ON assistant_sessions FOR EACH ROW EXECUTE FUNCTION app_assistant_events();
CREATE FUNCTION app_context_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_OP='INSERT' OR NEW.context<>OLD.context OR NEW.channel_id IS DISTINCT FROM OLD.channel_id THEN PERFORM app_emit('app.context_changed',jsonb_build_object('user_id',NEW.user_id,'context',NEW.context::jsonb),NEW.user_id,NEW.channel_id,NULL,NEW.installation_id);END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_context_events AFTER INSERT OR UPDATE ON app_view_contexts FOR EACH ROW EXECUTE FUNCTION app_context_events();
CREATE FUNCTION app_function_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM app_emit('function.executed',to_jsonb(NEW)||jsonb_build_object('inputs',NEW.inputs::jsonb),NEW.actor_id,NEW.channel_id,NULL,NEW.installation_id);RETURN NULL;
END $$;
CREATE TRIGGER app_function_events AFTER INSERT ON app_function_executions FOR EACH ROW EXECUTE FUNCTION app_function_events();
CREATE FUNCTION app_lifecycle_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=jsonb_build_object('app_id',NEW.id,'name',NEW.name,'owner_id',NEW.owner_id,'revision',NEW.revision);installation text;
BEGIN
 SELECT id INTO installation FROM app_installations WHERE app_id=NEW.id;
 IF TG_OP='INSERT' OR OLD.status IS DISTINCT FROM NEW.status THEN
  IF NEW.status='pending' THEN PERFORM app_emit('app.requested',data||jsonb_build_object('scopes',NEW.manifest::jsonb->'scopes'),NEW.owner_id);END IF;
  IF NEW.status='installed' THEN
   PERFORM app_emit('app.installed_workspace',data,NEW.owner_id);
   IF installation IS NOT NULL THEN PERFORM app_emit('app.installed',data,NEW.owner_id,NULL,NULL,installation);END IF;
  ELSIF NEW.status='disabled' THEN
   PERFORM app_emit('app.uninstalled_workspace',data,NEW.owner_id);
   IF installation IS NOT NULL THEN PERFORM app_emit('app.uninstalled',data,NEW.owner_id,NULL,NULL,installation);END IF;
  END IF;
 END IF;
 IF OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN PERFORM app_emit('app.deleted',data,NEW.owner_id);END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER app_lifecycle AFTER INSERT OR UPDATE ON apps FOR EACH ROW EXECUTE FUNCTION app_lifecycle_v2();
CREATE FUNCTION app_commands_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (OLD.manifest::jsonb->'commands',OLD.manifest::jsonb->'shortcuts',OLD.manifest::jsonb->'message_shortcuts',OLD.manifest::jsonb->'functions') IS DISTINCT FROM (NEW.manifest::jsonb->'commands',NEW.manifest::jsonb->'shortcuts',NEW.manifest::jsonb->'message_shortcuts',NEW.manifest::jsonb->'functions') THEN
  PERFORM app_emit('commands.changed',jsonb_build_object('app_id',NEW.app_id,'commands',NEW.manifest::jsonb->'commands','shortcuts',NEW.manifest::jsonb->'shortcuts','message_shortcuts',NEW.manifest::jsonb->'message_shortcuts','functions',NEW.manifest::jsonb->'functions'),NEW.approved_by);

 END IF;RETURN NEW;
END $$;
CREATE CONSTRAINT TRIGGER app_commands_events AFTER UPDATE ON app_installations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION app_commands_events();
CREATE FUNCTION app_revocation_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF OLD.revoked_at IS NULL AND NEW.revoked_at IS NOT NULL THEN PERFORM app_emit('tokens.revoked',jsonb_build_object('credential_id',NEW.id,'user_id',NEW.user_id,'app_id',(SELECT app_id FROM app_installations WHERE id=NEW.installation_id)),NEW.user_id,NULL,NULL,NEW.installation_id);END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_revocation_events AFTER UPDATE ON app_credentials FOR EACH ROW EXECUTE FUNCTION app_revocation_events();
CREATE FUNCTION app_file_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE data jsonb:=jsonb_build_object('id',NEW.id,'name',NEW.name,'description',NEW.description,'content_type',NEW.content_type,'size_bytes',NEW.size_bytes,'public',NEW.public_token_hash IS NOT NULL); share record;
BEGIN
 IF NEW.completed AND NOT OLD.completed THEN
  INSERT INTO app_file_shares VALUES(NEW.id,NEW.channel_id,NEW.user_id) ON CONFLICT DO NOTHING;
  PERFORM app_emit('file.created',data,NEW.user_id,NEW.channel_id);
 ELSIF NEW.completed THEN
  FOR share IN SELECT channel_id FROM app_file_shares WHERE file_id=NEW.id LOOP
   IF OLD.deleted_at IS NULL AND NEW.deleted_at IS NOT NULL THEN PERFORM app_emit('file.deleted',jsonb_build_object('id',NEW.id),NEW.user_id,share.channel_id);
   ELSIF NEW.deleted_at IS NULL THEN
    IF (OLD.name,OLD.description) IS DISTINCT FROM (NEW.name,NEW.description) THEN PERFORM app_emit('file.changed',data,NEW.user_id,share.channel_id);END IF;
    IF NEW.public_token_hash IS NOT NULL AND OLD.public_token_hash IS DISTINCT FROM NEW.public_token_hash THEN PERFORM app_emit('file.public',data,NEW.user_id,share.channel_id);END IF;
    IF OLD.public_token_hash IS NOT NULL AND NEW.public_token_hash IS NULL THEN PERFORM app_emit('file.unshared',jsonb_build_object('id',NEW.id,'visibility','public'),NEW.user_id,share.channel_id);END IF;
   END IF;
  END LOOP;
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_file_events AFTER UPDATE ON app_files FOR EACH ROW EXECUTE FUNCTION app_file_events_v2();
CREATE FUNCTION app_file_share_events() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE row_data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
BEGIN
 PERFORM app_emit(CASE WHEN TG_OP='DELETE' THEN 'file.unshared' ELSE 'file.shared' END,jsonb_build_object('id',row_data->>'file_id'),row_data->>'created_by',row_data->>'channel_id');RETURN NULL;
END $$;
CREATE TRIGGER app_file_share_events AFTER INSERT OR DELETE ON app_file_shares FOR EACH ROW EXECUTE FUNCTION app_file_share_events();

CREATE FUNCTION app_endpoint_verification_reset() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.manifest IS DISTINCT FROM OLD.manifest OR NEW.signing_secret IS DISTINCT FROM OLD.signing_secret THEN NEW.endpoint_verified_at:=NULL;END IF;RETURN NEW;
END $$;
CREATE TRIGGER app_endpoint_verification_reset BEFORE UPDATE ON app_installations FOR EACH ROW EXECUTE FUNCTION app_endpoint_verification_reset();
CREATE INDEX app_deliveries_hourly_count ON app_deliveries(installation_id,created_at) WHERE event_type<>'app.rate_limited';

ALTER TABLE app_deliveries ADD COLUMN first_delivered_at timestamptz;
CREATE INDEX app_deliveries_dispatch_count ON app_deliveries(installation_id,first_delivered_at) WHERE event_type<>'app.rate_limited';

-- Review fixes: current authorization and transport limits.
-- Harden event delivery and bound transport/retention work without rewriting deployed migrations.
CREATE INDEX app_deliveries_retention ON app_deliveries(created_at,id);
CREATE INDEX app_deliveries_ready ON app_deliveries(installation_id,available_at,created_at,id) WHERE acked_at IS NULL;
CREATE TABLE app_socket_sessions(id text PRIMARY KEY,installation_id text NOT NULL REFERENCES app_installations(id),expires_at timestamptz NOT NULL);
CREATE INDEX app_socket_sessions_active ON app_socket_sessions(installation_id,expires_at);
CREATE TABLE app_socket_presence_limits(installation_id text NOT NULL REFERENCES app_installations(id),window_start timestamptz NOT NULL,requests integer NOT NULL,PRIMARY KEY(installation_id,window_start));
CREATE FUNCTION app_redact_metadata(value jsonb) RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
 SELECT CASE jsonb_typeof(value)
 WHEN 'object' THEN COALESCE((SELECT jsonb_object_agg(key,app_redact_metadata(item)) FROM jsonb_each(value) AS fields(key,item) WHERE key NOT IN('metadata','previous_metadata')),'{}'::jsonb)
 WHEN 'array' THEN COALESCE((SELECT jsonb_agg(app_redact_metadata(item)) FROM jsonb_array_elements(value) AS elements(item)),'[]'::jsonb)
 ELSE value END
$$;
CREATE FUNCTION app_link_urls(content text,domains jsonb) RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
 SELECT COALESCE(jsonb_agg(DISTINCT url[1]),'[]'::jsonb) FROM regexp_matches(content,'https://[^[:space:]<>"\)]+','g') url
 WHERE EXISTS(SELECT 1 FROM jsonb_array_elements_text(COALESCE(domains,'[]')) domain WHERE url[1] ~ ('^https://' || replace(domain,'.','[.]') || '([/:?#]|$)'))
$$;
CREATE FUNCTION app_filter_payload(payload jsonb,manifest text,scopes text) RETURNS jsonb LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
 IF payload->>'type'='link.shared' AND payload->'data' ? 'content' THEN
  payload:=jsonb_set(payload,'{data}',jsonb_build_object('id',payload->'data'->'id','links',app_link_urls(payload->'data'->>'content',manifest::jsonb->'link_domains')));
 END IF;
 IF NOT scopes::jsonb ? 'metadata:read' THEN payload:=app_redact_metadata(payload);END IF;
 RETURN payload;
END $$;
CREATE FUNCTION app_delivery_payload(delivery app_deliveries) RETURNS jsonb LANGUAGE sql STABLE AS $$
 SELECT app_filter_payload(delivery.payload::jsonb,i.manifest,i.scopes) FROM app_installations i WHERE i.id=delivery.installation_id
$$;


CREATE OR REPLACE FUNCTION app_event_access(installation text,scope_name text,channel text,subject text,departed text DEFAULT NULL,terminal boolean DEFAULT false) RETURNS text LANGUAGE sql STABLE AS $$
 SELECT identity.user_id FROM app_installations i JOIN apps a ON a.id=i.app_id
 CROSS JOIN LATERAL (
  SELECT a.bot_user_id AS user_id,true AS bot
  UNION ALL
  SELECT t.user_id,false FROM app_credentials t JOIN users u ON u.id=t.user_id
   WHERE t.installation_id=i.id AND t.revoked_at IS NULL AND t.expires_at>now() AND NOT u.is_deactivated
   AND t.scopes::jsonb <@ i.scopes::jsonb AND (scope_name IS NULL OR t.scopes::jsonb ? scope_name)
 ) identity
 WHERE i.id=installation AND ((i.active AND a.status='installed' AND a.deleted_at IS NULL) OR terminal AND scope_name IS NULL)
 AND (scope_name IS NULL OR i.scopes::jsonb ? scope_name)
 AND (subject IS NULL OR identity.user_id=subject)
 AND (channel IS NULL OR EXISTS(SELECT 1 FROM channels c WHERE c.id=channel
  AND (c.deleted_at IS NULL OR terminal)
  AND (EXISTS(SELECT 1 FROM channel_user bot WHERE bot.channel_id=c.id AND bot.user_id=a.bot_user_id)
   OR departed=a.bot_user_id
   OR scope_name='channels:read' AND NOT c.is_private AND NOT c.is_dm AND subject IS NULL)
  AND (identity.user_id=departed OR scope_name='channels:read' AND NOT c.is_private AND NOT c.is_dm AND NOT EXISTS(SELECT 1 FROM users WHERE id=identity.user_id AND is_guest) OR EXISTS(SELECT 1 FROM channel_user cu WHERE cu.channel_id=c.id AND cu.user_id=identity.user_id))))
 ORDER BY identity.bot DESC LIMIT 1
$$;

CREATE OR REPLACE FUNCTION app_emit(event_name text,event_data jsonb,actor text,channel text DEFAULT NULL,subject text DEFAULT NULL,target_installation text DEFAULT NULL,departed text DEFAULT NULL) RETURNS void LANGUAGE plpgsql AS $$
DECLARE definition app_event_types; installation record; identity text; effective_subject text; event_uuid text:=md5(CASE WHEN event_name='app.rate_limited' THEN date_trunc('hour',now())::text||COALESCE(target_installation,'') ELSE pg_current_xact_id()::text||event_data::text||COALESCE(actor,'')||COALESCE(channel,'')||COALESCE(subject,'') END||event_name)::uuid::text; count_now bigint; terminal boolean; envelope jsonb;
BEGIN
 SELECT * INTO STRICT definition FROM app_event_types WHERE name=event_name;
 terminal:=event_name IN('app.uninstalled','app.deleted','tokens.revoked','channel.deleted');
 FOR installation IN SELECT i.*,a.bot_user_id FROM app_installations i JOIN apps a ON a.id=i.app_id
 WHERE (target_installation IS NULL OR i.id=target_installation)
 AND (definition.audience NOT IN('app','control') OR target_installation IS NOT NULL)
 AND (app_event_matches(i.manifest,event_name,channel,actor,app_event_access(i.id,definition.required_scope,channel,subject,departed,terminal))
 OR event_name IN('member.joined','member.left') AND app_event_access(i.id,definition.required_scope,channel,actor,departed,terminal)=actor AND app_event_matches(i.manifest,event_name,channel,actor,actor))
 LOOP
  identity:=app_event_access(installation.id,definition.required_scope,channel,subject,departed,terminal);
  effective_subject:=subject;
  IF NOT app_event_matches(installation.manifest,event_name,channel,actor,identity) THEN
   effective_subject:=actor;
   identity:=app_event_access(installation.id,definition.required_scope,channel,effective_subject,departed,terminal);
  END IF;
  IF identity IS NULL THEN CONTINUE; END IF;
  -- Suppression applies only to bot-authored message/typing loops, never channel or bot lifecycle changes.
  IF (event_name LIKE 'message.%' OR event_name IN('app.mentioned','user.typing'))
   AND EXISTS(SELECT 1 FROM users WHERE id=actor AND is_bot)
   AND NOT COALESCE((installation.manifest::jsonb->>'include_bot_events')::boolean,false)
   AND NOT COALESCE(installation.manifest::jsonb->'events' ? 'message.bot_message',false) THEN CONTINUE; END IF;
  IF EXISTS(SELECT 1 FROM app_deliveries WHERE installation_id=installation.id AND event_id=event_uuid) THEN CONTINUE; END IF;
  IF event_name<>'app.rate_limited' THEN
   -- Append-only accounting avoids taking installation locks in source transactions.
   SELECT COALESCE((SELECT events FROM app_event_limits WHERE installation_id=installation.id AND window_start=date_trunc('hour',now())),0)+count(*)+1 INTO count_now
    FROM app_deliveries WHERE installation_id=installation.id AND created_at>=date_trunc('hour',now()) AND event_type<>'app.rate_limited';
   IF count_now>30000 THEN
    IF app_event_matches(installation.manifest,'app.rate_limited',NULL,NULL) THEN
     PERFORM app_emit('app.rate_limited',jsonb_build_object('installation_id',installation.id,'window_start',date_trunc('hour',now()),'limit',30000),NULL,NULL,NULL,installation.id);
    END IF;
    CONTINUE;
   END IF;
  END IF;
  envelope:=jsonb_build_object('id',event_uuid,'version',1,'type',event_name,'occurred_at',now(),'actor_id',actor,'channel_id',channel,'subject_user_id',effective_subject,'data',event_data);
  envelope:=app_filter_payload(envelope,installation.manifest,installation.scopes);
  INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload,required_scope,subject_user_id,access_user_id,allow_departed)
   VALUES(gen_random_uuid()::text,event_uuid,installation.id,channel,event_name,envelope::text,definition.required_scope,effective_subject,identity,COALESCE(identity=departed,false)) ON CONFLICT(installation_id,event_id) DO NOTHING;
 END LOOP;
END $$;

CREATE OR REPLACE FUNCTION app_member_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE row_data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
BEGIN
 PERFORM app_emit(CASE WHEN TG_OP='DELETE' THEN 'member.left' ELSE 'member.joined' END,row_data,row_data->>'user_id',row_data->>'channel_id',NULL,NULL,CASE WHEN TG_OP='DELETE' THEN row_data->>'user_id' END);
 RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;


CREATE FUNCTION app_mark_channel_created() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM set_config('zoo.created_channels',(COALESCE(NULLIF(current_setting('zoo.created_channels',true),''),'[]')::jsonb||jsonb_build_array(NEW.id))::text,true);
 RETURN NEW;
END $$;
CREATE TRIGGER app_mark_channel_created BEFORE INSERT ON channels FOR EACH ROW EXECUTE FUNCTION app_mark_channel_created();

CREATE FUNCTION app_conversation_created() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 PERFORM app_emit(CASE WHEN c.is_dm THEN 'conversation.created' ELSE 'channel.created' END,to_jsonb(c),c.created_user_id,c.id)
 FROM channels c WHERE c.id=NEW.channel_id AND (c.is_private OR c.is_dm) AND COALESCE(NULLIF(current_setting('zoo.created_channels',true),''),'[]')::jsonb ? c.id;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER app_conversation_created AFTER INSERT ON channel_user DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION app_conversation_created();
-- Remove any sensitive values queued before this forward migration.
UPDATE app_deliveries d SET payload=app_delivery_payload(d)::text WHERE acked_at IS NULL AND expires_at>now();


CREATE FUNCTION app_revoke_artifacts() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.status<>'installed' OR NEW.deleted_at IS NOT NULL THEN
  DELETE FROM app_ui WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=NEW.id);
  DELETE FROM app_responses WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=NEW.id);
 END IF;
 IF NEW.status='disabled' OR NEW.deleted_at IS NOT NULL THEN
  UPDATE app_files SET public_token_hash=NULL WHERE installation_id IN(SELECT id FROM app_installations WHERE app_id=NEW.id) AND public_token_hash IS NOT NULL;
 END IF;RETURN NULL;
END $$;
CREATE TRIGGER app_revoke_artifacts AFTER UPDATE OF status,deleted_at ON apps FOR EACH ROW EXECUTE FUNCTION app_revoke_artifacts();
-- This is a historical authorization hint, not ownership. Its foreign key makes
-- unrelated profile writes lock every recipient bot and deadlock each other.
-- app_delivery_allowed always resolves the current user and consent before send.
ALTER TABLE app_deliveries DROP CONSTRAINT app_deliveries_access_user_id_fkey;

-- Lifecycle control events must survive both source and dispatcher quotas.
CREATE FUNCTION app_event_quota_exempt(event_name text) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
 SELECT event_name IN('app.rate_limited','app.uninstalled','app.deleted','tokens.revoked')
$$;
ALTER TABLE app_installations ADD COLUMN last_http_dispatch_at timestamptz;
CREATE INDEX app_installations_http_rotation ON app_installations(last_http_dispatch_at,id);

CREATE OR REPLACE FUNCTION app_emit(event_name text,event_data jsonb,actor text,channel text DEFAULT NULL,subject text DEFAULT NULL,target_installation text DEFAULT NULL,departed text DEFAULT NULL) RETURNS void LANGUAGE plpgsql AS $$
DECLARE definition app_event_types; installation record; identity text; effective_subject text; event_uuid text:=md5(CASE WHEN event_name='app.rate_limited' THEN date_trunc('hour',now())::text||COALESCE(target_installation,'') ELSE pg_current_xact_id()::text||event_data::text||COALESCE(actor,'')||COALESCE(channel,'')||COALESCE(subject,'') END||event_name)::uuid::text; count_now bigint; terminal boolean; envelope jsonb;
BEGIN
 SELECT * INTO STRICT definition FROM app_event_types WHERE name=event_name;
 terminal:=event_name IN('app.uninstalled','app.deleted','tokens.revoked','channel.deleted');
 FOR installation IN SELECT i.*,a.bot_user_id FROM app_installations i JOIN apps a ON a.id=i.app_id
 WHERE (target_installation IS NULL OR i.id=target_installation)
 AND (definition.audience NOT IN('app','control') OR target_installation IS NOT NULL)
 AND (app_event_matches(i.manifest,event_name,channel,actor,app_event_access(i.id,definition.required_scope,channel,subject,departed,terminal))
 OR event_name IN('member.joined','member.left') AND app_event_access(i.id,definition.required_scope,channel,actor,departed,terminal)=actor AND app_event_matches(i.manifest,event_name,channel,actor,actor))
 LOOP
  identity:=app_event_access(installation.id,definition.required_scope,channel,subject,departed,terminal);
  effective_subject:=subject;
  IF NOT app_event_matches(installation.manifest,event_name,channel,actor,identity) THEN
   effective_subject:=actor;
   identity:=app_event_access(installation.id,definition.required_scope,channel,effective_subject,departed,terminal);
  END IF;
  IF identity IS NULL THEN CONTINUE; END IF;
  -- Suppression applies only to bot-authored message/typing loops, never channel or bot lifecycle changes.
  IF (event_name LIKE 'message.%' OR event_name IN('app.mentioned','user.typing'))
   AND EXISTS(SELECT 1 FROM users WHERE id=actor AND is_bot)
   AND NOT COALESCE((installation.manifest::jsonb->>'include_bot_events')::boolean,false)
   AND NOT COALESCE(installation.manifest::jsonb->'events' ? 'message.bot_message',false) THEN CONTINUE; END IF;
  IF EXISTS(SELECT 1 FROM app_deliveries WHERE installation_id=installation.id AND event_id=event_uuid) THEN CONTINUE; END IF;
  IF NOT app_event_quota_exempt(event_name) THEN
   -- Append-only accounting avoids taking installation locks in source transactions.
   SELECT COALESCE((SELECT events FROM app_event_limits WHERE installation_id=installation.id AND window_start=date_trunc('hour',now())),0)+count(*)+1 INTO count_now
    FROM app_deliveries WHERE installation_id=installation.id AND created_at>=date_trunc('hour',now()) AND event_type<>'app.rate_limited' AND NOT app_event_quota_exempt(event_type);
   IF count_now>30000 THEN
    IF app_event_matches(installation.manifest,'app.rate_limited',NULL,NULL) THEN
     PERFORM app_emit('app.rate_limited',jsonb_build_object('installation_id',installation.id,'window_start',date_trunc('hour',now()),'limit',30000),NULL,NULL,NULL,installation.id);
    END IF;
    CONTINUE;
   END IF;
  END IF;
  envelope:=jsonb_build_object('id',event_uuid,'version',1,'type',event_name,'occurred_at',now(),'actor_id',actor,'channel_id',channel,'subject_user_id',effective_subject,'data',event_data);
  envelope:=app_filter_payload(envelope,installation.manifest,installation.scopes);
  INSERT INTO app_deliveries(id,event_id,installation_id,channel_id,event_type,payload,required_scope,subject_user_id,access_user_id,allow_departed)
   VALUES(gen_random_uuid()::text,event_uuid,installation.id,channel,event_name,envelope::text,definition.required_scope,effective_subject,identity,COALESCE(identity=departed,false)) ON CONFLICT(installation_id,event_id) DO NOTHING;
 END LOOP;
END $$;

CREATE TABLE attachments (
  id text NOT NULL,
  message_id text,
  uploader_id text NOT NULL,
  channel_id text NOT NULL,
  file_name text NOT NULL,
  file_size bigint NOT NULL,
  content_type text NOT NULL,
  storage_key text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (id),
  CONSTRAINT attachments_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT attachments_uploader_id_fkey
    FOREIGN KEY (uploader_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT attachments_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_attachments_message_id ON attachments (message_id);
CREATE INDEX idx_attachments_channel_id ON attachments (channel_id);
CREATE INDEX idx_attachments_uploader_id ON attachments (uploader_id);

-- Shared ephemeral bubble chat state; the database clock is authoritative for expiry.
CREATE TABLE bubble_chat_presence (
  user_id text NOT NULL,
  session_id text NOT NULL,
  content text NOT NULL,
  seq bigint NOT NULL DEFAULT 0,
  updated_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  retention_until timestamptz NOT NULL,
  PRIMARY KEY (user_id, session_id),
  CONSTRAINT bubble_chat_presence_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_bubble_chat_presence_expires_at ON bubble_chat_presence (expires_at);
CREATE INDEX idx_bubble_chat_presence_retention_until ON bubble_chat_presence (retention_until);

-- Home posts are explicitly public and separate from private channel messages.
CREATE TABLE home_posts (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users (id) ON UPDATE CASCADE ON DELETE CASCADE,
  content text NOT NULL CHECK (char_length(content) BETWEEN 1 AND 1000 AND btrim(content) <> ''),
  created_at timestamptz NOT NULL DEFAULT statement_timestamp()
);
CREATE INDEX idx_home_posts_created ON home_posts (created_at DESC, id DESC);
CREATE INDEX idx_home_posts_author ON home_posts (user_id, created_at DESC, id DESC);
ALTER TABLE home_posts DROP CONSTRAINT home_posts_content_check;
ALTER TABLE home_posts ADD CONSTRAINT home_posts_content_check CHECK(char_length(content) <= 40000);
ALTER TABLE home_posts ADD COLUMN apub boolean NOT NULL DEFAULT false,
  ADD COLUMN visibility text NOT NULL DEFAULT 'local' CHECK (visibility IN ('local','public','unlisted','followers')),
  ADD COLUMN channel_id text REFERENCES channels(id) ON DELETE SET NULL,
  ADD COLUMN updated_at timestamptz,
  ADD COLUMN deleted_at timestamptz,
  ADD CONSTRAINT home_posts_audience CHECK (apub = (visibility <> 'local')),
  ADD CONSTRAINT home_posts_channel_audience CHECK (channel_id IS NULL OR visibility <> 'followers');
ALTER TABLE messages ADD COLUMN home_post_id text UNIQUE,
  ADD COLUMN apub boolean NOT NULL DEFAULT false,
  ADD COLUMN apub_visibility text;
CREATE TABLE activitypub_settings (
  id boolean PRIMARY KEY DEFAULT true CHECK(id),
  ltl_channel_pattern text NOT NULL DEFAULT '' CHECK(length(ltl_channel_pattern)<=200)
);
INSERT INTO activitypub_settings DEFAULT VALUES;
CREATE TABLE apub_keys (
  user_id text PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  private_key text NOT NULL,
  public_key text NOT NULL
);
CREATE TABLE apub_actors (
  id text PRIMARY KEY,
  inbox text NOT NULL,
  followers_url text NOT NULL,
  user_name text NOT NULL,
  display_name text NOT NULL,
  key_id text NOT NULL,
  public_key text NOT NULL,
  fetched_at timestamptz NOT NULL DEFAULT now(),
  blocked boolean NOT NULL DEFAULT false
);
CREATE TABLE apub_follows (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL REFERENCES apub_actors(id),
  direction text NOT NULL CHECK(direction IN ('incoming','outgoing')),
  activity_id text NOT NULL UNIQUE,
  accepted boolean NOT NULL DEFAULT false,
  PRIMARY KEY(user_id,actor_id,direction)
);
CREATE TABLE apub_deliveries (
  id text PRIMARY KEY,
  sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL REFERENCES apub_actors(id),
  payload text NOT NULL,
  attempts integer NOT NULL DEFAULT 0,
  next_attempt_at timestamptz NOT NULL DEFAULT now(),
  lease_until timestamptz,
  delivered_at timestamptz,
  last_status integer,
  failed boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX idx_apub_delivery_pending ON apub_deliveries(next_attempt_at) WHERE delivered_at IS NULL AND NOT failed;
CREATE TABLE apub_inbox_receipts (
  id text NOT NULL,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL,
  PRIMARY KEY(id,user_id),
  received_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE apub_remote_posts (
  id text PRIMARY KEY,
  actor_id text NOT NULL REFERENCES apub_actors(id),
  content text NOT NULL,
  visibility text NOT NULL CHECK(visibility IN ('public','unlisted','followers')),
  published timestamptz NOT NULL,
  updated_at timestamptz NOT NULL,
  deleted_at timestamptz
);
CREATE TABLE apub_remote_recipients (
  post_id text NOT NULL REFERENCES apub_remote_posts(id) ON DELETE CASCADE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(post_id,user_id)
);
CREATE INDEX idx_apub_remote_published ON apub_remote_posts(published DESC,id DESC);

-- Synchronize both views inside the originating transaction. Comparisons prevent recursion.
CREATE FUNCTION sync_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE kind text; event_id text := gen_random_uuid()::text;
BEGIN
  IF NEW.channel_id IS NOT NULL THEN
    IF (TG_OP='INSERT' OR NEW.apub AND NEW.deleted_at IS NULL) AND (NEW.visibility='followers' OR NOT EXISTS (
      SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_private AND NOT is_dm AND deleted_at IS NULL
    )) THEN RAISE EXCEPTION 'Home posts require a public channel' USING ERRCODE='23514'; END IF;
    INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,deleted_at,home_post_id,apub,apub_visibility)
      VALUES(NEW.id,NEW.user_id,NEW.channel_id,NEW.content,NEW.created_at AT TIME ZONE 'UTC',NEW.updated_at AT TIME ZONE 'UTC',NEW.deleted_at AT TIME ZONE 'UTC',NEW.id,NEW.apub,NEW.visibility)
      ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content,updated_at=EXCLUDED.updated_at,
        deleted_at=EXCLUDED.deleted_at,home_post_id=EXCLUDED.home_post_id,apub=EXCLUDED.apub,apub_visibility=EXCLUDED.apub_visibility
      WHERE (messages.content,messages.deleted_at,messages.home_post_id,messages.apub,messages.apub_visibility)
        IS DISTINCT FROM (EXCLUDED.content,EXCLUDED.deleted_at,EXCLUDED.home_post_id,EXCLUDED.apub,EXCLUDED.apub_visibility);
  END IF;
  IF TG_OP='INSERT' THEN
    IF NEW.apub AND NEW.deleted_at IS NULL THEN kind:='Create'; END IF;
  ELSIF NEW.apub AND NEW.deleted_at IS NULL AND NOT OLD.apub THEN kind:='Create';
  ELSIF OLD.apub AND OLD.deleted_at IS NULL AND (NOT NEW.apub OR NEW.deleted_at IS NOT NULL) THEN kind:='Delete';
  ELSIF NEW.apub AND NEW.deleted_at IS NULL AND (NEW.content,NEW.visibility) IS DISTINCT FROM (OLD.content,OLD.visibility) THEN kind:='Update';
  END IF;
  IF kind IS NOT NULL THEN
    INSERT INTO apub_deliveries(id,user_id,actor_id,payload)
      SELECT gen_random_uuid()::text,NEW.user_id,f.actor_id,
        json_build_object('zoo_post',CASE WHEN kind='Delete' THEN row_to_json(OLD) ELSE row_to_json(NEW) END,'type',kind,'activity_id',event_id)::text
      FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id
      WHERE f.user_id=NEW.user_id AND f.direction='incoming' AND f.accepted AND NOT a.blocked;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER home_post_sync AFTER INSERT OR UPDATE ON home_posts FOR EACH ROW EXECUTE FUNCTION sync_home_post();
CREATE FUNCTION valid_ltl_channel_regex(pattern text) RETURNS boolean LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
  PERFORM '' ~ pattern;
  RETURN true;
EXCEPTION WHEN invalid_regular_expression THEN
  RETURN false;
END $$;
CREATE FUNCTION channel_matches_ltl(channel_name text, pattern text) RETURNS boolean LANGUAGE plpgsql IMMUTABLE AS $$
BEGIN
  RETURN pattern<>'' AND channel_name ~ pattern;
EXCEPTION WHEN invalid_regular_expression THEN
  RETURN false;
END $$;
CREATE FUNCTION sync_channel_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.home_post_id IS NOT NULL THEN
    UPDATE home_posts SET content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC');
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
      AND channel_matches_ltl(c.channel_name,s.ltl_channel_pattern)
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC') ON CONFLICT DO NOTHING;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER channel_home_post_sync AFTER INSERT OR UPDATE ON messages FOR EACH ROW EXECUTE FUNCTION sync_channel_home_post();
-- Already federated content cannot silently become a private channel conversation.
CREATE FUNCTION protect_federated_channel() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.is_private AND NOT OLD.is_private AND EXISTS(SELECT 1 FROM home_posts WHERE channel_id=NEW.id AND apub AND deleted_at IS NULL) THEN
    RAISE EXCEPTION 'Delete federated posts before making this channel private' USING ERRCODE='23514';
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER protect_federated_channel_privacy BEFORE UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION protect_federated_channel();

CREATE FUNCTION delete_channel_home_posts() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.deleted_at IS NOT NULL AND OLD.deleted_at IS NULL THEN
    UPDATE home_posts SET deleted_at=now(),updated_at=now() WHERE channel_id=NEW.id AND deleted_at IS NULL;
  END IF;
  RETURN NEW;
END $$;
CREATE TRIGGER delete_channel_home_posts AFTER UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION delete_channel_home_posts();
-- Visibility and federation are independent choices in the Home composer.
-- Legacy 'local' stays valid for non-federated API clients.
ALTER TABLE home_posts DROP CONSTRAINT home_posts_audience;
ALTER TABLE home_posts ADD CONSTRAINT home_posts_audience
  CHECK (NOT apub OR visibility <> 'local');

-- Invalidation only: no post contents or identities are broadcast.
CREATE FUNCTION notify_home_timeline() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_notify('home_timeline', 'refresh');
    RETURN NULL;
END;
$$;
CREATE TRIGGER home_posts_home_live AFTER INSERT OR UPDATE OR DELETE ON home_posts FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_remote_posts_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_remote_posts FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_remote_recipients_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_remote_recipients FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_follows_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_follows FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_actors_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_actors FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER channels_home_live AFTER INSERT OR UPDATE OR DELETE ON channels FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER channel_user_home_live AFTER INSERT OR UPDATE OR DELETE ON channel_user FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER users_home_live AFTER INSERT OR UPDATE OR DELETE ON users FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER user_role_home_live AFTER INSERT OR UPDATE OR DELETE ON user_role FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
ALTER TABLE apub_follows ADD COLUMN activity_json text;
CREATE TABLE apub_reactions (
  post_id text NOT NULL,
  actor_id text NOT NULL,
  user_id text REFERENCES users(id) ON DELETE CASCADE,
  activity_id text NOT NULL UNIQUE,
  activity_json text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(post_id, actor_id)
);
CREATE TABLE apub_undone_activities (
  activity_id text NOT NULL,
  actor_id text NOT NULL,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(activity_id, actor_id, user_id)
);
CREATE TABLE apub_notifications (
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  actor_id text NOT NULL,
  activity_id text NOT NULL,
  kind text NOT NULL CHECK(kind IN ('follow_request','follow_accepted','like')),
  post_id text,
  is_read boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  UNIQUE(user_id,activity_id,kind)
);
CREATE INDEX apub_notifications_owner ON apub_notifications(user_id,created_at DESC);
CREATE TRIGGER apub_reactions_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_reactions FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_notifications_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_notifications FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
ALTER TABLE home_posts ADD COLUMN parent_id text;
ALTER TABLE home_posts ADD COLUMN parent_uri text;
ALTER TABLE home_posts ADD COLUMN reply_actor text;
ALTER TABLE home_posts ADD COLUMN reply_user_id text REFERENCES users(id) ON DELETE SET NULL;
ALTER TABLE apub_remote_posts ADD COLUMN parent_id text;
ALTER TABLE apub_remote_posts ADD COLUMN emojis jsonb NOT NULL DEFAULT '[]';
ALTER TABLE apub_actors ADD COLUMN emojis jsonb NOT NULL DEFAULT '[]';
ALTER TABLE apub_reactions ADD COLUMN reaction text NOT NULL DEFAULT '❤️';
ALTER TABLE apub_reactions ADD COLUMN emoji jsonb;
CREATE TABLE apub_emoji_images (
  id text PRIMARY KEY,
  url text NOT NULL UNIQUE CHECK(length(url)<=2048)
);
CREATE TABLE apub_emoji_catalog (
  origin text PRIMARY KEY,
  software text NOT NULL,
  emojis jsonb NOT NULL,
  fetched_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE apub_reposts (
  post_id text NOT NULL,
  actor_id text NOT NULL,
  user_id text REFERENCES users(id) ON DELETE CASCADE,
  activity_id text NOT NULL UNIQUE,
  activity_json text NOT NULL,
  visibility text NOT NULL CHECK(visibility IN ('local','public','unlisted','followers')),
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(post_id,actor_id)
);
CREATE TABLE apub_repost_recipients (
  activity_id text NOT NULL REFERENCES apub_reposts(activity_id) ON DELETE CASCADE,
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  PRIMARY KEY(activity_id,user_id)
);
ALTER TABLE apub_notifications DROP CONSTRAINT apub_notifications_kind_check;
ALTER TABLE apub_notifications ADD CONSTRAINT apub_notifications_kind_check CHECK(kind IN ('follow_request','follow_accepted','like','reply','repost'));
CREATE INDEX home_reply_parent ON home_posts(parent_id);
CREATE INDEX remote_reply_parent ON apub_remote_posts(parent_id);
CREATE TRIGGER apub_reposts_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_reposts FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER apub_repost_recipients_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_repost_recipients FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
ALTER TABLE apub_notifications ADD COLUMN reaction text;
ALTER TABLE apub_notifications ADD COLUMN emoji jsonb;

CREATE OR REPLACE FUNCTION sync_home_post() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE kind text; event_id text := gen_random_uuid()::text;
BEGIN
  IF NEW.channel_id IS NOT NULL THEN
    IF (TG_OP='INSERT' OR NEW.apub AND NEW.deleted_at IS NULL) AND (NEW.visibility='followers' OR NOT EXISTS (
      SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_private AND NOT is_dm AND deleted_at IS NULL
    )) THEN RAISE EXCEPTION 'Home posts require a public channel' USING ERRCODE='23514'; END IF;
    INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,deleted_at,home_post_id,apub,apub_visibility)
      VALUES(NEW.id,NEW.user_id,NEW.channel_id,NEW.content,NEW.created_at AT TIME ZONE 'UTC',NEW.updated_at AT TIME ZONE 'UTC',NEW.deleted_at AT TIME ZONE 'UTC',NEW.id,NEW.apub,NEW.visibility)
      ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content,updated_at=EXCLUDED.updated_at,
        deleted_at=EXCLUDED.deleted_at,home_post_id=EXCLUDED.home_post_id,apub=EXCLUDED.apub,apub_visibility=EXCLUDED.apub_visibility
      WHERE (messages.content,messages.deleted_at,messages.home_post_id,messages.apub,messages.apub_visibility)
        IS DISTINCT FROM (EXCLUDED.content,EXCLUDED.deleted_at,EXCLUDED.home_post_id,EXCLUDED.apub,EXCLUDED.apub_visibility);
  END IF;
  IF TG_OP='INSERT' THEN
    IF NEW.apub AND NEW.deleted_at IS NULL THEN kind:='Create'; END IF;
  ELSIF NEW.apub AND NEW.deleted_at IS NULL AND NOT OLD.apub THEN kind:='Create';
  ELSIF OLD.apub AND OLD.deleted_at IS NULL AND (NOT NEW.apub OR NEW.deleted_at IS NOT NULL) THEN kind:='Delete';
  ELSIF NEW.apub AND NEW.deleted_at IS NULL AND (NEW.content,NEW.visibility) IS DISTINCT FROM (OLD.content,OLD.visibility) THEN kind:='Update';
  END IF;
  IF kind IS NOT NULL THEN
    INSERT INTO apub_deliveries(id,user_id,actor_id,payload)
      SELECT gen_random_uuid()::text,NEW.user_id,a.id,
        json_build_object('zoo_post',CASE WHEN kind='Delete' THEN row_to_json(OLD) ELSE row_to_json(NEW) END,'type',kind,'activity_id',event_id)::text
      FROM apub_actors a
      WHERE NOT a.blocked AND (a.id=NEW.reply_actor OR EXISTS(SELECT 1 FROM apub_follows f WHERE f.actor_id=a.id AND f.user_id=NEW.user_id AND f.direction='incoming' AND f.accepted));
  END IF;
  RETURN NEW;
END $$;

ALTER TABLE apub_inbox_receipts DROP CONSTRAINT apub_inbox_receipts_pkey;
ALTER TABLE apub_inbox_receipts ADD PRIMARY KEY(id,actor_id,user_id);
CREATE TABLE apub_remote_tombstones (
  id text NOT NULL,
  actor_id text NOT NULL REFERENCES apub_actors(id) ON DELETE CASCADE,
  created_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY(id,actor_id)
);
-- Preserve early Delete receipts without reserving another actor's canonical Note ID.
INSERT INTO apub_remote_tombstones(id,actor_id) SELECT id,actor_id FROM apub_remote_posts WHERE deleted_at IS NOT NULL;
DELETE FROM apub_remote_posts WHERE deleted_at IS NOT NULL;
ALTER TABLE apub_emoji_images ADD COLUMN last_seen_at timestamptz NOT NULL DEFAULT now();
CREATE INDEX apub_emoji_images_last_seen ON apub_emoji_images(last_seen_at);

ALTER TABLE apub_remote_posts ADD COLUMN origin_verified boolean NOT NULL DEFAULT false;

CREATE TABLE apub_profiles (
  user_id text PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  summary text NOT NULL DEFAULT '' CHECK (char_length(summary)<=1000),
  display_name text CHECK (display_name IS NULL OR (char_length(display_name) BETWEEN 1 AND 100))
);
ALTER TABLE apub_actors ADD COLUMN avatar jsonb, ADD COLUMN summary text NOT NULL DEFAULT '';
CREATE TRIGGER apub_profiles_home_live AFTER INSERT OR UPDATE OR DELETE ON apub_profiles FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
