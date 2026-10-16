CREATE TABLE users (
  id text NOT NULL,
  user_name text NOT NULL,
  display_name varchar(255) NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  oidc_iss text,
  oidc_sub text,
  affiliation varchar(32) NOT NULL DEFAULT 'unknown',
  PRIMARY KEY (id),
  CONSTRAINT users_user_name_key UNIQUE (user_name),
  CONSTRAINT users_oidc_iss_sub_key UNIQUE (oidc_iss, oidc_sub)
);

CREATE TABLE channels (
  id text NOT NULL,
  channel_name varchar(255) NOT NULL,
  description varchar(255),
  created_user_id text NOT NULL,
  is_private boolean NOT NULL,
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

CREATE TABLE messages (
  id text NOT NULL,
  user_id text NOT NULL,
  channel_id text NOT NULL,
  content text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  deleted_at timestamp(3),
  PRIMARY KEY (id),
  CONSTRAINT messages_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT messages_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
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
  PRIMARY KEY (id)
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
