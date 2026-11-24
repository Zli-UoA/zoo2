-- bridge が push ごとに付ける ID。close は、この ID の行だけを一覧から外す（同じ話者を push し直した新しい行を、
-- 遅れて届いた前の close が外さないように）。NULL 許容の列の追加なので、適用前のバックエンドが動いたままでも壊れない。
ALTER TABLE call_sfu_tracks ADD COLUMN bridge_publish_id text;
