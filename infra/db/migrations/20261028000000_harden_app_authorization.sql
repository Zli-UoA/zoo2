-- Harden event delivery and bound transport/retention work without rewriting deployed migrations.
CREATE INDEX app_deliveries_retention ON app_deliveries(created_at,id);
CREATE INDEX app_deliveries_ready ON app_deliveries(installation_id,available_at,created_at,id) WHERE acked_at IS NULL;
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
