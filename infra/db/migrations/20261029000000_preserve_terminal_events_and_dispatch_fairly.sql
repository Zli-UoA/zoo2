-- Lifecycle control events must survive both source and dispatcher quotas.
CREATE FUNCTION app_event_quota_exempt(event_name text) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
 SELECT event_name IN('app.rate_limited','app.uninstalled','app.deleted','tokens.revoked')
$$;
ALTER TABLE app_installations ADD COLUMN last_http_dispatch_at timestamptz;
CREATE INDEX app_installations_http_rotation ON app_installations(last_http_dispatch_at,id);

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
  IF NOT app_event_quota_exempt(event_name) THEN
   -- Append-only accounting avoids taking installation locks in source transactions.
   SELECT COALESCE((SELECT events FROM app_event_limits WHERE installation_id=installation.id AND window_start=date_trunc('hour',now())),0)+count(*)+1 INTO count_now
    FROM app_deliveries WHERE installation_id=installation.id AND created_at>=date_trunc('hour',now()) AND event_type<>'app.rate_limited' AND NOT app_event_quota_exempt(event_type);
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

