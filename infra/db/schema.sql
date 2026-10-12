CREATE TABLE "User" (
  id text NOT NULL,
  user_name text NOT NULL,
  display_name varchar(255) NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  PRIMARY KEY (id),
  CONSTRAINT "User_user_name_key" UNIQUE (user_name)
);

CREATE TABLE "Channel" (
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
  CONSTRAINT "Channel_channel_name_key" UNIQUE (channel_name),
  CONSTRAINT "Channel_created_user_id_fkey"
    FOREIGN KEY (created_user_id) REFERENCES "User" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE TABLE "Message" (
  id text NOT NULL,
  user_id text NOT NULL,
  channel_id text NOT NULL,
  content text NOT NULL,
  created_at timestamp(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  updated_at timestamp(3),
  deleted_at timestamp(3),
  PRIMARY KEY (id),
  CONSTRAINT "Message_user_id_fkey"
    FOREIGN KEY (user_id) REFERENCES "User" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT "Message_channel_id_fkey"
    FOREIGN KEY (channel_id) REFERENCES "Channel" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE TABLE "ChannelUser" (
  user_id text NOT NULL,
  channel_id text NOT NULL,
  joined_at timestamp(3) NOT NULL,
  PRIMARY KEY (user_id, channel_id),
  CONSTRAINT "ChannelUser_user_id_fkey"
    FOREIGN KEY (user_id) REFERENCES "User" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT "ChannelUser_channel_id_fkey"
    FOREIGN KEY (channel_id) REFERENCES "Channel" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE TABLE "Role" (
  id text NOT NULL,
  role_name varchar(255) NOT NULL,
  PRIMARY KEY (id)
);

CREATE TABLE "UserRole" (
  user_id text NOT NULL,
  role_id text NOT NULL,
  PRIMARY KEY (user_id, role_id),
  CONSTRAINT "UserRole_user_id_fkey"
    FOREIGN KEY (user_id) REFERENCES "User" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT,
  CONSTRAINT "UserRole_role_id_fkey"
    FOREIGN KEY (role_id) REFERENCES "Role" (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);
