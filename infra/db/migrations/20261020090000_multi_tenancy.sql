CREATE TABLE tenants (
  id text PRIMARY KEY,
  name varchar(100) NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE tenant_users (
  tenant_id text NOT NULL REFERENCES tenants (id) ON DELETE RESTRICT,
  user_id text NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
  joined_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (tenant_id, user_id)
);
CREATE INDEX idx_tenant_users_user_id ON tenant_users (user_id);
ALTER TABLE tenant_users REPLICA IDENTITY FULL;

-- Preserve all existing data and grants inside the original workspace.
INSERT INTO tenants (id, name) VALUES ('00000000-0000-0000-0000-000000000001', 'Zoo');
INSERT INTO tenant_users (tenant_id, user_id) SELECT '00000000-0000-0000-0000-000000000001', id FROM users;
INSERT INTO role (id, role_name) VALUES ('role_admin', 'admin') ON CONFLICT (id) DO NOTHING;
ALTER TABLE channels ADD COLUMN tenant_id text NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
ALTER TABLE channels ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE messages ADD COLUMN tenant_id text NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
ALTER TABLE messages ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE message_reactions ADD COLUMN tenant_id text NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
ALTER TABLE message_reactions ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE channel_user ADD COLUMN tenant_id text NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
ALTER TABLE channel_user ALTER COLUMN tenant_id DROP DEFAULT;
ALTER TABLE user_role ADD COLUMN tenant_id text NOT NULL DEFAULT '00000000-0000-0000-0000-000000000001';
ALTER TABLE user_role ALTER COLUMN tenant_id DROP DEFAULT;

ALTER TABLE channels DROP CONSTRAINT channels_channel_name_key;
ALTER TABLE channels ADD CONSTRAINT channels_tenant_name_key UNIQUE (tenant_id, channel_name);
ALTER TABLE channels ADD CONSTRAINT channels_tenant_id_key UNIQUE (tenant_id, id);
ALTER TABLE channels ADD CONSTRAINT channels_tenant_fkey FOREIGN KEY (tenant_id) REFERENCES tenants (id);
ALTER TABLE channels ADD CONSTRAINT channels_tenant_creator_fkey FOREIGN KEY (tenant_id, created_user_id) REFERENCES tenant_users (tenant_id, user_id);
ALTER TABLE messages ADD CONSTRAINT messages_tenant_id_key UNIQUE (tenant_id, id);
ALTER TABLE messages ADD CONSTRAINT messages_channel_id_id_key UNIQUE (channel_id, id);
ALTER TABLE messages ADD CONSTRAINT messages_tenant_channel_fkey FOREIGN KEY (tenant_id, channel_id) REFERENCES channels (tenant_id, id);
ALTER TABLE messages ADD CONSTRAINT messages_tenant_author_fkey FOREIGN KEY (tenant_id, user_id) REFERENCES tenant_users (tenant_id, user_id);
ALTER TABLE messages ADD CONSTRAINT messages_parent_channel_fkey FOREIGN KEY (channel_id, parent_id) REFERENCES messages (channel_id, id);
ALTER TABLE channel_user ADD CONSTRAINT channel_user_tenant_channel_fkey FOREIGN KEY (tenant_id, channel_id) REFERENCES channels (tenant_id, id);
ALTER TABLE channel_user ADD CONSTRAINT channel_user_tenant_member_fkey FOREIGN KEY (tenant_id, user_id) REFERENCES tenant_users (tenant_id, user_id);
ALTER TABLE message_reactions ADD CONSTRAINT message_reactions_tenant_message_fkey FOREIGN KEY (tenant_id, message_id) REFERENCES messages (tenant_id, id) ON DELETE CASCADE;
ALTER TABLE message_reactions ADD CONSTRAINT message_reactions_tenant_member_fkey FOREIGN KEY (tenant_id, user_id) REFERENCES tenant_users (tenant_id, user_id);
ALTER TABLE user_role DROP CONSTRAINT "UserRole_pkey";
ALTER TABLE user_role ADD PRIMARY KEY (tenant_id, user_id, role_id);
ALTER TABLE user_role ADD CONSTRAINT user_role_tenant_member_fkey FOREIGN KEY (tenant_id, user_id) REFERENCES tenant_users (tenant_id, user_id);
CREATE INDEX idx_messages_tenant_channel ON messages (tenant_id, channel_id);
CREATE INDEX idx_channel_user_tenant ON channel_user (tenant_id);
CREATE INDEX idx_message_reactions_tenant ON message_reactions (tenant_id);
