-- The always-lock version of app_guest_membership_rules is deferred to
-- infra/db/deferred-migrations/20261201010009_guest_membership_lock_order.sql.
-- Applying it before the new backend replaces old instances can deadlock with
-- the old lock_members order. The standard deploy applies only this canonical
-- directory; the follow-up migration is applied after the rollout.
--
-- The conditional single-guest check takes FOR NO KEY UPDATE instead of
-- FOR UPDATE. FOR UPDATE also conflicts with the KEY SHARE lock that a
-- channel_user INSERT takes on the referenced users row (foreign key
-- validation), so the old form could still block concurrent membership
-- inserts of the same user from cooperative writers. NO KEY UPDATE leaves
-- the key columns free while still serializing the guest_kind check with
-- updates that read or change guest_kind. Deadlock-avoidance needs the
-- unconditional lock, which stays deferred until the rollout completes.
CREATE OR REPLACE FUNCTION app_guest_membership_rules() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM users WHERE id=NEW.user_id AND guest_kind='single') THEN PERFORM 1 FROM users WHERE id=NEW.user_id FOR NO KEY UPDATE;END IF;
 IF EXISTS(SELECT 1 FROM users WHERE id=NEW.user_id AND guest_kind='single')
 AND EXISTS(SELECT 1 FROM channels WHERE id=NEW.channel_id AND NOT is_dm)
 AND EXISTS(SELECT 1 FROM channel_user cu JOIN channels c ON c.id=cu.channel_id WHERE cu.user_id=NEW.user_id AND cu.channel_id<>NEW.channel_id AND NOT c.is_dm AND c.deleted_at IS NULL) THEN RAISE EXCEPTION USING ERRCODE='23514',MESSAGE='Single-channel guests can join only one channel';END IF;
 RETURN NEW;
END $$;
