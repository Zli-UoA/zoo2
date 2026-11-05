-- This is a historical authorization hint, not ownership. Its foreign key makes
-- unrelated profile writes lock every recipient bot and deadlock each other.
-- app_delivery_allowed always resolves the current user and consent before send.
ALTER TABLE app_deliveries DROP CONSTRAINT app_deliveries_access_user_id_fkey;
