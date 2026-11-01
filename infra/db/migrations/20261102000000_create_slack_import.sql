-- Slack エクスポート ZIP の取り込み (#295)
-- 取り込み状態・Slack ID との対応表はすべてサーバー専用。Electric の Shape には載せない。

CREATE TABLE slack_import_jobs (
  id text NOT NULL,
  created_by text NOT NULL,
  status text NOT NULL DEFAULT 'uploading',
  file_name text NOT NULL,
  file_size bigint NOT NULL,
  storage_key text NOT NULL,
  team_id text,
  analysis text,
  mappings text,
  total_messages bigint NOT NULL DEFAULT 0,
  processed_messages bigint NOT NULL DEFAULT 0,
  total_files bigint NOT NULL DEFAULT 0,
  processed_files bigint NOT NULL DEFAULT 0,
  error text,
  attempts integer NOT NULL DEFAULT 0,
  lease_until timestamptz,
  lease_token text,
  available_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  finished_at timestamptz,
  PRIMARY KEY (id),
  CONSTRAINT slack_import_jobs_status_check CHECK (status IN (
    'uploading', 'analyzing', 'awaiting_mapping', 'importing',
    'fetching_files', 'completed', 'failed', 'cancelled'
  )),
  CONSTRAINT slack_import_jobs_created_by_fkey
    FOREIGN KEY (created_by) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_slack_import_jobs_runnable ON slack_import_jobs (available_at)
  WHERE status IN ('analyzing', 'importing', 'fetching_files');

-- Slack ユーザーと zoo ユーザーの対応。linked=false はプレースホルダ。
CREATE TABLE slack_users (
  team_id text NOT NULL,
  slack_user_id text NOT NULL,
  user_id text NOT NULL,
  linked boolean NOT NULL DEFAULT false,
  name text NOT NULL DEFAULT '',
  real_name text NOT NULL DEFAULT '',
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_user_id),
  CONSTRAINT slack_users_user_id_fkey
    FOREIGN KEY (user_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_slack_users_user_id ON slack_users (user_id);

-- Slack チャンネルと zoo チャンネルの対応と同期カーソル (最後に取り込んだ ts)。
CREATE TABLE slack_channels (
  team_id text NOT NULL,
  slack_channel_id text NOT NULL,
  channel_id text NOT NULL,
  last_ts text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_channel_id),
  CONSTRAINT slack_channels_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

CREATE INDEX idx_slack_channels_channel_id ON slack_channels (channel_id);

-- エクスポートに実体が含まれない添付ファイル。トークンで取得して attachments に移す。
CREATE TABLE slack_files (
  team_id text NOT NULL,
  slack_file_id text NOT NULL,
  job_id text,
  message_id text NOT NULL,
  channel_id text NOT NULL,
  uploader_id text NOT NULL,
  name text NOT NULL,
  mimetype text NOT NULL,
  size bigint NOT NULL DEFAULT 0,
  url_private_download text,
  status text NOT NULL DEFAULT 'pending',
  attachment_id text,
  error text,
  attempts integer NOT NULL DEFAULT 0,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY (team_id, slack_file_id),
  CONSTRAINT slack_files_status_check CHECK (status IN ('pending', 'done', 'skipped', 'failed')),
  CONSTRAINT slack_files_job_id_fkey
    FOREIGN KEY (job_id) REFERENCES slack_import_jobs (id)
    ON UPDATE CASCADE ON DELETE SET NULL,
  CONSTRAINT slack_files_message_id_fkey
    FOREIGN KEY (message_id) REFERENCES messages (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT slack_files_channel_id_fkey
    FOREIGN KEY (channel_id) REFERENCES channels (id)
    ON UPDATE CASCADE ON DELETE CASCADE,
  CONSTRAINT slack_files_uploader_id_fkey
    FOREIGN KEY (uploader_id) REFERENCES users (id)
    ON UPDATE CASCADE ON DELETE RESTRICT
);

CREATE INDEX idx_slack_files_pending ON slack_files (job_id) WHERE status = 'pending';

-- 過去ログの一括取り込みでアプリへイベントを大量配送しないよう、
-- トランザクション内で zoo.suppress_app_events=on のときは app_emit を素通りさせる。
CREATE OR REPLACE FUNCTION app_emit(event_name text,event_data jsonb,actor text,channel text DEFAULT NULL,subject text DEFAULT NULL,target_installation text DEFAULT NULL,departed text DEFAULT NULL) RETURNS void LANGUAGE plpgsql AS $$
DECLARE definition app_event_types; installation record; identity text; effective_subject text; event_uuid text:=md5(CASE WHEN event_name='app.rate_limited' THEN date_trunc('hour',now())::text||COALESCE(target_installation,'') ELSE pg_current_xact_id()::text||event_data::text||COALESCE(actor,'')||COALESCE(channel,'')||COALESCE(subject,'') END||event_name)::uuid::text; count_now bigint; terminal boolean; envelope jsonb;
BEGIN
 IF COALESCE(current_setting('zoo.suppress_app_events',true),'')='on' THEN RETURN; END IF;
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
