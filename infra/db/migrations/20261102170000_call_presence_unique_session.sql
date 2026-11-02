-- Ensure session_id is globally unique across all channels to prevent cross-channel session hijacking.
ALTER TABLE call_presence ADD CONSTRAINT call_presence_session_id_key UNIQUE (session_id);
