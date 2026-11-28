-- 参加・退出イベントは従来公開していた user_id / channel_id / joined_at だけを渡す。
-- notification_level は本人用の Shape / API でのみ同期する (#620)。
CREATE OR REPLACE FUNCTION app_member_events_v2() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE src jsonb:=CASE WHEN TG_OP='DELETE' THEN to_jsonb(OLD) ELSE to_jsonb(NEW) END;
DECLARE row_data jsonb:=jsonb_build_object('user_id',src->>'user_id','channel_id',src->>'channel_id','joined_at',src->>'joined_at');
BEGIN
 PERFORM app_emit(CASE WHEN TG_OP='DELETE' THEN 'member.left' ELSE 'member.joined' END,row_data,row_data->>'user_id',row_data->>'channel_id',NULL,NULL,CASE WHEN TG_OP='DELETE' THEN row_data->>'user_id' END);
 RETURN CASE WHEN TG_OP='DELETE' THEN OLD ELSE NEW END;
END $$;
