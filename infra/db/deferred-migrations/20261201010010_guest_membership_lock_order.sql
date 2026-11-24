-- Replace the guest-membership trigger function only after the new backend is
-- rolled out. Old lock_members takes the member advisory lock before locking
-- users; enabling this lock during a rolling deploy can deadlock with deactivation.
--
-- Direct-SQL bulk inserts into channel_user that touch multiple users must first
-- lock the target users rows in ascending id order, for example:
--   SELECT id FROM users WHERE id = ANY($1) ORDER BY id FOR NO KEY UPDATE;
-- The trigger below locks each NEW.user_id one row at a time, so two concurrent
-- bulk inserts that meet the same users in opposite order can deadlock (40P01).
CREATE OR REPLACE FUNCTION app_guest_membership_rules() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 -- Always serialize membership inserts with guest_kind updates. A conditional lock misses a
 -- concurrent transition from a non-single kind while the new membership is still uncommitted.
 PERFORM 1 FROM users WHERE id=NEW.user_id FOR NO KEY UPDATE;
 IF EXISTS(SELECT 1 FROM users WHERE id=NEW.user_id AND guest_kind='single')
 AND EXISTS(SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_dm)
 AND EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.user_id=NEW.user_id AND cu.channel_id<>NEW.channel_id AND NOT c.is_dm AND c.deleted_at IS NULL) THEN
   RAISE EXCEPTION USING ERRCODE='23514',MESSAGE='Single-channel guests can join only one channel';
 END IF;
 RETURN NEW;
END $$;
