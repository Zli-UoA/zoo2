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
  imported_from_slack boolean NOT NULL DEFAULT false,
  slack_import_kind text,
  slack_imported_at timestamp(3),
  CONSTRAINT "Channel_pkey" PRIMARY KEY (id),
  CONSTRAINT channels_channel_name_key UNIQUE (channel_name),
  CONSTRAINT channels_slack_import_kind_check
    CHECK (slack_import_kind IS NULL OR slack_import_kind IN ('created', 'existing')),
  CONSTRAINT channels_created_user_id_fkey
    FOREIGN KEY (created_user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_channels_is_dm ON channels (is_dm);


CREATE TABLE messages (
  display_data jsonb NOT NULL DEFAULT '{}',
  id text NOT NULL,
  user_id text NOT NULL,
  channel_id text NOT NULL,
  content text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  deleted_at timestamp(3),
  parent_id text,
  thread_archive_at timestamp(3),
  thread_activity_at timestamp(3),
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
CREATE INDEX idx_messages_thread_channel_active ON messages (channel_id, thread_activity_at DESC, id DESC)
  WHERE deleted_at IS NULL AND thread_archive_at IS NULL AND thread_activity_at IS NOT NULL;
CREATE INDEX idx_messages_thread_channel_archived ON messages (channel_id, thread_activity_at DESC, id DESC)
  WHERE deleted_at IS NULL AND thread_archive_at IS NOT NULL AND thread_activity_at IS NOT NULL;
CREATE INDEX idx_messages_thread_idle ON messages (thread_activity_at, id)
  WHERE deleted_at IS NULL AND thread_archive_at IS NULL AND thread_activity_at IS NOT NULL;
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
 envelope:=jsonb_build_object('id',event_uuid,'version',1,'type',kind,'occurred_at',CURRENT_TIMESTAMP,'actor_id',NEW.user_id,'channel_id',NEW.channel_id,'data',to_jsonb(NEW)-'display_data');
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
CREATE TABLE workspace_emojis(width integer,height integer,image_id text,name text PRIMARY KEY,url text NOT NULL,created_by text NOT NULL REFERENCES users(id),updated_at timestamptz NOT NULL DEFAULT now(),category text NOT NULL DEFAULT '',aliases jsonb NOT NULL DEFAULT '[]'::jsonb);
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
 row_data:=(CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END)-'display_data';
 old_data:=(CASE WHEN TG_OP='INSERT' THEN NULL ELSE to_jsonb(OLD) END)-'display_data';
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
 ELSIF (row_data-'thread_archive_at'-'thread_activity_at') IS DISTINCT FROM (old_data-'thread_archive_at'-'thread_activity_at') THEN
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
-- 滞留（未 ACK・期限内）の集計用。期限切れの未 ACK が溜まっても、行ごとにヒープを読まずに済む。
-- app_deliveries_backlog: GET /api/apps/{id}/metrics の滞留（インストールごと）。
-- app_deliveries_queue: zoo-dispatcher の zoo_dispatch_queue_depth・oldest_pending_seconds（全体）。
CREATE INDEX app_deliveries_backlog ON app_deliveries(installation_id,expires_at) INCLUDE (created_at,event_type) WHERE acked_at IS NULL;
CREATE INDEX app_deliveries_queue ON app_deliveries(expires_at) INCLUDE (created_at) WHERE acked_at IS NULL;
CREATE FUNCTION app_delivery_notify() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS (SELECT 1 FROM new_rows n JOIN app_installations i ON i.id=n.installation_id WHERE i.manifest::jsonb->>'transport'='http') THEN
  PERFORM pg_notify('app_delivery','');
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER app_deliveries_notify AFTER INSERT ON app_deliveries REFERENCING NEW TABLE AS new_rows FOR EACH STATEMENT EXECUTE FUNCTION app_delivery_notify();
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

-- Ephemeral call room presence across multi-replica backend instances.
CREATE TABLE call_presence (
  channel_id text NOT NULL,
  session_id uuid NOT NULL,
  user_id text NOT NULL,
  user_name text NOT NULL,
  display_name text NOT NULL,
  is_muted boolean NOT NULL DEFAULT false,
  joined_at text NOT NULL,
  expires_at timestamptz NOT NULL,
  PRIMARY KEY (channel_id, session_id),
  CONSTRAINT call_presence_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT call_presence_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_call_presence_channel ON call_presence (channel_id, expires_at);
CREATE INDEX idx_call_presence_expires_at ON call_presence (expires_at);

ALTER TABLE call_presence ADD CONSTRAINT call_presence_session_id_key UNIQUE (session_id);

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
    UPDATE home_posts SET parent_id=COALESCE(NEW.parent_id,parent_id),content=NEW.content,deleted_at=NEW.deleted_at AT TIME ZONE 'UTC',updated_at=NEW.updated_at AT TIME ZONE 'UTC'
    WHERE id=NEW.home_post_id AND (content,deleted_at,parent_id) IS DISTINCT FROM (NEW.content,NEW.deleted_at AT TIME ZONE 'UTC',COALESCE(NEW.parent_id,parent_id));
  ELSIF TG_OP='INSERT' AND NEW.deleted_at IS NULL AND EXISTS (
    SELECT 1 FROM channels c CROSS JOIN activitypub_settings s WHERE c.id=NEW.channel_id
      AND NOT c.is_private AND NOT c.is_dm AND c.deleted_at IS NULL
      AND (channel_matches_ltl(c.channel_name,s.ltl_channel_pattern) OR EXISTS(
        SELECT 1 FROM home_posts parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL
      ))
  ) THEN
    INSERT INTO home_posts(id,user_id,content,channel_id,created_at,parent_id,visibility,reply_user_id)
      VALUES(NEW.id,NEW.user_id,NEW.content,NEW.channel_id,NEW.created_at AT TIME ZONE 'UTC',NEW.parent_id,
        COALESCE((SELECT visibility FROM home_posts WHERE id=NEW.parent_id),'local'),
        (SELECT user_id FROM home_posts WHERE id=NEW.parent_id)) ON CONFLICT DO NOTHING;
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
CREATE TRIGGER users_home_live AFTER INSERT OR DELETE ON users FOR EACH ROW EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER users_update_home_live AFTER UPDATE ON users FOR EACH ROW WHEN (OLD.* IS DISTINCT FROM NEW.*) EXECUTE FUNCTION notify_home_timeline();
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
    INSERT INTO messages(id,user_id,channel_id,content,created_at,updated_at,deleted_at,home_post_id,apub,apub_visibility,parent_id)
      VALUES(NEW.id,NEW.user_id,NEW.channel_id,NEW.content,NEW.created_at AT TIME ZONE 'UTC',NEW.updated_at AT TIME ZONE 'UTC',NEW.deleted_at AT TIME ZONE 'UTC',NEW.id,NEW.apub,NEW.visibility,CASE WHEN EXISTS(SELECT 1 FROM messages parent WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id) THEN NEW.parent_id END)
      ON CONFLICT(id) DO UPDATE SET content=EXCLUDED.content,updated_at=EXCLUDED.updated_at,
        deleted_at=EXCLUDED.deleted_at,home_post_id=EXCLUDED.home_post_id,apub=EXCLUDED.apub,apub_visibility=EXCLUDED.apub_visibility,parent_id=EXCLUDED.parent_id
      WHERE (messages.content,messages.deleted_at,messages.home_post_id,messages.apub,messages.apub_visibility,messages.parent_id)
        IS DISTINCT FROM (EXCLUDED.content,EXCLUDED.deleted_at,EXCLUDED.home_post_id,EXCLUDED.apub,EXCLUDED.apub_visibility,EXCLUDED.parent_id);
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

-- Slack エクスポート ZIP の取り込み (#295)
-- 取り込み状態・Slack ID との対応表はすべてサーバー専用。Electric の Shape には載せない。

CREATE TABLE slack_import_jobs (
  id text NOT NULL,
  created_by text NOT NULL,
  status text NOT NULL DEFAULT 'uploading',
  file_name text NOT NULL,
  file_size bigint NOT NULL,
  storage_key text NOT NULL,
  team_id text,
  analysis text,
  mappings text,
  total_messages bigint NOT NULL DEFAULT 0,
  processed_messages bigint NOT NULL DEFAULT 0,
  total_files bigint NOT NULL DEFAULT 0,
  processed_files bigint NOT NULL DEFAULT 0,
  error text,
  attempts integer NOT NULL DEFAULT 0,
  lease_until timestamptz,
  lease_token text,
  available_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  -- 取り込みのジョブなら、ZIP を読むエクスポート (NULL ならエクスポート自身)
  source_id text,
  import_progress text,
  handoffs integer NOT NULL DEFAULT 0,
  PRIMARY KEY (id),
  CONSTRAINT slack_import_jobs_status_check CHECK (status IN (
    'uploading', 'analyzing', 'ready', 'importing',
    'fetching_files', 'completed', 'failed', 'cancelled'
  )),
  CONSTRAINT slack_import_jobs_created_by_fkey
    FOREIGN KEY (created_by) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT slack_import_jobs_source_id_fkey
    FOREIGN KEY (source_id) REFERENCES slack_import_jobs (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_slack_import_jobs_runnable ON slack_import_jobs (available_at)
  WHERE status IN ('analyzing', 'importing', 'fetching_files');
CREATE INDEX idx_slack_import_jobs_source_id ON slack_import_jobs (source_id);

-- Slack ユーザーと zoo ユーザーの対応。linked=false はプレースホルダ。
CREATE TABLE slack_users (
  team_id text NOT NULL,
  slack_user_id text NOT NULL,
  user_id text NOT NULL,
  linked boolean NOT NULL DEFAULT false,
  name text NOT NULL DEFAULT '',
  real_name text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_user_id),
  CONSTRAINT slack_users_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_slack_users_user_id ON slack_users (user_id);

-- Slack チャンネルと zoo チャンネルの対応と同期カーソル (最後に取り込んだ ts)。
CREATE TABLE slack_channels (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  channel_id text NOT NULL,
  last_ts text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id),
  CONSTRAINT slack_channels_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_slack_channels_channel_id ON slack_channels (channel_id);

-- Slack の差分同期 (#292) のキューと再開位置。サーバー専用で、Electric の Shape には載せない。
-- Slack チャンネル (slack_channels) ごとに 1 行。同期の進み具合は slack_channels.last_ts (チャンネル単位で共有するカーソル) で、
-- ここには 1 回の同期の途中経過 (ページのカーソル、残りのスレッド) と、次に実行する日時・リースを持つ。
-- 新しいテーブルの追加だけなので、適用前のバックエンドが動いたままでも壊れない。
CREATE TABLE slack_channel_syncs (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  available_at timestamptz NOT NULL DEFAULT now(),
  lease_until timestamptz,
  lease_token text,
  attempts integer NOT NULL DEFAULT 0,
  running boolean NOT NULL DEFAULT false,
  phase text NOT NULL DEFAULT 'history',
  run_oldest text,
  run_max_ts text,
  page_cursor text,
  pending_threads text NOT NULL DEFAULT '[]',
  known_threads text NOT NULL DEFAULT '[]',
  token_user_id text,
  last_synced_at timestamptz,
  last_error text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id),
  CONSTRAINT slack_channel_syncs_phase_check CHECK (phase IN ('history', 'threads')),
  CONSTRAINT slack_channel_syncs_channel_fkey
    FOREIGN KEY (team_id, slack_channel_id) REFERENCES slack_channels (team_id, slack_channel_id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_slack_channel_syncs_due ON slack_channel_syncs (available_at);

-- エクスポートに実体が含まれない添付ファイル。トークンで取得して attachments に移す。
-- 同じファイルが複数のメッセージに共有されることがあるため、メッセージごとに 1 行持つ。
CREATE TABLE slack_files (
  team_id text NOT NULL,
  slack_file_id text NOT NULL,
  job_id text,
  message_id text NOT NULL,
  channel_id text NOT NULL,
  uploader_id text NOT NULL,
  name text NOT NULL,
  mimetype text NOT NULL,
  size bigint NOT NULL DEFAULT 0,
  url_private_download text,
  status text NOT NULL DEFAULT 'pending',
  attachment_id text,
  error text,
  attempts integer NOT NULL DEFAULT 0,
  in_content boolean NOT NULL DEFAULT false,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_file_id, message_id),
  CONSTRAINT slack_files_status_check CHECK (status IN ('pending', 'done', 'skipped', 'failed')),
  CONSTRAINT slack_files_job_id_fkey
    FOREIGN KEY (job_id) REFERENCES slack_import_jobs (id)
    ON UPDATE CASCADE ON DELETE SET NULL,
  CONSTRAINT slack_files_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT slack_files_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT slack_files_uploader_id_fkey
    FOREIGN KEY (uploader_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_slack_files_job_status ON slack_files (job_id, status);
CREATE INDEX idx_slack_files_message ON slack_files (team_id, message_id);

-- 過去ログの一括取り込みでアプリへイベントを大量配送しないよう、
-- トランザクション内で zoo.suppress_app_events=on のときは app_emit を素通りさせる。
CREATE OR REPLACE FUNCTION app_emit(event_name text,event_data jsonb,actor text,channel text DEFAULT NULL,subject text DEFAULT NULL,target_installation text DEFAULT NULL,departed text DEFAULT NULL) RETURNS void LANGUAGE plpgsql AS $$
DECLARE definition app_event_types; installation record; identity text; effective_subject text; event_uuid text:=md5(CASE WHEN event_name='app.rate_limited' THEN date_trunc('hour',now())::text||COALESCE(target_installation,'') ELSE pg_current_xact_id()::text||event_data::text||COALESCE(actor,'')||COALESCE(channel,'')||COALESCE(subject,'') END||event_name)::uuid::text; count_now bigint; terminal boolean; envelope jsonb;
BEGIN
 IF COALESCE(current_setting('zoo.suppress_app_events',true),'')='on' THEN RETURN; END IF;
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

-- Keep legacy local Home reactions available in the channel during rolling upgrades.
CREATE FUNCTION sync_linked_post_reaction() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP <> 'INSERT' AND OLD.user_id IS NOT NULL THEN
    IF TG_OP = 'DELETE' OR (OLD.post_id,OLD.user_id,OLD.reaction) IS DISTINCT FROM (NEW.post_id,NEW.user_id,NEW.reaction) THEN
      DELETE FROM message_reactions r USING home_posts p WHERE p.id=OLD.post_id AND p.channel_id IS NOT NULL
        AND r.message_id=p.id AND r.user_id=OLD.user_id AND r.emoji=OLD.reaction;
    END IF;
  END IF;
  IF TG_OP <> 'DELETE' AND NEW.user_id IS NOT NULL THEN
    INSERT INTO message_reactions(id,message_id,user_id,emoji,created_at)
      SELECT gen_random_uuid()::text,p.id,NEW.user_id,NEW.reaction,NEW.created_at AT TIME ZONE 'UTC'
      FROM home_posts p JOIN messages m ON m.id=p.id WHERE p.id=NEW.post_id AND p.channel_id IS NOT NULL
        AND p.deleted_at IS NULL AND m.deleted_at IS NULL
      ON CONFLICT(message_id,user_id,emoji) DO NOTHING;
  END IF;
  RETURN NULL;
END $$;
CREATE TRIGGER linked_post_reaction_sync AFTER INSERT OR UPDATE OR DELETE ON apub_reactions
  FOR EACH ROW EXECUTE FUNCTION sync_linked_post_reaction();

CREATE FUNCTION notify_linked_message_reaction() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF EXISTS(SELECT 1 FROM home_posts WHERE id=CASE WHEN TG_OP='DELETE' THEN OLD.message_id ELSE NEW.message_id END AND channel_id IS NOT NULL) THEN
    PERFORM pg_notify('home_timeline','');
  END IF;
  RETURN NULL;
END $$;
CREATE TRIGGER linked_message_reaction_live AFTER INSERT OR DELETE ON message_reactions
  FOR EACH ROW EXECUTE FUNCTION notify_linked_message_reaction();

-- Canonical inbox. Legacy tables remain rolling-deployment projections only.
CREATE TABLE notification_events (
  display_data jsonb NOT NULL DEFAULT '{}',
  id text PRIMARY KEY,
  user_id text NOT NULL REFERENCES users(id) ON UPDATE CASCADE ON DELETE CASCADE,
  source text NOT NULL DEFAULT 'channel' CHECK (source IN ('channel', 'activitypub')),
  actor_id text NOT NULL,
  channel_id text REFERENCES channels(id) ON UPDATE CASCADE ON DELETE CASCADE,
  message_id text REFERENCES messages(id) ON UPDATE CASCADE ON DELETE CASCADE,
  notification_type text NOT NULL,
  activity_id text,
  post_id text,
  reaction text,
  emoji jsonb,
  is_read boolean NOT NULL DEFAULT false,
  created_at timestamp(6) NOT NULL DEFAULT timezone('UTC', now()),
  CHECK ((source = 'channel' AND channel_id IS NOT NULL AND message_id IS NOT NULL AND activity_id IS NULL)
      OR (source = 'activitypub' AND channel_id IS NULL AND message_id IS NULL AND activity_id IS NOT NULL
          AND notification_type IN ('follow_request','follow_accepted','like','reply','repost'))),
  UNIQUE (user_id, activity_id, notification_type)
);
CREATE INDEX notification_events_owner ON notification_events(user_id, created_at DESC, id DESC);
CREATE INDEX notification_events_unread ON notification_events(user_id) WHERE NOT is_read;
CREATE INDEX notification_events_message ON notification_events(message_id) WHERE message_id IS NOT NULL;
ALTER TABLE notification_events REPLICA IDENTITY FULL;

-- Updatable, storage-free projections preserve the existing source-specific DTOs.
CREATE VIEW channel_notifications AS
  SELECT id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at
  FROM notification_events WHERE source = 'channel' WITH LOCAL CHECK OPTION;
CREATE VIEW federated_notifications AS
  SELECT id,user_id,actor_id,activity_id,notification_type AS kind,post_id,is_read,
         created_at AT TIME ZONE 'UTC' AS created_at,reaction,emoji
  FROM notification_events WHERE source = 'activitypub';

CREATE FUNCTION sync_notification_events() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP = 'DELETE' THEN
    IF TG_TABLE_NAME = 'notification_events' THEN
      IF OLD.source = 'channel' THEN DELETE FROM notifications WHERE id = OLD.id;
      ELSE DELETE FROM apub_notifications WHERE id = OLD.id; END IF;
    ELSE
      DELETE FROM notification_events WHERE id = OLD.id
        AND source = CASE WHEN TG_TABLE_NAME = 'notifications' THEN 'channel' ELSE 'activitypub' END;
    END IF;
    RETURN OLD;
  END IF;
  IF TG_TABLE_NAME = 'notifications' THEN
    INSERT INTO notification_events AS current (id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at,source) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.channel_id,NEW.message_id,NEW.notification_type,NEW.is_read,NEW.created_at,'channel')
    ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,channel_id=EXCLUDED.channel_id,message_id=EXCLUDED.message_id,notification_type=EXCLUDED.notification_type,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,source=EXCLUDED.source
    WHERE (current.user_id,current.actor_id,current.channel_id,current.message_id,current.notification_type,current.is_read,current.created_at,current.source) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.channel_id,EXCLUDED.message_id,EXCLUDED.notification_type,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.source);
  ELSIF TG_TABLE_NAME = 'apub_notifications' THEN
    INSERT INTO notification_events AS current (id,user_id,actor_id,activity_id,notification_type,post_id,is_read,created_at,reaction,emoji,source) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.activity_id,NEW.kind,NEW.post_id,NEW.is_read,NEW.created_at AT TIME ZONE 'UTC',NEW.reaction,NEW.emoji,'activitypub')
    ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,activity_id=EXCLUDED.activity_id,notification_type=EXCLUDED.notification_type,post_id=EXCLUDED.post_id,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji,source=EXCLUDED.source
    WHERE (current.user_id,current.actor_id,current.activity_id,current.notification_type,current.post_id,current.is_read,current.created_at,current.reaction,current.emoji,current.source) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.activity_id,EXCLUDED.notification_type,EXCLUDED.post_id,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.reaction,EXCLUDED.emoji,EXCLUDED.source);
  ELSE
    IF NEW.source = 'channel' THEN
      INSERT INTO notifications AS current (id,user_id,actor_id,channel_id,message_id,notification_type,is_read,created_at) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.channel_id,NEW.message_id,NEW.notification_type,NEW.is_read,NEW.created_at)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,channel_id=EXCLUDED.channel_id,message_id=EXCLUDED.message_id,notification_type=EXCLUDED.notification_type,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at
      WHERE (current.user_id,current.actor_id,current.channel_id,current.message_id,current.notification_type,current.is_read,current.created_at) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.channel_id,EXCLUDED.message_id,EXCLUDED.notification_type,EXCLUDED.is_read,EXCLUDED.created_at);
    ELSE
      INSERT INTO apub_notifications AS current (id,user_id,actor_id,activity_id,kind,post_id,is_read,created_at,reaction,emoji) VALUES (NEW.id,NEW.user_id,NEW.actor_id,NEW.activity_id,NEW.notification_type,NEW.post_id,NEW.is_read,NEW.created_at AT TIME ZONE 'UTC',NEW.reaction,NEW.emoji)
      ON CONFLICT (id) DO UPDATE SET user_id=EXCLUDED.user_id,actor_id=EXCLUDED.actor_id,activity_id=EXCLUDED.activity_id,kind=EXCLUDED.kind,post_id=EXCLUDED.post_id,is_read=EXCLUDED.is_read,created_at=EXCLUDED.created_at,reaction=EXCLUDED.reaction,emoji=EXCLUDED.emoji
      WHERE (current.user_id,current.actor_id,current.activity_id,current.kind,current.post_id,current.is_read,current.created_at,current.reaction,current.emoji) IS DISTINCT FROM (EXCLUDED.user_id,EXCLUDED.actor_id,EXCLUDED.activity_id,EXCLUDED.kind,EXCLUDED.post_id,EXCLUDED.is_read,EXCLUDED.created_at,EXCLUDED.reaction,EXCLUDED.emoji);
    END IF;
  END IF;
  RETURN NEW;
END;
$$;
CREATE TRIGGER notifications_sync_inbox AFTER INSERT OR UPDATE OR DELETE ON notifications
  FOR EACH ROW EXECUTE FUNCTION sync_notification_events();
CREATE TRIGGER apub_notifications_sync_inbox AFTER INSERT OR UPDATE OR DELETE ON apub_notifications
  FOR EACH ROW EXECUTE FUNCTION sync_notification_events();
CREATE TRIGGER notification_events_sync_inbox AFTER INSERT OR UPDATE OR DELETE ON notification_events
  FOR EACH ROW EXECUTE FUNCTION sync_notification_events();

-- Home images use unguessable capability URLs, including for federated delivery.
CREATE TABLE home_images (
  id text PRIMARY KEY,
  uploader_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  file_size bigint NOT NULL DEFAULT 0,
  content_type text NOT NULL,
  name text NOT NULL,
  width integer,
  height integer,
  blurhash text
);
ALTER TABLE home_posts ADD COLUMN media jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE apub_remote_posts ADD COLUMN media jsonb NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE home_images ADD COLUMN created_at timestamptz NOT NULL DEFAULT now();
CREATE INDEX home_images_uploader_created ON home_images(uploader_id,created_at);
CREATE TABLE home_image_upload_rates (
  user_id text NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  window_start timestamptz NOT NULL,
  requests integer NOT NULL DEFAULT 0,
  PRIMARY KEY(user_id,window_start)
);
CREATE TABLE home_image_deletions (
  id text PRIMARY KEY,
  uploader_id text NOT NULL,
  file_size bigint NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now()
);
CREATE FUNCTION queue_home_image_deletion() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  INSERT INTO home_image_deletions(id,uploader_id,file_size) VALUES(OLD.id,OLD.uploader_id,OLD.file_size) ON CONFLICT DO NOTHING;
  RETURN OLD;
END $$;
CREATE TRIGGER home_image_deleted AFTER DELETE ON home_images FOR EACH ROW EXECUTE FUNCTION queue_home_image_deletion();

CREATE INDEX home_posts_live_media ON home_posts USING gin(media jsonb_path_ops) WHERE deleted_at IS NULL;
CREATE INDEX attachments_storage_key ON attachments(storage_key);
CREATE INDEX home_images_created ON home_images(created_at);
CREATE INDEX home_image_deletions_uploader ON home_image_deletions(uploader_id);
CREATE INDEX home_image_deletions_created ON home_image_deletions(created_at);
-- Shared service read projection; management and ownership remain in the emoji service.
CREATE TABLE shared_emoji_source(id text PRIMARY KEY CHECK (id='workspace'),url text);
CREATE TABLE shared_emojis(
  name text PRIMARY KEY,url text NOT NULL,category text NOT NULL DEFAULT '',
  aliases jsonb NOT NULL DEFAULT '[]'::jsonb,width integer,height integer,image_id text,
  updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE VIEW display_workspace_emojis AS
SELECT width,height,image_id,name,url,''::text AS created_by,updated_at,category,aliases FROM shared_emojis
WHERE EXISTS(SELECT 1 FROM shared_emoji_source WHERE id='workspace' AND url IS NOT NULL)
UNION ALL
SELECT width,height,image_id,name,url,created_by,updated_at,category,aliases FROM workspace_emojis
WHERE NOT EXISTS(SELECT 1 FROM shared_emoji_source WHERE id='workspace' AND url IS NOT NULL);

-- 各ユーザーによる Slack アカウントの連携 (#291)
-- ユーザートークンは AES-256-GCM で暗号化して保存する。Electric の Shape には載せない。
CREATE TABLE slack_user_links (
  user_id text NOT NULL,
  team_id text NOT NULL,
  team_name text NOT NULL DEFAULT '',
  slack_user_id text NOT NULL,
  slack_user_name text NOT NULL DEFAULT '',
  access_token_enc text NOT NULL,
  scopes text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (user_id),
  CONSTRAINT slack_user_links_slack_user_key UNIQUE (team_id, slack_user_id),
  CONSTRAINT slack_user_links_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

-- Slack アカウント連携の付け替え (slack::link::move_content) は、プレースホルダの投稿・添付を
-- 投稿者で引き、Home の返信先も付け替える。連携済みユーザーごとに投稿テーブルを全件走査しないよう、インデックスを足す。
CREATE INDEX idx_messages_user_id ON messages (user_id);
CREATE INDEX idx_slack_files_uploader_id ON slack_files (uploader_id);
CREATE INDEX idx_home_posts_reply_user_id ON home_posts (reply_user_id) WHERE reply_user_id IS NOT NULL;
-- Display read models, atomically cached for Electric replication.
CREATE VIEW message_display_reactions AS
SELECT r.id,r.message_id,r.user_id,r.emoji,r.created_at,u.user_name,u.display_name,NULL::jsonb AS custom_emoji
FROM message_reactions r JOIN users u ON u.id=r.user_id
UNION ALL
SELECT r.activity_id,p.id,r.actor_id,
  CASE WHEN jsonb_typeof(r.emoji)='object' THEN COALESCE(
    (SELECT ':'||e.name||':' FROM display_workspace_emojis e WHERE e.image_id=r.emoji->>'image_id' ORDER BY e.name LIMIT 1),
    'apub:'||(r.emoji->>'image_id')||':'||(r.emoji->>'name')) ELSE r.reaction END,
  r.created_at AT TIME ZONE 'UTC',a.user_name||'@'||split_part(a.id,'/',3),a.display_name,r.emoji
FROM apub_reactions r JOIN home_posts p ON p.id=r.post_id JOIN messages m ON m.id=p.id
JOIN apub_actors a ON a.id=r.actor_id
WHERE p.channel_id IS NOT NULL AND p.deleted_at IS NULL AND m.deleted_at IS NULL
  AND r.user_id IS NULL AND NOT a.blocked;

CREATE FUNCTION message_content_emojis(content text) RETURNS jsonb LANGUAGE sql STABLE AS $$
  SELECT COALESCE(jsonb_object_agg(names.name,jsonb_build_object('url',e.url,'width',e.width,'height',e.height,'image_id',e.image_id)), '{}'::jsonb)
  FROM display_workspace_emojis e CROSS JOIN LATERAL (
    SELECT e.name UNION SELECT jsonb_array_elements_text(e.aliases)
  ) names WHERE strpos(content,':'||names.name||':')>0;
$$;

CREATE VIEW message_display AS
SELECT m.id, jsonb_build_object(
  'author', (SELECT jsonb_build_object('id',u.id,'user_name',u.user_name,
    'display_name',u.display_name,'avatar_url',u.avatar_url,'is_bot',u.is_bot,'app_id',u.app_id)
    FROM users u WHERE u.id=m.user_id),
  'emojis', COALESCE((SELECT jsonb_object_agg(names.name,jsonb_build_object('url',e.url,'width',e.width,'height',e.height,'image_id',e.image_id))
    FROM display_workspace_emojis e CROSS JOIN LATERAL (
      SELECT e.name UNION SELECT jsonb_array_elements_text(e.aliases)
    ) names WHERE strpos(m.content,':'||names.name||':')>0 OR EXISTS (
      SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND r.emoji=':'||names.name||':'
    )), '{}'::jsonb) || COALESCE((SELECT jsonb_object_agg(r.emoji,r.custom_emoji) FROM message_display_reactions r WHERE r.message_id=m.id AND r.custom_emoji IS NOT NULL AND r.emoji LIKE 'apub:%'),'{}'::jsonb),
  'attachments', COALESCE((SELECT jsonb_agg(to_jsonb(a) ORDER BY a.created_at,a.id)
    FROM attachments a WHERE a.message_id=m.id),'[]'::jsonb),
  'reactions', COALESCE((SELECT jsonb_agg(to_jsonb(r) ORDER BY r.created_at,r.id)
    FROM message_display_reactions r WHERE r.message_id=m.id),'[]'::jsonb),
  'reply_count', (SELECT count(*) FROM messages reply WHERE reply.parent_id=m.id AND reply.deleted_at IS NULL)
) AS display_data FROM messages m;

CREATE VIEW notification_display AS
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

CREATE FUNCTION refresh_message_display(target text) RETURNS void LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  -- Serialize writers for this read model before taking the aggregate snapshot.
  PERFORM 1 FROM messages WHERE id=target FOR NO KEY UPDATE;
  UPDATE messages m SET display_data=v.display_data FROM message_display v
    WHERE m.id=target AND v.id=m.id AND m.display_data IS DISTINCT FROM v.display_data;
END;
$$;

CREATE FUNCTION refresh_notification_display(target text) RETURNS void LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM 1 FROM notification_events WHERE id=target FOR NO KEY UPDATE;
  UPDATE notification_events n SET display_data=v.display_data FROM notification_display v
    WHERE n.id=target AND v.id=n.id AND n.display_data IS DISTINCT FROM v.display_data;
END;
$$;

CREATE FUNCTION sync_message_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  -- Include the row and both roots in the same deterministic lock order.
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.parent_id END
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.parent_id END
  ) targets WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
    INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER messages_display_insert AFTER INSERT ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();
CREATE TRIGGER messages_display_update AFTER UPDATE OF content,user_id,parent_id,deleted_at ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();
CREATE TRIGGER messages_display_delete AFTER DELETE ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_display();

CREATE FUNCTION sync_message_relation_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.message_id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.message_id END
  ) ids WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
    INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER attachments_display AFTER INSERT OR UPDATE OR DELETE ON attachments FOR EACH ROW EXECUTE FUNCTION sync_message_relation_display();
CREATE TRIGGER reactions_display AFTER INSERT OR UPDATE OR DELETE ON message_reactions FOR EACH ROW EXECUTE FUNCTION sync_message_relation_display();

CREATE FUNCTION sync_notification_display() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM refresh_notification_display(NEW.id);
  IF NEW.source='channel' THEN
    INSERT INTO display_refresh_queue(kind,target) VALUES ('notification',NEW.id) ON CONFLICT DO NOTHING;
  END IF;
  RETURN NULL;
END;
$$;
CREATE TRIGGER notifications_display_insert AFTER INSERT ON notification_events FOR EACH ROW EXECUTE FUNCTION sync_notification_display();
CREATE TRIGGER notifications_display_update AFTER UPDATE OF actor_id,channel_id,message_id,notification_type,source ON notification_events FOR EACH ROW EXECUTE FUNCTION sync_notification_display();

CREATE FUNCTION sync_message_notifications_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF current_setting('zoo.display_refresh',true)='metadata' THEN
    INSERT INTO display_refresh_queue(kind,target) SELECT 'notification',id FROM notification_events
      WHERE source='channel' AND message_id=NEW.id ORDER BY id ON CONFLICT DO NOTHING;
    RETURN NULL;
  END IF;
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
CREATE TRIGGER message_notifications_display AFTER UPDATE OF content,user_id,display_data,deleted_at ON messages FOR EACH ROW EXECUTE FUNCTION sync_message_notifications_display();




CREATE FUNCTION sync_federated_reaction_display() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE target text;
BEGIN
  IF TG_OP <> 'DELETE' AND jsonb_typeof(NEW.emoji)='object' THEN
    INSERT INTO apub_emoji_images(id,url) VALUES (NEW.emoji->>'image_id',NEW.emoji->>'url')
      ON CONFLICT(id) DO NOTHING;
  END IF;
  FOR target IN SELECT DISTINCT id FROM (
    SELECT CASE WHEN TG_OP <> 'INSERT' THEN OLD.post_id END AS id
    UNION SELECT CASE WHEN TG_OP <> 'DELETE' THEN NEW.post_id END
  ) ids WHERE id IS NOT NULL ORDER BY id LOOP
    PERFORM refresh_message_display(target);
    INSERT INTO display_refresh_queue(kind,target) VALUES ('message',target) ON CONFLICT DO NOTHING;
  END LOOP;
  RETURN NULL;
END;
$$;
CREATE TRIGGER federated_reactions_display AFTER INSERT OR UPDATE OR DELETE ON apub_reactions FOR EACH ROW EXECUTE FUNCTION sync_federated_reaction_display();



-- Metadata changes enqueue read-model work; they never lock message rows.
-- Keep this private table out of Electric Shapes and application events.
CREATE TABLE display_refresh_queue (
  kind text NOT NULL CHECK (kind IN ('message','notification','user_messages','user_notifications','emoji','emoji_image','actor','channel')),
  target text NOT NULL,
  generation bigint NOT NULL DEFAULT txid_current(),
  cursor_id text NOT NULL DEFAULT '',
  attempts integer NOT NULL DEFAULT 0,
  retry_at timestamptz NOT NULL DEFAULT '-infinity',
  PRIMARY KEY(kind,target,generation)
);

CREATE INDEX display_refresh_scopes_idx ON display_refresh_queue(kind,target,generation)
  WHERE kind NOT IN ('message','notification');


-- Separate runnable lookups avoid scanning row jobs to find a scope (or vice versa).
CREATE INDEX display_refresh_rows_ready_idx ON display_refresh_queue(retry_at,kind,target,generation)
  WHERE kind IN ('message','notification');
CREATE INDEX display_refresh_scopes_ready_idx ON display_refresh_queue(retry_at,kind,target,generation)
  WHERE kind NOT IN ('message','notification');

CREATE FUNCTION sync_user_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF ROW(OLD.user_name,OLD.display_name,OLD.avatar_url,OLD.is_bot,OLD.app_id)
    IS NOT DISTINCT FROM ROW(NEW.user_name,NEW.display_name,NEW.avatar_url,NEW.is_bot,NEW.app_id) THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES
    ('user_messages',NEW.id),('user_notifications',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;

CREATE FUNCTION sync_emoji_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM enqueue_emoji_display_scopes(COALESCE((
    WITH changed AS (
      SELECT name,url,aliases,width,height,image_id FROM new_emojis
      EXCEPT SELECT name,url,aliases,width,height,image_id FROM old_emojis
    ), removed AS (
      SELECT name,url,aliases,width,height,image_id FROM old_emojis
      EXCEPT SELECT name,url,aliases,width,height,image_id FROM new_emojis
    ), affected AS (SELECT * FROM changed UNION SELECT * FROM removed)
    SELECT jsonb_agg(to_jsonb(a)) FROM affected a
  ),'[]'::jsonb));
  RETURN NULL;
END;
$$;
-- A common transition-table shape for all three operations keeps filtering in
-- one function while preserving both sides of renames/deletions.
CREATE FUNCTION sync_emoji_insert_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM enqueue_emoji_display_scopes(COALESCE((SELECT jsonb_agg(to_jsonb(e)) FROM new_emojis e),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE FUNCTION sync_emoji_delete_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  PERFORM enqueue_emoji_display_scopes(COALESCE((SELECT jsonb_agg(to_jsonb(e)) FROM old_emojis e),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE TRIGGER emojis_display_update AFTER UPDATE ON workspace_emojis REFERENCING OLD TABLE AS old_emojis NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_display();
CREATE TRIGGER emojis_display_insert AFTER INSERT ON workspace_emojis REFERENCING NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_insert_display();
CREATE TRIGGER emojis_display_delete AFTER DELETE ON workspace_emojis REFERENCING OLD TABLE AS old_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_delete_display();

CREATE FUNCTION sync_federated_actor_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF ROW(OLD.user_name,OLD.display_name,OLD.blocked) IS NOT DISTINCT FROM ROW(NEW.user_name,NEW.display_name,NEW.blocked) THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES ('actor',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE FUNCTION sync_channel_notification_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF OLD IS NOT DISTINCT FROM NEW THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES ('channel',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;

-- One item per transaction: no workspace-wide gate and no multi-message lock
-- order to invert with old backends. Busy rows are retried on a later pass.
CREATE FUNCTION refresh_next_display() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
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

CREATE TRIGGER users_display AFTER UPDATE OF user_name,display_name,avatar_url,is_bot,app_id ON users FOR EACH ROW EXECUTE FUNCTION sync_user_display();
CREATE TRIGGER channels_notification_display AFTER UPDATE ON channels FOR EACH ROW EXECUTE FUNCTION sync_channel_notification_display();
CREATE TRIGGER federated_actors_display AFTER UPDATE OF user_name,display_name,blocked ON apub_actors FOR EACH ROW EXECUTE FUNCTION sync_federated_actor_display();

CREATE FUNCTION enqueue_emoji_display_scopes(changes jsonb) RETURNS void LANGUAGE sql AS $$
  INSERT INTO display_refresh_queue(kind,target)
    SELECT 'emoji',e->>'name' FROM jsonb_array_elements(changes) e
    UNION SELECT 'emoji',jsonb_array_elements_text(e->'aliases') FROM jsonb_array_elements(changes) e
    UNION SELECT 'emoji_image',e->>'image_id' FROM jsonb_array_elements(changes) e
      WHERE COALESCE(e->>'image_id','')<>''
    ON CONFLICT DO NOTHING;
$$;

CREATE FUNCTION display_scope_targets(scope_kind text,scope_target text)
RETURNS TABLE(row_kind text,row_id text) LANGUAGE sql STABLE AS $$
  SELECT 'message'::text,m.id FROM messages m WHERE
    (scope_kind='user_messages' AND (m.user_id=scope_target OR EXISTS (
      SELECT 1 FROM message_reactions r WHERE r.message_id=m.id AND r.user_id=scope_target))) OR
    (scope_kind='emoji' AND (strpos(m.content,':'||scope_target||':')>0 OR EXISTS (
      SELECT 1 FROM message_display_reactions r WHERE r.message_id=m.id AND r.emoji=':'||scope_target||':'))) OR
    (scope_kind='emoji_image' AND EXISTS (
      SELECT 1 FROM apub_reactions r WHERE r.post_id=m.id AND r.emoji->>'image_id'=scope_target)) OR
    (scope_kind='actor' AND EXISTS (
      SELECT 1 FROM apub_reactions r WHERE r.post_id=m.id AND r.actor_id=scope_target))
  UNION ALL
  SELECT 'notification',n.id FROM notification_events n WHERE n.source='channel' AND (
    (scope_kind='user_notifications' AND n.actor_id=scope_target) OR
    (scope_kind='channel' AND n.channel_id=scope_target));
$$;

CREATE FUNCTION expand_next_display_scope() RETURNS boolean LANGUAGE plpgsql SET jit=off AS $$
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

CREATE TRIGGER shared_emojis_display_update AFTER UPDATE ON shared_emojis REFERENCING OLD TABLE AS old_emojis NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_display();
CREATE TRIGGER shared_emojis_display_insert AFTER INSERT ON shared_emojis REFERENCING NEW TABLE AS new_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_insert_display();
CREATE TRIGGER shared_emojis_display_delete AFTER DELETE ON shared_emojis REFERENCING OLD TABLE AS old_emojis FOR EACH STATEMENT EXECUTE FUNCTION sync_emoji_delete_display();
CREATE FUNCTION sync_shared_emoji_source_display() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF TG_OP='UPDATE' AND OLD.url IS NOT DISTINCT FROM NEW.url THEN RETURN NULL; END IF;
  PERFORM enqueue_emoji_display_scopes(COALESCE((SELECT jsonb_agg(to_jsonb(e)) FROM (
    SELECT name,aliases,image_id FROM workspace_emojis UNION ALL SELECT name,aliases,image_id FROM shared_emojis
  ) e),'[]'::jsonb));
  RETURN NULL;
END;
$$;
CREATE TRIGGER shared_emoji_source_display AFTER INSERT OR UPDATE ON shared_emoji_source FOR EACH ROW EXECUTE FUNCTION sync_shared_emoji_source_display();

-- zoo-dispatcher 専用の DB ロール `zoo_dispatcher` の権限。
-- ロールはクラスタ共通で、本番ではアプリのロール（superuser でも CREATEROLE でもない）には作れない。
-- そのためインフラ側（CNPG の managed role など）で先に作る。ここでは、
--   * ロールが無く、作る権限があるときだけ NOLOGIN で作る（ローカル・CI・preview）
--   * ロールが無く作れないときは NOTICE を出して権限の付与を飛ばす（マイグレーションは失敗させない）
--   * ロールがあれば権限を付ける
-- 権限の付与を飛ばした場合は、ロールを作ったあとにこのファイルを同じ DB のオーナーで
-- `psql -f` すれば付与できる（何度流しても同じ結果になる）。
-- 配信関数（app_emit / app_event_access など）は SECURITY DEFINER ではないため、
-- 配信に必要な表の権限をこのロール自身が持つ。
-- 読めないもの: messages（本文の表）、トークンのハッシュ、ユーザー名・チャンネル名、API 側の表。
-- 読めるもの（送信に必要）: 配信キューの payload（イベント本文を含む）と、全アプリの署名鍵。
-- このロールの接続情報が漏れると、キュー内のイベントの中身と署名鍵が漏れる。
DO $$
BEGIN
 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='zoo_dispatcher') THEN
  BEGIN
   CREATE ROLE zoo_dispatcher NOLOGIN;
  EXCEPTION WHEN insufficient_privilege THEN
   RAISE NOTICE 'role zoo_dispatcher does not exist and cannot be created here; create it first, then run this file again to grant privileges';
  END;
 END IF;
 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='zoo_dispatcher') THEN
  GRANT USAGE ON SCHEMA public TO zoo_dispatcher;
  -- 配信キュー。UPDATE は配信の進行に使う列だけ。
  GRANT SELECT, INSERT, DELETE ON app_deliveries TO zoo_dispatcher;
  GRANT UPDATE (attempts, first_delivered_at, lease_until, lease_token, available_at, acked_at, last_error) ON app_deliveries TO zoo_dispatcher;
  GRANT SELECT ON app_installations TO zoo_dispatcher;
  GRANT UPDATE (last_http_dispatch_at) ON app_installations TO zoo_dispatcher;
  -- 配信可否の判定（app_event_access / app_event_matches / app_emit）が読む表。
  GRANT SELECT ON app_event_types, app_event_aliases TO zoo_dispatcher;
  GRANT SELECT (id, bot_user_id, status, deleted_at) ON apps TO zoo_dispatcher;
  GRANT SELECT (installation_id, user_id, scopes, revoked_at, expires_at) ON app_credentials TO zoo_dispatcher;
  GRANT SELECT (id, is_bot, is_guest, is_deactivated) ON users TO zoo_dispatcher;
  GRANT SELECT (id, is_private, is_dm, deleted_at) ON channels TO zoo_dispatcher;
  GRANT SELECT (channel_id, user_id) ON channel_user TO zoo_dispatcher;
  -- 配信履歴の 30 日保持は app_deliveries の DELETE だけで足りる。API 側の期限切れ削除は backend が行う。
 END IF;
END $$;
-- Coalesce local Actor updates durably before expanding them into signed deliveries.
CREATE TABLE apub_actor_updates (
  user_id text PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
  queued_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX apub_actor_updates_queued_at ON apub_actor_updates(queued_at, user_id);
CREATE FUNCTION enqueue_apub_actor_update(target_user text) RETURNS void LANGUAGE sql AS $$
  INSERT INTO apub_actor_updates(user_id)
    SELECT u.id FROM users u WHERE u.id=target_user AND NOT u.is_bot
      AND EXISTS(SELECT 1 FROM apub_follows f JOIN apub_actors a ON a.id=f.actor_id
        WHERE f.user_id=u.id AND f.direction='incoming' AND f.accepted AND NOT a.blocked)
    ON CONFLICT(user_id) DO UPDATE SET queued_at=EXCLUDED.queued_at;
$$;
CREATE FUNCTION enqueue_apub_avatar_update() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  PERFORM enqueue_apub_actor_update(NEW.id);
  RETURN NULL;
END $$;
CREATE TRIGGER users_apub_avatar_update AFTER UPDATE OF avatar_url ON users
  FOR EACH ROW WHEN (OLD.avatar_url IS DISTINCT FROM NEW.avatar_url)
  EXECUTE FUNCTION enqueue_apub_avatar_update();

ALTER TABLE attachments ADD COLUMN width integer, ADD COLUMN height integer, ADD COLUMN blurhash text;

-- 投稿経路によらず、返信と同じトランザクションで活動時刻と再開状態を更新する。
CREATE FUNCTION update_thread_activity() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  IF NEW.parent_id IS NULL OR NEW.deleted_at IS NOT NULL THEN RETURN NULL; END IF;
  IF TG_OP='UPDATE' AND NEW.parent_id IS NOT DISTINCT FROM OLD.parent_id THEN RETURN NULL; END IF;
  UPDATE messages parent SET
    thread_activity_at=GREATEST(parent.thread_activity_at,parent.created_at,NEW.created_at),
    -- INSERT 前にアーカイブされた通常の返信も再開し、過去の投稿の取り込みと区別する。
    thread_archive_at=CASE WHEN NEW.created_at >= parent.thread_archive_at
      OR NEW.created_at >= date_trunc('milliseconds',transaction_timestamp() AT TIME ZONE 'UTC')
      THEN NULL ELSE parent.thread_archive_at END
    WHERE parent.id=NEW.parent_id AND parent.channel_id=NEW.channel_id AND parent.deleted_at IS NULL;
  RETURN NULL;
END $$;
CREATE TRIGGER messages_thread_activity_insert AFTER INSERT ON messages
  FOR EACH ROW EXECUTE FUNCTION update_thread_activity();
CREATE TRIGGER messages_thread_activity_parent_update AFTER UPDATE OF parent_id ON messages
  FOR EACH ROW EXECUTE FUNCTION update_thread_activity();
-- E2EE: 端末と鍵（#354。仕様は docs/e2ee.md の「DB」）。
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
  -- 払い出した時刻。払い出した鍵も行を消さずに残し、アップロードの応答をなくした再送が、主キーの
  -- 衝突で入り直らないようにする（並行した払い出しのコミット前に再送が始まっても）。端末の行と一緒に消える。
  claimed_at timestamptz,
  PRIMARY KEY (user_id, device_id, algorithm, key_id),
  FOREIGN KEY (user_id, device_id) REFERENCES e2ee_devices ON DELETE CASCADE
);
-- 払い出しと件数の確認は、まだ払い出していない鍵だけを読む。
CREATE INDEX e2ee_one_time_keys_unclaimed ON e2ee_one_time_keys (user_id, device_id, created_at, key_id)
  WHERE claimed_at IS NULL;

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

-- keys/claim の払い出しの要求の回数（当日（UTC）の行だけ持ち、前日までの行はバッチで回収する）。
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
    -- 未受信の to-device メッセージも消す（#355）。to-device の送信は同じ端末単位のロックのあとに
    -- deleted_at を読み直すので、このあとで削除済みの端末に入り直らない。
    DELETE FROM e2ee_to_device_messages WHERE recipient_user_id = NEW.id AND recipient_device_id = d;
  END LOOP;
  UPDATE e2ee_devices SET deleted_at = now() WHERE user_id = NEW.id AND deleted_at IS NULL;
  RETURN NULL;
END $$;
CREATE TRIGGER users_e2ee_deactivate AFTER UPDATE OF is_deactivated, guest_kind ON users
  FOR EACH ROW WHEN (NEW.is_deactivated AND NOT OLD.is_deactivated)
  EXECUTE FUNCTION e2ee_delete_devices_on_deactivate();

-- Explicit content labels; defaults preserve existing clients during rollout.
ALTER TABLE channels ADD COLUMN is_nsfw boolean NOT NULL DEFAULT false;
ALTER TABLE attachments ADD COLUMN is_sensitive boolean NOT NULL DEFAULT false;

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

-- E2EE: 暗号化チャンネル（#357。仕様は docs/e2ee.md の「DB」と「membership_epoch と送信の順序」）。
-- channels.encrypted と CHECK channels_encrypted_private は #354 で足した。
ALTER TABLE channels ADD COLUMN membership_epoch bigint NOT NULL DEFAULT 0;

ALTER TABLE messages ADD COLUMN ciphertext jsonb;
ALTER TABLE messages ADD COLUMN epoch bigint;
ALTER TABLE messages ADD COLUMN revision integer NOT NULL DEFAULT 0;
ALTER TABLE messages ADD CONSTRAINT messages_ciphertext_shape
  CHECK (ciphertext IS NULL OR (content = '' AND epoch IS NOT NULL));

-- encrypted は作成時だけ指定できる（両方向の変更を拒否する）。暗号化チャンネルを公開にする変更は
-- CHECK channels_encrypted_private が拒否する。
CREATE FUNCTION e2ee_protect_channel_encryption() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE EXCEPTION 'channels.encrypted cannot be changed' USING ERRCODE = '23514';
END $$;
CREATE TRIGGER channels_encrypted_immutable BEFORE UPDATE ON channels
  FOR EACH ROW WHEN (OLD.encrypted IS DISTINCT FROM NEW.encrypted)
  EXECUTE FUNCTION e2ee_protect_channel_encryption();

-- 論理削除した行（DELETE /api/messages/{id} と履歴の消去）の暗号文は残さない。
CREATE FUNCTION e2ee_clear_deleted_ciphertext() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  NEW.ciphertext := NULL;
  RETURN NEW;
END $$;
CREATE TRIGGER messages_clear_deleted_ciphertext BEFORE INSERT OR UPDATE ON messages
  FOR EACH ROW WHEN (NEW.deleted_at IS NOT NULL AND NEW.ciphertext IS NOT NULL)
  EXECUTE FUNCTION e2ee_clear_deleted_ciphertext();

-- そのユーザーが参加しているすべての暗号化チャンネル（削除済みを除く）の membership_epoch を 1 増やす。
-- 端末の削除、アカウントの無効化、クロス署名鍵の変更、鍵バックアップの版の作成で使う。
-- 行は id の昇順にまとめてロックし、ロックした行だけを更新する（並行した一括更新と行き詰まらない）。
-- 参加先を読む前に、メンバー単位のロック (3, hashtext(user_id)) を取る。暗号化チャンネルへのメンバーの追加も、
-- 追加する人について同じロックを取ってから channel_user に書く。これで、参加先を確定してからコミットするまでに
-- 新しい所属が確定せず、追加されたチャンネルが増加から漏れない。
-- ロックの順：users の行、ユーザー単位のロック (1)、端末単位のロック (2)、メンバー単位のロック (3)、channels の行。
CREATE FUNCTION e2ee_bump_membership_epochs(target_user text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE
  ids text[];
BEGIN
  PERFORM pg_advisory_xact_lock(3, hashtext(target_user));
  SELECT array_agg(id) INTO ids FROM (
    SELECT id FROM channels
      WHERE encrypted AND deleted_at IS NULL
        AND id IN (SELECT channel_id FROM channel_user WHERE user_id = target_user)
      ORDER BY id FOR NO KEY UPDATE
  ) locked;
  IF ids IS NOT NULL THEN
    UPDATE channels SET membership_epoch = membership_epoch + 1
      WHERE id = ANY (ids) AND encrypted AND deleted_at IS NULL;
  END IF;
END $$;

-- アカウントの無効化でも、参加しているすべての暗号化チャンネルの membership_epoch を増やす。
-- ロックの順（users の行、端末単位のロック、channels の行、e2ee_devices の行）に合わせ、端末単位のロックのあと、
-- e2ee_devices の更新の前に増やす。
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
  PERFORM e2ee_bump_membership_epochs(NEW.id);
  UPDATE e2ee_devices SET deleted_at = now() WHERE user_id = NEW.id AND deleted_at IS NULL;
  RETURN NULL;
END $$;

-- membership_epoch だけの変更では、通知の表示、App のイベント、Home の同期を作り直さない。
CREATE OR REPLACE FUNCTION sync_channel_notification_display() RETURNS trigger LANGUAGE plpgsql SET jit=off AS $$
BEGIN
  IF (to_jsonb(OLD) - 'membership_epoch') IS NOT DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch') THEN RETURN NULL; END IF;
  INSERT INTO display_refresh_queue(kind,target) VALUES ('channel',NEW.id) ON CONFLICT DO NOTHING;
  RETURN NULL;
END;
$$;
CREATE OR REPLACE FUNCTION app_channel_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE row_data jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;channel text:=row_data->>'id';actor text:=COALESCE(NULLIF(current_setting('zoo.actor_id',true),''),row_data->>'created_user_id');
BEGIN
 -- membership_epoch だけの変更（暗号化チャンネルのメンバーや端末の変化）は、チャンネルのイベントにしない。
 IF TG_OP='UPDATE' AND (to_jsonb(OLD) - 'membership_epoch') IS NOT DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch') THEN RETURN NULL; END IF;
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

-- 文レベルのトリガーは OLD と NEW を見られないので、UPDATE だけを行レベルのトリガーに分ける。
DROP TRIGGER channels_home_live ON channels;
CREATE TRIGGER channels_home_live AFTER INSERT OR DELETE ON channels FOR EACH STATEMENT EXECUTE FUNCTION notify_home_timeline();
CREATE TRIGGER channels_home_live_update AFTER UPDATE ON channels FOR EACH ROW
  WHEN ((to_jsonb(OLD) - 'membership_epoch') IS DISTINCT FROM (to_jsonb(NEW) - 'membership_epoch'))
  EXECUTE FUNCTION notify_home_timeline();

-- 通知の表示に埋め込むチャンネルは列を明示し、membership_epoch を含めない。
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
  'channel', jsonb_build_object('id',c.id,'channel_name',c.channel_name,'description',c.description,'created_user_id',c.created_user_id,'is_private',c.is_private,'is_dm',c.is_dm,'created_at',c.created_at,'updated_at',c.updated_at,'archive_at',c.archive_at,'deleted_at',c.deleted_at,'imported_from_slack',c.imported_from_slack,'slack_import_kind',c.slack_import_kind,'slack_imported_at',c.slack_imported_at,'topic',c.topic,'posting_policy',c.posting_policy,'encrypted',c.encrypted,'is_nsfw',c.is_nsfw)
) AS display_data
FROM notification_events n JOIN users u ON u.id=n.actor_id JOIN channels c ON c.id=n.channel_id
LEFT JOIN messages m ON m.id=n.message_id AND m.channel_id=n.channel_id
LEFT JOIN LATERAL (
  SELECT entry.value AS reaction FROM jsonb_array_elements(COALESCE(m.display_data->'reactions','[]'::jsonb)) AS entry(value)
  WHERE n.notification_type='reaction' AND entry.value->>'user_id'=n.actor_id
  ORDER BY entry.value->>'created_at' DESC,entry.value->>'id' LIMIT 1
) badge ON true
WHERE n.source='channel';
