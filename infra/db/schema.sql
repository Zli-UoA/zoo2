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


