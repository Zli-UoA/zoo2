-- Call SFU (docs/calls-sfu.md). Additive only: old backends ignore the new columns and tables.
ALTER TABLE call_presence
  ADD COLUMN sfu_capable boolean NOT NULL DEFAULT false,
  ADD COLUMN sfu_id text,
  ADD COLUMN sfu_room text,
  ADD COLUMN cf_session_id text,
  ADD COLUMN livekit_identity text;

-- sfu-bridge looks up rooms that have LiveKit participants (docs/calls-sfu.md).
CREATE INDEX idx_call_presence_livekit ON call_presence (channel_id) WHERE sfu_id = 'livekit';

-- Media mode of a call room. 'mesh' is P2P between clients; 'sfu' routes audio through an SFU.
-- epoch increases each time an empty room is reopened, so stale clients and the bridge can tell rooms apart.
CREATE TABLE call_rooms (
  channel_id text PRIMARY KEY REFERENCES channels(id) ON UPDATE CASCADE ON DELETE CASCADE,
  mode text NOT NULL DEFAULT 'mesh' CHECK (mode IN ('mesh','sfu')),
  epoch bigint NOT NULL DEFAULT 0,
  track_version bigint NOT NULL DEFAULT 0
);

-- Audio tracks published on Cloudflare Realtime. 'direct' is pushed by the speaker's client; 'bridge' is a
-- LiveKit speaker relayed by sfu-bridge. Rows go away with the speaker's presence.
CREATE TABLE call_sfu_tracks (
  channel_id text NOT NULL,
  session_id uuid NOT NULL,
  source text NOT NULL CHECK (source IN ('direct','bridge')),
  cf_session_id text NOT NULL,
  track_name text NOT NULL,
  bridge_epoch bigint,
  PRIMARY KEY (channel_id, session_id, source),
  CONSTRAINT call_sfu_tracks_presence_fkey
    FOREIGN KEY (channel_id, session_id) REFERENCES call_presence (channel_id, session_id)
    ON UPDATE CASCADE ON DELETE CASCADE
);

-- Sessions that left while assigned to an SFU. A backend worker closes their SFU media and deletes the row.
-- claimed_until: a worker claims rows in a short statement and calls the SFU APIs outside any transaction.
-- Another worker can take the row after this time (crashed worker), and failed rows wait until it to retry.
-- The newest registered sfu-bridge. Tracks pushed by an older bridge are rejected, so a bridge that
-- outlives its replacement cannot overwrite the new bridge's rows.
CREATE TABLE call_sfu_bridge (
  id smallint PRIMARY KEY CHECK (id = 1),
  epoch bigint NOT NULL
);

CREATE TABLE call_sfu_departures (
  id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  channel_id text NOT NULL,
  session_id uuid NOT NULL,
  sfu_id text NOT NULL,
  sfu_room text,
  cf_session_id text,
  livekit_identity text,
  attempts integer NOT NULL DEFAULT 0,
  claimed_until timestamptz,
  created_at timestamptz NOT NULL DEFAULT now()
);

-- The departure worker claims each SFU's rows separately (sfu_id = '<sfu>' ORDER BY id). When that SFU has
-- few rows, the claim uses this index and does not read the other SFU's rows (e.g. LiveKit rows kept while
-- LiveKit is disabled). When it has many, the planner may scan the primary key instead and filter out the
-- other SFU's older rows, at most the other SFU's open departures.
CREATE INDEX idx_call_sfu_departures_sfu ON call_sfu_departures (sfu_id, id);

-- Every path that deletes call_presence (leave, sweep, kick, channel delete) records SFU departures here,
-- bumps the room's track list version, and wakes the backends.
CREATE FUNCTION record_call_sfu_departures() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE room record;
BEGIN
  INSERT INTO call_sfu_departures (channel_id, session_id, sfu_id, sfu_room, cf_session_id, livekit_identity)
  SELECT channel_id, session_id, sfu_id, sfu_room, cf_session_id, livekit_identity
  FROM departed_presence WHERE sfu_id IS NOT NULL;
  IF FOUND THEN
    FOR room IN
      UPDATE call_rooms r SET track_version = r.track_version + 1
      FROM (SELECT DISTINCT channel_id FROM departed_presence WHERE sfu_id IS NOT NULL) d
      WHERE r.channel_id = d.channel_id
      RETURNING r.channel_id, r.epoch, r.track_version
    LOOP
      PERFORM pg_notify('zoo_call_events', json_build_object(
        'type', 'sfu_tracks_changed', 'channel_id', room.channel_id,
        'epoch', room.epoch, 'version', room.track_version)::text);
    END LOOP;
    PERFORM pg_notify('zoo_call_events', json_build_object('type', 'sfu_departed')::text);
  END IF;
  RETURN NULL;
END;
$$;
CREATE TRIGGER call_presence_after_delete AFTER DELETE ON call_presence
  REFERENCING OLD TABLE AS departed_presence
  FOR EACH STATEMENT EXECUTE FUNCTION record_call_sfu_departures();
