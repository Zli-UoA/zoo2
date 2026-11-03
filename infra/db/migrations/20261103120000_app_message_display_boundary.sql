-- Keep UI-only joined profiles and attachment metadata out of App events.
CREATE OR REPLACE FUNCTION enqueue_app_special_message() RETURNS trigger LANGUAGE plpgsql AS $$
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
