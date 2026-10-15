-- Drop foreign key constraints before rename
ALTER TABLE "ChannelUser" DROP CONSTRAINT "ChannelUser_channel_id_fkey";
ALTER TABLE "ChannelUser" DROP CONSTRAINT "ChannelUser_user_id_fkey";
ALTER TABLE "Message" DROP CONSTRAINT "Message_channel_id_fkey";
ALTER TABLE "Message" DROP CONSTRAINT "Message_user_id_fkey";
ALTER TABLE "UserRole" DROP CONSTRAINT "UserRole_role_id_fkey";
ALTER TABLE "UserRole" DROP CONSTRAINT "UserRole_user_id_fkey";
ALTER TABLE "Channel" DROP CONSTRAINT "Channel_created_user_id_fkey";

-- Rename tables to snake_case
ALTER TABLE "User" RENAME TO users;
ALTER TABLE "Channel" RENAME TO channels;
ALTER TABLE "Message" RENAME TO messages;
ALTER TABLE "ChannelUser" RENAME TO channel_user;
ALTER TABLE "Role" RENAME TO role;
ALTER TABLE "UserRole" RENAME TO user_role;

-- Rename unique constraints
ALTER TABLE users RENAME CONSTRAINT "User_user_name_key" TO users_user_name_key;
ALTER TABLE channels RENAME CONSTRAINT "Channel_channel_name_key" TO channels_channel_name_key;

-- Re-add foreign key constraints with snake_case names
ALTER TABLE channels ADD CONSTRAINT channels_created_user_id_fkey FOREIGN KEY (created_user_id) REFERENCES users (id) ON UPDATE CASCADE ON DELETE RESTRICT;
ALTER TABLE messages ADD CONSTRAINT messages_user_id_fkey FOREIGN KEY (user_id) REFERENCES users (id) ON UPDATE CASCADE ON DELETE RESTRICT;
ALTER TABLE messages ADD CONSTRAINT messages_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES channels (id) ON UPDATE CASCADE ON DELETE RESTRICT;
ALTER TABLE channel_user ADD CONSTRAINT channel_user_user_id_fkey FOREIGN KEY (user_id) REFERENCES users (id) ON UPDATE CASCADE ON DELETE RESTRICT;
ALTER TABLE channel_user ADD CONSTRAINT channel_user_channel_id_fkey FOREIGN KEY (channel_id) REFERENCES channels (id) ON UPDATE CASCADE ON DELETE RESTRICT;
ALTER TABLE user_role ADD CONSTRAINT user_role_user_id_fkey FOREIGN KEY (user_id) REFERENCES users (id) ON UPDATE CASCADE ON DELETE RESTRICT;
ALTER TABLE user_role ADD CONSTRAINT user_role_role_id_fkey FOREIGN KEY (role_id) REFERENCES role (id) ON UPDATE CASCADE ON DELETE RESTRICT;

-- Set REPLICA IDENTITY FULL for ElectricSQL sync
ALTER TABLE users REPLICA IDENTITY FULL;
ALTER TABLE channels REPLICA IDENTITY FULL;
ALTER TABLE messages REPLICA IDENTITY FULL;
ALTER TABLE channel_user REPLICA IDENTITY FULL;
